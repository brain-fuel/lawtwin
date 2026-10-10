//! Decoding the wire into the history: each request and response a broker
//! exchanged becomes the operations of the contract's `Broker` ability it
//! performed, with the broker's answers, in the history format every twin
//! writes (ref:DEC-history-as-ability-calls).
//!
//! The decoder mirrors how the twin maps requests onto operations
//! (`lawtwin_kafka::wire` and `group`), so that a trace recorded from a real
//! broker and a history written by a twin say the same thing in the same
//! words. It reads only what crossed the wire, so it decodes a real
//! broker's traffic and a twin core's alike. ref:DEC-recorded-history

use crate::frame::{Batch, Frame, batches};
use bytes::{Buf, Bytes};
use kafka_protocol::messages::*;
use kafka_protocol::protocol::Decodable;
use lawtwin_kafka::history::{
    Answer, Fetched, FetchOutcome, Header, Isolation, MemberView, Octets, OffsetEntry, OffsetSpec, Op, Outcome, ProducerTag,
    Record, Source, Tp,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// What a frame decodes to: an operation and the broker's answer, or a note
/// about the wire that is not an operation of the contract.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// An operation of `Broker` and the broker's answer.
    Call {
        /// Who caused it.
        source: Source,
        /// The operation.
        op: Op,
        /// The broker's answer.
        answer: Answer,
    },
    /// Something the wire showed that no operation states: a retriable
    /// error, a request the contract does not model, a fault the recorder
    /// applied.
    Note {
        /// Who caused it.
        source: Source,
        /// What happened.
        control: String,
        /// Its details.
        detail: Value,
    },
}

/// The contract's name for a Kafka error code, when the contract states it.
pub fn contract_error(code: i16) -> Option<&'static str> {
    Some(match code {
        3 => "UnknownTopicOrPartition",
        36 => "TopicAlreadyExists",
        17 => "InvalidTopic",
        37 => "InvalidPartitions",
        87 => "InvalidRecord",
        42 => "InvalidRequest",
        1 => "OffsetOutOfRange",
        45 => "OutOfOrderSequenceNumber",
        47 => "InvalidProducerEpoch",
        90 => "ProducerFenced",
        59 => "UnknownProducerId",
        49 => "InvalidProducerIdMapping",
        48 => "InvalidTxnState",
        24 => "InvalidGroupId",
        25 => "UnknownMemberId",
        22 => "IllegalGeneration",
        _ => return None,
    })
}

/// The Kafka error code a contract error is answered with.
pub fn contract_code(name: &str) -> Option<i16> {
    Some(match name {
        "UnknownTopicOrPartition" => 3,
        "TopicAlreadyExists" => 36,
        "InvalidTopic" => 17,
        "InvalidPartitions" => 37,
        "InvalidRecord" => 87,
        "InvalidRequest" => 42,
        "OffsetOutOfRange" => 1,
        "OutOfOrderSequenceNumber" => 45,
        "InvalidProducerEpoch" => 47,
        "ProducerFenced" => 90,
        "UnknownProducerId" => 59,
        "InvalidProducerIdMapping" => 49,
        "InvalidTxnState" | "IllegalState" => 48,
        "InvalidGroupId" => 24,
        "UnknownMemberId" => 25,
        "IllegalGeneration" => 22,
        _ => return None,
    })
}

