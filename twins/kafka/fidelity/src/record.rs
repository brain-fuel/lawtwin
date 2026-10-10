//! The recorder: a Kafka protocol proxy in front of a real broker.
//!
//! Clients connect to the recorder as to a broker. Each request is
//! forwarded to the real broker and each response back, and every exchange
//! is kept as a frame and decoded into the history: the contract's
//! operations with the real broker's answers (ref:DEC-recorded-history).
//! As it goes, each operation is run on the reference model and the
//! broker's answer compared with the model's, up to a renaming of the
//! identifiers the broker chooses, and the contract's laws are checked over
//! the history.
//!
//! So that a trace replays through a twin, the recorder offers clients only
//! the requests and versions the twin answers, turns fetch sessions off,
//! and tells clients its own address for the broker
//! (ref:DEC-recorder-negotiates-twin-versions). It serves LawTwin's control
//! plane on an admin port, so the tests written for a twin run against the
//! real broker through it: the declared faults are applied on the wire, a
//! forced state is reached by sending the real broker the contract's
//! operations, and the history, the state and the laws are the recorder's
//! (ref:DEC-recorder-control-plane).

use crate::compare::{self, Divergence, Renaming};
use crate::decode::{Decoder, Item};
use crate::frame::{Frame, batches, frames_text};
use anyhow::{Context, Result, anyhow, bail};
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use bytes::{Bytes, BytesMut};
use kafka_protocol::messages::*;
use kafka_protocol::protocol::{Decodable, Encodable, Request, StrBytes};
use kafka_protocol::records::{Compression, RecordBatchEncoder, RecordEncodeOptions};
use lawtwin_kafka::Twin;
use lawtwin_kafka::clock::ClockMode;
use lawtwin_kafka::faults::Fault;
use lawtwin_kafka::force::ForceDoc;
use lawtwin_kafka::history::{Entry, Event, Op, Source};
use lawtwin_kafka::laws::{Violation, check_history};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

/// How a recorder is started.
#[derive(Clone, Debug)]
pub struct RecorderConfig {
    /// The real broker's address.
    pub upstream: String,
    /// Where clients connect.
    pub listen: String,
    /// Where the control plane listens.
    pub admin_listen: String,
    /// The address clients are told for the broker; the listener's by default.
    pub advertise: Option<String>,
    /// How the recorder's clock moves: faults raised for a time end on it.
    pub clock: ClockMode,
    /// Write the trace as it is recorded: `<prefix>.history.jsonl` and
    /// `<prefix>.frames.jsonl`.
    pub out: Option<PathBuf>,
    /// Say each divergence and broken law on standard error as it is found.
    pub verbose: bool,
}

impl Default for RecorderConfig {
    fn default() -> Self {
        Self {
            upstream: "127.0.0.1:9092".into(),
            listen: "127.0.0.1:0".into(),
            admin_listen: "127.0.0.1:0".into(),
            advertise: None,
            clock: ClockMode::Wall,
            out: None,
            verbose: false,
        }
    }
}

/// What the recorder has found so far.
#[derive(Default)]
struct Found {
    decoder: Decoder,
    renaming: Renaming,
    frames: Vec<Frame>,
    divergences: Vec<Divergence>,
    violations: Vec<Violation>,
    notes: BTreeMap<String, u64>,
    checked: usize,
    /// For each group a rebalance storm is waiting on: the generation it
    /// told the leader to leave.
    storm_waiting: BTreeMap<String, i32>,
}

/// A running recorder.
pub struct Recorder {
    /// The config it runs with.
    pub config: RecorderConfig,
    /// The shadow twin: its history is the recorded one, its model the
    /// reference model run over it, its faults and clock the control
    /// plane's.
    pub shadow: Arc<Twin>,
    found: Mutex<Found>,
    start: Instant,
    next_request: AtomicU64,
    next_conn: AtomicU64,
    /// Bumped when the broker becomes unavailable, so open connections close.
    epoch: AtomicU64,
    files: Mutex<Option<(std::fs::File, std::fs::File)>>,
    advertised: Mutex<(String, i32)>,
    upstream_node: Mutex<Option<i32>>,
}

/// Where a running recorder listens.
pub struct Running {
    /// The recorder.
    pub recorder: Arc<Recorder>,
    /// Its Kafka listener.
    pub kafka: SocketAddr,
    /// Its control plane.
    pub admin: SocketAddr,
}

fn wall_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn host_port(text: &str) -> (String, i32) {
    match text.rsplit_once(':') {
        Some((h, p)) => {
            let h = h.trim_matches(['[', ']']);
            let h = if h == "0.0.0.0" || h == "::" { "localhost" } else { h };
            (h.to_string(), p.parse().unwrap_or(9092))
        }
        None => (text.to_string(), 9092),
    }
}

