//! Replaying a trace through a twin core: `lawtwin replay <trace> --core <core>`.
//!
//! Each recorded request is sent to a fresh core over the Kafka protocol,
//! on a connection of its own for each recorded connection. A request is
//! sent once every response that came back before it in the recording has
//! come back in the replay, so concurrent clients interleave as they did
//! (a group's members join one round). The identifiers the broker chose
//! (member ids, generations, producer ids and epochs) are renamed to the
//! core's as the core hands them out, both in the requests sent and in the
//! comparison (ref:DEC-compare-up-to-renaming). Both sides' responses are
//! decoded into the history and compared frame by frame; a group's rounds
//! are compared as a whole, since the two sides may choose another leader
//! to answer with the members. The core's own history is then checked as
//! the conformance kit checks it. ref:DEC-replay-over-the-wire

use crate::decode::{Decoder, Item, split_request, split_response};
use crate::frame::{Frame, parse_frames};
use anyhow::{Context, Result, anyhow};
use bytes::{Bytes, BytesMut};
use kafka_protocol::messages::*;
use kafka_protocol::protocol::{Decodable, Encodable, StrBytes};
use lawtwin_kafka::history::{Answer, Op, Outcome, Source};
use lawtwin_kafka_conformance::core::TestTwin;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Notify;

/// The identifiers a recorded broker chose, and the core's for them.
#[derive(Clone, Debug, Default)]
pub struct Names {
    /// Member ids.
    pub members: HashMap<String, String>,
    /// Generations, by group.
    pub generations: HashMap<(String, i32), i32>,
    /// Producer ids, with how far the recorded epochs are ahead.
    pub producers: HashMap<i64, (i64, i16)>,
}

impl Names {
    fn member(&self, m: &str) -> String {
        self.members.get(m).cloned().unwrap_or_else(|| m.to_string())
    }

    fn generation(&self, group: &str, g: i32) -> i32 {
        self.generations.get(&(group.to_string(), g)).copied().unwrap_or(g)
    }

    fn producer(&self, pid: i64, epoch: i16) -> (i64, i16) {
        match self.producers.get(&pid) {
            Some((p, ahead)) => (*p, epoch.saturating_sub(*ahead)),
            None => (pid, epoch),
        }
    }

    /// Learns from a recorded response and the core's response to the same
    /// request.
    fn learn(&mut self, key: i16, version: i16, request: &[u8], recorded: &[u8], replayed: &[u8]) {
        let Ok(api) = ApiKey::try_from(key) else { return };
        let (Ok((_, mut a)), Ok((_, mut b))) = (split_response(recorded, api, version), split_response(replayed, api, version)) else {
            return;
        };
        match api {
            ApiKey::JoinGroup => {
                let (Ok(r), Ok(p)) = (JoinGroupResponse::decode(&mut a, version), JoinGroupResponse::decode(&mut b, version)) else {
                    return;
                };
                if !r.member_id.is_empty() && !p.member_id.is_empty() {
                    self.members.insert(r.member_id.to_string(), p.member_id.to_string());
                }
                if r.error_code == 0 && p.error_code == 0 {
                    let group = split_request(request)
                        .ok()
                        .and_then(|(_, mut q)| JoinGroupRequest::decode(&mut q, version).ok())
                        .map(|q| q.group_id.0.to_string())
                        .unwrap_or_default();
                    self.generations.insert((group, r.generation_id), p.generation_id);
                }
            }
            ApiKey::InitProducerId => {
                let (Ok(r), Ok(p)) = (InitProducerIdResponse::decode(&mut a, version), InitProducerIdResponse::decode(&mut b, version))
                else {
                    return;
                };
                if r.error_code == 0 && p.error_code == 0 {
                    self.producers.insert(r.producer_id.0, (p.producer_id.0, r.producer_epoch.saturating_sub(p.producer_epoch)));
                }
            }
            _ => {}
        }
    }