/// Kafka's name for an error code.
pub fn kafka_error_name(code: i16) -> String {
    let name = match code {
        -1 => "UNKNOWN_SERVER_ERROR",
        0 => "NONE",
        1 => "OFFSET_OUT_OF_RANGE",
        2 => "CORRUPT_MESSAGE",
        3 => "UNKNOWN_TOPIC_OR_PARTITION",
        5 => "LEADER_NOT_AVAILABLE",
        6 => "NOT_LEADER_OR_FOLLOWER",
        7 => "REQUEST_TIMED_OUT",
        13 => "NETWORK_EXCEPTION",
        14 => "COORDINATOR_LOAD_IN_PROGRESS",
        15 => "COORDINATOR_NOT_AVAILABLE",
        16 => "NOT_COORDINATOR",
        17 => "INVALID_TOPIC_EXCEPTION",
        19 => "NOT_ENOUGH_REPLICAS",
        20 => "NOT_ENOUGH_REPLICAS_AFTER_APPEND",
        22 => "ILLEGAL_GENERATION",
        23 => "INCONSISTENT_GROUP_PROTOCOL",
        24 => "INVALID_GROUP_ID",
        25 => "UNKNOWN_MEMBER_ID",
        26 => "INVALID_SESSION_TIMEOUT",
        27 => "REBALANCE_IN_PROGRESS",
        35 => "UNSUPPORTED_VERSION",
        36 => "TOPIC_ALREADY_EXISTS",
        37 => "INVALID_PARTITIONS",
        38 => "INVALID_REPLICATION_FACTOR",
        41 => "NOT_CONTROLLER",
        42 => "INVALID_REQUEST",
        45 => "OUT_OF_ORDER_SEQUENCE_NUMBER",
        46 => "DUPLICATE_SEQUENCE_NUMBER",
        47 => "INVALID_PRODUCER_EPOCH",
        48 => "INVALID_TXN_STATE",
        49 => "INVALID_PRODUCER_ID_MAPPING",
        50 => "INVALID_TRANSACTION_TIMEOUT",
        51 => "CONCURRENT_TRANSACTIONS",
        52 => "TRANSACTION_COORDINATOR_FENCED",
        55 => "OPERATION_NOT_ATTEMPTED",
        59 => "UNKNOWN_PRODUCER_ID",
        68 => "NON_EMPTY_GROUP",
        69 => "GROUP_ID_NOT_FOUND",
        74 => "FENCED_LEADER_EPOCH",
        75 => "UNKNOWN_LEADER_EPOCH",
        79 => "MEMBER_ID_REQUIRED",
        82 => "FENCED_INSTANCE_ID",
        87 => "INVALID_RECORD",
        89 => "THROTTLING_QUOTA_EXCEEDED",
        90 => "PRODUCER_FENCED",
        _ => return format!("KAFKA_ERROR_{code}"),
    };
    name.into()
}

/// Errors a client retries, which say nothing about the cluster's state:
/// the request never reached the log or the coordinator, or will be
/// answered again. ref:DEC-retriable-errors-not-answers
pub fn retriable(code: i16) -> bool {
    matches!(code, 5 | 6 | 7 | 13 | 14 | 15 | 16 | 19 | 20 | 51 | 74 | 75 | 89)
}

/// The answer to an operation the broker refused with a code.
pub fn refused(code: i16) -> Answer {
    Answer::Error(contract_error(code).map(str::to_string).unwrap_or_else(|| kafka_error_name(code)))
}

#[derive(Default, Debug)]
struct GroupState {
    /// The generation the decoder last wrote joins for.
    generation: i32,
    /// Members the contract's coordinator counts.
    in_model: BTreeSet<String>,
    /// Members of the round that made `generation`.
    round: BTreeSet<String>,
    /// Members whose JoinGroup answer for `generation` has been seen.
    answered: BTreeSet<String>,
    /// The leader of `generation`.
    leader: String,
}

/// Decodes frames into the history, frame by frame, in the order their
/// responses came back.
#[derive(Default)]
pub struct Decoder {
    topics: BTreeMap<String, i32>,
    pending_auto: BTreeSet<String>,
    groups: BTreeMap<String, GroupState>,
}

fn tag_of(b: &Batch) -> Option<ProducerTag> {
    (b.producer_id >= 0).then(|| ProducerTag {
        producer_id: b.producer_id,
        producer_epoch: b.producer_epoch,
        base_sequence: b.base_sequence,
    })
}

/// A record as the contract holds it.
pub fn record_of(r: &kafka_protocol::records::Record) -> Record {
    Record {
        key: r.key.as_ref().map(|k| Octets(k.to_vec())),
        value: r.value.as_ref().map(|v| Octets(v.to_vec())),
        headers: r.headers.iter().map(|(k, v)| Header { name: k.to_string(), content: v.as_ref().map(|v| Octets(v.to_vec())) }).collect(),
    }
}

/// The partitions an assignment in the consumer protocol's encoding names.
pub fn assignment_partitions(bytes: &Bytes) -> Vec<Tp> {
    if bytes.len() < 2 {
        return vec![];
    }
    let mut buf = bytes.clone();
    let version = buf.get_i16().clamp(0, 3);
    match ConsumerProtocolAssignment::decode(&mut buf, version) {
        Ok(a) => {
            let mut out: Vec<Tp> = a
                .assigned_partitions
                .iter()
                .flat_map(|t| t.partitions.iter().map(|p| Tp { topic: t.topic.0.to_string(), partition: *p }))
                .collect();
            out.sort();
            out
        }
        Err(_) => vec![],
    }
}

/// The request's header and body.
pub fn split_request(raw: &[u8]) -> anyhow::Result<(RequestHeader, Bytes)> {
    let mut buf = Bytes::copy_from_slice(raw);
    let header = kafka_protocol::protocol::decode_request_header_from_buffer(&mut buf)?;
    Ok((header, buf))
}