/// Starts a recorder on the current runtime.
pub async fn start(config: RecorderConfig) -> Result<Running> {
    let kafka_listener = TcpListener::bind(&config.listen).await.with_context(|| format!("listen on {}", config.listen))?;
    let admin_listener =
        TcpListener::bind(&config.admin_listen).await.with_context(|| format!("listen on {}", config.admin_listen))?;
    let kafka = kafka_listener.local_addr()?;
    let admin = admin_listener.local_addr()?;
    let advertised = host_port(&config.advertise.clone().unwrap_or_else(|| kafka.to_string()));
    let shadow = Twin::new(lawtwin_kafka::Config { clock: config.clock, advertised: Some(format!("{}:{}", advertised.0, advertised.1)), ..Default::default() });
    let files = match &config.out {
        Some(prefix) => {
            if let Some(dir) = prefix.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir)?;
            }
            let h = std::fs::File::create(with_suffix(prefix, "history.jsonl"))?;
            let f = std::fs::File::create(with_suffix(prefix, "frames.jsonl"))?;
            Some((h, f))
        }
        None => None,
    };
    let recorder = Arc::new(Recorder {
        config,
        shadow,
        found: Mutex::new(Found::default()),
        start: Instant::now(),
        next_request: AtomicU64::new(1),
        next_conn: AtomicU64::new(1),
        epoch: AtomicU64::new(0),
        files: Mutex::new(files),
        advertised: Mutex::new(advertised),
        upstream_node: Mutex::new(None),
    });
    tokio::spawn(accept(recorder.clone(), kafka_listener));
    let router = control_plane(recorder.clone());
    tokio::spawn(async move {
        let _ = axum::serve(admin_listener, router).await;
    });
    // The clock, faults that end and the laws, as the recording goes.
    let ticker = recorder.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            {
                let mut core = ticker.shadow.lock();
                if core.clock.mode() == ClockMode::Wall {
                    core.expire_faults();
                }
            }
            ticker.check_laws();
        }
    });
    Ok(Running { recorder, kafka, admin })
}

/// `<prefix>.<suffix>`.
pub fn with_suffix(prefix: &std::path::Path, suffix: &str) -> PathBuf {
    let mut s = prefix.as_os_str().to_owned();
    s.push(".");
    s.push(suffix);
    PathBuf::from(s)
}

async fn accept(rec: Arc<Recorder>, listener: TcpListener) {
    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
        if rec.shadow.lock().unavailable() {
            drop(stream);
            continue;
        }
        let rec = rec.clone();
        tokio::spawn(async move {
            let _ = stream.set_nodelay(true);
            let conn = rec.next_conn.fetch_add(1, Ordering::SeqCst);
            if let Err(e) = connection(rec, stream, conn).await {
                if std::env::var_os("LAWTWIN_DEBUG").is_some() {
                    eprintln!("lawtwin record: connection {conn}: {e:#}");
                }
            }
        });
    }
}

/// A request waiting for its response, in the order it was sent.
struct Pending {
    n: u64,
    key: i16,
    version: i16,
    request: Vec<u8>,
    sent_us: u64,
    /// An answer the recorder gave itself, for a fault.
    local: Option<Vec<u8>>,
    /// Whether the broker answers it: not a produce with `acks=0`.
    answered: bool,
}

async fn read_frame<R: AsyncReadExt + Unpin>(r: &mut R) -> Result<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let len = i32::from_be_bytes(len);
    if !(0..=104_857_600).contains(&len) {
        bail!("a frame of {len} bytes");
    }
    let mut frame = vec![0u8; len as usize];
    r.read_exact(&mut frame).await?;
    Ok(frame)
}

async fn write_frame<W: AsyncWriteExt + Unpin>(w: &mut W, body: &[u8]) -> Result<()> {
    let mut out = Vec::with_capacity(body.len() + 4);
    out.extend_from_slice(&(body.len() as i32).to_be_bytes());
    out.extend_from_slice(body);
    w.write_all(&out).await?;
    Ok(())
}

fn api_name(key: i16) -> String {
    ApiKey::try_from(key).map(|k| format!("{k:?}")).unwrap_or_else(|_| format!("Api{key}"))
}

async fn connection(rec: Arc<Recorder>, client: TcpStream, conn: u64) -> Result<()> {
    let upstream = TcpStream::connect(&rec.config.upstream).await.with_context(|| format!("connect to {}", rec.config.upstream))?;
    let _ = upstream.set_nodelay(true);
    let (mut cr, mut cw) = client.into_split();
    let (mut ur, mut uw) = upstream.into_split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Pending>();
    let epoch = rec.epoch.load(Ordering::SeqCst);
    let sender = rec.clone();
    let requests = tokio::spawn(async move {
        loop {
            let raw = read_frame(&mut cr).await?;
            if sender.shadow.lock().unavailable() || sender.epoch.load(Ordering::SeqCst) != epoch {
                bail!("the broker is unavailable");
            }
            let (header, body) = crate::decode::split_request(&raw)?;
            let key = header.request_api_key;
            let version = header.request_api_version;
            let n = sender.next_request.fetch_add(1, Ordering::SeqCst);
            let sent_us = sender.start.elapsed().as_micros() as u64;
            if let Some(local) = sender.not_leader_answer(key, version, &header, &body) {
                tx.send(Pending { n, key, version, request: raw, sent_us, local: Some(local), answered: true })?;
                continue;
            }
            let forwarded = rewrite_request(key, version, &header, &body).unwrap_or(raw);
            let answered = !(key == ApiKey::Produce as i16 && produce_acks(version, &body) == Some(0));
            write_frame(&mut uw, &forwarded).await?;
            tx.send(Pending { n, key, version, request: forwarded, sent_us, local: None, answered })?;
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    });
    let result: Result<()> = async {
        while let Some(p) = rx.recv().await {
            if let Some(local) = p.local {
                write_frame(&mut cw, &local).await?;
                continue;
            }
            if !p.answered {
                rec.record(Frame {
                    n: p.n,
                    conn,
                    api: api_name(p.key),
                    key: p.key,
                    version: p.version,
                    request: p.request,
                    response: None,
                    delivered: None,
                    sent_us: p.sent_us,
                    answered_us: p.sent_us,
                    source: "wire".into(),
                });
                continue;
            }
            let response = read_frame(&mut ur).await?;
            let answered_us = rec.start.elapsed().as_micros() as u64;
            let mut frame = Frame {
                n: p.n,
                conn,
                api: api_name(p.key),
                key: p.key,
                version: p.version,
                request: p.request,
                response: Some(response),
                delivered: None,
                sent_us: p.sent_us,
                answered_us,
                source: "wire".into(),
            };
            let (client_bytes, delay) = rec.respond(&mut frame);
            rec.record(frame);
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay as u64)).await;
            }
            write_frame(&mut cw, &client_bytes).await?;
        }
        Ok(())
    }
    .await;
    requests.abort();
    result
}

