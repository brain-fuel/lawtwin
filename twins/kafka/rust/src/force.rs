//! Forcing a state: a test says what it needs the cluster to hold, and the
//! twin reaches it through the contract's own operations.
//!
//! A forced state is checked first: one that no sequence of the contract's
//! operations reaches is refused with the law that forbids it, and nothing
//! is applied. A state that passes is applied to a copy of the model, every
//! operation as the contract states it; if the model refuses one, or the
//! result breaks an invariant, the copy is dropped and the twin is as it
//! was. Otherwise the copy becomes the twin's state, and each operation is
//! written to the history with source `force`. ref:DEC-force-through-operations

use crate::history::{self, Answer, Header, Octets, Op, OffsetEntry, Outcome, ProducerTag, Record, Source, Tp};
use crate::laws::{check_state, valid_topic_name};
use crate::model::{ClusterModel, Failure};
use crate::twin::Core;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A topic to hold.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TopicSpec {
    /// Its name.
    pub name: String,
    /// Its partition count; a topic that exists grows to it.
    pub partitions: i32,
}

/// A record to write, as a forced state gives it.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordSpec {
    /// Its key.
    #[serde(default)]
    pub key: Option<Octets>,
    /// Its value.
    #[serde(default)]
    pub value: Option<Octets>,
    /// Its headers.
    #[serde(default)]
    pub headers: BTreeMap<String, Option<Octets>>,
}

impl RecordSpec {
    fn record(&self) -> Record {
        Record {
            key: self.key.clone(),
            value: self.value.clone(),
            headers: self.headers.iter().map(|(k, v)| Header { name: k.clone(), content: v.clone() }).collect(),
        }
    }
}

/// Records to write to one partition.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RecordsSpec {
    /// The topic.
    pub topic: String,
    /// The partition.
    #[serde(default)]
    pub partition: i32,
    /// The offset the first record must get: the partition's log end.
    #[serde(default)]
    pub offset: Option<i64>,
    /// The records.
    #[serde(default)]
    pub records: Vec<RecordSpec>,
    /// Instead of records, write numbered ones until the log ends here.
    #[serde(default)]
    pub log_end_offset: Option<i64>,
}

/// A transactional producer at an epoch.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProducerSpec {
    /// Its transactional id.
    pub transactional_id: String,
    /// The epoch it must have.
    pub epoch: i16,
}

/// How a forced transaction ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TxnEnd {
    /// It stays open.
    Open,
    /// It commits.
    Commit,
    /// It aborts.
    Abort,
}

/// A record written in a transaction.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TxnRecord {
    /// The topic.
    pub topic: String,
    /// The partition.
    #[serde(default)]
    pub partition: i32,
    /// The record.
    #[serde(flatten)]
    pub record: RecordSpec,
}

/// A transaction.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TxnSpec {
    /// Its transactional id; the producer is initialised again, which
    /// aborts any transaction it had open.
    pub transactional_id: String,
    /// The records it writes.
    #[serde(default)]
    pub records: Vec<TxnRecord>,
    /// The group whose offsets it sends, with them.
    #[serde(default)]
    pub group: Option<String>,
    /// The offsets it sends.
    #[serde(default)]
    pub offsets: Vec<OffsetEntry>,
    /// How it ends.
    pub end: TxnEnd,
}

/// A group member.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemberSpec {
    /// Its member id.
    pub member: String,
    /// The topics it subscribes to.
    pub topics: Vec<String>,
}

/// A consumer group.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GroupSpec {
    /// The group id.
    pub group: String,
    /// Its members.
    pub members: Vec<MemberSpec>,
    /// The assignment the test expects: each member's partitions. The
    /// coordinator computes the assignment; this only checks it.
    #[serde(default)]
    pub assignment: Option<BTreeMap<String, Vec<Tp>>>,
}

/// A committed offset.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommitSpec {
    /// The group.
    pub group: String,
    /// The topic.
    pub topic: String,
    /// The partition.
    #[serde(default)]
    pub partition: i32,
    /// The offset.
    pub offset: i64,
}

