//! Simulated peers: the other parties on a topic, played by LawTwin
//! (ref:DEC-simulated-peers).
//!
//! `lawtwin peer kafka produce` is an external producer feeding a topic the
//! application consumes: records valid under the Kafka contract, with
//! values generated from a seed and, given one, a LawSpec type and its
//! refinements, at a rate, with keys, headers, transactions and faults a
//! real producer shows. `lawtwin peer kafka consume` is an external
//! consumer, a member of a group, reading what the application produces and
//! checking it against the contract's consumer-side laws and the value
//! type. Both are real clients (librdkafka), so they work against a twin and
//! against real Kafka alike.

use crate::cli::Args;
use crate::decode::readable;
use crate::frame::batches;
use crate::lawtype::{Format, Ty, Types};
use bytes::Bytes;
use kafka_protocol::messages::*;
use kafka_protocol::protocol::{Decodable, Encodable, HeaderVersion, Request, StrBytes};
use lawtwin_kafka::history::Isolation;
use rdkafka::ClientConfig;
use rdkafka::ClientContext;
use rdkafka::Message;
use rdkafka::consumer::{BaseConsumer, CommitMode, Consumer, ConsumerContext, Rebalance};
use rdkafka::message::{Header, OwnedHeaders};
use rdkafka::producer::{BaseProducer, BaseRecord, DeliveryResult, Producer, ProducerContext};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read as _, Write as _};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The seeded generator a peer draws from: SplitMix64, as every twin core.
#[derive(Clone, Debug)]
pub struct Rng(lawtwin_kafka::clock::Random);

impl Rng {
    /// The generator for a seed.
    pub fn new(seed: u64) -> Self {
        Self(lawtwin_kafka::clock::Random::new(seed))
    }

    /// A number below `n` (0 when `n` is 0).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.0.next_u64() % n }
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        p > 0.0 && (self.0.next_u64() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }
}

/// The header that numbers a peer's records, unless turned off.
pub const SEQ_HEADER: &str = "lawtwin-seq";

/// A fault a producer peer shows, with how often.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PeerFault {
    /// A record sent twice, as a producer without idempotence retries one
    /// whose acknowledgement it lost.
    Duplicate(f64),
    /// A value that is not of the value type: bytes that do not decode.
    Poison(f64),
    /// A record with no value, a tombstone.
    Tombstone(f64),
    /// A transaction aborted instead of committed.
    Abort(f64),
    /// A timestamp in the past, by `--late-by`.
    Late(f64),
    /// The last transaction left open when the peer stops, as a producer
    /// that dies mid-transaction leaves it.
    Hang,
}

impl PeerFault {
    fn parse(text: &str) -> Result<Self, String> {
        let (name, p) = match text.split_once(':') {
            Some((n, p)) => (n, p.parse::<f64>().map_err(|_| format!("{p} is not a probability"))?),
            None => (text, 1.0),
        };
        if !(0.0..=1.0).contains(&p) {
            return Err(format!("{p} is not a probability between 0 and 1"));
        }
        Ok(match name {
            "duplicate" => PeerFault::Duplicate(p),
            "poison" => PeerFault::Poison(p),
            "tombstone" => PeerFault::Tombstone(p),
            "abort" => PeerFault::Abort(p),
            "late" => PeerFault::Late(p),
            "hang" => PeerFault::Hang,
            other => return Err(format!("{other} is not a producer fault: duplicate, poison, tombstone, abort, late or hang")),
        })
    }
}

/// What a producer peer is told to do.
#[derive(Clone, Debug)]
pub struct ProduceOptions {
    /// The broker.
    pub bootstrap: String,
    /// The topic.
    pub topic: String,
    /// How many records; none to run for `duration`.
    pub count: Option<u64>,
    /// How long to run, when no count is given.
    pub duration: Option<Duration>,
    /// Records a second; none for as fast as the broker takes them.
    pub rate: Option<f64>,
    /// The seed.
    pub seed: u64,
    /// How many distinct keys; none for records without keys.
    pub keys: Option<u64>,
    /// Headers every record carries.
    pub headers: Vec<(String, String)>,
    /// Number each record in the `lawtwin-seq` header.
    pub seq_header: bool,
    /// One partition for every record; by default the key's.
    pub partition: Option<i32>,
    /// The value type and the LawSpec declarations it is read from.
    pub value_type: Option<(Types, Ty)>,
    /// How values are encoded.
    pub format: Format,
    /// The transactional id, for records sent in transactions.
    pub transactional_id: Option<String>,
    /// Records in each transaction.
    pub txn_size: u64,
    /// An idempotent producer.
    pub idempotent: bool,
    /// The faults to show.
    pub faults: Vec<PeerFault>,
    /// How far in the past a late record's timestamp is, in milliseconds.
    pub late_by_ms: i64,
    /// Write every record sent, one JSON line each.
    pub log: Option<PathBuf>,
}