fn produce_acks(version: i16, body: &Bytes) -> Option<i16> {
    let mut b = body.clone();
    ProduceRequest::decode(&mut b, version).ok().map(|r| r.acks)
}

fn encode_request<R: Encodable>(header: &RequestHeader, key: ApiKey, version: i16, req: &R) -> Result<Vec<u8>> {
    let mut buf = BytesMut::new();
    header.encode(&mut buf, key.request_header_version(version))?;
    req.encode(&mut buf, version)?;
    Ok(buf.to_vec())
}

fn encode_response<R: Encodable>(correlation: i32, key: ApiKey, version: i16, resp: &R) -> Result<Vec<u8>> {
    let mut buf = BytesMut::new();
    ResponseHeader::default().with_correlation_id(correlation).encode(&mut buf, key.response_header_version(version))?;
    resp.encode(&mut buf, version)?;
    Ok(buf.to_vec())
}

/// A fetch as the recorder forwards it: without a fetch session, so every
/// fetch names every partition it reads.
fn rewrite_request(key: i16, version: i16, header: &RequestHeader, body: &Bytes) -> Option<Vec<u8>> {
    if key != ApiKey::Fetch as i16 || version < 7 {
        return None;
    }
    let mut b = body.clone();
    let mut req = FetchRequest::decode(&mut b, version).ok()?;
    req.session_id = 0;
    req.session_epoch = -1;
    req.forgotten_topics_data.clear();
    encode_request(header, ApiKey::Fetch, version, &req).ok()
}

impl Recorder {
    /// The address clients are told for the broker.
    pub fn advertised(&self) -> (String, i32) {
        self.advertised.lock().unwrap().clone()
    }

    /// The control plane's description of the recorder.
    pub fn describe(&self) -> Value {
        let (host, port) = self.advertised();
        let core = self.shadow.lock();
        json!({
            "twin": "kafka",
            "recorder": true,
            "upstream": self.config.upstream,
            "bootstrap": format!("{host}:{port}"),
            "contract": { "package": "lawtwin.kafka", "version": "0.1.0", "ability": "Broker", "handler": "cluster" },
            "clock": { "mode": core.clock.mode(), "nowMs": core.clock.now_ms() },
            "historyLength": core.history.len(),
        })
    }

    /// A NOT_LEADER_OR_FOLLOWER answer the recorder gives itself, when the
    /// leader-change fault covers every partition of a produce, fetch or
    /// list-offsets request.
    fn not_leader_answer(&self, key: i16, version: i16, header: &RequestHeader, body: &Bytes) -> Option<Vec<u8>> {
        let api = ApiKey::try_from(key).ok()?;
        if !matches!(api, ApiKey::Produce | ApiKey::Fetch | ApiKey::ListOffsets) {
            return None;
        }
        let mut core = self.shadow.lock();
        if core.not_leader.is_empty() {
            return None;
        }
        let mut b = body.clone();
        let tps: Vec<(String, i32)> = match api {
            ApiKey::Produce => {
                let r = ProduceRequest::decode(&mut b, version).ok()?;
                r.topic_data.iter().flat_map(|t| t.partition_data.iter().map(|p| (t.name.0.to_string(), p.index))).collect()
            }
            ApiKey::Fetch => {
                let r = FetchRequest::decode(&mut b, version).ok()?;
                r.topics.iter().flat_map(|t| t.partitions.iter().map(|p| (t.topic.0.to_string(), p.partition))).collect()
            }
            _ => {
                let r = ListOffsetsRequest::decode(&mut b, version).ok()?;
                r.topics.iter().flat_map(|t| t.partitions.iter().map(|p| (t.name.0.to_string(), p.partition_index))).collect()
            }
        };
        if tps.is_empty() || !tps.iter().all(|tp| core.not_leader.contains(tp)) {
            return None;
        }
        for (t, p) in &tps {
            core.take_not_leader(t, *p);
        }
        core.control(Source::Fault, "fault applied", json!({ "name": "leader-change", "api": format!("{api:?}"), "partitions": tps.iter().map(|(t, p)| json!({"topic": t, "partition": p})).collect::<Vec<_>>() }));
        drop(core);
        let corr = header.correlation_id;
        let mut topics: BTreeMap<String, Vec<i32>> = BTreeMap::new();
        for (t, p) in tps {
            topics.entry(t).or_default().push(p);
        }
        match api {
            ApiKey::Produce => {
                let r = ProduceResponse::default().with_responses(
                    topics
                        .into_iter()
                        .map(|(t, ps)| {
                            produce_response::TopicProduceResponse::default().with_name(TopicName(StrBytes::from_string(t))).with_partition_responses(
                                ps.into_iter()
                                    .map(|p| {
                                        produce_response::PartitionProduceResponse::default()
                                            .with_index(p)
                                            .with_error_code(6)
                                            .with_base_offset(-1)
                                            .with_log_append_time_ms(-1)
                                    })
                                    .collect(),
                            )
                        })
                        .collect(),
                );
                encode_response(corr, api, version, &r).ok()
            }
            ApiKey::Fetch => {
                let r = FetchResponse::default().with_responses(
                    topics
                        .into_iter()
                        .map(|(t, ps)| {
                            fetch_response::FetchableTopicResponse::default().with_topic(TopicName(StrBytes::from_string(t))).with_partitions(
                                ps.into_iter()
                                    .map(|p| {
                                        fetch_response::PartitionData::default()
                                            .with_partition_index(p)
                                            .with_error_code(6)
                                            .with_high_watermark(-1)
                                            .with_last_stable_offset(-1)
                                            .with_preferred_read_replica(BrokerId(-1))
                                    })
                                    .collect(),
                            )
                        })
                        .collect(),
                );
                encode_response(corr, api, version, &r).ok()
            }
            _ => {
                let r = ListOffsetsResponse::default().with_topics(
                    topics
                        .into_iter()
                        .map(|(t, ps)| {
                            list_offsets_response::ListOffsetsTopicResponse::default().with_name(TopicName(StrBytes::from_string(t))).with_partitions(
                                ps.into_iter()
                                    .map(|p| {
                                        list_offsets_response::ListOffsetsPartitionResponse::default()
                                            .with_partition_index(p)
                                            .with_error_code(6)
                                            .with_timestamp(-1)
                                            .with_offset(-1)
                                    })
                                    .collect(),
                            )
                        })
                        .collect(),
                );
                encode_response(corr, api, version, &r).ok()
            }
        }
    }