    /// A recorded request, with the core's identifiers.
    pub fn request(&self, raw: &[u8]) -> Result<Vec<u8>> {
        let (header, mut body) = split_request(raw)?;
        let v = header.request_api_version;
        let api = ApiKey::try_from(header.request_api_key).map_err(|_| anyhow!("unknown API key"))?;
        let s = |m: &StrBytes| StrBytes::from_string(self.member(m.as_str()));
        let out = match api {
            ApiKey::Produce => {
                let mut r = ProduceRequest::decode(&mut body, v)?;
                for t in r.topic_data.iter_mut() {
                    for p in t.partition_data.iter_mut() {
                        if let Some(records) = &p.records {
                            p.records = Some(self.batches(records));
                        }
                    }
                }
                encode(&header, api, v, &r)?
            }
            ApiKey::InitProducerId => {
                let mut r = InitProducerIdRequest::decode(&mut body, v)?;
                if r.producer_id.0 >= 0 {
                    let (p, e) = self.producer(r.producer_id.0, r.producer_epoch);
                    r.producer_id = ProducerId(p);
                    r.producer_epoch = e;
                }
                encode(&header, api, v, &r)?
            }
            ApiKey::AddPartitionsToTxn => {
                let mut r = AddPartitionsToTxnRequest::decode(&mut body, v)?;
                let (p, e) = self.producer(r.v3_and_below_producer_id.0, r.v3_and_below_producer_epoch);
                r.v3_and_below_producer_id = ProducerId(p);
                r.v3_and_below_producer_epoch = e;
                encode(&header, api, v, &r)?
            }
            ApiKey::AddOffsetsToTxn => {
                let mut r = AddOffsetsToTxnRequest::decode(&mut body, v)?;
                let (p, e) = self.producer(r.producer_id.0, r.producer_epoch);
                r.producer_id = ProducerId(p);
                r.producer_epoch = e;
                encode(&header, api, v, &r)?
            }
            ApiKey::EndTxn => {
                let mut r = EndTxnRequest::decode(&mut body, v)?;
                let (p, e) = self.producer(r.producer_id.0, r.producer_epoch);
                r.producer_id = ProducerId(p);
                r.producer_epoch = e;
                encode(&header, api, v, &r)?
            }
            ApiKey::TxnOffsetCommit => {
                let mut r = TxnOffsetCommitRequest::decode(&mut body, v)?;
                let (p, e) = self.producer(r.producer_id.0, r.producer_epoch);
                r.producer_id = ProducerId(p);
                r.producer_epoch = e;
                r.member_id = s(&r.member_id);
                r.generation_id = self.generation(r.group_id.0.as_str(), r.generation_id);
                encode(&header, api, v, &r)?
            }
            ApiKey::JoinGroup => {
                let mut r = JoinGroupRequest::decode(&mut body, v)?;
                r.member_id = s(&r.member_id);
                encode(&header, api, v, &r)?
            }
            ApiKey::SyncGroup => {
                let mut r = SyncGroupRequest::decode(&mut body, v)?;
                r.generation_id = self.generation(r.group_id.0.as_str(), r.generation_id);
                r.member_id = s(&r.member_id);
                for a in r.assignments.iter_mut() {
                    a.member_id = s(&a.member_id);
                }
                encode(&header, api, v, &r)?
            }
            ApiKey::Heartbeat => {
                let mut r = HeartbeatRequest::decode(&mut body, v)?;
                r.generation_id = self.generation(r.group_id.0.as_str(), r.generation_id);
                r.member_id = s(&r.member_id);
                encode(&header, api, v, &r)?
            }
            ApiKey::LeaveGroup => {
                let mut r = LeaveGroupRequest::decode(&mut body, v)?;
                r.member_id = s(&r.member_id);
                for m in r.members.iter_mut() {
                    m.member_id = s(&m.member_id);
                }
                encode(&header, api, v, &r)?
            }
            ApiKey::OffsetCommit => {
                let mut r = OffsetCommitRequest::decode(&mut body, v)?;
                r.generation_id_or_member_epoch = self.generation(r.group_id.0.as_str(), r.generation_id_or_member_epoch);
                r.member_id = s(&r.member_id);
                encode(&header, api, v, &r)?
            }
            _ => raw.to_vec(),
        };
        Ok(out)
    }