/// The response's header and body.
pub fn split_response(raw: &[u8], key: ApiKey, version: i16) -> anyhow::Result<(ResponseHeader, Bytes)> {
    let mut buf = Bytes::copy_from_slice(raw);
    let header = ResponseHeader::decode(&mut buf, key.response_header_version(version))?;
    Ok((header, buf))
}

fn note(control: &str, detail: Value) -> Item {
    Item::Note { source: Source::Wire, control: control.into(), detail }
}

fn call(op: Op, answer: Answer) -> Item {
    Item::Call { source: Source::Wire, op, answer }
}

fn retriable_note(api: &str, code: i16, what: Value) -> Item {
    note("retriable error", json!({ "api": api, "error": kafka_error_name(code), "of": what }))
}

impl Decoder {
    /// A decoder that knows nothing of the cluster yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The current generation the decoder knows for a group.
    pub fn generation(&self, group: &str) -> Option<i32> {
        self.groups.get(group).map(|g| g.generation)
    }

    /// The leader of a group's current generation, as its JoinGroup answers
    /// named it.
    pub fn leader(&self, group: &str) -> Option<&str> {
        self.groups.get(group).map(|g| g.leader.as_str()).filter(|l| !l.is_empty())
    }

    /// The operations a frame performed, with the broker's answers.
    pub fn frame(&mut self, f: &Frame) -> Vec<Item> {
        let Ok(key) = ApiKey::try_from(f.key) else {
            return vec![note("unmodelled request", json!({ "key": f.key, "version": f.version }))];
        };
        let Some(response) = f.answer_for_history() else {
            return match key {
                ApiKey::Produce => vec![note("unacknowledged produce", json!({ "n": f.n }))],
                _ => vec![],
            };
        };
        match self.decode(key, f.version, &f.request, response) {
            Ok(items) => items,
            Err(e) => vec![note("undecodable frame", json!({ "n": f.n, "api": f.api, "error": e.to_string() }))],
        }
    }