    /// What the client gets for a response, and how long it is held back:
    /// the broker's response with the recorder's address and versions, and
    /// any fault the control plane raised applied to it.
    fn respond(&self, frame: &mut Frame) -> (Vec<u8>, i64) {
        let raw = frame.response.clone().unwrap_or_default();
        let Ok(key) = ApiKey::try_from(frame.key) else { return (raw, 0) };
        let version = frame.version;
        let delay = self.shadow.lock().latency_ms(lawtwin_kafka::wire::api_name(key));
        let Ok((header, body)) = crate::decode::split_response(&raw, key, version) else { return (raw, delay) };
        let corr = header.correlation_id;
        let rewritten: Option<Vec<u8>> = match key {
            ApiKey::ApiVersions => self.api_versions(corr, version, body.clone()),
            ApiKey::Metadata => self.metadata(corr, version, body.clone()),
            ApiKey::FindCoordinator => self.find_coordinator(corr, version, body.clone()),
            _ => None,
        };
        if let Some(r) = rewritten {
            // The address and the versions are the recorder's own: not a
            // fault, and not part of the broker's answer.
            return (r, delay);
        }
        let mut extra = 0;
        let faulted: Option<Vec<u8>> = match key {
            ApiKey::Produce => self.duplicate_delivery(corr, version, body),
            ApiKey::Fetch => {
                let (r, d) = self.consumer_lag(corr, version, body, &frame.request);
                extra = d;
                r
            }
            ApiKey::Heartbeat => self.rebalance_storm(corr, version, body, &frame.request),
            _ => None,
        };
        match faulted {
            Some(r) => {
                frame.delivered = Some(r.clone());
                (r, delay + extra)
            }
            None => (raw, delay + extra),
        }
    }

    fn api_versions(&self, corr: i32, version: i16, mut body: Bytes) -> Option<Vec<u8>> {
        let decoded = ApiVersionsResponse::decode(&mut body.clone(), version);
        let (mut r, v) = match decoded {
            Ok(r) if r.error_code != 35 => (r, version),
            _ => (ApiVersionsResponse::decode(&mut body, 0).ok()?, 0),
        };
        r.api_keys = r
            .api_keys
            .into_iter()
            .filter_map(|k| {
                let (_, lo, hi) = lawtwin_kafka::wire::SUPPORTED.iter().find(|(s, _, _)| *s as i16 == k.api_key)?;
                let min = k.min_version.max(*lo);
                let max = k.max_version.min(*hi);
                (min <= max).then(|| k.with_min_version(min).with_max_version(max))
            })
            .collect();
        r.supported_features.clear();
        r.finalized_features.clear();
        r.finalized_features_epoch = -1;
        encode_response(corr, ApiKey::ApiVersions, v, &r).ok()
    }

    fn metadata(&self, corr: i32, version: i16, mut body: Bytes) -> Option<Vec<u8>> {
        let mut r = MetadataResponse::decode(&mut body, version).ok()?;
        let (host, port) = self.advertised();
        if r.brokers.len() > 1 && std::env::var_os("LAWTWIN_DEBUG").is_some() {
            eprintln!("lawtwin record: the upstream cluster has {} brokers; the recorder speaks for one", r.brokers.len());
        }
        let node = r.brokers.first().map(|b| b.node_id.0).unwrap_or(1);
        *self.upstream_node.lock().unwrap() = Some(node);
        r.brokers = vec![metadata_response::MetadataResponseBroker::default().with_node_id(BrokerId(node)).with_host(StrBytes::from_string(host)).with_port(port)];
        encode_response(corr, ApiKey::Metadata, version, &r).ok()
    }

    fn find_coordinator(&self, corr: i32, version: i16, mut body: Bytes) -> Option<Vec<u8>> {
        let mut r = FindCoordinatorResponse::decode(&mut body, version).ok()?;
        let (host, port) = self.advertised();
        if version >= 4 {
            for c in r.coordinators.iter_mut() {
                if c.error_code == 0 {
                    c.host = StrBytes::from_string(host.clone());
                    c.port = port;
                }
            }
        } else if r.error_code == 0 {
            r.host = StrBytes::from_string(host);
            r.port = port;
        }
        encode_response(corr, ApiKey::FindCoordinator, version, &r).ok()
    }

