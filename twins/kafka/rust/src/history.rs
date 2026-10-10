//! The twin's history: every operation of the contract's `Broker` ability the
//! twin performed, with its arguments and what the model answered, in order.
//!
//! An operation is written as the ability call it is (`produceRequest`,
//! `joinGroupRequest`, ...), so a history can be replayed through a fresh
//! model, checked against the contract's laws, and compared byte for byte
//! with another run's. Control-plane events (faults, clock advances, forced
//! states) are entries of their own; replay skips them. ref:DEC-history-as-ability-calls

use crate::model::{ClusterModel, Failure, data};
use base64::Engine as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Bytes, written as a string when they are printable UTF-8 and as
/// `{"base64": ...}` otherwise.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Octets(pub Vec<u8>);

impl Serialize for Octets {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match std::str::from_utf8(&self.0) {
            Ok(text) if !text.chars().any(|c| c.is_control()) => s.serialize_str(text),
            _ => {
                use serde::ser::SerializeMap;
                let mut m = s.serialize_map(Some(1))?;
                m.serialize_entry("base64", &base64::engine::general_purpose::STANDARD.encode(&self.0))?;
                m.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Octets {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Form {
            Text(String),
            Encoded { base64: String },
        }
        match Form::deserialize(d)? {
            Form::Text(t) => Ok(Octets(t.into_bytes())),
            Form::Encoded { base64 } => base64::engine::general_purpose::STANDARD
                .decode(base64)
                .map(Octets)
                .map_err(serde::de::Error::custom),
        }
    }
}

/// A topic and partition.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Tp {
    /// The topic.
    pub topic: String,
    /// The partition.
    pub partition: i32,
}

/// A record header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    /// Its name.
    pub name: String,
    /// Its content, if any.
    #[serde(default)]
    pub content: Option<Octets>,
}

/// A record, as a producer sends it.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Record {
    /// Its key, if any.
    #[serde(default)]
    pub key: Option<Octets>,
    /// Its value, if any.
    #[serde(default)]
    pub value: Option<Octets>,
    /// Its headers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<Header>,
}

/// What an idempotent or transactional producer stamps on a batch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProducerTag {
    /// The producer id.
    pub producer_id: i64,
    /// The producer's epoch.
    pub producer_epoch: i16,
    /// The sequence number of the batch's first record.
    pub base_sequence: i32,
}

/// An offset to commit for a partition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OffsetEntry {
    /// The topic.
    pub topic: String,
    /// The partition.
    pub partition: i32,
    /// The offset.
    pub offset: i64,
}

/// Read uncommitted or read committed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Isolation {
    /// Every record up to the high watermark.
    ReadUncommitted,
    /// Only committed records, up to the last stable offset.
    ReadCommitted,
}

/// The earliest or the latest offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OffsetSpec {
    /// The log start offset.
    Earliest,
    /// The high watermark, or the last stable offset when reading committed.
    Latest,
}

/// One operation of the contract's `Broker` ability, with its arguments.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[allow(clippy::enum_variant_names, missing_docs)]
pub enum Op {
    /// CreateTopics: a topic with this many partitions.
    CreateTopicsRequest { name: String, partitions: i32 },
    /// CreatePartitions: grow a topic to this many partitions.
    CreatePartitionsRequest { name: String, total: i32 },
    /// Metadata: a topic's partition count.
    MetadataRequest { name: String },
    /// Produce: a batch to one partition.
    ProduceRequest { tp: Tp, tag: Option<ProducerTag>, records: Vec<Record> },
    /// Fetch: records from an offset.
    FetchRequest { tp: Tp, offset: i64, max_records: i32, isolation: Isolation },
    /// ListOffsets: the earliest or latest offset.
    ListOffsetsRequest { tp: Tp, spec: OffsetSpec, isolation: Isolation },
    /// InitProducerId: a producer id and epoch.
    InitProducerIdRequest { transactional_id: Option<String> },
    /// AddPartitionsToTxn.
    AddPartitionsToTxnRequest { transactional_id: String, producer_id: i64, producer_epoch: i16, partitions: Vec<Tp> },
    /// AddOffsetsToTxn and TxnOffsetCommit.
    TxnOffsetCommitRequest { transactional_id: String, producer_id: i64, producer_epoch: i16, group: String, offsets: Vec<OffsetEntry> },
    /// EndTxn: commit or abort.
    EndTxnRequest { transactional_id: String, producer_id: i64, producer_epoch: i16, commit: bool },
    /// JoinGroup.
    JoinGroupRequest { group: String, member: String, topics: Vec<String> },
    /// SyncGroup.
    SyncGroupRequest { group: String, member: String, generation: i32 },
    /// Heartbeat.
    HeartbeatRequest { group: String, member: String, generation: i32 },
    /// LeaveGroup.
    LeaveGroupRequest { group: String, member: String },
    /// DescribeGroups.
    DescribeGroupRequest { group: String },
    /// OffsetCommit.
    OffsetCommitRequest { group: String, member: String, generation: i32, offsets: Vec<OffsetEntry> },
    /// OffsetFetch.
    OffsetFetchRequest { group: String, tp: Tp },
}