impl Default for ProduceOptions {
    fn default() -> Self {
        Self {
            bootstrap: "127.0.0.1:9092".into(),
            topic: String::new(),
            count: Some(10),
            duration: None,
            rate: None,
            seed: 0,
            keys: Some(8),
            headers: vec![],
            seq_header: true,
            partition: None,
            value_type: None,
            format: Format::Json,
            transactional_id: None,
            txn_size: 10,
            idempotent: false,
            faults: vec![],
            late_by_ms: 3_600_000,
            log: None,
        }
    }
}

/// One record a producer peer sent.
#[derive(Clone, Debug, Serialize)]
pub struct Sent {
    /// Its number, from 0.
    pub seq: u64,
    /// Its key.
    pub key: Option<String>,
    /// Its value: the JSON for a JSON value, base64 otherwise.
    pub value: Value,
    /// The faults it shows.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub faults: Vec<String>,
    /// Its transaction, from 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction: Option<u64>,
    /// How its transaction ended: committed, aborted or open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Where it was written, once acknowledged.
    pub partition: Option<i32>,
    /// Its offset, once acknowledged.
    pub offset: Option<i64>,
    /// Why it was not written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Default)]
struct Deliveries(Arc<Mutex<BTreeMap<usize, Result<(i32, i64), String>>>>);

impl ClientContext for Deliveries {}
impl ProducerContext for Deliveries {
    type DeliveryOpaque = Box<usize>;
    fn delivery(&self, result: &DeliveryResult<'_>, opaque: Box<usize>) {
        let r = match result {
            Ok(m) => Ok((m.partition(), m.offset())),
            Err((e, _)) => Err(e.to_string()),
        };
        self.0.lock().unwrap().insert(*opaque, r);
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Checks what the Kafka contract asks of a record a producer sends: a
/// valid topic name, header names, a batch of one record.
fn contract_valid(o: &ProduceOptions) -> Result<(), String> {
    if !lawtwin_kafka::laws::valid_topic_name(&o.topic) {
        return Err(format!("{:?} is not a valid topic name (an invalid topic name is refused)", o.topic));
    }
    if let Some((n, _)) = o.headers.iter().find(|(n, _)| n.is_empty()) {
        return Err(format!("a header needs a name, not {n:?}"));
    }
    if o.transactional_id.as_deref() == Some("") {
        return Err("a transactional id is not empty".into());
    }
    Ok(())
}

/// Runs a producer peer: what it sent, and a summary.
pub fn produce(o: &ProduceOptions) -> Result<(Vec<Sent>, Value), String> {
    contract_valid(o)?;
    let mut config = ClientConfig::new();
    config
        .set("bootstrap.servers", &o.bootstrap)
        .set("client.id", "lawtwin-peer-producer")
        .set("linger.ms", "5")
        .set("message.timeout.ms", "30000");
    if o.idempotent {
        config.set("enable.idempotence", "true");
    }
    if let Some(t) = &o.transactional_id {
        config.set("transactional.id", t);
    }
    let producer: BaseProducer<Deliveries> = config.create_with_context(Deliveries::default()).map_err(|e| e.to_string())?;
    let transactional = o.transactional_id.is_some();
    if transactional {
        producer.init_transactions(Duration::from_secs(30)).map_err(|e| format!("init transactions: {e}"))?;
    }
    let mut rng = Rng::new(o.seed);
    let chance = |faults: &[PeerFault], f: fn(&PeerFault) -> Option<f64>| faults.iter().find_map(f).unwrap_or(0.0);
    let p_dup = chance(&o.faults, |f| if let PeerFault::Duplicate(p) = f { Some(*p) } else { None });
    let p_poison = chance(&o.faults, |f| if let PeerFault::Poison(p) = f { Some(*p) } else { None });
    let p_tomb = chance(&o.faults, |f| if let PeerFault::Tombstone(p) = f { Some(*p) } else { None });
    let p_abort = chance(&o.faults, |f| if let PeerFault::Abort(p) = f { Some(*p) } else { None });
    let p_late = chance(&o.faults, |f| if let PeerFault::Late(p) = f { Some(*p) } else { None });
    let hang = o.faults.contains(&PeerFault::Hang);
    let started = Instant::now();
    let mut sent: Vec<Sent> = vec![];
    let mut txn: Option<u64> = None;
    let mut txn_count = 0u64;
    let mut in_txn = 0u64;
    let mut outcomes: BTreeMap<u64, &str> = BTreeMap::new();
    let mut seq = 0u64;
    let end_txn = |producer: &BaseProducer<Deliveries>, rng: &mut Rng, t: u64, outcomes: &mut BTreeMap<u64, &str>| -> Result<(), String> {
        producer.flush(Duration::from_secs(30)).map_err(|e| e.to_string())?;
        if rng.chance(p_abort) {
            producer.abort_transaction(Duration::from_secs(30)).map_err(|e| format!("abort: {e}"))?;
            outcomes.insert(t, "aborted");
        } else {
            producer.commit_transaction(Duration::from_secs(30)).map_err(|e| format!("commit: {e}"))?;
            outcomes.insert(t, "committed");
        }
        Ok(())
    };
    loop {
        if let Some(n) = o.count {
            if seq >= n {
                break;
            }
        } else if o.duration.is_none_or(|d| started.elapsed() >= d) {
            break;
        }
        if let Some(rate) = o.rate.filter(|r| *r > 0.0) {
            let due = Duration::from_secs_f64(seq as f64 / rate);
            let elapsed = started.elapsed();
            if due > elapsed {
                std::thread::sleep(due - elapsed);
            }
        }
        if transactional && txn.is_none() {
            producer.begin_transaction().map_err(|e| format!("begin: {e}"))?;
            txn = Some(txn_count);
            txn_count += 1;
            in_txn = 0;
        }
        let key = o.keys.map(|k| format!("key-{}", rng.below(k.max(1))));
        let mut faults = vec![];
        let (value_bytes, value_json): (Option<Vec<u8>>, Value) = if rng.chance(p_tomb) {
            faults.push("tombstone".to_string());
            (None, Value::Null)
        } else if rng.chance(p_poison) {
            faults.push("poison".to_string());
            let junk: Vec<u8> = (0..8).map(|_| 0x80 | rng.below(64) as u8).collect();
            let shown = json!({ "base64": base64_of(&junk) });
            (Some(junk), shown)
        } else {
            match &o.value_type {
                Some((types, ty)) => {
                    let v = types.generate(ty, &mut rng)?;
                    let bytes = types.encode(ty, &v, o.format);
                    let shown = match o.format {
                        Format::Json => types.to_json(ty, &v),
                        Format::LawSpec => json!({ "base64": base64_of(&bytes) }),
                    };
                    (Some(bytes), shown)
                }
                None => {
                    let text = format!("value-{seq}-{:08x}", rng.below(u32::MAX as u64));
                    (Some(text.clone().into_bytes()), Value::String(text))
                }
            }
        };
        let late = rng.chance(p_late);
        if late {
            faults.push("late".to_string());
        }
        let copies = if rng.chance(p_dup) {
            faults.push("duplicate".to_string());
            2
        } else {
            1
        };
        let mut headers = OwnedHeaders::new();
        for (n, v) in &o.headers {
            headers = headers.insert(Header { key: n, value: Some(v.as_bytes()) });
        }
        let seq_text = seq.to_string();
        if o.seq_header {
            headers = headers.insert(Header { key: SEQ_HEADER, value: Some(seq_text.as_bytes()) });
        }
        let index = sent.len();
        for copy in 0..copies {
            let mut record: BaseRecord<'_, String, Vec<u8>, Box<usize>> =
                BaseRecord::with_opaque_to(&o.topic, Box::new(index + copy)).headers(headers.clone());
            if let Some(k) = &key {
                record = record.key(k);
            }
            if let Some(v) = &value_bytes {
                record = record.payload(v);
            }
            if let Some(p) = o.partition {
                record = record.partition(p);
            }
            if late {
                record = record.timestamp(now_ms() - o.late_by_ms);
            }
            let mut record = record;
            loop {
                match producer.send(record) {
                    Ok(()) => break,
                    Err((e, r)) if e.to_string().contains("Queue full") => {
                        producer.poll(Duration::from_millis(50));
                        record = r;
                    }
                    Err((e, _)) => return Err(format!("send: {e}")),
                }
            }
            sent.push(Sent {
                seq,
                key: key.clone(),
                value: value_json.clone(),
                faults: faults.clone(),
                transaction: txn,
                outcome: None,
                partition: None,
                offset: None,
                error: None,
            });
        }
        producer.poll(Duration::ZERO);
        seq += 1;
        in_txn += 1;
        if let Some(t) = txn {
            if in_txn >= o.txn_size.max(1) {
                end_txn(&producer, &mut rng, t, &mut outcomes)?;
                txn = None;
            }
        }
    }
    if let Some(t) = txn {
        if hang {
            producer.flush(Duration::from_secs(30)).map_err(|e| e.to_string())?;
            outcomes.insert(t, "open");
        } else {
            end_txn(&producer, &mut rng, t, &mut outcomes)?;
        }
    }
    producer.flush(Duration::from_secs(30)).map_err(|e| format!("flush: {e}"))?;
    let deliveries = producer.context().0.lock().unwrap().clone();
    for (i, s) in sent.iter_mut().enumerate() {
        match deliveries.get(&i) {
            Some(Ok((p, off))) => {
                s.partition = Some(*p);
                s.offset = Some(*off);
            }
            Some(Err(e)) => s.error = Some(e.clone()),
            None => s.error = Some("no acknowledgement".into()),
        }
        s.outcome = s.transaction.and_then(|t| outcomes.get(&t)).map(|o| o.to_string());
    }
    if hang {
        // Leave without ending the transaction, as a dead producer would.
        std::mem::forget(producer);
    }
    if let Some(path) = &o.log {
        let mut text = String::new();
        for s in &sent {
            text.push_str(&serde_json::to_string(s).unwrap_or_default());
            text.push('\n');
        }
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let mut fault_counts: BTreeMap<String, u64> = BTreeMap::new();
    for s in &sent {
        for f in &s.faults {
            *fault_counts.entry(f.clone()).or_default() += 1;
        }
    }
    let summary = json!({
        "peer": "produce",
        "topic": o.topic,
        "seed": o.seed,
        "records": seq,
        "sent": sent.len(),
        "written": sent.iter().filter(|s| s.offset.is_some()).count(),
        "failed": sent.iter().filter(|s| s.error.is_some()).count(),
        "transactions": {
            "committed": outcomes.values().filter(|o| **o == "committed").count(),
            "aborted": outcomes.values().filter(|o| **o == "aborted").count(),
            "open": outcomes.values().filter(|o| **o == "open").count(),
        },
        "faults": fault_counts,
        "millis": started.elapsed().as_millis() as u64,
    });
    Ok((sent, summary))
}

fn base64_of(b: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(b)
}

// ---------------------------------------------------------------------------
// The consumer peer
// ---------------------------------------------------------------------------

/// What a consumer peer is told to do.
#[derive(Clone, Debug)]
pub struct ConsumeOptions {
    /// The broker.
    pub bootstrap: String,
    /// The topic.
    pub topic: String,
    /// The group it joins.
    pub group: String,
    /// Read committed or uncommitted.
    pub isolation: Isolation,
    /// The value type and the LawSpec declarations it is read from.
    pub value_type: Option<(Types, Ty)>,
    /// How values are encoded.
    pub format: Format,
    /// Stop after this many records.
    pub expect: Option<u64>,
    /// Stop after this long in all.
    pub duration: Duration,
    /// Stop after no record for this long, once any expected count is read.
    pub idle: Duration,
    /// Where a group with no committed offset starts: `earliest` or `latest`.
    pub from: String,
    /// Commit the offsets read.
    pub commit: bool,
}

impl Default for ConsumeOptions {
    fn default() -> Self {
        Self {
            bootstrap: "127.0.0.1:9092".into(),
            topic: String::new(),
            group: "lawtwin-peer".into(),
            isolation: Isolation::ReadCommitted,
            value_type: None,
            format: Format::Json,
            expect: None,
            duration: Duration::from_secs(60),
            idle: Duration::from_secs(3),
            from: "earliest".into(),
            commit: true,
        }
    }
}

/// A law a consumer peer found broken, and where.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PeerViolation {
    /// The law.
    pub law: String,
    /// The partition.
    pub partition: i32,
    /// The offset, if one record breaks it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
    /// What breaks it.
    pub detail: String,
}

/// A record a consumer peer read.
#[derive(Clone, Debug, Serialize)]
pub struct Read {
    /// Its partition.
    pub partition: i32,
    /// Its offset.
    pub offset: i64,
    /// Its key.
    pub key: Option<String>,
    /// Its value: text when it is printable UTF-8, `{"base64": ...}` otherwise.
    pub value: Option<lawtwin_kafka::history::Octets>,
    /// The rebalance it was read after, from 0.
    pub assignment: u64,
}

#[derive(Clone, Default)]
struct Assignments(Arc<Mutex<u64>>);

impl ClientContext for Assignments {}
impl ConsumerContext for Assignments {
    fn post_rebalance(&self, _: &BaseConsumer<Self>, r: &Rebalance<'_>) {
        if matches!(r, Rebalance::Assign(_)) {
            *self.0.lock().unwrap() += 1;
        }
    }
}

/// The consumer-side laws over what a consumer read: offset order within a
/// partition, keys staying on one partition, and the value type. A record
/// read again after a rebalance is a redelivery, which at-least-once
/// delivery allows, not a break of order.
pub fn check_reads(reads: &[Read], value: Option<(&Types, &Ty)>, format: Format) -> (Vec<PeerViolation>, u64, u64) {
    let mut out = vec![];
    let mut last: BTreeMap<i32, (i64, u64)> = BTreeMap::new();
    let mut seen: BTreeSet<(i32, i64)> = BTreeSet::new();
    let mut key_partition: BTreeMap<String, i32> = BTreeMap::new();
    let mut redelivered = 0;
    let mut invalid = 0;
    for r in reads {
        if !seen.insert((r.partition, r.offset)) {
            redelivered += 1;
        }
        if let Some((prev, assignment)) = last.get(&r.partition) {
            if r.offset <= *prev && *assignment == r.assignment {
                out.push(PeerViolation {
                    law: "a poll reads each partition in offset order".into(),
                    partition: r.partition,
                    offset: Some(r.offset),
                    detail: format!("offset {} read after {prev}", r.offset),
                });
            }
        }
        last.insert(r.partition, (r.offset, r.assignment));
        if let Some(k) = &r.key {
            match key_partition.get(k) {
                Some(p) if *p != r.partition => out.push(PeerViolation {
                    law: "records with one key go to one partition".into(),
                    partition: r.partition,
                    offset: Some(r.offset),
                    detail: format!("the key {k:?} was read from partitions {p} and {}", r.partition),
                }),
                _ => {
                    key_partition.insert(k.clone(), r.partition);
                }
            }
        }
        if let Some((types, ty)) = value {
            let problems = match &r.value {
                None => vec!["value: none (a tombstone)".to_string()],
                Some(v) => types.validate(ty, &v.0, format),
            };
            if !problems.is_empty() {
                invalid += 1;
                out.push(PeerViolation {
                    law: format!("every value is a {}", type_label(ty)),
                    partition: r.partition,
                    offset: Some(r.offset),
                    detail: problems.join("; "),
                });
            }
        }
    }
    (out, redelivered, invalid)
}

fn type_label(t: &Ty) -> String {
    match t {
        Ty::Named(n) => n.clone(),
        Ty::List(a) => format!("List {}", type_label(a)),
        Ty::Maybe(a) => format!("Maybe {}", type_label(a)),
        other => format!("{other:?}"),
    }
}

/// What the broker holds in a partition between two offsets: every data
/// record's offset, and the offsets a read-committed consumer may read.
pub struct Audit {
    /// Offsets of every data record.
    pub all: BTreeSet<i64>,
    /// Offsets a read-committed consumer may read.
    pub committed: BTreeSet<i64>,
    /// The last stable offset.
    pub stable: i64,
}

struct Wire {
    stream: std::net::TcpStream,
    correlation: i32,
}

impl Wire {
    fn connect(bootstrap: &str) -> Result<Self, String> {
        let stream = std::net::TcpStream::connect(bootstrap).map_err(|e| format!("{bootstrap}: {e}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
        Ok(Self { stream, correlation: 0 })
    }

    fn send<R: Request>(&mut self, req: R, version: i16) -> Result<R::Response, String> {
        self.correlation += 1;
        let header = RequestHeader::default()
            .with_request_api_key(R::KEY)
            .with_request_api_version(version)
            .with_correlation_id(self.correlation)
            .with_client_id(Some(StrBytes::from_static_str("lawtwin-peer-audit")));
        let mut buf = bytes::BytesMut::new();
        header.encode(&mut buf, R::header_version(version)).map_err(|e| e.to_string())?;
        req.encode(&mut buf, version).map_err(|e| e.to_string())?;
        self.stream.write_all(&(buf.len() as i32).to_be_bytes()).map_err(|e| e.to_string())?;
        self.stream.write_all(&buf).map_err(|e| e.to_string())?;
        let mut len = [0u8; 4];
        self.stream.read_exact(&mut len).map_err(|e| e.to_string())?;
        let mut body = vec![0u8; i32::from_be_bytes(len).max(0) as usize];
        self.stream.read_exact(&mut body).map_err(|e| e.to_string())?;
        let mut body = Bytes::from(body);
        ResponseHeader::decode(&mut body, R::Response::header_version(version)).map_err(|e| e.to_string())?;
        R::Response::decode(&mut body, version).map_err(|e| e.to_string())
    }

    /// Reads a partition from an offset to its high watermark, as a
    /// consumer reading committed or uncommitted would.
    fn scan(&mut self, topic: &str, partition: i32, from: i64, isolation: Isolation) -> Result<(BTreeSet<i64>, i64), String> {
        let mut offsets = BTreeSet::new();
        let mut at = from;
        let mut stable = from;
        for _ in 0..100_000 {
            let req = FetchRequest::default()
                .with_replica_id(BrokerId(-1))
                .with_max_wait_ms(0)
                .with_min_bytes(0)
                .with_max_bytes(1 << 20)
                .with_isolation_level(if isolation == Isolation::ReadCommitted { 1 } else { 0 })
                .with_session_epoch(-1)
                .with_topics(vec![fetch_request::FetchTopic::default().with_topic(TopicName(StrBytes::from_string(topic.to_string()))).with_partitions(vec![
                    fetch_request::FetchPartition::default().with_partition(partition).with_fetch_offset(at).with_partition_max_bytes(1 << 20),
                ])]);
            let r = self.send(req, 11)?;
            let Some(p) = r.responses.first().and_then(|t| t.partitions.first()) else { break };
            if p.error_code != 0 {
                return Err(format!("{topic}-{partition}: {}", crate::decode::kafka_error_name(p.error_code)));
            }
            stable = if isolation == Isolation::ReadCommitted { p.last_stable_offset } else { p.high_watermark };
            let bs = p.records.as_ref().map(batches).transpose()?.unwrap_or_default();
            let aborted: Vec<(i64, i64)> = p.aborted_transactions.iter().flatten().map(|a| (a.producer_id.0, a.first_offset)).collect();
            let (records, next) = readable(&bs, at, isolation, &aborted);
            offsets.extend(records.iter().map(|r| r.offset));
            if next <= at || next >= stable {
                break;
            }
            at = next;
        }
        Ok((offsets, stable))
    }
}

/// Reads a partition both ways to tell committed records from records of
/// open or aborted transactions.
pub fn audit(bootstrap: &str, topic: &str, partition: i32, from: i64) -> Result<Audit, String> {
    let mut w = Wire::connect(bootstrap)?;
    let (all, _) = w.scan(topic, partition, from, Isolation::ReadUncommitted)?;
    let (committed, stable) = w.scan(topic, partition, from, Isolation::ReadCommitted)?;
    Ok(Audit { all, committed, stable })
}

/// Runs a consumer peer: what it read, and a report with every law it
/// found broken.
pub fn consume(o: &ConsumeOptions) -> Result<(Vec<Read>, Value), String> {
    let consumer: BaseConsumer<Assignments> = ClientConfig::new()
        .set("bootstrap.servers", &o.bootstrap)
        .set("client.id", "lawtwin-peer-consumer")
        .set("group.id", &o.group)
        .set("auto.offset.reset", &o.from)
        .set("enable.auto.commit", "false")
        .set("isolation.level", if o.isolation == Isolation::ReadCommitted { "read_committed" } else { "read_uncommitted" })
        .set("fetch.wait.max.ms", "100")
        .set("session.timeout.ms", "10000")
        .create_with_context(Assignments::default())
        .map_err(|e| e.to_string())?;
    consumer.subscribe(&[&o.topic]).map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut last_record = Instant::now();
    let mut reads: Vec<Read> = vec![];
    loop {
        if started.elapsed() >= o.duration {
            break;
        }
        let enough = o.expect.is_none_or(|n| reads.len() as u64 >= n);
        if enough && o.expect.is_some() {
            break;
        }
        if enough && !reads.is_empty() && last_record.elapsed() >= o.idle {
            break;
        }
        match consumer.poll(Duration::from_millis(100)) {
            Some(Ok(m)) => {
                last_record = Instant::now();
                reads.push(Read {
                    partition: m.partition(),
                    offset: m.offset(),
                    key: m.key().map(|k| String::from_utf8_lossy(k).to_string()),
                    value: m.payload().map(|v| lawtwin_kafka::history::Octets(v.to_vec())),
                    assignment: *consumer.context().0.lock().unwrap(),
                });
            }
            Some(Err(e)) => return Err(format!("poll: {e}")),
            None => {}
        }
    }
    if o.commit && !reads.is_empty() {
        let _ = consumer.commit_consumer_state(CommitMode::Sync);
    }
    consumer.unsubscribe();
    drop(consumer);
    let value = o.value_type.as_ref().map(|(t, ty)| (t, ty));
    let (mut violations, redelivered, invalid) = check_reads(&reads, value, o.format);
    // What the broker holds, read both ways, against what was read.
    let mut partitions: BTreeMap<i32, (i64, i64, u64)> = BTreeMap::new();
    for r in &reads {
        let e = partitions.entry(r.partition).or_insert((r.offset, r.offset, 0));
        e.0 = e.0.min(r.offset);
        e.1 = e.1.max(r.offset);
        e.2 += 1;
    }
    for (p, (first, last, _)) in &partitions {
        let audit = audit(&o.bootstrap, &o.topic, *p, *first)?;
        let read: BTreeSet<i64> = reads.iter().filter(|r| r.partition == *p).map(|r| r.offset).collect();
        if o.isolation == Isolation::ReadCommitted {
            for off in read.iter().filter(|off| audit.all.contains(off) && !audit.committed.contains(off)) {
                violations.push(PeerViolation {
                    law: "read-committed fetches never see open or aborted transactions".into(),
                    partition: *p,
                    offset: Some(*off),
                    detail: format!("offset {off} belongs to a transaction that is {}", if *off >= audit.stable { "open" } else { "aborted" }),
                });
            }
            let skipped: Vec<i64> = audit.committed.iter().filter(|off| **off <= *last && !read.contains(off)).copied().collect();
            if let Some(off) = skipped.first() {
                violations.push(PeerViolation {
                    law: "committed records are read committed together and in order".into(),
                    partition: *p,
                    offset: Some(*off),
                    detail: format!("{} committed record(s) between offsets {first} and {last} were never read, from offset {off}", skipped.len()),
                });
            }
        } else {
            let skipped: Vec<i64> = audit.all.iter().filter(|off| **off <= *last && !read.contains(off)).copied().collect();
            if let Some(off) = skipped.first() {
                violations.push(PeerViolation {
                    law: "a poll reads each partition in offset order".into(),
                    partition: *p,
                    offset: Some(*off),
                    detail: format!("{} record(s) between offsets {first} and {last} were never read, from offset {off}", skipped.len()),
                });
            }
        }
    }
    let report = json!({
        "peer": "consume",
        "topic": o.topic,
        "group": o.group,
        "isolation": o.isolation,
        "records": reads.len(),
        "redelivered": redelivered,
        "invalid": invalid,
        "partitions": partitions.iter().map(|(p, (a, b, n))| json!({ "partition": p, "first": a, "last": b, "records": n })).collect::<Vec<_>>(),
        "violations": violations,
        "millis": started.elapsed().as_millis() as u64,
    });
    Ok((reads, report))
}

// ---------------------------------------------------------------------------
// The command line
// ---------------------------------------------------------------------------

fn duration(text: &str) -> Result<Duration, String> {
    lawtwin_kafka::clock::parse_duration(text).map(|ms| Duration::from_millis(ms.max(0) as u64))
}

fn value_type(a: &Args) -> Result<Option<(Types, Ty)>, String> {
    match (a.get("value-type"), a.get("spec")) {
        (None, None) => Ok(None),
        (Some(_), None) => Err("--value-type needs --spec <LawSpec file or directory>".into()),
        (None, Some(_)) => Err("--spec needs --value-type <type>".into()),
        (Some(t), Some(spec)) => {
            let types = Types::load(std::path::Path::new(spec))?;
            let ty = types.parse_type(t)?;
            Ok(Some((types, ty)))
        }
    }
}

fn write_report(a: &Args, report: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(report).unwrap_or_default();
    match a.get("report") {
        Some(path) => std::fs::write(path, text + "\n").map_err(|e| format!("{path}: {e}")),
        None => {
            println!("{text}");
            Ok(())
        }
    }
}

/// `lawtwin peer kafka produce|consume ...`.
pub fn command(rest: &[String]) -> Result<(), String> {
    let rest: Vec<String> = match rest.first().map(String::as_str) {
        Some("kafka") => rest[1..].to_vec(),
        _ => rest.to_vec(),
    };
    match rest.first().map(String::as_str) {
        Some("produce") => {
            let a = Args::parse(&rest[1..], &["idempotent", "no-keys", "no-seq-header"]);
            a.known(&[
                "bootstrap", "topic", "count", "duration", "rate", "seed", "keys", "no-keys", "header", "no-seq-header", "partition",
                "value-type", "spec", "format", "transactional-id", "txn-size", "idempotent", "fault", "late-by", "log", "report",
            ])?;
            let mut o = ProduceOptions {
                bootstrap: a.get("bootstrap").unwrap_or("127.0.0.1:9092").into(),
                topic: a.get("topic").ok_or("produce needs --topic")?.into(),
                seed: a.parsed("seed", 0)?,
                keys: if a.has("no-keys") { None } else { Some(a.parsed("keys", 8)?) },
                seq_header: !a.has("no-seq-header"),
                partition: a.get("partition").map(|p| p.parse().map_err(|_| format!("--partition does not take {p}"))).transpose()?,
                value_type: value_type(&a)?,
                format: a.parsed("format", Format::Json)?,
                transactional_id: a.get("transactional-id").map(str::to_string),
                txn_size: a.parsed("txn-size", 10)?,
                idempotent: a.has("idempotent"),
                late_by_ms: a.get("late-by").map(lawtwin_kafka::clock::parse_duration).transpose()?.unwrap_or(3_600_000),
                log: a.get("log").map(PathBuf::from),
                ..Default::default()
            };
            o.duration = a.get("duration").map(duration).transpose()?;
            o.count = match (a.get("count"), o.duration) {
                (Some(c), _) => Some(c.parse().map_err(|_| format!("--count does not take {c}"))?),
                (None, Some(_)) => None,
                (None, None) => Some(10),
            };
            o.rate = a.get("rate").map(|r| r.trim_end_matches("/s").parse::<f64>().map_err(|_| format!("--rate does not take {r}"))).transpose()?;
            for h in a.all("header") {
                let (n, v) = h.split_once('=').ok_or_else(|| format!("--header {h}: write name=value"))?;
                o.headers.push((n.to_string(), v.to_string()));
            }
            for f in a.all("fault") {
                o.faults.push(PeerFault::parse(&f)?);
            }
            if o.faults.iter().any(|f| matches!(f, PeerFault::Abort(_) | PeerFault::Hang)) && o.transactional_id.is_none() {
                return Err("the abort and hang faults need --transactional-id".into());
            }
            let (_, summary) = produce(&o)?;
            write_report(&a, &summary)?;
            if summary["failed"].as_u64().unwrap_or(0) > 0 { Err("some records were not written".into()) } else { Ok(()) }
        }
        Some("consume") => {
            let a = Args::parse(&rest[1..], &["no-commit"]);
            a.known(&[
                "bootstrap", "topic", "group", "isolation", "value-type", "spec", "format", "expect", "duration", "idle", "from", "no-commit",
                "report", "log",
            ])?;
            let o = ConsumeOptions {
                bootstrap: a.get("bootstrap").unwrap_or("127.0.0.1:9092").into(),
                topic: a.get("topic").ok_or("consume needs --topic")?.into(),
                group: a.get("group").unwrap_or("lawtwin-peer").into(),
                isolation: match a.get("isolation").unwrap_or("read_committed") {
                    "read_committed" => Isolation::ReadCommitted,
                    "read_uncommitted" => Isolation::ReadUncommitted,
                    other => return Err(format!("{other} is not an isolation level: read_committed or read_uncommitted")),
                },
                value_type: value_type(&a)?,
                format: a.parsed("format", Format::Json)?,
                expect: a.get("expect").map(|n| n.parse().map_err(|_| format!("--expect does not take {n}"))).transpose()?,
                duration: a.get("duration").map(duration).transpose()?.unwrap_or(Duration::from_secs(60)),
                idle: a.get("idle").map(duration).transpose()?.unwrap_or(Duration::from_secs(3)),
                from: a.get("from").unwrap_or("earliest").into(),
                commit: !a.has("no-commit"),
            };
            let (reads, report) = consume(&o)?;
            if let Some(path) = a.get("log") {
                let mut text = String::new();
                for r in &reads {
                    text.push_str(&serde_json::to_string(r).unwrap_or_default());
                    text.push('\n');
                }
                std::fs::write(path, text).map_err(|e| format!("{path}: {e}"))?;
            }
            write_report(&a, &report)?;
            let broken = report["violations"].as_array().map(|v| v.len()).unwrap_or(0);
            if broken == 0 { Ok(()) } else { Err(format!("{broken} law(s) broken by what was read")) }
        }
        _ => Err(format!("peer needs kafka produce or kafka consume\n{}", crate::cli::USAGE)),
    }
}