/// A state to force. Each part is reached in this order: topics, records,
/// producers, transactions, groups, committed offsets.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ForceDoc {
    /// Start from an empty cluster first.
    #[serde(default)]
    pub reset: bool,
    /// Topics to hold.
    #[serde(default)]
    pub topics: Vec<TopicSpec>,
    /// Records to write.
    #[serde(default)]
    pub records: Vec<RecordsSpec>,
    /// Transactional producers at an epoch.
    #[serde(default)]
    pub producers: Vec<ProducerSpec>,
    /// Transactions.
    #[serde(default)]
    pub transactions: Vec<TxnSpec>,
    /// Consumer groups.
    #[serde(default)]
    pub groups: Vec<GroupSpec>,
    /// Committed offsets.
    #[serde(default)]
    pub committed: Vec<CommitSpec>,
}

/// Why a forced state was refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    /// The law that forbids it.
    pub law: String,
    /// What in the forced state breaks it.
    pub reason: String,
}

fn refuse(law: &str, reason: String) -> Refusal {
    Refusal { law: law.into(), reason }
}

/// What forcing a state did.
#[derive(Clone, Debug, Serialize)]
pub struct Forced {
    /// The operations it ran, in order.
    pub operations: usize,
}

/// The law most directly behind a refusal of the model.
fn law_for(op: &Op, f: &Failure) -> &'static str {
    use crate::model::data::KafkaError::*;
    match f {
        Failure::Kafka(TopicAlreadyExists) => "a topic is created once",
        Failure::Kafka(InvalidTopic) => "an invalid topic name is refused",
        Failure::Kafka(InvalidPartitions) => "a topic's partitions only grow",
        Failure::Kafka(UnknownTopicOrPartition) => "a send to a partition the topic lacks is refused",
        Failure::Kafka(OutOfOrderSequenceNumber) => "a gap in sequence numbers is refused",
        Failure::Kafka(ProducerFenced | InvalidProducerEpoch) => "a new epoch fences the older producer",
        Failure::Kafka(IllegalGeneration) => "a commit from an older generation is refused",
        Failure::Kafka(InvalidTxnState) if matches!(op, Op::EndTxnRequest { .. }) => "a transaction must begin before it ends",
        _ => "offsets in a partition are dense and increasing",
    }
}