impl Op {
    /// The operation's name in the contract.
    pub fn name(&self) -> &'static str {
        match self {
            Op::CreateTopicsRequest { .. } => "createTopicsRequest",
            Op::CreatePartitionsRequest { .. } => "createPartitionsRequest",
            Op::MetadataRequest { .. } => "metadataRequest",
            Op::ProduceRequest { .. } => "produceRequest",
            Op::FetchRequest { .. } => "fetchRequest",
            Op::ListOffsetsRequest { .. } => "listOffsetsRequest",
            Op::InitProducerIdRequest { .. } => "initProducerIdRequest",
            Op::AddPartitionsToTxnRequest { .. } => "addPartitionsToTxnRequest",
            Op::TxnOffsetCommitRequest { .. } => "txnOffsetCommitRequest",
            Op::EndTxnRequest { .. } => "endTxnRequest",
            Op::JoinGroupRequest { .. } => "joinGroupRequest",
            Op::SyncGroupRequest { .. } => "syncGroupRequest",
            Op::HeartbeatRequest { .. } => "heartbeatRequest",
            Op::LeaveGroupRequest { .. } => "leaveGroupRequest",
            Op::DescribeGroupRequest { .. } => "describeGroupRequest",
            Op::OffsetCommitRequest { .. } => "offsetCommitRequest",
            Op::OffsetFetchRequest { .. } => "offsetFetchRequest",
        }
    }
}

/// A record a fetch returned, with its offset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fetched {
    /// Its offset.
    pub offset: i64,
    /// The record.
    #[serde(flatten)]
    pub record: Record,
}

/// What a fetch answered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchOutcome {
    /// The records, in offset order.
    pub records: Vec<Fetched>,
    /// Where the next fetch starts.
    pub next_offset: i64,
    /// The high watermark.
    pub high_watermark: i64,
    /// The last stable offset.
    pub last_stable_offset: i64,
}

/// A member's partitions in a group view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberView {
    /// The member id.
    pub member: String,
    /// Its partitions.
    pub assigned: Vec<Tp>,
}

/// What the model answered an operation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Outcome {
    /// Nothing: the operation succeeded.
    Done,
    /// An offset, a generation or a partition count.
    Number(i64),
    /// A partition count or committed offset that may be absent.
    Maybe(Option<i64>),
    /// A producer id and epoch.
    Producer {
        /// The producer id.
        #[serde(rename = "producerId")]
        producer_id: i64,
        /// Its epoch.
        #[serde(rename = "producerEpoch")]
        producer_epoch: i16,
    },
    /// A fetch's records.
    Fetch(FetchOutcome),
    /// A member's partitions.
    Partitions(Vec<Tp>),
    /// A group's generation and members.
    Group {
        /// The generation.
        generation: i32,
        /// Each member's partitions.
        members: Vec<MemberView>,
    },
}

/// What the model answered: a result, or the `KafkaError` the contract
/// refused the operation with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Answer {
    /// The operation succeeded with this result.
    #[serde(rename = "ok")]
    Ok(Outcome),
    /// The contract refused it with this error.
    #[serde(rename = "error")]
    Error(String),
}

impl Answer {
    /// The answer to an operation the model ran.
    pub fn of(result: &Result<Outcome, Failure>) -> Answer {
        match result {
            Ok(o) => Answer::Ok(o.clone()),
            Err(Failure::Kafka(k)) => Answer::Error(error_name(k)),
            Err(f @ Failure::Model(_)) => Answer::Error(f.to_string()),
        }
    }
}