    fn duplicate_delivery(&self, corr: i32, version: i16, mut body: Bytes) -> Option<Vec<u8>> {
        let mut core = self.shadow.lock();
        if !core.faults.iter().any(|a| matches!(a.fault, Fault::DuplicateDelivery { .. })) {
            return None;
        }
        let mut r = ProduceResponse::decode(&mut body, version).ok()?;
        let mut changed = vec![];
        for t in r.responses.iter_mut() {
            for p in t.partition_responses.iter_mut() {
                if p.error_code == 0 && core.take_duplicate(t.name.0.as_str()) {
                    changed.push(json!({ "topic": t.name.0.to_string(), "partition": p.index, "offset": p.base_offset }));
                    p.error_code = 7;
                    p.base_offset = -1;
                }
            }
        }
        if changed.is_empty() {
            return None;
        }
        core.control(Source::Fault, "fault applied", json!({ "name": "duplicate-delivery", "written": changed }));
        encode_response(corr, ApiKey::Produce, version, &r).ok()
    }

    fn consumer_lag(&self, corr: i32, version: i16, mut body: Bytes, request: &[u8]) -> (Option<Vec<u8>>, i64) {
        let mut core = self.shadow.lock();
        if !core.faults.iter().any(|a| matches!(a.fault, Fault::ConsumerLag { .. })) {
            return (None, 0);
        }
        let Ok(mut r) = FetchResponse::decode(&mut body, version) else { return (None, 0) };
        let offsets: BTreeMap<(String, i32), i64> = crate::decode::split_request(request)
            .ok()
            .and_then(|(_, mut b)| FetchRequest::decode(&mut b, version).ok())
            .map(|q| q.topics.iter().flat_map(|t| t.partitions.iter().map(|p| ((t.topic.0.to_string(), p.partition), p.fetch_offset))).collect())
            .unwrap_or_default();
        let mut delay = 0;
        let mut changed = false;
        for t in r.responses.iter_mut() {
            let topic = t.topic.0.to_string();
            let Some((limit, d)) = core.lag(&topic) else { continue };
            delay = delay.max(d);
            for p in t.partitions.iter_mut() {
                let Some(records) = &p.records else { continue };
                let from = offsets.get(&(topic.clone(), p.partition_index)).copied().unwrap_or(0);
                if let Some(cut) = cut_records(records, from, limit) {
                    p.records = Some(cut);
                    changed = true;
                }
            }
        }
        if changed {
            core.control(Source::Fault, "fault applied", json!({ "name": "consumer-lag" }));
        }
        drop(core);
        (if changed { encode_response(corr, ApiKey::Fetch, version, &r).ok() } else { None }, delay)
    }

    fn rebalance_storm(&self, corr: i32, version: i16, mut body: Bytes, request: &[u8]) -> Option<Vec<u8>> {
        let mut core = self.shadow.lock();
        let storms: Vec<Option<String>> = core
            .faults
            .iter()
            .filter_map(|a| match &a.fault {
                Fault::RebalanceStorm { rounds, group } if *rounds > 0 => Some(group.clone()),
                _ => None,
            })
            .collect();
        if storms.is_empty() {
            return None;
        }
        let (_, mut b) = crate::decode::split_request(request).ok()?;
        let req = HeartbeatRequest::decode(&mut b, version).ok()?;
        let group = req.group_id.0.to_string();
        if !storms.iter().any(|g| g.as_deref().is_none_or(|g| g == group)) {
            return None;
        }
        let mut found = self.found.lock().unwrap();
        if found.storm_waiting.contains_key(&group) || found.decoder.leader(&group) != Some(req.member_id.as_str()) {
            return None;
        }
        let mut r = HeartbeatResponse::decode(&mut body, version).ok()?;
        if r.error_code != 0 {
            return None;
        }
        r.error_code = 27;
        let generation = found.decoder.generation(&group).unwrap_or(req.generation_id);
        found.storm_waiting.insert(group.clone(), generation);
        core.control(Source::Fault, "fault applied", json!({ "name": "rebalance-storm", "group": group, "member": req.member_id.to_string(), "generation": generation }));
        encode_response(corr, ApiKey::Heartbeat, version, &r).ok()
    }

    /// Keeps a frame and writes what it decodes to into the history,
    /// comparing each operation's answer with the reference model's.
    pub fn record(&self, frame: Frame) {
        let mut found = self.found.lock().unwrap();
        let items = found.decoder.frame(&frame);
        let force = frame.source == "force";
        let mut core = self.shadow.lock();
        let mut written = vec![];
        for item in items {
            let seq = core.history.len() as u64 + 1;
            let entry = match item {
                Item::Call { source, op, answer } => {
                    let source = if force && source == Source::Wire { Source::Force } else { source };
                    let renaming = &mut found.renaming;
                    if let Some(d) = compare::step(&mut core.model, renaming, seq, &op, &answer) {
                        if self.config.verbose {
                            eprintln!("lawtwin record: divergence at {seq}: {} answered {} by the broker, {} by the contract", d.op, d.recorded, d.model);
                        }
                        found.divergences.push(d);
                    }
                    Entry { seq, at: wall_ms(), source, event: Event::Call { op, answer } }
                }
                Item::Note { source, control, detail } => {
                    *found.notes.entry(note_kind(&control, &detail)).or_default() += 1;
                    Entry { seq, at: wall_ms(), source, event: Event::Control { control, detail } }
                }
            };
            core.history.push(entry.clone());
            written.push(entry);
        }
        // A storm's round is over when its group reaches a new generation.
        let waiting: Vec<(String, i32)> = found.storm_waiting.iter().map(|(g, n)| (g.clone(), *n)).collect();
        for (group, generation) in waiting {
            if found.decoder.generation(&group).is_some_and(|now| now > generation) {
                found.storm_waiting.remove(&group);
                for a in core.faults.iter_mut() {
                    if let Fault::RebalanceStorm { rounds, group: only } = &mut a.fault {
                        if *rounds > 0 && only.as_deref().is_none_or(|g| g == group) {
                            *rounds -= 1;
                            break;
                        }
                    }
                }
                core.faults.retain(|a| !matches!(a.fault, Fault::RebalanceStorm { rounds: 0, .. }));
            }
        }
        drop(core);
        if let Some((h, f)) = self.files.lock().unwrap().as_mut() {
            for e in &written {
                let _ = writeln!(h, "{}", serde_json::to_string(e).unwrap_or_default());
            }
            let _ = write!(f, "{}", frames_text(std::slice::from_ref(&frame)));
        }
        found.frames.push(frame);
    }