/// Checks a forced state against the current one, before anything is
/// applied.
fn check(doc: &ForceDoc, model: &ClusterModel) -> Result<(), Refusal> {
    let mut counts: BTreeMap<String, i32> = if doc.reset { BTreeMap::new() } else { model.topics() };
    for t in &doc.topics {
        if !valid_topic_name(&t.name) {
            return Err(refuse("an invalid topic name is refused", format!("{:?} is not a valid topic name", t.name)));
        }
        if !(1..=1024).contains(&t.partitions) {
            return Err(refuse(
                "a new topic has the partitions asked for",
                format!("{} would have {} partitions; a topic has 1 to 1024", t.name, t.partitions),
            ));
        }
        if let Some(now) = counts.get(&t.name) {
            if t.partitions < *now {
                return Err(refuse(
                    "a topic's partitions only grow",
                    format!("{} has {now} partitions and cannot have {}", t.name, t.partitions),
                ));
            }
        }
        counts.insert(t.name.clone(), t.partitions);
    }
    let known = |topic: &str, p: i32| counts.get(topic).is_some_and(|n| p >= 0 && p < *n);
    let lacks = |what: &str, topic: &str, p: i32| {
        refuse("a send to a partition the topic lacks is refused", format!("{what} {topic}-{p}, which does not exist"))
    };
    let mut ends: BTreeMap<(String, i32), i64> = BTreeMap::new();
    for r in &doc.records {
        if !known(&r.topic, r.partition) {
            return Err(lacks("records for", &r.topic, r.partition));
        }
        let key = (r.topic.clone(), r.partition);
        let now = match ends.get(&key) {
            Some(e) => *e,
            None if doc.reset => 0,
            None => {
                let mut m = model.clone();
                match history::apply(
                    &mut m,
                    &Op::ListOffsetsRequest {
                        tp: Tp { topic: r.topic.clone(), partition: r.partition },
                        spec: history::OffsetSpec::Latest,
                        isolation: history::Isolation::ReadUncommitted,
                    },
                ) {
                    Ok(Outcome::Number(e)) => e,
                    _ => 0,
                }
            }
        };
        if let Some(o) = r.offset {
            if o != now {
                return Err(refuse(
                    "offsets in a partition are dense and increasing",
                    format!("records for {}-{} would start at {o}, but the log ends at {now}", r.topic, r.partition),
                ));
            }
        }
        let end = match r.log_end_offset {
            Some(e) if e < now => {
                return Err(refuse(
                    "offsets in a partition are dense and increasing",
                    format!("{}-{} ends at {now} and a log never shrinks to {e}", r.topic, r.partition),
                ));
            }
            Some(e) => e,
            None => now + r.records.len() as i64,
        };
        if r.log_end_offset.is_some() && !r.records.is_empty() {
            return Err(refuse(
                "offsets in a partition are dense and increasing",
                format!("records for {}-{} give both records and a log end offset", r.topic, r.partition),
            ));
        }
        ends.insert(key, end);
    }
    for p in &doc.producers {
        if p.epoch < 0 {
            return Err(refuse("a new epoch fences the older producer", format!("{} cannot have epoch {}", p.transactional_id, p.epoch)));
        }
        if !doc.reset {
            if let Some((_, now)) = model.producer_of(&p.transactional_id) {
                if p.epoch < now {
                    return Err(refuse(
                        "a new epoch fences the older producer",
                        format!("{} is at epoch {now}; epochs only go up, so it cannot be at {}", p.transactional_id, p.epoch),
                    ));
                }
            }
        }
    }
    for t in &doc.transactions {
        for r in &t.records {
            if !known(&r.topic, r.partition) {
                return Err(lacks("a transaction writes to", &r.topic, r.partition));
            }
        }
        for o in &t.offsets {
            if !known(&o.topic, o.partition) {
                return Err(lacks("a transaction sends an offset for", &o.topic, o.partition));
            }
        }
        if !t.offsets.is_empty() && t.group.is_none() {
            return Err(refuse("offsets sent in a transaction are committed only with it", "offsets need a group".into()));
        }
        if t.records.is_empty() && t.offsets.is_empty() && t.end != TxnEnd::Open {
            return Err(refuse(
                "a transaction must begin before it ends",
                format!("the transaction of {} ends with nothing in it", t.transactional_id),
            ));
        }
    }
    for g in &doc.groups {
        if let Some(a) = &g.assignment {
            let mut owners: BTreeMap<&Tp, &String> = BTreeMap::new();
            for (m, tps) in a {
                for tp in tps {
                    if let Some(other) = owners.insert(tp, m) {
                        return Err(refuse(
                            "every partition has exactly one owner in a generation",
                            format!("in group {}, {}-{} would belong to {other} and {m}", g.group, tp.topic, tp.partition),
                        ));
                    }
                }
            }
            let topics: std::collections::BTreeSet<&String> = g.members.iter().flat_map(|m| &m.topics).collect();
            for t in topics {
                for p in 0..counts.get(t).copied().unwrap_or(0) {
                    let tp = Tp { topic: t.clone(), partition: p };
                    if !owners.contains_key(&tp) {
                        return Err(refuse(
                            "every partition has exactly one owner in a generation",
                            format!("in group {}, {t}-{p} would have no owner", g.group),
                        ));
                    }
                }
            }
        }
    }
    for c in &doc.committed {
        if !known(&c.topic, c.partition) {
            return Err(lacks("a committed offset for", &c.topic, c.partition));
        }
    }
    Ok(())
}