    /// Record batches with the core's producer ids and epochs, each batch's
    /// checksum written again.
    fn batches(&self, records: &Bytes) -> Bytes {
        let mut b = records.to_vec();
        let mut at = 0usize;
        while at + 61 <= b.len() {
            let length = i32::from_be_bytes(b[at + 8..at + 12].try_into().unwrap());
            let end = at + 12 + length.max(0) as usize;
            if length < 49 || end > b.len() {
                break;
            }
            let pid = i64::from_be_bytes(b[at + 43..at + 51].try_into().unwrap());
            if pid >= 0 {
                let epoch = i16::from_be_bytes(b[at + 51..at + 53].try_into().unwrap());
                let (p, e) = self.producer(pid, epoch);
                b[at + 43..at + 51].copy_from_slice(&p.to_be_bytes());
                b[at + 51..at + 53].copy_from_slice(&e.to_be_bytes());
                let crc = crc32c::crc32c(&b[at + 21..end]);
                b[at + 17..at + 21].copy_from_slice(&crc.to_be_bytes());
            }
            at = end;
        }
        Bytes::from(b)
    }

    /// A recorded history item, with the core's identifiers.
    pub fn item(&self, item: &Item) -> Item {
        match item {
            Item::Note { .. } => item.clone(),
            Item::Call { source, op, answer } => {
                let mut op = op.clone();
                let mut answer = answer.clone();
                match &mut op {
                    Op::ProduceRequest { tag: Some(t), .. } => (t.producer_id, t.producer_epoch) = self.producer(t.producer_id, t.producer_epoch),
                    Op::AddPartitionsToTxnRequest { producer_id, producer_epoch, .. }
                    | Op::TxnOffsetCommitRequest { producer_id, producer_epoch, .. }
                    | Op::EndTxnRequest { producer_id, producer_epoch, .. } => {
                        (*producer_id, *producer_epoch) = self.producer(*producer_id, *producer_epoch)
                    }
                    Op::JoinGroupRequest { group, member, .. } => {
                        *member = self.member(member);
                        if let Answer::Ok(Outcome::Number(g)) = &mut answer {
                            *g = self.generation(group, *g as i32) as i64;
                        }
                    }
                    Op::LeaveGroupRequest { member, .. } => *member = self.member(member),
                    Op::SyncGroupRequest { group, member, generation }
                    | Op::HeartbeatRequest { group, member, generation }
                    | Op::OffsetCommitRequest { group, member, generation, .. } => {
                        *member = self.member(member);
                        *generation = self.generation(group, *generation);
                    }
                    Op::DescribeGroupRequest { group } => {
                        if let Answer::Ok(Outcome::Group { generation, members }) = &mut answer {
                            *generation = self.generation(group, *generation);
                            for m in members.iter_mut() {
                                m.member = self.member(&m.member);
                            }
                            members.sort_by(|a, b| a.member.cmp(&b.member));
                        }
                    }
                    _ => {}
                }
                if let (Op::InitProducerIdRequest { .. }, Answer::Ok(Outcome::Producer { producer_id, producer_epoch })) = (&op, &mut answer) {
                    (*producer_id, *producer_epoch) = self.producer(*producer_id, *producer_epoch);
                }
                Item::Call { source: source.clone(), op, answer }
            }
        }
    }
}

fn encode<R: Encodable>(header: &RequestHeader, key: ApiKey, version: i16, req: &R) -> Result<Vec<u8>> {
    let mut buf = BytesMut::new();
    header.encode(&mut buf, key.request_header_version(version))?;
    req.encode(&mut buf, version)?;
    Ok(buf.to_vec())
}

/// A frame whose decoded answers differ between the recording and the replay.
#[derive(Clone, Debug, Serialize)]
pub struct FrameDivergence {
    /// The frame.
    pub n: u64,
    /// Its API.
    pub api: String,
    /// What the recording decodes to, renamed to the core's identifiers.
    pub recorded: Vec<Value>,
    /// What the replay decodes to.
    pub replayed: Vec<Value>,
}

/// What a replay found.
#[derive(Clone, Debug, Serialize)]
pub struct ReplayReport {
    /// The trace.
    pub trace: String,
    /// The core it was replayed through.
    pub core: String,
    /// Requests sent.
    pub requests: usize,
    /// Requests the core answered.
    pub answered: usize,
    /// Requests that waited past their time for the requests before them.
    pub stalled: Vec<u64>,
    /// Frames whose answers differ.
    pub divergences: Vec<FrameDivergence>,
    /// Laws broken over the core's history, and its answers that are not
    /// the reference model's.
    pub violations: Vec<Value>,
}