    fn decode(&mut self, key: ApiKey, v: i16, request: &[u8], response: &[u8]) -> anyhow::Result<Vec<Item>> {
        let (_, mut req) = split_request(request)?;
        let (_, mut resp) = split_response(response, key, v)?;
        Ok(match key {
            ApiKey::Metadata => self.metadata(MetadataRequest::decode(&mut req, v)?, MetadataResponse::decode(&mut resp, v)?, v),
            ApiKey::CreateTopics => self.create_topics(CreateTopicsRequest::decode(&mut req, v)?, CreateTopicsResponse::decode(&mut resp, v)?),
            ApiKey::CreatePartitions => {
                self.create_partitions(CreatePartitionsRequest::decode(&mut req, v)?, CreatePartitionsResponse::decode(&mut resp, v)?)
            }
            ApiKey::Produce => produce(ProduceRequest::decode(&mut req, v)?, ProduceResponse::decode(&mut resp, v)?)?,
            ApiKey::Fetch => fetch(FetchRequest::decode(&mut req, v)?, FetchResponse::decode(&mut resp, v)?)?,
            ApiKey::ListOffsets => list_offsets(ListOffsetsRequest::decode(&mut req, v)?, ListOffsetsResponse::decode(&mut resp, v)?, v),
            ApiKey::OffsetCommit => offset_commit(OffsetCommitRequest::decode(&mut req, v)?, OffsetCommitResponse::decode(&mut resp, v)?),
            ApiKey::OffsetFetch => offset_fetch(OffsetFetchRequest::decode(&mut req, v)?, OffsetFetchResponse::decode(&mut resp, v)?),
            ApiKey::InitProducerId => {
                let rq = InitProducerIdRequest::decode(&mut req, v)?;
                let rs = InitProducerIdResponse::decode(&mut resp, v)?;
                let op = Op::InitProducerIdRequest { transactional_id: rq.transactional_id.map(|t| t.0.to_string()) };
                if retriable(rs.error_code) {
                    vec![retriable_note("InitProducerId", rs.error_code, serde_json::to_value(&op)?)]
                } else if rs.error_code != 0 {
                    vec![call(op, refused(rs.error_code))]
                } else {
                    vec![call(op, Answer::Ok(Outcome::Producer { producer_id: rs.producer_id.0, producer_epoch: rs.producer_epoch }))]
                }
            }
            ApiKey::AddPartitionsToTxn => {
                let rq = AddPartitionsToTxnRequest::decode(&mut req, v)?;
                let rs = AddPartitionsToTxnResponse::decode(&mut resp, v)?;
                let partitions: Vec<Tp> = rq
                    .v3_and_below_topics
                    .iter()
                    .flat_map(|t| t.partitions.iter().map(|p| Tp { topic: t.name.0.to_string(), partition: *p }))
                    .collect();
                let op = Op::AddPartitionsToTxnRequest {
                    transactional_id: rq.v3_and_below_transactional_id.0.to_string(),
                    producer_id: rq.v3_and_below_producer_id.0,
                    producer_epoch: rq.v3_and_below_producer_epoch,
                    partitions,
                };
                let codes: Vec<i16> = rs
                    .results_by_topic_v3_and_below
                    .iter()
                    .flat_map(|t| t.results_by_partition.iter().map(|p| p.partition_error_code))
                    .collect();
                let code = codes.iter().copied().find(|c| *c != 0 && *c != 55).unwrap_or(0);
                single("AddPartitionsToTxn", op, fenced(code, key, v))?
            }
            ApiKey::AddOffsetsToTxn => {
                let rq = AddOffsetsToTxnRequest::decode(&mut req, v)?;
                let rs = AddOffsetsToTxnResponse::decode(&mut resp, v)?;
                let op = Op::TxnOffsetCommitRequest {
                    transactional_id: rq.transactional_id.0.to_string(),
                    producer_id: rq.producer_id.0,
                    producer_epoch: rq.producer_epoch,
                    group: rq.group_id.0.to_string(),
                    offsets: vec![],
                };
                single("AddOffsetsToTxn", op, fenced(rs.error_code, key, v))?
            }
            ApiKey::TxnOffsetCommit => {
                let rq = TxnOffsetCommitRequest::decode(&mut req, v)?;
                let rs = TxnOffsetCommitResponse::decode(&mut resp, v)?;
                let offsets = rq
                    .topics
                    .iter()
                    .flat_map(|t| {
                        t.partitions.iter().map(|p| OffsetEntry { topic: t.name.0.to_string(), partition: p.partition_index, offset: p.committed_offset })
                    })
                    .collect();
                let op = Op::TxnOffsetCommitRequest {
                    transactional_id: rq.transactional_id.0.to_string(),
                    producer_id: rq.producer_id.0,
                    producer_epoch: rq.producer_epoch,
                    group: rq.group_id.0.to_string(),
                    offsets,
                };
                let code = rs.topics.iter().flat_map(|t| t.partitions.iter().map(|p| p.error_code)).find(|c| *c != 0).unwrap_or(0);
                single("TxnOffsetCommit", op, fenced(code, key, v))?
            }
            ApiKey::EndTxn => {
                let rq = EndTxnRequest::decode(&mut req, v)?;
                let rs = EndTxnResponse::decode(&mut resp, v)?;
                let op = Op::EndTxnRequest {
                    transactional_id: rq.transactional_id.0.to_string(),
                    producer_id: rq.producer_id.0,
                    producer_epoch: rq.producer_epoch,
                    commit: rq.committed,
                };
                single("EndTxn", op, fenced(rs.error_code, key, v))?
            }
            ApiKey::JoinGroup => self.join(JoinGroupRequest::decode(&mut req, v)?, JoinGroupResponse::decode(&mut resp, v)?),
            ApiKey::SyncGroup => self.sync(SyncGroupRequest::decode(&mut req, v)?, SyncGroupResponse::decode(&mut resp, v)?),
            ApiKey::Heartbeat => self.heartbeat(HeartbeatRequest::decode(&mut req, v)?, HeartbeatResponse::decode(&mut resp, v)?),
            ApiKey::LeaveGroup => self.leave(LeaveGroupRequest::decode(&mut req, v)?, LeaveGroupResponse::decode(&mut resp, v)?, v),
            ApiKey::DescribeGroups => self.describe(DescribeGroupsResponse::decode(&mut resp, v)?),
            ApiKey::ApiVersions | ApiKey::FindCoordinator | ApiKey::ListGroups | ApiKey::DescribeConfigs => vec![],
            other => vec![note("unmodelled request", json!({ "api": format!("{other:?}"), "version": v }))],
        })
    }

