//! The contract's laws, checked over what the twin holds and what it did.
//!
//! `check_state` checks the invariants a state of the model must satisfy,
//! each named by the law that states it; a forced state is checked with it.
//! `check_history` checks a history (the twin's, or one recorded elsewhere)
//! against the laws a sequence of operations must satisfy: the assertions
//! mirror the contract's laws, so a history from real clients is checked
//! against the same guarantees the contract states.

use crate::history::{Answer, Entry, Event, Isolation, Op, Outcome, Record, Tp};
use crate::model::data;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A law that does not hold, and where.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violation {
    /// The law, as the contract names it.
    pub law: String,
    /// What breaks it.
    pub detail: String,
    /// The history entry, for a history check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
}

fn v(law: &str, detail: String, seq: Option<u64>) -> Violation {
    Violation {
        law: law.into(),
        detail,
        seq,
    }
}

/// Checks a topic name as the contract does.
pub fn valid_topic_name(name: &str) -> bool {
    (1..=249).contains(&name.len())
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

/// The invariants of a state of the model, each named by its law.
pub fn check_state(b: &data::BrokerState) -> Vec<Violation> {
    let mut out = vec![];
    for (name, n) in &b.topics {
        if !valid_topic_name(name) {
            out.push(v(
                "an invalid topic name is refused",
                format!("the topic {name:?} exists"),
                None,
            ));
        }
        if !(1..=1024).contains(n) {
            out.push(v(
                "a new topic has the partitions asked for",
                format!("{name} has {n} partitions"),
                None,
            ));
        }
    }
    for (tp, log) in &b.logs {
        let known = b
            .topics
            .get(&tp.topic)
            .is_some_and(|n| tp.partition >= 0 && tp.partition < *n);
        if !known {
            out.push(v(
                "a send to a partition the topic lacks is refused",
                format!(
                    "{}-{} has a log but the topic has no such partition",
                    tp.topic, tp.partition
                ),
                None,
            ));
        }
        // Entries are newest first; their offsets count down from the log end to 0.
        let mut expected = log.logEndOffset;
        for e in &log.entries {
            let o = match e {
                data::LogEntry::DataEntry { value } => value.entryOffset,
                data::LogEntry::ControlEntry { value } => value.markerOffset,
            };
            expected -= 1;
            if o != expected {
                out.push(v(
                    "offsets in a partition are dense and increasing",
                    format!(
                        "{}-{} has an entry at {o} where {expected} belongs",
                        tp.topic, tp.partition
                    ),
                    None,
                ));
                break;
            }
        }
        if expected != 0
            && out
                .iter()
                .all(|x| x.law != "offsets in a partition are dense and increasing")
        {
            out.push(v(
                "offsets in a partition are dense and increasing",
                format!(
                    "{}-{} has {} entries below a log end of {}",
                    tp.topic,
                    tp.partition,
                    log.entries.len(),
                    log.logEndOffset
                ),
                None,
            ));
        }
        let lso = log
            .openTxns
            .values()
            .copied()
            .fold(log.logEndOffset, i64::min);
        if lso > log.logEndOffset || log.openTxns.values().any(|f| *f > log.logEndOffset) {
            out.push(v(
                "the stable offset never passes the high watermark",
                format!(
                    "{}-{}: stable offset {lso}, high watermark {}",
                    tp.topic, tp.partition, log.logEndOffset
                ),
                None,
            ));
        }
    }
    for (pid, p) in &b.producers {
        if p.epoch < 0 {
            out.push(v(
                "a new epoch fences the older producer",
                format!("producer {pid} has epoch {}", p.epoch),
                None,
            ));
        }
    }
    for (g, group) in &b.groups {
        let mut owners: BTreeMap<(String, i32), Vec<&str>> = BTreeMap::new();
        for (m, tps) in &group.assignment {
            for tp in tps {
                owners
                    .entry((tp.topic.clone(), tp.partition))
                    .or_default()
                    .push(m);
            }
        }
        let topics: BTreeSet<&String> = group.subscriptions.values().flatten().collect();
        for t in topics {
            let Some(n) = b.topics.get(t) else { continue };
            for p in 0..*n {
                let count = owners.get(&(t.clone(), p)).map(|o| o.len()).unwrap_or(0);
                if count != 1 {
                    out.push(v(
                        "every partition has exactly one owner in a generation",
                        format!("in group {g}, {t}-{p} has {count} owners"),
                        None,
                    ));
                }
            }
        }
    }
    out
}

#[derive(Default)]
struct Log {
    end: i64,
    records: HashMap<i64, (Record, i64, bool)>,
}

#[derive(Default)]
struct Producer {
    transactional_id: Option<String>,
    epoch: i16,
    partitions: BTreeSet<Tp>,
    offsets: Vec<(String, Tp, i64)>,
    open: bool,
    first: BTreeMap<Tp, i64>,
}

/// The laws a history must satisfy, checked operation by operation.
pub fn check_history(entries: &[Entry]) -> Vec<Violation> {
    let mut out = vec![];
    let mut logs: BTreeMap<Tp, Log> = BTreeMap::new();
    let mut batches: HashMap<(i64, i16, i32, usize, Tp), i64> = HashMap::new();
    let mut producers: BTreeMap<i64, Producer> = BTreeMap::new();
    let mut by_txid: BTreeMap<String, i64> = BTreeMap::new();
    let mut aborted: BTreeMap<Tp, Vec<(i64, i64, i64)>> = BTreeMap::new();
    let mut generations: BTreeMap<String, i32> = BTreeMap::new();
    let mut owners: BTreeMap<(String, i32, Tp), String> = BTreeMap::new();
    let mut committed: BTreeMap<(String, Tp), i64> = BTreeMap::new();

    fn end_txn(
        pid: i64,
        commit: bool,
        p: &mut Producer,
        logs: &mut BTreeMap<Tp, Log>,
        aborted: &mut BTreeMap<Tp, Vec<(i64, i64, i64)>>,
        committed: &mut BTreeMap<(String, Tp), i64>,
    ) {
        for tp in std::mem::take(&mut p.partitions) {
            let log = logs.entry(tp.clone()).or_default();
            if !commit {
                if let Some(first) = p.first.get(&tp) {
                    aborted
                        .entry(tp.clone())
                        .or_default()
                        .push((pid, *first, log.end));
                }
            }
            log.end += 1;
        }
        if commit {
            for (g, tp, o) in std::mem::take(&mut p.offsets) {
                committed.insert((g, tp), o);
            }
        }
        p.offsets.clear();
        p.first.clear();
        p.open = false;
    }

    for e in entries {
        let seq = Some(e.seq);
        let (op, answer) = match &e.event {
            Event::Call { op, answer } => (op, answer),
            Event::Control { control, .. } => {
                if control == "reset" {
                    logs.clear();
                    batches.clear();
                    producers.clear();
                    by_txid.clear();
                    aborted.clear();
                    generations.clear();
                    owners.clear();
                    committed.clear();
                }
                continue;
            }
        };
        let ok = match answer {
            Answer::Ok(o) => Some(o),
            Answer::Error(_) => None,
        };
        match (op, ok) {
            (Op::ProduceRequest { tp, tag, records }, Some(Outcome::Number(base))) => {
                let key = tag.as_ref().map(|t| {
                    (
                        t.producer_id,
                        t.producer_epoch,
                        t.base_sequence,
                        records.len(),
                        tp.clone(),
                    )
                });
                if let Some(first) = key.as_ref().and_then(|k| batches.get(k)) {
                    if first != base {
                        out.push(v(
                            "an idempotent retry is written once",
                            format!(
                                "a retry to {}-{} got offset {base}; the first write got {first}",
                                tp.topic, tp.partition
                            ),
                            seq,
                        ));
                    }
                    continue;
                }
                let log = logs.entry(tp.clone()).or_default();
                if *base != log.end {
                    out.push(v(
                        "offsets in a partition are dense and increasing",
                        format!(
                            "a batch to {}-{} got offset {base}; the log ended at {}",
                            tp.topic, tp.partition, log.end
                        ),
                        seq,
                    ));
                }
                let writer = tag.as_ref().map(|t| t.producer_id).unwrap_or(-1);
                let transactional = producers
                    .get(&writer)
                    .is_some_and(|p| p.transactional_id.is_some());
                for (i, r) in records.iter().enumerate() {
                    log.records
                        .insert(base + i as i64, (r.clone(), writer, transactional));
                }
                log.end = base + records.len() as i64;
                if let Some(k) = key {
                    batches.insert(k, *base);
                }
                if transactional {
                    if let Some(p) = producers.get_mut(&writer) {
                        p.first.entry(tp.clone()).or_insert(*base);
                    }
                }
            }
            (Op::ProduceRequest { tag: Some(t), .. }, None) => {
                let _ = t;
            }
            (
                Op::FetchRequest {
                    tp,
                    offset,
                    isolation,
                    ..
                },
                Some(Outcome::Fetch(f)),
            ) => {
                if f.last_stable_offset > f.high_watermark {
                    out.push(v(
                        "the stable offset never passes the high watermark",
                        format!(
                            "{}-{}: stable offset {}, high watermark {}",
                            tp.topic, tp.partition, f.last_stable_offset, f.high_watermark
                        ),
                        seq,
                    ));
                }
                let mut last = offset - 1;
                for r in &f.records {
                    if r.offset <= last {
                        out.push(v(
                            "a batch reads back whole and in order",
                            format!(
                                "a fetch from {}-{} gave offset {} after {last}",
                                tp.topic, tp.partition, r.offset
                            ),
                            seq,
                        ));
                    }
                    last = r.offset;
                    let Some(log) = logs.get(tp) else { continue };
                    if let Some((written, writer, transactional)) = log.records.get(&r.offset) {
                        if written != &r.record {
                            out.push(v(
                                "a batch reads back whole and in order",
                                format!(
                                    "{}-{} at {} reads back other than written",
                                    tp.topic, tp.partition, r.offset
                                ),
                                seq,
                            ));
                        }
                        if *isolation == Isolation::ReadCommitted && *transactional {
                            let in_aborted = aborted.get(tp).is_some_and(|a| {
                                a.iter().any(|(w, lo, hi)| {
                                    w == writer && r.offset >= *lo && r.offset <= *hi
                                })
                            });
                            let open = producers.get(writer).is_some_and(|p| {
                                p.first.get(tp).is_some_and(|first| r.offset >= *first)
                            });
                            if in_aborted || open {
                                out.push(v(
                                    "read-committed fetches never see open or aborted transactions",
                                    format!(
                                        "a read-committed fetch from {}-{} gave offset {} of an {} transaction",
                                        tp.topic,
                                        tp.partition,
                                        r.offset,
                                        if open { "open" } else { "aborted" }
                                    ),
                                    seq,
                                ));
                            }
                        }
                    }
                }
            }
            (
                Op::InitProducerIdRequest { transactional_id },
                Some(Outcome::Producer {
                    producer_id,
                    producer_epoch,
                }),
            ) => {
                if let Some(id) = transactional_id {
                    if let Some(old) = by_txid.get(id).copied() {
                        if let Some(mut p) = producers.remove(&old) {
                            if p.open {
                                end_txn(
                                    old,
                                    false,
                                    &mut p,
                                    &mut logs,
                                    &mut aborted,
                                    &mut committed,
                                );
                            }
                            if old == *producer_id
                                && *producer_epoch <= p.epoch
                                && p.epoch != i16::MAX
                            {
                                out.push(v(
                                    "a new epoch fences the older producer",
                                    format!("{id} was initialised again at epoch {producer_epoch}, not above {}", p.epoch),
                                    seq,
                                ));
                            }
                        }
                    }
                    by_txid.insert(id.clone(), *producer_id);
                }
                producers.insert(
                    *producer_id,
                    Producer {
                        transactional_id: transactional_id.clone(),
                        epoch: *producer_epoch,
                        ..Default::default()
                    },
                );
            }
            (
                Op::AddPartitionsToTxnRequest {
                    producer_id,
                    producer_epoch,
                    partitions,
                    ..
                },
                Some(_),
            ) => {
                if let Some(p) = producers.get_mut(producer_id) {
                    if *producer_epoch < p.epoch {
                        out.push(v(
                            "a new epoch fences the older producer",
                            format!(
                                "epoch {producer_epoch} added partitions after epoch {}",
                                p.epoch
                            ),
                            seq,
                        ));
                    }
                    p.partitions.extend(partitions.iter().cloned());
                    p.open = true;
                }
            }
            (
                Op::TxnOffsetCommitRequest {
                    producer_id,
                    producer_epoch,
                    group,
                    offsets,
                    ..
                },
                Some(_),
            ) => {
                if let Some(p) = producers.get_mut(producer_id) {
                    if *producer_epoch < p.epoch {
                        out.push(v(
                            "a new epoch fences the older producer",
                            format!(
                                "epoch {producer_epoch} sent offsets after epoch {}",
                                p.epoch
                            ),
                            seq,
                        ));
                    }
                    for o in offsets {
                        p.offsets.push((
                            group.clone(),
                            Tp {
                                topic: o.topic.clone(),
                                partition: o.partition,
                            },
                            o.offset,
                        ));
                    }
                    p.open = true;
                }
            }
            (
                Op::EndTxnRequest {
                    producer_id,
                    producer_epoch,
                    commit,
                    ..
                },
                Some(_),
            ) => {
                if let Some(mut p) = producers.remove(producer_id) {
                    if *producer_epoch < p.epoch {
                        out.push(v(
                            "a new epoch fences the older producer",
                            format!(
                                "epoch {producer_epoch} ended a transaction after epoch {}",
                                p.epoch
                            ),
                            seq,
                        ));
                    }
                    if !p.open {
                        out.push(v(
                            "a transaction must begin before it ends",
                            format!("producer {producer_id} ended no transaction"),
                            seq,
                        ));
                    }
                    end_txn(
                        *producer_id,
                        *commit,
                        &mut p,
                        &mut logs,
                        &mut aborted,
                        &mut committed,
                    );
                    producers.insert(*producer_id, p);
                }
            }
            (Op::JoinGroupRequest { group, .. }, Some(Outcome::Number(g))) => {
                let known = generations.entry(group.clone()).or_insert(*g as i32);
                if (*g as i32) < *known {
                    out.push(v(
                        "a member of an older generation cannot commit",
                        format!("group {group} went back to generation {g}"),
                        seq,
                    ));
                }
                *known = (*known).max(*g as i32);
            }
            (
                Op::SyncGroupRequest {
                    group,
                    member,
                    generation,
                },
                Some(Outcome::Partitions(tps)),
            ) => {
                for tp in tps {
                    let key = (group.clone(), *generation, tp.clone());
                    match owners.get(&key) {
                        Some(other) if other != member => out.push(v(
                            "every partition has exactly one owner in a generation",
                            format!("in group {group} generation {generation}, {}-{} went to {other} and {member}", tp.topic, tp.partition),
                            seq,
                        )),
                        _ => {
                            owners.insert(key, member.clone());
                        }
                    }
                }
                stale(&generations, group, *generation, seq, &mut out);
            }
            (
                Op::HeartbeatRequest {
                    group, generation, ..
                },
                Some(_),
            ) => stale(&generations, group, *generation, seq, &mut out),
            (
                Op::OffsetCommitRequest {
                    group,
                    generation,
                    offsets,
                    member,
                },
                Some(_),
            ) => {
                if !(member.is_empty() && *generation == -1) {
                    stale(&generations, group, *generation, seq, &mut out);
                }
                for o in offsets {
                    committed.insert(
                        (
                            group.clone(),
                            Tp {
                                topic: o.topic.clone(),
                                partition: o.partition,
                            },
                        ),
                        o.offset,
                    );
                }
            }
            (Op::OffsetFetchRequest { group, tp }, Some(Outcome::Maybe(got))) => {
                if let Some(want) = committed.get(&(group.clone(), tp.clone())) {
                    if got != &Some(*want) {
                        out.push(v(
                            "a committed offset can be fetched",
                            format!(
                                "group {group} committed {want} for {}-{} and fetched {got:?}",
                                tp.topic, tp.partition
                            ),
                            seq,
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn stale(
    generations: &BTreeMap<String, i32>,
    group: &str,
    generation: i32,
    seq: Option<u64>,
    out: &mut Vec<Violation>,
) {
    if let Some(known) = generations.get(group) {
        if generation < *known {
            out.push(v(
                "a member of an older generation cannot commit",
                format!("group {group} accepted generation {generation} after generation {known}"),
                seq,
            ));
        }
    }
}