/// The trace's prefix, given the prefix or one of its files.
pub fn trace_prefix(given: &str) -> PathBuf {
    for suffix in [".frames.jsonl", ".history.jsonl", ".summary.json"] {
        if let Some(p) = given.strip_suffix(suffix) {
            return PathBuf::from(p);
        }
    }
    PathBuf::from(given)
}

struct Shared {
    names: Names,
    replies: BTreeMap<u64, (Vec<u8>, Vec<u8>, u64)>,
    answered: std::collections::HashSet<u64>,
    prefix: usize,
    stalled: Vec<u64>,
}

/// Replays a trace's requests through a core and compares the answers.
pub fn replay_trace(prefix: &Path, core: &str) -> Result<ReplayReport> {
    let frames_path = crate::record::with_suffix(prefix, "frames.jsonl");
    let text = std::fs::read_to_string(&frames_path).with_context(|| format!("read {}", frames_path.display()))?;
    let frames = parse_frames(&text).map_err(|e| anyhow!(e))?;
    let core_arg = match core {
        "" | "rust" | "in-process" => None,
        c => Some(c),
    };
    let mut report = replay_frames(&frames, core_arg)?;
    report.trace = prefix.display().to_string();
    report.core = core.to_string();
    Ok(report)
}

/// Replays frames through a core (a command, `image:<image>`, or the Rust
/// core in this process when none) and compares the answers.
pub fn replay_frames(frames: &[Frame], core: Option<&str>) -> Result<ReplayReport> {
    let twin = TestTwin::start_core(core, 0);
    // State the broker held before the recording started.
    let observed = observed_topics(frames);
    if !observed.is_empty() {
        let topics: Vec<Value> = observed.iter().map(|(n, p)| json!({ "name": n, "partitions": p })).collect();
        twin.post("/v1/state", json!({ "topics": topics }));
    }
    let runtime = lawtwin_kafka::runtime()?;
    let shared = runtime.block_on(send_all(frames, twin.kafka));
    let shared = shared?;
    let mut report = compare(frames, &shared);
    report.core = core.unwrap_or("the Rust core").to_string();
    // The core's own history, checked as the conformance kit checks it.
    let text = twin.history_text();
    let entries = lawtwin_kafka::admin::parse_history(&text).map_err(|e| anyhow!(e))?;
    let (model, divergences) = lawtwin_kafka::history::replay(&entries);
    report.violations.extend(divergences.iter().map(|d| json!({ "divergence": d })));
    report.violations.extend(lawtwin_kafka::laws::check_history(&entries).iter().map(|v| json!({ "law": v })));
    report.violations.extend(lawtwin_kafka::laws::check_state(&model.snapshot()).iter().map(|v| json!({ "law": v })));
    Ok(report)
}

/// Topics the recording found already there, with their partitions.
fn observed_topics(frames: &[Frame]) -> Vec<(String, i32)> {
    let mut d = Decoder::new();
    let mut out = vec![];
    let mut order: Vec<&Frame> = frames.iter().collect();
    order.sort_by_key(|f| (f.answered_us, f.n));
    for f in order {
        for item in d.frame(f) {
            if let Item::Call { source: Source::Observed, op: Op::CreateTopicsRequest { name, partitions }, .. } = item {
                out.push((name, partitions));
            }
        }
    }
    out
}