    fn metadata(&mut self, rq: MetadataRequest, rs: MetadataResponse, v: i16) -> Vec<Item> {
        let explicit: Option<BTreeSet<String>> =
            rq.topics.as_ref().map(|ts| ts.iter().filter_map(|t| t.name.as_ref().map(|n| n.0.to_string())).collect());
        let auto = v < 4 || rq.allow_auto_topic_creation;
        let mut out = vec![];
        // In name order, as the twin lists its topics.
        let mut topics: Vec<&metadata_response::MetadataResponseTopic> = rs.topics.iter().collect();
        topics.sort_by(|a, b| a.name.as_ref().map(|n| n.0.as_str()).cmp(&b.name.as_ref().map(|n| n.0.as_str())));
        for t in topics {
            if t.is_internal {
                continue;
            }
            let Some(name) = t.name.as_ref().map(|n| n.0.to_string()) else { continue };
            let asked = explicit.as_ref().is_some_and(|e| e.contains(&name));
            match t.error_code {
                0 => {
                    let n = t.partitions.len() as i32;
                    match self.topics.get(&name).copied() {
                        Some(known) if known == n => {}
                        Some(_) => out.push(Item::Call {
                            source: Source::Observed,
                            op: Op::CreatePartitionsRequest { name: name.clone(), total: n },
                            answer: Answer::Ok(Outcome::Done),
                        }),
                        None if asked && auto => {
                            // Created by this request, or by one that saw it
                            // pending: as the twin writes an auto-creation.
                            self.pending_auto.remove(&name);
                            out.push(call(Op::MetadataRequest { name: name.clone() }, Answer::Ok(Outcome::Maybe(None))));
                            out.push(call(Op::CreateTopicsRequest { name: name.clone(), partitions: n }, Answer::Ok(Outcome::Done)));
                        }
                        None => out.push(Item::Call {
                            source: Source::Observed,
                            op: Op::CreateTopicsRequest { name: name.clone(), partitions: n },
                            answer: Answer::Ok(Outcome::Done),
                        }),
                    }
                    self.topics.insert(name.clone(), n);
                    out.push(call(Op::MetadataRequest { name }, Answer::Ok(Outcome::Maybe(Some(n as i64)))));
                }
                3 if !self.topics.contains_key(&name) => {
                    out.push(call(Op::MetadataRequest { name }, Answer::Ok(Outcome::Maybe(None))));
                }
                17 => {
                    out.push(call(Op::MetadataRequest { name: name.clone() }, Answer::Ok(Outcome::Maybe(None))));
                    if asked && auto {
                        out.push(call(Op::CreateTopicsRequest { name, partitions: 1 }, refused(17)));
                    }
                }
                code => {
                    if asked && auto && !self.topics.contains_key(&name) {
                        self.pending_auto.insert(name.clone());
                    }
                    out.push(retriable_note("Metadata", code, json!({ "topic": name })));
                }
            }
        }
        out
    }

    fn create_topics(&mut self, rq: CreateTopicsRequest, rs: CreateTopicsResponse) -> Vec<Item> {
        if rq.validate_only {
            return vec![];
        }
        let mut out = vec![];
        for r in &rs.topics {
            let name = r.name.0.to_string();
            let asked = rq.topics.iter().find(|t| t.name.0.as_str() == name);
            let partitions = match asked {
                Some(t) if t.num_partitions != -1 => t.num_partitions,
                Some(t) if !t.assignments.is_empty() => t.assignments.len() as i32,
                _ => r.num_partitions,
            };
            let op = Op::CreateTopicsRequest { name: name.clone(), partitions };
            match r.error_code {
                0 => {
                    self.topics.insert(name, partitions);
                    out.push(call(op, Answer::Ok(Outcome::Done)));
                }
                code if retriable(code) => out.push(retriable_note("CreateTopics", code, serde_json::to_value(&op).unwrap_or_default())),
                code => out.push(call(op, refused(code))),
            }
        }
        out
    }

    fn create_partitions(&mut self, rq: CreatePartitionsRequest, rs: CreatePartitionsResponse) -> Vec<Item> {
        if rq.validate_only {
            return vec![];
        }
        let mut out = vec![];
        for r in &rs.results {
            let Some(t) = rq.topics.iter().find(|t| t.name == r.name) else { continue };
            let op = Op::CreatePartitionsRequest { name: t.name.0.to_string(), total: t.count };
            match r.error_code {
                0 => {
                    self.topics.insert(t.name.0.to_string(), t.count);
                    out.push(call(op, Answer::Ok(Outcome::Done)));
                }
                code if retriable(code) => out.push(retriable_note("CreatePartitions", code, serde_json::to_value(&op).unwrap_or_default())),
                code => out.push(call(op, refused(code))),
            }
        }
        out
    }