/// Where an operation came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// A client's request on the Kafka port.
    Wire,
    /// A forced state.
    Force,
    /// A declared fault.
    Fault,
    /// The twin's coordinator: a session or rebalance that timed out.
    Coordinator,
    /// A replayed history.
    Replay,
    /// State a real broker already held when a recording of it started,
    /// written as the operations that reach it.
    Observed,
}

/// One entry of the history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Its position, from 1.
    pub seq: u64,
    /// The twin clock, in milliseconds since the Unix epoch.
    pub at: i64,
    /// What caused it.
    pub source: Source,
    /// An operation of the contract, or a control-plane event.
    #[serde(flatten)]
    pub event: Event,
}

/// An operation and its answer, or a control-plane event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Event {
    /// An operation of `Broker` and what the model answered.
    Call {
        /// The operation.
        #[serde(flatten)]
        op: Op,
        /// The answer.
        answer: Answer,
    },
    /// A control-plane event: a fault raised or cleared, the clock advanced,
    /// a forced state, a new seed.
    Control {
        /// What happened.
        control: String,
        /// Its details.
        #[serde(default)]
        detail: serde_json::Value,
    },
}

fn tp_native(tp: &Tp) -> data::TopicPartition {
    data::TopicPartition { topic: tp.topic.clone(), partition: tp.partition }
}

fn tp_of(tp: &data::TopicPartition) -> Tp {
    Tp { topic: tp.topic.clone(), partition: tp.partition }
}

/// The contract's record for a record.
pub fn record_native(r: &Record) -> data::Record {
    data::Record {
        key: r.key.as_ref().map(|o| o.0.clone()),
        value: r.value.as_ref().map(|o| o.0.clone()),
        headers: r
            .headers
            .iter()
            .map(|h| data::Header { name: h.name.clone(), content: h.content.as_ref().map(|o| o.0.clone()) })
            .collect(),
    }
}

/// A record of the contract, for the history.
pub fn record_of(r: &data::Record) -> Record {
    Record {
        key: r.key.clone().map(Octets),
        value: r.value.clone().map(Octets),
        headers: r
            .headers
            .iter()
            .map(|h| Header { name: h.name.clone(), content: h.content.clone().map(Octets) })
            .collect(),
    }
}

fn offsets_native(os: &[OffsetEntry]) -> Vec<data::OffsetEntry> {
    os.iter()
        .map(|o| data::OffsetEntry {
            topicPartition: data::TopicPartition { topic: o.topic.clone(), partition: o.partition },
            offset: o.offset,
        })
        .collect()
}

fn isolation_native(i: Isolation) -> data::IsolationLevel {
    match i {
        Isolation::ReadUncommitted => data::IsolationLevel::ReadUncommitted,
        Isolation::ReadCommitted => data::IsolationLevel::ReadCommitted,
    }
}