async fn send_all(frames: &[Frame], kafka: std::net::SocketAddr) -> Result<Shared> {
    // The recording's order of answers, and for each request how many of
    // them came back before it was sent.
    let mut answer_order: Vec<&Frame> = frames.iter().collect();
    answer_order.sort_by_key(|f| (f.answered_us, f.n));
    let order: Vec<u64> = answer_order.iter().map(|f| f.n).collect();
    let before: HashMap<u64, usize> =
        frames.iter().map(|f| (f.n, answer_order.iter().filter(|o| o.answered_us < f.sent_us && o.n != f.n).count())).collect();
    let shared = Arc::new(Mutex::new(Shared {
        names: Names::default(),
        replies: BTreeMap::new(),
        answered: Default::default(),
        prefix: 0,
        stalled: vec![],
    }));
    let notify = Arc::new(Notify::new());
    let stalled = Arc::new(Mutex::new(vec![]));
    let start = Instant::now();
    let mut by_conn: BTreeMap<u64, Vec<Frame>> = BTreeMap::new();
    for f in frames {
        by_conn.entry(f.conn).or_default().push(f.clone());
    }
    let order = Arc::new(order);
    let mut tasks = vec![];
    for (_, mut list) in by_conn {
        list.sort_by_key(|f| f.n);
        let (shared, notify, before, order, stalled) = (shared.clone(), notify.clone(), before.clone(), order.clone(), stalled.clone());
        tasks.push(tokio::spawn(async move {
            let stream = TcpStream::connect(kafka).await?;
            let _ = stream.set_nodelay(true);
            let (mut r, mut w) = stream.into_split();
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(Frame, Vec<u8>)>();
            let reader_shared = shared.clone();
            let reader_notify = notify.clone();
            let reader_order = order.clone();
            let reader = tokio::spawn(async move {
                while let Some((f, sent)) = rx.recv().await {
                    let reply = if f.response.is_some() {
                        let mut len = [0u8; 4];
                        if r.read_exact(&mut len).await.is_err() {
                            break;
                        }
                        let mut body = vec![0u8; i32::from_be_bytes(len).max(0) as usize];
                        if r.read_exact(&mut body).await.is_err() {
                            break;
                        }
                        body
                    } else {
                        vec![]
                    };
                    let at = start.elapsed().as_micros() as u64;
                    let mut s = reader_shared.lock().unwrap();
                    if let (Some(recorded), false) = (&f.response, reply.is_empty()) {
                        s.names.learn(f.key, f.version, &f.request, recorded, &reply);
                    }
                    s.replies.insert(f.n, (sent, reply, at));
                    s.answered.insert(f.n);
                    while s.prefix < reader_order.len() && s.answered.contains(&reader_order[s.prefix]) {
                        s.prefix += 1;
                    }
                    drop(s);
                    reader_notify.notify_waiters();
                }
            });
            for f in list {
                let need = before.get(&f.n).copied().unwrap_or(0);
                let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
                loop {
                    let wait = notify.notified();
                    tokio::pin!(wait);
                    wait.as_mut().enable();
                    if shared.lock().unwrap().prefix >= need {
                        break;
                    }
                    if tokio::time::timeout_at(deadline, wait).await.is_err() {
                        stalled.lock().unwrap().push(f.n);
                        break;
                    }
                }
                let sent = shared.lock().unwrap().names.request(&f.request).unwrap_or_else(|_| f.request.clone());
                let mut out = Vec::with_capacity(sent.len() + 4);
                out.extend_from_slice(&(sent.len() as i32).to_be_bytes());
                out.extend_from_slice(&sent);
                if w.write_all(&out).await.is_err() {
                    break;
                }
                if tx.send((f, sent)).is_err() {
                    break;
                }
            }
            drop(tx);
            let _ = tokio::time::timeout(Duration::from_secs(60), reader).await;
            Ok::<(), anyhow::Error>(())
        }));
    }
    for t in tasks {
        let _ = t.await;
    }
    let mut s = Arc::try_unwrap(shared).map_err(|_| anyhow!("a replay connection is still open"))?.into_inner().unwrap();
    s.stalled = stalled.lock().unwrap().clone();
    s.stalled.sort();
    Ok(s)
}

fn values(items: &[Item]) -> Vec<Value> {
    items
        .iter()
        .filter_map(|i| match i {
            Item::Call { source, op, answer } => Some(json!({ "source": source, "op": op, "answer": answer })),
            Item::Note { .. } => None,
        })
        .collect()
}