    /// Checks the laws over the history recorded so far, saying each newly
    /// broken one.
    pub fn check_laws(&self) {
        let entries = {
            let core = self.shadow.lock();
            let found = self.found.lock().unwrap();
            if core.history.len() == found.checked {
                return;
            }
            core.history.clone()
        };
        let broken = check_history(&entries);
        let mut found = self.found.lock().unwrap();
        for v in broken {
            if !found.violations.contains(&v) {
                if self.config.verbose {
                    eprintln!("lawtwin record: the law `{}` does not hold at {}: {}", v.law, v.seq.unwrap_or(0), v.detail);
                }
                found.violations.push(v);
            }
        }
        found.checked = entries.len();
    }

    /// The frames so far.
    pub fn frames(&self) -> Vec<Frame> {
        self.found.lock().unwrap().frames.clone()
    }

    /// The history so far.
    pub fn history(&self) -> Vec<Entry> {
        self.shadow.lock().history.clone()
    }

    /// Every divergence so far.
    pub fn divergences(&self) -> Vec<Divergence> {
        self.found.lock().unwrap().divergences.clone()
    }

    /// Every broken law so far, checked over the whole history now.
    pub fn violations(&self) -> Vec<Violation> {
        self.check_laws();
        let state = lawtwin_kafka::laws::check_state(&self.shadow.lock().model.snapshot());
        let mut v = self.found.lock().unwrap().violations.clone();
        v.extend(state);
        v
    }

    /// How often each kind of note was written.
    pub fn notes(&self) -> BTreeMap<String, u64> {
        self.found.lock().unwrap().notes.clone()
    }

    /// What the recording found: its size, divergences, broken laws and notes.
    pub fn summary(&self) -> Value {
        let history = self.history();
        let calls = history.iter().filter(|e| matches!(e.event, Event::Call { .. })).count();
        json!({
            "upstream": self.config.upstream,
            "frames": self.found.lock().unwrap().frames.len(),
            "operations": calls,
            "divergences": self.divergences(),
            "violations": self.violations(),
            "notes": self.notes(),
        })
    }