/// The operations that reach a forced state, run on a copy of the model.
fn run(doc: &ForceDoc, model: &mut ClusterModel, now: i64) -> Result<Vec<(Op, Answer, Vec<i64>)>, Refusal> {
    let mut done: Vec<(Op, Answer, Vec<i64>)> = vec![];
    let mut step = |model: &mut ClusterModel, op: Op, stamps: usize| -> Result<Outcome, Refusal> {
        let result = history::apply(model, &op);
        match result {
            Ok(o) => {
                done.push((op, Answer::Ok(o.clone()), vec![now; stamps]));
                Ok(o)
            }
            Err(f) => Err(refuse(law_for(&op, &f), format!("the contract refuses {}: {f}", op.name()))),
        }
    };
    let topics = model.topics();
    for t in &doc.topics {
        match topics.get(&t.name) {
            None => step(model, Op::CreateTopicsRequest { name: t.name.clone(), partitions: t.partitions }, 0)?,
            Some(n) if *n < t.partitions => {
                step(model, Op::CreatePartitionsRequest { name: t.name.clone(), total: t.partitions }, 0)?
            }
            Some(_) => Outcome::Done,
        };
    }
    for r in &doc.records {
        let tp = Tp { topic: r.topic.clone(), partition: r.partition };
        let records: Vec<Record> = match r.log_end_offset {
            Some(end) => {
                let now_end = match history::apply(
                    &mut model.clone(),
                    &Op::ListOffsetsRequest {
                        tp: tp.clone(),
                        spec: history::OffsetSpec::Latest,
                        isolation: history::Isolation::ReadUncommitted,
                    },
                ) {
                    Ok(Outcome::Number(e)) => e,
                    _ => 0,
                };
                (now_end..end)
                    .map(|o| Record { key: None, value: Some(Octets(format!("forced-{o}").into_bytes())), headers: vec![] })
                    .collect()
            }
            None => r.records.iter().map(RecordSpec::record).collect(),
        };
        for chunk in records.chunks(10_000) {
            let n = chunk.len();
            step(model, Op::ProduceRequest { tp: tp.clone(), tag: None, records: chunk.to_vec() }, n)?;
        }
    }
    for p in &doc.producers {
        loop {
            let at = model.producer_of(&p.transactional_id).map(|(_, e)| e);
            if at.is_some_and(|e| e >= p.epoch) {
                break;
            }
            step(model, Op::InitProducerIdRequest { transactional_id: Some(p.transactional_id.clone()) }, 0)?;
        }
    }
    for t in &doc.transactions {
        let (pid, epoch) =
            match step(model, Op::InitProducerIdRequest { transactional_id: Some(t.transactional_id.clone()) }, 0)? {
                Outcome::Producer { producer_id, producer_epoch } => (producer_id, producer_epoch),
                _ => unreachable!("InitProducerId gives a producer"),
            };
        let mut tps: Vec<Tp> = vec![];
        for r in &t.records {
            let tp = Tp { topic: r.topic.clone(), partition: r.partition };
            if !tps.contains(&tp) {
                tps.push(tp);
            }
        }
        if !tps.is_empty() {
            step(
                model,
                Op::AddPartitionsToTxnRequest {
                    transactional_id: t.transactional_id.clone(),
                    producer_id: pid,
                    producer_epoch: epoch,
                    partitions: tps.clone(),
                },
                0,
            )?;
        }
        let mut sequences: BTreeMap<Tp, i32> = BTreeMap::new();
        for r in &t.records {
            let tp = Tp { topic: r.topic.clone(), partition: r.partition };
            let s = sequences.entry(tp.clone()).or_insert(0);
            let tag = ProducerTag { producer_id: pid, producer_epoch: epoch, base_sequence: *s };
            *s += 1;
            step(model, Op::ProduceRequest { tp, tag: Some(tag), records: vec![r.record.record()] }, 1)?;
        }
        if let Some(g) = &t.group {
            step(
                model,
                Op::TxnOffsetCommitRequest {
                    transactional_id: t.transactional_id.clone(),
                    producer_id: pid,
                    producer_epoch: epoch,
                    group: g.clone(),
                    offsets: t.offsets.clone(),
                },
                0,
            )?;
        }
        if t.end != TxnEnd::Open {
            step(
                model,
                Op::EndTxnRequest {
                    transactional_id: t.transactional_id.clone(),
                    producer_id: pid,
                    producer_epoch: epoch,
                    commit: t.end == TxnEnd::Commit,
                },
                0,
            )?;
        }
    }
    for g in &doc.groups {
        for m in &g.members {
            step(model, Op::JoinGroupRequest { group: g.group.clone(), member: m.member.clone(), topics: m.topics.clone() }, 0)?;
        }
        if let Some(want) = &g.assignment {
            let got = match history::apply(&mut model.clone(), &Op::DescribeGroupRequest { group: g.group.clone() }) {
                Ok(Outcome::Group { members, .. }) => members,
                _ => vec![],
            };
            for member in &got {
                let mut a = member.assigned.clone();
                let mut b = want.get(&member.member).cloned().unwrap_or_default();
                a.sort();
                b.sort();
                if a != b {
                    return Err(refuse(
                        "the coordinator assigns every partition exactly once",
                        format!(
                            "in group {}, the coordinator gives {} {:?}, not {:?}",
                            g.group,
                            member.member,
                            a.iter().map(|t| format!("{}-{}", t.topic, t.partition)).collect::<Vec<_>>(),
                            b.iter().map(|t| format!("{}-{}", t.topic, t.partition)).collect::<Vec<_>>()
                        ),
                    ));
                }
            }
        }
    }
    for c in &doc.committed {
        let view = history::apply(&mut model.clone(), &Op::DescribeGroupRequest { group: c.group.clone() });
        let (member, generation) = match view {
            Ok(Outcome::Group { generation, members }) if !members.is_empty() => (members[0].member.clone(), generation),
            _ => (String::new(), -1),
        };
        step(
            model,
            Op::OffsetCommitRequest {
                group: c.group.clone(),
                member,
                generation,
                offsets: vec![OffsetEntry { topic: c.topic.clone(), partition: c.partition, offset: c.offset }],
            },
            0,
        )?;
    }
    Ok(done)
}