    fn join(&mut self, rq: JoinGroupRequest, rs: JoinGroupResponse) -> Vec<Item> {
        let group = rq.group_id.0.to_string();
        if rs.error_code != 0 {
            // MEMBER_ID_REQUIRED, a rejoin refused, a protocol mismatch: the
            // coordinator answers these before any operation.
            return vec![];
        }
        let member = rs.member_id.to_string();
        let g = rs.generation_id;
        let state = self.groups.entry(group.clone()).or_default();
        let mut out = vec![];
        if g > state.generation || (g == state.generation && state.round.is_empty()) {
            if rs.members.is_empty() {
                // A follower's answer: the leader's, which names every member,
                // writes the round.
                return out;
            }
            let protocol_type = rq.protocol_type.to_string();
            let mut members: Vec<(String, Vec<String>)> = rs
                .members
                .iter()
                .map(|m| (m.member_id.to_string(), lawtwin_kafka::group::subscribed_topics(&protocol_type, &m.metadata)))
                .collect();
            members.sort();
            let ids: BTreeSet<String> = members.iter().map(|(id, _)| id.clone()).collect();
            for gone in state.in_model.difference(&ids) {
                out.push(Item::Call {
                    source: Source::Coordinator,
                    op: Op::LeaveGroupRequest { group: group.clone(), member: gone.clone() },
                    answer: Answer::Ok(Outcome::Done),
                });
            }
            for (id, topics) in members {
                out.push(call(Op::JoinGroupRequest { group: group.clone(), member: id, topics }, Answer::Ok(Outcome::Number(g as i64))));
            }
            state.generation = g;
            state.in_model = ids.clone();
            state.round = ids;
            state.answered = [member].into_iter().collect();
            state.leader = rs.leader.to_string();
        } else if g == state.generation {
            if state.round.contains(&member) && !state.answered.contains(&member) {
                state.answered.insert(member);
            } else {
                // An unchanged member rejoining a stable group learns the
                // current generation.
                let topics = rq
                    .protocols
                    .first()
                    .map(|p| lawtwin_kafka::group::subscribed_topics(rq.protocol_type.as_str(), &p.metadata))
                    .unwrap_or_default();
                state.in_model.insert(member.clone());
                out.push(call(Op::JoinGroupRequest { group, member, topics }, Answer::Ok(Outcome::Number(g as i64))));
            }
        } else {
            out.push(note("stale generation", json!({ "group": group, "member": member, "generation": g, "current": state.generation })));
        }
        out
    }

    fn sync(&mut self, rq: SyncGroupRequest, rs: SyncGroupResponse) -> Vec<Item> {
        let op = Op::SyncGroupRequest { group: rq.group_id.0.to_string(), member: rq.member_id.to_string(), generation: rq.generation_id };
        match rs.error_code {
            0 => {
                let consumer = rq.protocol_type.as_ref().is_none_or(|t| t.as_str() == "consumer");
                let tps = if consumer { assignment_partitions(&rs.assignment) } else { vec![] };
                vec![call(op, Answer::Ok(Outcome::Partitions(tps)))]
            }
            27 => vec![],
            code if retriable(code) => vec![retriable_note("SyncGroup", code, serde_json::to_value(&op).unwrap_or_default())],
            code => vec![call(op, refused(code))],
        }
    }

    fn heartbeat(&mut self, rq: HeartbeatRequest, rs: HeartbeatResponse) -> Vec<Item> {
        let group = rq.group_id.0.to_string();
        let op = Op::HeartbeatRequest { group: group.clone(), member: rq.member_id.to_string(), generation: rq.generation_id };
        match rs.error_code {
            0 => vec![call(op, Answer::Ok(Outcome::Done))],
            // The member must rejoin. The twin answers so to a heartbeat
            // while a round waits, and to one of a generation that a leave
            // has ended; which it was, the wire does not say.
            // ref:DEC-kafka-divergence-rebalance-heartbeat
            27 => vec![note("rebalance in progress", json!({ "group": group, "member": rq.member_id.to_string(), "generation": rq.generation_id }))],
            code if retriable(code) => vec![retriable_note("Heartbeat", code, serde_json::to_value(&op).unwrap_or_default())],
            code => vec![call(op, refused(code))],
        }
    }

    fn leave(&mut self, rq: LeaveGroupRequest, rs: LeaveGroupResponse, v: i16) -> Vec<Item> {
        let group = rq.group_id.0.to_string();
        let pairs: Vec<(String, i16)> = if v < 3 {
            vec![(rq.member_id.to_string(), rs.error_code)]
        } else {
            rq.members
                .iter()
                .map(|m| {
                    let code = rs.members.iter().find(|r| r.member_id == m.member_id).map(|r| r.error_code).unwrap_or(rs.error_code);
                    (m.member_id.to_string(), if rs.error_code != 0 { rs.error_code } else { code })
                })
                .collect()
        };
        let state = self.groups.entry(group.clone()).or_default();
        let mut out = vec![];
        for (member, code) in pairs {
            if !state.in_model.remove(&member) {
                continue;
            }
            state.round.remove(&member);
            let op = Op::LeaveGroupRequest { group: group.clone(), member };
            out.push(match code {
                0 => call(op, Answer::Ok(Outcome::Done)),
                code if retriable(code) => retriable_note("LeaveGroup", code, serde_json::to_value(&op).unwrap_or_default()),
                code => call(op, refused(code)),
            });
        }
        out
    }