/// Two decoded answers, the recorded (renamed) and the replayed, compared;
/// a fetch may return fewer records on one side, as Kafka sizes a fetch by
/// bytes and a twin by records (ref:DEC-fetch-sizing).
fn same_item(a: &Item, b: &Item) -> bool {
    match (a, b) {
        (
            Item::Call { op: Op::FetchRequest { tp: ta, offset: oa, isolation: ia, .. }, answer: Answer::Ok(Outcome::Fetch(fa)), .. },
            Item::Call { op: Op::FetchRequest { tp: tb, offset: ob, isolation: ib, .. }, answer: Answer::Ok(Outcome::Fetch(fb)), .. },
        ) => {
            let n = fa.records.len().min(fb.records.len());
            ta == tb
                && oa == ob
                && ia == ib
                && fa.records[..n] == fb.records[..n]
                && (fa.records.is_empty() == fb.records.is_empty())
                && fa.high_watermark == fb.high_watermark
                && fa.last_stable_offset == fb.last_stable_offset
        }
        (Item::Call { op: oa, answer: aa, .. }, Item::Call { op: ob, answer: ab, .. }) => {
            oa == ob
                && match (aa, ab) {
                    (Answer::Error(x), Answer::Error(y)) => {
                        x == y || crate::decode::contract_code(x).is_some_and(|c| crate::decode::contract_code(y) == Some(c))
                    }
                    _ => aa == ab,
                }
        }
        _ => false,
    }
}

fn compare(frames: &[Frame], s: &Shared) -> ReplayReport {
    let mut recorded_order: Vec<&Frame> = frames.iter().collect();
    recorded_order.sort_by_key(|f| (f.answered_us, f.n));
    let mut d = Decoder::new();
    let mut recorded: BTreeMap<u64, Vec<Item>> = BTreeMap::new();
    for f in &recorded_order {
        let items: Vec<Item> = d.frame(f).iter().map(|i| s.names.item(i)).collect();
        recorded.insert(f.n, items);
    }
    let mut replay_frames: Vec<Frame> = frames
        .iter()
        .filter_map(|f| {
            let (sent, reply, at) = s.replies.get(&f.n)?;
            Some(Frame {
                request: sent.clone(),
                response: if f.response.is_some() { Some(reply.clone()) } else { None },
                delivered: None,
                answered_us: *at,
                ..f.clone()
            })
        })
        .collect();
    replay_frames.sort_by_key(|f| (f.answered_us, f.n));
    let mut d = Decoder::new();
    let mut replayed: BTreeMap<u64, Vec<Item>> = BTreeMap::new();
    for f in &replay_frames {
        replayed.insert(f.n, d.frame(f));
    }
    let join = ApiKey::JoinGroup as i16;
    let mut rounds_recorded: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut rounds_replayed: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut divergences = vec![];
    for f in frames {
        let rec = recorded.get(&f.n).cloned().unwrap_or_default();
        let rep = replayed.get(&f.n).cloned().unwrap_or_default();
        if f.key == join {
            for (items, into) in [(&rec, &mut rounds_recorded), (&rep, &mut rounds_replayed)] {
                for v in values(items) {
                    let group = v["op"]["group"].as_str().unwrap_or("").to_string();
                    into.entry(group).or_default().push(v.to_string());
                }
            }
            continue;
        }
        let a: Vec<&Item> = rec.iter().filter(|i| matches!(i, Item::Call { .. })).collect();
        let b: Vec<&Item> = rep.iter().filter(|i| matches!(i, Item::Call { .. })).collect();
        let same = a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| same_item(x, y));
        if !same {
            divergences.push(FrameDivergence { n: f.n, api: f.api.clone(), recorded: values(&rec), replayed: values(&rep) });
        }
    }
    for (group, mut a) in rounds_recorded {
        let mut b = rounds_replayed.remove(&group).unwrap_or_default();
        a.sort();
        b.sort();
        if a != b {
            divergences.push(FrameDivergence {
                n: 0,
                api: format!("JoinGroup rounds of {group}"),
                recorded: a.iter().map(|x| serde_json::from_str(x).unwrap_or_default()).collect(),
                replayed: b.iter().map(|x| serde_json::from_str(x).unwrap_or_default()).collect(),
            });
        }
    }
    for (group, b) in rounds_replayed {
        divergences.push(FrameDivergence {
            n: 0,
            api: format!("JoinGroup rounds of {group}"),
            recorded: vec![],
            replayed: b.iter().map(|x| serde_json::from_str(x).unwrap_or_default()).collect(),
        });
    }
    let stalled = s.stalled.clone();
    ReplayReport {
        trace: String::new(),
        core: String::new(),
        requests: frames.len(),
        answered: s.replies.len(),
        stalled,
        divergences,
        violations: vec![],
    }
}