/// Runs one operation on the model, as the contract's `cluster` handler does.
pub fn apply(model: &mut ClusterModel, op: &Op) -> Result<Outcome, Failure> {
    Ok(match op {
        Op::CreateTopicsRequest { name, partitions } => {
            model.createTopicsRequest(name.clone(), *partitions)?;
            Outcome::Done
        }
        Op::CreatePartitionsRequest { name, total } => {
            model.createPartitionsRequest(name.clone(), *total)?;
            Outcome::Done
        }
        Op::MetadataRequest { name } => Outcome::Maybe(model.metadataRequest(name.clone())?.map(i64::from)),
        Op::ProduceRequest { tp, tag, records } => Outcome::Number(model.produceRequest(
            tp_native(tp),
            tag.as_ref().map(|t| data::ProducerTag {
                producerId: t.producer_id,
                producerEpoch: t.producer_epoch,
                baseSequence: t.base_sequence,
            }),
            records.iter().map(record_native).collect(),
        )?),
        Op::FetchRequest { tp, offset, max_records, isolation } => {
            let r = model.fetchRequest(tp_native(tp), *offset, *max_records, isolation_native(*isolation))?;
            Outcome::Fetch(FetchOutcome {
                records: r
                    .records
                    .iter()
                    .map(|f| Fetched { offset: f.offset, record: record_of(&f.entry) })
                    .collect(),
                next_offset: r.nextOffset,
                high_watermark: r.highWatermark,
                last_stable_offset: r.lastStableOffset,
            })
        }
        Op::ListOffsetsRequest { tp, spec, isolation } => Outcome::Number(model.listOffsetsRequest(
            tp_native(tp),
            match spec {
                OffsetSpec::Earliest => data::OffsetSpec::Earliest,
                OffsetSpec::Latest => data::OffsetSpec::Latest,
            },
            isolation_native(*isolation),
        )?),
        Op::InitProducerIdRequest { transactional_id } => {
            let p = model.initProducerIdRequest(transactional_id.clone())?;
            Outcome::Producer { producer_id: p.producerId, producer_epoch: p.producerEpoch }
        }
        Op::AddPartitionsToTxnRequest { transactional_id, producer_id, producer_epoch, partitions } => {
            model.addPartitionsToTxnRequest(
                transactional_id.clone(),
                *producer_id,
                *producer_epoch,
                partitions.iter().map(tp_native).collect(),
            )?;
            Outcome::Done
        }
        Op::TxnOffsetCommitRequest { transactional_id, producer_id, producer_epoch, group, offsets } => {
            model.txnOffsetCommitRequest(
                transactional_id.clone(),
                *producer_id,
                *producer_epoch,
                group.clone(),
                offsets_native(offsets),
            )?;
            Outcome::Done
        }
        Op::EndTxnRequest { transactional_id, producer_id, producer_epoch, commit } => {
            model.endTxnRequest(transactional_id.clone(), *producer_id, *producer_epoch, *commit)?;
            Outcome::Done
        }
        Op::JoinGroupRequest { group, member, topics } => {
            Outcome::Number(model.joinGroupRequest(group.clone(), member.clone(), topics.clone())?.into())
        }
        Op::SyncGroupRequest { group, member, generation } => Outcome::Partitions(
            model.syncGroupRequest(group.clone(), member.clone(), *generation)?.iter().map(tp_of).collect(),
        ),
        Op::HeartbeatRequest { group, member, generation } => {
            model.heartbeatRequest(group.clone(), member.clone(), *generation)?;
            Outcome::Done
        }
        Op::LeaveGroupRequest { group, member } => {
            model.leaveGroupRequest(group.clone(), member.clone())?;
            Outcome::Done
        }
        Op::DescribeGroupRequest { group } => {
            let v = model.describeGroupRequest(group.clone())?;
            Outcome::Group {
                generation: v.generation,
                members: v
                    .members
                    .iter()
                    .map(|m| MemberView { member: m.memberId.clone(), assigned: m.assigned.iter().map(tp_of).collect() })
                    .collect(),
            }
        }
        Op::OffsetCommitRequest { group, member, generation, offsets } => {
            model.offsetCommitRequest(group.clone(), member.clone(), *generation, offsets_native(offsets))?;
            Outcome::Done
        }
        Op::OffsetFetchRequest { group, tp } => Outcome::Maybe(model.offsetFetchRequest(group.clone(), tp_native(tp))?),
    })
}

/// The name of a contract error, as the history writes it.
pub fn error_name(e: &data::KafkaError) -> String {
    format!("{e:?}")
}

/// One difference between a recorded history and its replay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Divergence {
    /// The entry's position.
    pub seq: u64,
    /// The operation.
    pub op: String,
    /// What the history recorded.
    pub recorded: serde_json::Value,
    /// What the replay gave.
    pub replayed: serde_json::Value,
}

/// Replays a history's operations through a fresh model and reports every
/// answer that differs from the recorded one. Control entries are skipped.
pub fn replay(entries: &[Entry]) -> (ClusterModel, Vec<Divergence>) {
    let mut model = ClusterModel::start();
    let mut divergences = vec![];
    for e in entries {
        if let Event::Call { op, answer } = &e.event {
            let recorded = serde_json::to_value(answer).unwrap_or_default();
            let replayed = serde_json::to_value(Answer::of(&apply(&mut model, op))).unwrap_or_default();
            if recorded != replayed {
                divergences.push(Divergence { seq: e.seq, op: op.name().into(), recorded, replayed });
            }
        }
    }
    (model, divergences)
}