    fn describe(&mut self, rs: DescribeGroupsResponse) -> Vec<Item> {
        let mut out = vec![];
        for g in &rs.groups {
            let group = g.group_id.0.to_string();
            let op = Op::DescribeGroupRequest { group: group.clone() };
            if g.error_code != 0 {
                out.push(retriable_note("DescribeGroups", g.error_code, serde_json::to_value(&op).unwrap_or_default()));
                continue;
            }
            let generation = self.generation(&group).unwrap_or(0);
            let mut members: Vec<MemberView> = g
                .members
                .iter()
                .map(|m| MemberView { member: m.member_id.to_string(), assigned: assignment_partitions(&m.member_assignment) })
                .collect();
            members.sort_by(|a, b| a.member.cmp(&b.member));
            out.push(call(op, Answer::Ok(Outcome::Group { generation, members })));
        }
        out
    }
}

/// The code a fenced producer is answered with, read back: Kafka spells
/// PRODUCER_FENCED as INVALID_PRODUCER_EPOCH on some requests and versions.
/// ref:DEC-fenced-producer-spelling
fn fenced(code: i16, key: ApiKey, version: i16) -> i16 {
    if code == 47 && lawtwin_kafka::wire::fenced_as_epoch(key, version) { 90 } else { code }
}

fn single(api: &str, op: Op, code: i16) -> anyhow::Result<Vec<Item>> {
    Ok(vec![if code == 0 {
        call(op, Answer::Ok(Outcome::Done))
    } else if retriable(code) {
        retriable_note(api, code, serde_json::to_value(&op)?)
    } else {
        call(op, refused(code))
    }])
}

fn produce(rq: ProduceRequest, rs: ProduceResponse) -> anyhow::Result<Vec<Item>> {
    let mut out = vec![];
    for t in &rq.topic_data {
        let topic = t.name.0.to_string();
        for p in &t.partition_data {
            let tp = Tp { topic: topic.clone(), partition: p.index };
            let answer = rs
                .responses
                .iter()
                .filter(|r| r.name == t.name)
                .flat_map(|r| r.partition_responses.iter())
                .find(|r| r.index == p.index);
            let Some(answer) = answer else { continue };
            let Some(records) = &p.records else { continue };
            let batches = match batches(records) {
                Ok(b) => b,
                Err(e) => {
                    out.push(note("undecodable batch", json!({ "topic": topic, "partition": p.index, "error": e })));
                    continue;
                }
            };
            if retriable(answer.error_code) {
                out.push(retriable_note("Produce", answer.error_code, json!({ "tp": tp })));
                continue;
            }
            let mut base = answer.base_offset;
            for b in batches {
                let op = Op::ProduceRequest { tp: tp.clone(), tag: tag_of(&b), records: b.records.iter().map(record_of).collect() };
                if answer.error_code != 0 {
                    out.push(call(op, refused(answer.error_code)));
                    break;
                }
                out.push(call(op, Answer::Ok(Outcome::Number(base))));
                base += b.records.len() as i64;
            }
        }
    }
    Ok(out)
}

/// The records a consumer reads from a fetch's batches, as the clients
/// read them: records below the fetch offset, transaction markers and, when
/// reading committed, records of aborted transactions are left out.
pub fn readable(batches: &[Batch], from: i64, isolation: Isolation, aborted: &[(i64, i64)]) -> (Vec<Fetched>, i64) {
    let mut pending: Vec<(i64, i64)> = aborted.to_vec();
    pending.sort_by_key(|(_, first)| *first);
    let mut pending = pending.into_iter().peekable();
    let mut aborting: BTreeSet<i64> = BTreeSet::new();
    let mut out = vec![];
    let mut next = from;
    for b in batches {
        if isolation == Isolation::ReadCommitted {
            while let Some((pid, first)) = pending.peek().copied() {
                if first > b.last {
                    break;
                }
                aborting.insert(pid);
                pending.next();
            }
        }
        next = next.max(b.last + 1);
        if b.control {
            aborting.remove(&b.producer_id);
            continue;
        }
        if isolation == Isolation::ReadCommitted && b.transactional && aborting.contains(&b.producer_id) {
            continue;
        }
        for r in &b.records {
            if r.offset >= from {
                out.push(Fetched { offset: r.offset, record: record_of(r) });
            }
        }
    }
    (out, next)
}