    /// Forces a state on the real broker: the forced state is checked on a
    /// copy of the reference model as a twin checks it, and then reached
    /// by sending the broker the contract's operations, recorded with the
    /// source `force`. Only topics, records and committed offsets of an
    /// empty group can be reached so; anything else is refused.
    pub async fn force(self: &Arc<Self>, doc: ForceDoc) -> std::result::Result<Value, (StatusCode, Value)> {
        let unreachable: Vec<&str> = [
            (!doc.producers.is_empty(), "producers"),
            (!doc.transactions.is_empty(), "transactions"),
            (!doc.groups.is_empty(), "groups"),
            (doc.reset, "reset"),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, n)| *n)
        .collect();
        if !unreachable.is_empty() {
            return Err((
                StatusCode::NOT_IMPLEMENTED,
                json!({ "error": format!("the recorder cannot reach {} on a real broker; force them on a twin", unreachable.join(", ")) }),
            ));
        }
        let ops = {
            let core = self.shadow.lock();
            let mut trial = lawtwin_kafka::twin::Core::new(&lawtwin_kafka::Config::default());
            trial.model = core.model.clone();
            drop(core);
            match lawtwin_kafka::force::force(&mut trial, &doc) {
                Err(r) => return Err((StatusCode::UNPROCESSABLE_ENTITY, json!({ "forced": false, "law": r.law, "reason": r.reason }))),
                Ok(_) => trial
                    .history
                    .into_iter()
                    .filter_map(|e| match e.event {
                        Event::Call { op, .. } => Some(op),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            }
        };
        let mut client = Upstream::connect(&self.config.upstream).await.map_err(|e| (StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() })))?;
        let mut created = BTreeSet::new();
        for op in &ops {
            let result = match op {
                Op::CreateTopicsRequest { name, partitions } => {
                    created.insert(name.clone());
                    let req = CreateTopicsRequest::default().with_timeout_ms(10_000).with_topics(vec![
                        create_topics_request::CreatableTopic::default()
                            .with_name(TopicName(StrBytes::from_string(name.clone())))
                            .with_num_partitions(*partitions)
                            .with_replication_factor(-1),
                    ]);
                    client.send(self, req, 5).await.map(|r: CreateTopicsResponse| r.topics.first().map(|t| t.error_code).unwrap_or(0))
                }
                Op::CreatePartitionsRequest { name, total } => {
                    let req = CreatePartitionsRequest::default().with_timeout_ms(10_000).with_topics(vec![
                        create_partitions_request::CreatePartitionsTopic::default()
                            .with_name(TopicName(StrBytes::from_string(name.clone())))
                            .with_count(*total),
                    ]);
                    client.send(self, req, 1).await.map(|r: CreatePartitionsResponse| r.results.first().map(|t| t.error_code).unwrap_or(0))
                }
                Op::ProduceRequest { tp, tag: None, records } => {
                    self.wait_for_leaders(&mut client, &created).await;
                    created.clear();
                    let batch = encode_batch(records);
                    let req = ProduceRequest::default().with_acks(-1).with_timeout_ms(10_000).with_topic_data(vec![
                        produce_request::TopicProduceData::default().with_name(TopicName(StrBytes::from_string(tp.topic.clone()))).with_partition_data(vec![
                            produce_request::PartitionProduceData::default().with_index(tp.partition).with_records(Some(batch)),
                        ]),
                    ]);
                    client
                        .send(self, req, 9)
                        .await
                        .map(|r: ProduceResponse| r.responses.first().and_then(|t| t.partition_responses.first()).map(|p| p.error_code).unwrap_or(0))
                }
                Op::OffsetCommitRequest { group, member, generation, offsets } if member.is_empty() && *generation == -1 => {
                    self.wait_for_leaders(&mut client, &created).await;
                    created.clear();
                    let mut topics: BTreeMap<String, Vec<(i32, i64)>> = BTreeMap::new();
                    for o in offsets {
                        topics.entry(o.topic.clone()).or_default().push((o.partition, o.offset));
                    }
                    let req = OffsetCommitRequest::default()
                        .with_group_id(GroupId(StrBytes::from_string(group.clone())))
                        .with_generation_id_or_member_epoch(-1)
                        .with_member_id(StrBytes::from_static_str(""))
                        .with_topics(
                            topics
                                .into_iter()
                                .map(|(t, ps)| {
                                    offset_commit_request::OffsetCommitRequestTopic::default().with_name(TopicName(StrBytes::from_string(t))).with_partitions(
                                        ps.into_iter()
                                            .map(|(p, o)| offset_commit_request::OffsetCommitRequestPartition::default().with_partition_index(p).with_committed_offset(o))
                                            .collect(),
                                    )
                                })
                                .collect(),
                        );
                    let mut result = Ok(14);
                    for _ in 0..50 {
                        // The group coordinator may still be loading.
                        result = client.send(self, req.clone(), 7).await.map(|r: OffsetCommitResponse| {
                            r.topics.iter().flat_map(|t| t.partitions.iter().map(|p| p.error_code)).find(|c| *c != 0).unwrap_or(0)
                        });
                        if !matches!(result, Ok(14..=16)) {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    result
                }
                Op::DescribeGroupRequest { .. } | Op::MetadataRequest { .. } => Ok(0),
                other => {
                    return Err((
                        StatusCode::NOT_IMPLEMENTED,
                        json!({ "error": format!("the recorder cannot send {} for a forced state", other.name()) }),
                    ));
                }
            };
            match result {
                Ok(0) => {}
                Ok(code) => {
                    return Err((
                        StatusCode::BAD_GATEWAY,
                        json!({ "error": format!("the broker answered {} with {}", op.name(), crate::decode::kafka_error_name(code)) }),
                    ));
                }
                Err(e) => return Err((StatusCode::BAD_GATEWAY, json!({ "error": e.to_string() }))),
            }
        }
        self.wait_for_leaders(&mut client, &created).await;
        Ok(json!({ "forced": true, "operations": ops.len() }))
    }

    async fn wait_for_leaders(self: &Arc<Self>, client: &mut Upstream, topics: &BTreeSet<String>) {
        if topics.is_empty() {
            return;
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            let req = MetadataRequest::default().with_topics(Some(
                topics.iter().map(|t| metadata_request::MetadataRequestTopic::default().with_name(Some(TopicName(StrBytes::from_string(t.clone()))))).collect(),
            ));
            let ready = client
                .send(self, req, 9)
                .await
                .map(|r: MetadataResponse| r.topics.iter().all(|t| t.error_code == 0 && t.partitions.iter().all(|p| p.leader_id.0 >= 0)))
                .unwrap_or(false);
            if ready {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

/// A note's kind, as reports count them: what happened, and for a
/// retriable error the request and the error.
pub fn note_kind(control: &str, detail: &Value) -> String {
    match (detail.get("api").and_then(Value::as_str), detail.get("error").and_then(Value::as_str)) {
        (Some(api), Some(error)) => format!("{control}: {api} {error}"),
        _ => control.to_string(),
    }
}

/// A batch of records with no producer id, as a forced state writes it.
pub fn encode_batch(records: &[lawtwin_kafka::history::Record]) -> Bytes {
    let now = wall_ms();
    let wire: Vec<kafka_protocol::records::Record> = records
        .iter()
        .enumerate()
        .map(|(i, r)| kafka_protocol::records::Record {
            transactional: false,
            control: false,
            delete_horizon: false,
            partition_leader_epoch: -1,
            producer_id: -1,
            producer_epoch: -1,
            timestamp_type: kafka_protocol::records::TimestampType::Creation,
            offset: i as i64,
            // No sequence: -1 for the batch, with the offset deltas the
            // encoder needs to keep the records in one batch.
            sequence: i as i32 - 1,
            timestamp: now,
            key: r.key.as_ref().map(|k| Bytes::from(k.0.clone())),
            value: r.value.as_ref().map(|v| Bytes::from(v.0.clone())),
            headers: r.headers.iter().map(|h| (StrBytes::from_string(h.name.clone()), h.content.as_ref().map(|c| Bytes::from(c.0.clone())))).collect(),
        })
        .collect();
    let mut buf = BytesMut::new();
    let _ = RecordBatchEncoder::encode(&mut buf, wire.iter(), &RecordEncodeOptions { version: 2, compression: Compression::None });
    buf.freeze()
}

/// The first `limit` records at or after `from`, as whole batches and one
/// cut short; none when the records are within the limit already.
pub fn cut_records(records: &Bytes, from: i64, limit: i32) -> Option<Bytes> {
    let batches = batches(records).ok()?;
    let mut kept = BytesMut::new();
    let mut left = limit.max(1) as usize;
    for b in &batches {
        if left == 0 {
            break;
        }
        let readable = if b.control { 0 } else { b.records.iter().filter(|r| r.offset >= from).count() };
        if readable <= left {
            kept.extend_from_slice(&b.raw);
            left -= readable;
            continue;
        }
        let mut taken = 0;
        let part: Vec<_> = b
            .records
            .iter()
            .take_while(|r| {
                if r.offset < from {
                    return true;
                }
                taken += 1;
                taken <= left
            })
            .cloned()
            .collect();
        RecordBatchEncoder::encode(&mut kept, part.iter(), &RecordEncodeOptions { version: 2, compression: Compression::None }).ok()?;
        left = 0;
    }
    let kept = kept.freeze();
    (kept.len() < records.len()).then_some(kept)
}

/// The recorder's own connection to the broker, for a forced state.
struct Upstream {
    stream: TcpStream,
    correlation: i32,
    conn: u64,
}

impl Upstream {
    async fn connect(addr: &str) -> Result<Self> {
        let stream = TcpStream::connect(addr).await.with_context(|| format!("connect to {addr}"))?;
        Ok(Self { stream, correlation: 0, conn: 0 })
    }

    async fn send<R: Request + Clone, S: Decodable>(&mut self, rec: &Arc<Recorder>, req: R, version: i16) -> Result<S> {
        self.correlation += 1;
        let header = RequestHeader::default()
            .with_request_api_key(R::KEY)
            .with_request_api_version(version)
            .with_correlation_id(self.correlation)
            .with_client_id(Some(StrBytes::from_static_str("lawtwin-force")));
        let key = ApiKey::try_from(R::KEY).map_err(|_| anyhow!("unknown API key"))?;
        let raw = encode_request(&header, key, version, &req)?;
        let n = rec.next_request.fetch_add(1, Ordering::SeqCst);
        let sent_us = rec.start.elapsed().as_micros() as u64;
        write_frame(&mut self.stream, &raw).await?;
        let response = read_frame(&mut self.stream).await?;
        let answered_us = rec.start.elapsed().as_micros() as u64;
        rec.record(Frame {
            n,
            conn: self.conn,
            api: api_name(R::KEY),
            key: R::KEY,
            version,
            request: raw,
            response: Some(response.clone()),
            delivered: None,
            sent_us,
            answered_us,
            source: "force".into(),
        });
        let (_, mut body) = crate::decode::split_response(&response, key, version)?;
        S::decode(&mut body, version)
    }
}

type Shared = State<Arc<Recorder>>;

/// The recorder's control plane: LawTwin's admin API, over the shadow twin,
/// with forcing a state and the description its own.
pub fn control_plane(rec: Arc<Recorder>) -> Router {
    let fallback = lawtwin_kafka::admin::router(rec.shadow.clone());
    Router::new()
        .route("/v1/twin", get(|State(r): Shared| async move { axum::Json(r.describe()) }))
        .route("/v1/recording", get(|State(r): Shared| async move { axum::Json(r.summary()) }))
        .route(
            "/v1/state",
            post(|State(r): Shared, body: String| async move {
                let doc: ForceDoc = match serde_json::from_str(&body) {
                    Ok(d) => d,
                    Err(e) => return (StatusCode::BAD_REQUEST, axum::Json(json!({ "error": format!("not a forced state: {e}") }))).into_response(),
                };
                match r.force(doc).await {
                    Ok(v) => (StatusCode::OK, axum::Json(v)).into_response(),
                    Err((s, v)) => (s, axum::Json(v)).into_response(),
                }
            })
            .get({
                let shadow = rec.shadow.clone();
                move || {
                    let shadow = shadow.clone();
                    async move {
                        let v = tokio::task::spawn_blocking(move || lawtwin_kafka::admin::inspect_state(&shadow, false)).await.unwrap_or_default();
                        axum::Json(v)
                    }
                }
            }),
        )
        .with_state(rec)
        .fallback_service(fallback)
}

/// Runs a recorder until the process is interrupted, then writes its
/// summary.
pub fn run(config: RecorderConfig) -> Result<Value> {
    let runtime = lawtwin_kafka::runtime()?;
    runtime.block_on(async move {
        let running = start(config).await?;
        let (host, port) = running.recorder.advertised();
        eprintln!(
            "lawtwin record: Kafka on {} (advertised {host}:{port}) forwarding to {}, control plane on http://{}",
            running.kafka, running.recorder.config.upstream, running.admin
        );
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
        let summary = running.recorder.summary();
        if let Some(prefix) = &running.recorder.config.out {
            std::fs::write(with_suffix(prefix, "summary.json"), serde_json::to_string_pretty(&summary)? + "\n")?;
        }
        Ok(summary)
    })
}