/// Forces a state on the twin, or refuses it with the law that forbids it.
pub fn force(core: &mut Core, doc: &ForceDoc) -> Result<Forced, Refusal> {
    let base = if doc.reset { ClusterModel::start() } else { core.model.clone() };
    check(doc, &base)?;
    let mut trial = base;
    let done = run(doc, &mut trial, core.clock.now_ms())?;
    if let Some(broken) = check_state(&trial.snapshot()).into_iter().next() {
        return Err(refuse(&broken.law, broken.detail));
    }
    if doc.reset {
        core.timestamps.clear();
        core.groups.clear();
        core.control(Source::Force, "reset", serde_json::json!({ "seed": core.seed }));
    }
    core.model = trial;
    // Forced members are members like any other: they leave when their
    // session times out on the twin clock, or when a rebalance finds they
    // did not rejoin.
    let now = core.clock.now_ms();
    for g in &doc.groups {
        let gw = core.groups.entry(g.group.clone()).or_default();
        gw.protocol_type.get_or_insert_with(|| "consumer".into());
        for m in &g.members {
            let mut topics = m.topics.clone();
            topics.sort();
            topics.dedup();
            gw.members.insert(
                m.member.clone(),
                crate::group::MemberWire {
                    client_id: "lawtwin-force".into(),
                    client_host: "/lawtwin".into(),
                    instance_id: None,
                    protocols: vec![],
                    topics,
                    session_timeout_ms: 10_000,
                    last_seen_ms: now,
                    in_model: true,
                },
            );
        }
    }
    let operations = done.len();
    for (op, answer, stamps) in done {
        if let (Op::ProduceRequest { tp, .. }, Answer::Ok(Outcome::Number(base))) = (&op, &answer) {
            core.stamp(&tp.topic, tp.partition, *base, &stamps);
        }
        let result = match &answer {
            Answer::Ok(o) => Ok(o.clone()),
            Answer::Error(e) => Err(Failure::Model(e.clone())),
        };
        core.record(Source::Force, op, &result);
    }
    Ok(Forced { operations })
}