fn fetch(rq: FetchRequest, rs: FetchResponse) -> anyhow::Result<Vec<Item>> {
    let isolation = if rq.isolation_level == 1 { Isolation::ReadCommitted } else { Isolation::ReadUncommitted };
    let mut out = vec![];
    for t in &rs.responses {
        let topic = t.topic.0.to_string();
        let asked = rq.topics.iter().find(|x| x.topic == t.topic);
        for p in &t.partitions {
            let Some(want) = asked.and_then(|a| a.partitions.iter().find(|x| x.partition == p.partition_index)) else { continue };
            let tp = Tp { topic: topic.clone(), partition: p.partition_index };
            let derived = (want.partition_max_bytes / 64).clamp(1, 5000);
            if p.error_code != 0 {
                let op = Op::FetchRequest { tp, offset: want.fetch_offset, max_records: derived, isolation };
                out.push(if retriable(p.error_code) {
                    retriable_note("Fetch", p.error_code, serde_json::to_value(&op)?)
                } else {
                    call(op, refused(p.error_code))
                });
                continue;
            }
            let batches = match p.records.as_ref().map(batches).transpose() {
                Ok(b) => b.unwrap_or_default(),
                Err(e) => {
                    out.push(note("undecodable batch", json!({ "topic": topic, "partition": p.partition_index, "error": e })));
                    continue;
                }
            };
            let aborted: Vec<(i64, i64)> =
                p.aborted_transactions.iter().flatten().map(|a| (a.producer_id.0, a.first_offset)).collect();
            let (records, next) = readable(&batches, want.fetch_offset, isolation, &aborted);
            let max_records = if records.is_empty() { derived } else { records.len() as i32 };
            out.push(call(
                Op::FetchRequest { tp, offset: want.fetch_offset, max_records, isolation },
                Answer::Ok(Outcome::Fetch(FetchOutcome {
                    records,
                    next_offset: next,
                    high_watermark: p.high_watermark,
                    last_stable_offset: p.last_stable_offset,
                })),
            ));
        }
    }
    Ok(out)
}

fn list_offsets(rq: ListOffsetsRequest, rs: ListOffsetsResponse, v: i16) -> Vec<Item> {
    let isolation = if v >= 2 && rq.isolation_level == 1 { Isolation::ReadCommitted } else { Isolation::ReadUncommitted };
    let mut out = vec![];
    for t in &rq.topics {
        for p in &t.partitions {
            let spec = match p.timestamp {
                -1 => OffsetSpec::Latest,
                -2 => OffsetSpec::Earliest,
                _ => continue, // a time: the contract states no timestamps
            };
            let Some(r) = rs
                .topics
                .iter()
                .filter(|x| x.name == t.name)
                .flat_map(|x| x.partitions.iter())
                .find(|x| x.partition_index == p.partition_index)
            else {
                continue;
            };
            let op = Op::ListOffsetsRequest { tp: Tp { topic: t.name.0.to_string(), partition: p.partition_index }, spec, isolation };
            out.push(match r.error_code {
                0 => call(op, Answer::Ok(Outcome::Number(r.offset))),
                code if retriable(code) => retriable_note("ListOffsets", code, serde_json::to_value(&op).unwrap_or_default()),
                code => call(op, refused(code)),
            });
        }
    }
    out
}

fn offset_commit(rq: OffsetCommitRequest, rs: OffsetCommitResponse) -> Vec<Item> {
    let offsets = rq
        .topics
        .iter()
        .flat_map(|t| t.partitions.iter().map(|p| OffsetEntry { topic: t.name.0.to_string(), partition: p.partition_index, offset: p.committed_offset }))
        .collect();
    let op = Op::OffsetCommitRequest {
        group: rq.group_id.0.to_string(),
        member: rq.member_id.to_string(),
        generation: rq.generation_id_or_member_epoch,
        offsets,
    };
    let code = rs.topics.iter().flat_map(|t| t.partitions.iter().map(|p| p.error_code)).find(|c| *c != 0).unwrap_or(0);
    match code {
        0 => vec![call(op, Answer::Ok(Outcome::Done))],
        code if retriable(code) => vec![retriable_note("OffsetCommit", code, serde_json::to_value(&op).unwrap_or_default())],
        code => vec![call(op, refused(code))],
    }
}

fn offset_fetch(rq: OffsetFetchRequest, rs: OffsetFetchResponse) -> Vec<Item> {
    let group = rq.group_id.0.to_string();
    if retriable(rs.error_code) {
        return vec![retriable_note("OffsetFetch", rs.error_code, json!({ "group": group }))];
    }
    let mut out = vec![];
    for t in &rs.topics {
        for p in &t.partitions {
            let tp = Tp { topic: t.name.0.to_string(), partition: p.partition_index };
            let op = Op::OffsetFetchRequest { group: group.clone(), tp };
            out.push(match p.error_code {
                0 => call(op, Answer::Ok(Outcome::Maybe((p.committed_offset >= 0).then_some(p.committed_offset)))),
                code if retriable(code) => retriable_note("OffsetFetch", code, serde_json::to_value(&op).unwrap_or_default()),
                code => call(op, refused(code)),
            });
        }
    }
    out
}
