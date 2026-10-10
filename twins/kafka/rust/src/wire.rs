//! The Kafka wire protocol: requests decoded with the `kafka-protocol`
//! crate, each mapped onto operations of the contract's `Broker` ability,
//! and the model's answers encoded back.
//!
//! What the wire needs and the contract does not state (record timestamps,
//! batch encoding and compression, member ids, protocol metadata, leader
//! epochs) is kept here or in the twin; everything the contract does state
//! is the model's answer. ref:DEC-kafka-wire-versions

use crate::group::{self, JoinRequest};
use crate::history::{Fetched, Isolation, Octets, Op, OffsetEntry, OffsetSpec, Outcome, ProducerTag, Record, Source, Tp};
use crate::model::{Failure, data::KafkaError};
use crate::twin::{NODE_ID, Twin};
use anyhow::{Result, anyhow, bail};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use kafka_protocol::messages::*;
use kafka_protocol::protocol::{Decodable, Encodable, StrBytes};
use kafka_protocol::records::{
    Compression, NO_PRODUCER_EPOCH, NO_PRODUCER_ID, RecordBatchDecoder, RecordBatchEncoder, RecordEncodeOptions, TimestampType,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The requests the twin answers, with the versions it supports.
pub const SUPPORTED: &[(ApiKey, i16, i16)] = &[
    (ApiKey::Produce, 3, 11),
    (ApiKey::Fetch, 4, 12),
    (ApiKey::ListOffsets, 1, 7),
    (ApiKey::Metadata, 0, 12),
    (ApiKey::OffsetCommit, 2, 8),
    (ApiKey::OffsetFetch, 1, 7),
    (ApiKey::FindCoordinator, 0, 4),
    (ApiKey::JoinGroup, 0, 9),
    (ApiKey::Heartbeat, 0, 4),
    (ApiKey::LeaveGroup, 0, 5),
    (ApiKey::SyncGroup, 0, 5),
    (ApiKey::DescribeGroups, 0, 5),
    (ApiKey::ListGroups, 0, 4),
    (ApiKey::ApiVersions, 0, 4),
    (ApiKey::CreateTopics, 2, 7),
    (ApiKey::InitProducerId, 0, 4),
    (ApiKey::AddPartitionsToTxn, 0, 3),
    (ApiKey::AddOffsetsToTxn, 0, 3),
    (ApiKey::EndTxn, 0, 3),
    (ApiKey::TxnOffsetCommit, 0, 3),
    (ApiKey::DescribeConfigs, 1, 4),
    (ApiKey::CreatePartitions, 0, 3),
];

/// Kafka's error code for a refusal of the contract.
pub fn error_code(f: &Failure) -> i16 {
    match f {
        Failure::Kafka(e) => match e {
            KafkaError::UnknownTopicOrPartition => 3,
            KafkaError::TopicAlreadyExists => 36,
            KafkaError::InvalidTopic => 17,
            KafkaError::InvalidPartitions => 37,
            KafkaError::InvalidRecord => 87,
            KafkaError::InvalidRequest => 42,
            KafkaError::OffsetOutOfRange => 1,
            KafkaError::OutOfOrderSequenceNumber => 45,
            KafkaError::InvalidProducerEpoch => 47,
            KafkaError::ProducerFenced => 90,
            KafkaError::UnknownProducerId => 59,
            KafkaError::InvalidProducerIdMapping => 49,
            KafkaError::InvalidTxnState => 48,
            KafkaError::InvalidGroupId => 24,
            KafkaError::UnknownMemberId => 25,
            KafkaError::IllegalGeneration => 22,
            KafkaError::IllegalState => 48,
        },
        Failure::Model(_) => -1,
    }
}

/// Kafka's error code for a refusal, as the request's API and version spell
/// it. Kafka answers a fenced producer INVALID_PRODUCER_EPOCH on the
/// versions of AddPartitionsToTxn, AddOffsetsToTxn and EndTxn from before
/// PRODUCER_FENCED (KIP-588, version 2), and on every version of
/// TxnOffsetCommit, whose fencing the group coordinator checks.
/// ref:DEC-fenced-producer-spelling
pub fn error_code_for(f: &Failure, key: ApiKey, version: i16) -> i16 {
    let code = error_code(f);
    if code == 90 && fenced_as_epoch(key, version) { 47 } else { code }
}

/// Whether Kafka spells PRODUCER_FENCED as INVALID_PRODUCER_EPOCH on this
/// request and version.
pub fn fenced_as_epoch(key: ApiKey, version: i16) -> bool {
    match key {
        ApiKey::AddPartitionsToTxn | ApiKey::AddOffsetsToTxn | ApiKey::EndTxn => version < 2,
        ApiKey::TxnOffsetCommit => true,
        _ => false,
    }
}

const NOT_LEADER_OR_FOLLOWER: i16 = 6;
const REQUEST_TIMED_OUT: i16 = 7;
const CORRUPT_MESSAGE: i16 = 2;
const UNSUPPORTED_VERSION: i16 = 35;
const UNSUPPORTED_FOR_MESSAGE_FORMAT: i16 = 43;

fn s(text: &str) -> StrBytes {
    StrBytes::from_string(text.to_string())
}

fn topic_name(text: &str) -> TopicName {
    TopicName(s(text))
}

/// A topic's id: fixed by its name, so every run gives the same one.
pub fn topic_id(name: &str) -> uuid::Uuid {
    let mut h1: u64 = 0xcbf2_9ce4_8422_2325;
    let mut h2: u64 = 0x8422_2325_cbf2_9ce4;
    for b in name.bytes() {
        h1 = (h1 ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        h2 = (h2 ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(5);
    }
    uuid::Uuid::from_u64_pair(h1, h2 | 1)
}

/// Serves the Kafka protocol on a listener until the process ends.
pub async fn serve(twin: Arc<Twin>, listener: TcpListener) {
    loop {
        let Ok((stream, peer)) = listener.accept().await else { continue };
        let twin = twin.clone();
        tokio::spawn(async move {
            if twin.lock().unavailable() {
                return; // the connection drops at once
            }
            let _ = stream.set_nodelay(true);
            if let Err(e) = connection(twin, stream, peer.ip().to_string()).await {
                if std::env::var_os("LAWTWIN_DEBUG").is_some() {
                    eprintln!("lawtwin-kafka: connection from {peer}: {e}");
                }
            }
        });
    }
}

async fn connection(twin: Arc<Twin>, mut stream: TcpStream, host: String) -> Result<()> {
    let epoch = twin.epoch.load(std::sync::atomic::Ordering::SeqCst);
    loop {
        let mut len = [0u8; 4];
        stream.read_exact(&mut len).await?;
        let len = i32::from_be_bytes(len);
        if !(0..=104_857_600).contains(&len) {
            bail!("a request of {len} bytes");
        }
        let mut frame = vec![0u8; len as usize];
        stream.read_exact(&mut frame).await?;
        if twin.lock().unavailable() || twin.epoch.load(std::sync::atomic::Ordering::SeqCst) != epoch {
            bail!("the broker is unavailable");
        }
        let mut buf = Bytes::from(frame);
        let header = kafka_protocol::protocol::decode_request_header_from_buffer(&mut buf)?;
        let key = ApiKey::try_from(header.request_api_key).map_err(|_| anyhow!("unknown API key"))?;
        let version = header.request_api_version;
        let client_id = header.client_id.as_ref().map(|c| c.to_string()).unwrap_or_default();
        let response = handle(&twin, key, version, buf, &client_id, &host).await?;
        let Some((body, response_version)) = response else { continue };
        let delay = twin.lock().latency_ms(api_name(key));
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay as u64)).await;
        }
        let mut out = BytesMut::new();
        out.put_i32(0);
        let header_version = key.response_header_version(response_version);
        ResponseHeader::default().with_correlation_id(header.correlation_id).encode(&mut out, header_version)?;
        out.extend_from_slice(&body);
        let size = (out.len() - 4) as i32;
        out[..4].copy_from_slice(&size.to_be_bytes());
        stream.write_all(&out).await?;
    }
}

/// The name of a request type, as faults name it.
pub fn api_name(key: ApiKey) -> &'static str {
    match key {
        ApiKey::Produce => "Produce",
        ApiKey::Fetch => "Fetch",
        ApiKey::ListOffsets => "ListOffsets",
        ApiKey::Metadata => "Metadata",
        ApiKey::OffsetCommit => "OffsetCommit",
        ApiKey::OffsetFetch => "OffsetFetch",
        ApiKey::FindCoordinator => "FindCoordinator",
        ApiKey::JoinGroup => "JoinGroup",
        ApiKey::Heartbeat => "Heartbeat",
        ApiKey::LeaveGroup => "LeaveGroup",
        ApiKey::SyncGroup => "SyncGroup",
        ApiKey::DescribeGroups => "DescribeGroups",
        ApiKey::ListGroups => "ListGroups",
        ApiKey::ApiVersions => "ApiVersions",
        ApiKey::CreateTopics => "CreateTopics",
        ApiKey::InitProducerId => "InitProducerId",
        ApiKey::AddPartitionsToTxn => "AddPartitionsToTxn",
        ApiKey::AddOffsetsToTxn => "AddOffsetsToTxn",
        ApiKey::EndTxn => "EndTxn",
        ApiKey::TxnOffsetCommit => "TxnOffsetCommit",
        ApiKey::DescribeConfigs => "DescribeConfigs",
        ApiKey::CreatePartitions => "CreatePartitions",
        _ => "Other",
    }
}

fn encode<M: Encodable + std::fmt::Debug>(m: &M, version: i16) -> Result<Option<(BytesMut, i16)>> {
    let mut b = BytesMut::new();
    if let Err(e) = m.encode(&mut b, version) {
        // A defect of the twin: say so, whatever the log level.
        eprintln!("lawtwin-kafka: cannot encode a response at version {version}: {e}: {m:?}");
        return Err(e);
    }
    Ok(Some((b, version)))
}

async fn handle(
    twin: &Arc<Twin>,
    key: ApiKey,
    version: i16,
    mut buf: Bytes,
    client_id: &str,
    host: &str,
) -> Result<Option<(BytesMut, i16)>> {
    let supported = SUPPORTED.iter().find(|(k, _, _)| *k == key);
    if key == ApiKey::ApiVersions {
        return api_versions(version);
    }
    match supported {
        Some((_, lo, hi)) if (*lo..=*hi).contains(&version) => {}
        _ => bail!("{key:?} v{version} is not supported"),
    }
    match key {
        ApiKey::Metadata => encode(&metadata(twin, MetadataRequest::decode(&mut buf, version)?, version), version),
        ApiKey::Produce => match produce(twin, ProduceRequest::decode(&mut buf, version)?) {
            Some(r) => encode(&r, version),
            None => Ok(None),
        },
        ApiKey::Fetch => encode(&fetch(twin, FetchRequest::decode(&mut buf, version)?, version).await, version),
        ApiKey::ListOffsets => encode(&list_offsets(twin, ListOffsetsRequest::decode(&mut buf, version)?, version), version),
        ApiKey::FindCoordinator => {
            encode(&find_coordinator(twin, FindCoordinatorRequest::decode(&mut buf, version)?, version), version)
        }
        ApiKey::JoinGroup => {
            let req = JoinGroupRequest::decode(&mut buf, version)?;
            encode(&join_group(twin, req, version, client_id, host).await, version)
        }
        ApiKey::SyncGroup => encode(&sync_group(twin, SyncGroupRequest::decode(&mut buf, version)?), version),
        ApiKey::Heartbeat => {
            let req = HeartbeatRequest::decode(&mut buf, version)?;
            let code = group::heartbeat(twin, &req.group_id.0, &req.member_id, req.generation_id);
            encode(&HeartbeatResponse::default().with_error_code(code), version)
        }
        ApiKey::LeaveGroup => encode(&leave_group(twin, LeaveGroupRequest::decode(&mut buf, version)?, version), version),
        ApiKey::DescribeGroups => encode(&describe_groups(twin, DescribeGroupsRequest::decode(&mut buf, version)?), version),
        ApiKey::ListGroups => encode(&list_groups(twin), version),
        ApiKey::OffsetCommit => encode(&offset_commit(twin, OffsetCommitRequest::decode(&mut buf, version)?), version),
        ApiKey::OffsetFetch => encode(&offset_fetch(twin, OffsetFetchRequest::decode(&mut buf, version)?), version),
        ApiKey::CreateTopics => encode(&create_topics(twin, CreateTopicsRequest::decode(&mut buf, version)?), version),
        ApiKey::CreatePartitions => {
            encode(&create_partitions(twin, CreatePartitionsRequest::decode(&mut buf, version)?), version)
        }
        ApiKey::DescribeConfigs => encode(&describe_configs(DescribeConfigsRequest::decode(&mut buf, version)?), version),
        ApiKey::InitProducerId => encode(&init_producer_id(twin, InitProducerIdRequest::decode(&mut buf, version)?), version),
        ApiKey::AddPartitionsToTxn => {
            encode(&add_partitions_to_txn(twin, AddPartitionsToTxnRequest::decode(&mut buf, version)?, version), version)
        }
        ApiKey::AddOffsetsToTxn => {
            let req = AddOffsetsToTxnRequest::decode(&mut buf, version)?;
            let result = twin.lock().call(
                Source::Wire,
                Op::TxnOffsetCommitRequest {
                    transactional_id: req.transactional_id.0.to_string(),
                    producer_id: req.producer_id.0,
                    producer_epoch: req.producer_epoch,
                    group: req.group_id.0.to_string(),
                    offsets: vec![],
                },
            );
            let code = result.err().map(|f| error_code_for(&f, ApiKey::AddOffsetsToTxn, version)).unwrap_or(0);
            encode(&AddOffsetsToTxnResponse::default().with_error_code(code), version)
        }
        ApiKey::EndTxn => {
            let req = EndTxnRequest::decode(&mut buf, version)?;
            let result = twin.lock().call(
                Source::Wire,
                Op::EndTxnRequest {
                    transactional_id: req.transactional_id.0.to_string(),
                    producer_id: req.producer_id.0,
                    producer_epoch: req.producer_epoch,
                    commit: req.committed,
                },
            );
            twin.data.notify_waiters();
            let code = result.err().map(|f| error_code_for(&f, ApiKey::EndTxn, version)).unwrap_or(0);
            encode(&EndTxnResponse::default().with_error_code(code), version)
        }
        ApiKey::TxnOffsetCommit => {
            encode(&txn_offset_commit(twin, TxnOffsetCommitRequest::decode(&mut buf, version)?, version), version)
        }
        _ => bail!("{key:?} is not supported"),
    }
}

fn api_versions(version: i16) -> Result<Option<(BytesMut, i16)>> {
    let keys: Vec<api_versions_response::ApiVersion> = SUPPORTED
        .iter()
        .map(|(k, lo, hi)| {
            api_versions_response::ApiVersion::default().with_api_key(*k as i16).with_min_version(*lo).with_max_version(*hi)
        })
        .collect();
    if version > 4 {
        // Answered at version 0, so the client can read the versions it may use.
        let r = ApiVersionsResponse::default().with_error_code(UNSUPPORTED_VERSION).with_api_keys(keys);
        return encode(&r, 0);
    }
    encode(&ApiVersionsResponse::default().with_api_keys(keys), version)
}

// ---------------------------------------------------------------------------
// Metadata, coordinators and topics
// ---------------------------------------------------------------------------

fn metadata(twin: &Arc<Twin>, req: MetadataRequest, version: i16) -> MetadataResponse {
    let (host, port) = twin.advertised();
    let mut core = twin.lock();
    let names: Vec<String> = match &req.topics {
        None => core.model.topics().into_keys().collect(),
        Some(ts) => ts.iter().filter_map(|t| t.name.as_ref().map(|n| n.0.to_string())).collect(),
    };
    let auto_create = twin.config.auto_create_topics && (version < 4 || req.allow_auto_topic_creation);
    let mut topics = vec![];
    for name in names {
        let mut count = core.call(Source::Wire, Op::MetadataRequest { name: name.clone() });
        if matches!(count, Ok(Outcome::Maybe(None))) && auto_create && req.topics.is_some() {
            let created = core.call(
                Source::Wire,
                Op::CreateTopicsRequest { name: name.clone(), partitions: twin.config.default_partitions },
            );
            count = match created {
                Ok(_) => core.call(Source::Wire, Op::MetadataRequest { name: name.clone() }),
                Err(f) => Err(f),
            };
        }
        let mut t = metadata_response::MetadataResponseTopic::default()
            .with_name(Some(topic_name(&name)))
            .with_topic_id(topic_id(&name));
        match count {
            Ok(Outcome::Maybe(Some(n))) => {
                t.partitions = (0..n as i32)
                    .map(|p| {
                        metadata_response::MetadataResponsePartition::default()
                            .with_partition_index(p)
                            .with_leader_id(BrokerId(NODE_ID))
                            .with_leader_epoch(core.leader_epoch(&name, p))
                            .with_replica_nodes(vec![BrokerId(NODE_ID)])
                            .with_isr_nodes(vec![BrokerId(NODE_ID)])
                    })
                    .collect();
            }
            Ok(_) => t.error_code = 3,
            Err(f) => t.error_code = error_code(&f),
        }
        topics.push(t);
    }
    MetadataResponse::default()
        .with_brokers(vec![
            metadata_response::MetadataResponseBroker::default().with_node_id(BrokerId(NODE_ID)).with_host(s(&host)).with_port(port),
        ])
        .with_cluster_id(Some(s(&core.cluster_id)))
        .with_controller_id(BrokerId(NODE_ID))
        .with_topics(topics)
}

fn find_coordinator(twin: &Arc<Twin>, req: FindCoordinatorRequest, version: i16) -> FindCoordinatorResponse {
    let (host, port) = twin.advertised();
    if version >= 4 {
        let coordinators = req
            .coordinator_keys
            .iter()
            .map(|k| {
                find_coordinator_response::Coordinator::default()
                    .with_key(k.clone())
                    .with_node_id(BrokerId(NODE_ID))
                    .with_host(s(&host))
                    .with_port(port)
            })
            .collect();
        return FindCoordinatorResponse::default().with_coordinators(coordinators);
    }
    FindCoordinatorResponse::default().with_node_id(BrokerId(NODE_ID)).with_host(s(&host)).with_port(port)
}

fn create_topics(twin: &Arc<Twin>, req: CreateTopicsRequest) -> CreateTopicsResponse {
    let mut core = twin.lock();
    let mut results = vec![];
    for t in req.topics {
        let name = t.name.0.to_string();
        let partitions = if t.num_partitions == -1 {
            if t.assignments.is_empty() { twin.config.default_partitions } else { t.assignments.len() as i32 }
        } else {
            t.num_partitions
        };
        let op = Op::CreateTopicsRequest { name: name.clone(), partitions };
        let result = if req.validate_only {
            let mut trial = core.model.clone();
            crate::history::apply(&mut trial, &op)
        } else {
            core.call(Source::Wire, op)
        };
        let mut r = create_topics_response::CreatableTopicResult::default()
            .with_name(topic_name(&name))
            .with_topic_id(topic_id(&name))
            .with_num_partitions(partitions)
            .with_replication_factor(1);
        if let Err(f) = result {
            r.error_code = error_code(&f);
            r.error_message = Some(s(&f.to_string()));
            r.num_partitions = -1;
            r.replication_factor = -1;
        }
        results.push(r);
    }
    CreateTopicsResponse::default().with_topics(results)
}

fn create_partitions(twin: &Arc<Twin>, req: CreatePartitionsRequest) -> CreatePartitionsResponse {
    let mut core = twin.lock();
    let results = req
        .topics
        .into_iter()
        .map(|t| {
            let op = Op::CreatePartitionsRequest { name: t.name.0.to_string(), total: t.count };
            let result = if req.validate_only {
                let mut trial = core.model.clone();
                crate::history::apply(&mut trial, &op)
            } else {
                core.call(Source::Wire, op)
            };
            let mut r = create_partitions_response::CreatePartitionsTopicResult::default().with_name(t.name);
            if let Err(f) = result {
                r.error_code = error_code(&f);
                r.error_message = Some(s(&f.to_string()));
            }
            r
        })
        .collect();
    CreatePartitionsResponse::default().with_results(results)
}

fn describe_configs(req: DescribeConfigsRequest) -> DescribeConfigsResponse {
    // The contract states no configuration. A topic answers the defaults a
    // client may ask for; nothing else.
    let results = req
        .resources
        .into_iter()
        .map(|r| {
            let configs = if r.resource_type == 2 {
                [("cleanup.policy", "delete"), ("message.timestamp.type", "CreateTime"), ("retention.ms", "-1")]
                    .iter()
                    .filter(|(k, _)| r.configuration_keys.as_ref().is_none_or(|ks| ks.iter().any(|x| x.as_str() == *k)))
                    .map(|(k, v)| {
                        describe_configs_response::DescribeConfigsResourceResult::default()
                            .with_name(s(k))
                            .with_value(Some(s(v)))
                            .with_read_only(true)
                            .with_config_source(5)
                    })
                    .collect()
            } else {
                vec![]
            };
            describe_configs_response::DescribeConfigsResult::default()
                .with_resource_type(r.resource_type)
                .with_resource_name(r.resource_name)
                .with_configs(configs)
        })
        .collect();
    DescribeConfigsResponse::default().with_results(results)
}

// ---------------------------------------------------------------------------
// Produce, fetch and offsets
// ---------------------------------------------------------------------------

/// Decoded batches: each batch's producer tag, records and timestamps.
type Batches = Vec<(Option<ProducerTag>, Vec<Record>, Vec<i64>)>;

/// Decodes v2 record batches, in any compression.
pub fn decode_batches(bytes: &Bytes) -> Result<Batches, i16> {
    let mut buf = bytes.clone();
    // The magic byte is the 17th of a batch; older formats are refused.
    if buf.len() > 16 && buf[16] < 2 {
        return Err(UNSUPPORTED_FOR_MESSAGE_FORMAT);
    }
    let sets = RecordBatchDecoder::decode_all(&mut buf).map_err(|_| CORRUPT_MESSAGE)?;
    let mut out = vec![];
    for set in sets {
        let Some(first) = set.records.first() else { continue };
        let tag = (first.producer_id >= 0).then(|| ProducerTag {
            producer_id: first.producer_id,
            producer_epoch: first.producer_epoch,
            base_sequence: first.sequence,
        });
        let records = set
            .records
            .iter()
            .map(|r| Record {
                key: r.key.as_ref().map(|k| Octets(k.to_vec())),
                value: r.value.as_ref().map(|v| Octets(v.to_vec())),
                headers: r
                    .headers
                    .iter()
                    .map(|(k, v)| crate::history::Header { name: k.to_string(), content: v.as_ref().map(|v| Octets(v.to_vec())) })
                    .collect(),
            })
            .collect();
        let stamps = set.records.iter().map(|r| r.timestamp).collect();
        out.push((tag, records, stamps));
    }
    Ok(out)
}

fn produce(twin: &Arc<Twin>, req: ProduceRequest) -> Option<ProduceResponse> {
    let mut wrote = false;
    let mut responses = vec![];
    {
        let mut core = twin.lock();
        for topic in &req.topic_data {
            let name = topic.name.0.to_string();
            let mut partitions = vec![];
            for p in &topic.partition_data {
                let mut r = produce_response::PartitionProduceResponse::default()
                    .with_index(p.index)
                    .with_log_append_time_ms(-1)
                    .with_log_start_offset(0);
                if core.take_not_leader(&name, p.index) {
                    r.error_code = NOT_LEADER_OR_FOLLOWER;
                    partitions.push(r);
                    continue;
                }
                let batches = match p.records.as_ref().map(decode_batches) {
                    None => Err(87),
                    Some(Err(code)) => Err(code),
                    Some(Ok(b)) => Ok(b),
                };
                match batches {
                    Err(code) => r.error_code = code,
                    Ok(batches) => {
                        let mut base = None;
                        for (tag, records, stamps) in batches {
                            // A deliberately broken core, for showing that a
                            // replay catches one: it forgets the producer tag,
                            // so a retry is written again.
                            #[cfg(feature = "mutant-ignore-dedupe")]
                            let tag: Option<ProducerTag> = {
                                let _ = tag;
                                None
                            };
                            let tp = Tp { topic: name.clone(), partition: p.index };
                            match core.call(Source::Wire, Op::ProduceRequest { tp, tag, records }) {
                                Ok(Outcome::Number(offset)) => {
                                    core.stamp(&name, p.index, offset, &stamps);
                                    base.get_or_insert(offset);
                                    wrote = true;
                                }
                                Ok(_) => r.error_code = -1,
                                Err(f) => {
                                    r.error_code = error_code(&f);
                                    break;
                                }
                            }
                        }
                        if r.error_code == 0 {
                            r.base_offset = base.unwrap_or(-1);
                            if core.take_duplicate(&name) {
                                // The batch is written; its acknowledgement is lost.
                                r.error_code = REQUEST_TIMED_OUT;
                            }
                        } else {
                            r.base_offset = -1;
                        }
                    }
                }
                partitions.push(r);
            }
            responses.push(
                produce_response::TopicProduceResponse::default().with_name(topic.name.clone()).with_partition_responses(partitions),
            );
        }
    }
    if wrote {
        twin.data.notify_waiters();
    }
    if req.acks == 0 {
        return None;
    }
    Some(ProduceResponse::default().with_responses(responses))
}

/// Encodes the records a fetch returned as v2 batches, with an empty batch
/// past any trailing transaction markers or aborted records the fetch skipped.
fn encode_fetched(core: &crate::twin::Core, topic: &str, partition: i32, records: &[Fetched], next: i64, from: i64) -> Bytes {
    let epoch = core.leader_epoch(topic, partition);
    let wire: Vec<kafka_protocol::records::Record> = records
        .iter()
        .map(|f| kafka_protocol::records::Record {
            transactional: false,
            control: false,
            delete_horizon: false,
            partition_leader_epoch: epoch,
            producer_id: NO_PRODUCER_ID,
            producer_epoch: NO_PRODUCER_EPOCH,
            timestamp_type: TimestampType::Creation,
            offset: f.offset,
            sequence: f.offset as i32,
            timestamp: core.timestamp(topic, partition, f.offset),
            key: f.record.key.as_ref().map(|k| Bytes::from(k.0.clone())),
            value: f.record.value.as_ref().map(|v| Bytes::from(v.0.clone())),
            headers: f
                .record
                .headers
                .iter()
                .map(|h| (s(&h.name), h.content.as_ref().map(|c| Bytes::from(c.0.clone()))))
                .collect(),
        })
        .collect();
    let mut buf = BytesMut::new();
    if !wire.is_empty() {
        let options = RecordEncodeOptions { version: 2, compression: Compression::None };
        if RecordBatchEncoder::encode(&mut buf, wire.iter(), &options).is_err() {
            buf.clear();
        }
    }
    let after = records.last().map(|r| r.offset + 1).unwrap_or(from);
    if next > after {
        empty_batch(&mut buf, after, next - 1, epoch, core.clock.now_ms());
    }
    buf.freeze()
}

/// A v2 batch with no records spanning `first..=last`, as compaction leaves:
/// it moves a consumer's position past offsets with nothing to read.
fn empty_batch(buf: &mut BytesMut, first: i64, last: i64, epoch: i32, now: i64) {
    let mut body = BytesMut::new();
    body.put_i16(0); // attributes
    body.put_i32((last - first) as i32); // last offset delta
    body.put_i64(now); // base timestamp
    body.put_i64(now); // max timestamp
    body.put_i64(-1); // producer id
    body.put_i16(-1); // producer epoch
    body.put_i32(-1); // base sequence
    body.put_i32(0); // records
    let crc = crc32c::crc32c(&body);
    buf.put_i64(first);
    buf.put_i32((4 + 1 + 4 + body.len()) as i32);
    buf.put_i32(epoch);
    buf.put_i8(2);
    buf.put_u32(crc);
    buf.extend_from_slice(&body);
}

struct FetchPart {
    topic: String,
    partition: i32,
    op: Option<Op>,
    result: Result<Outcome, Failure>,
    error: i16,
}

async fn fetch(twin: &Arc<Twin>, req: FetchRequest, version: i16) -> FetchResponse {
    let isolation = if req.isolation_level == 1 { Isolation::ReadCommitted } else { Isolation::ReadUncommitted };
    let wait = Duration::from_millis(req.max_wait_ms.max(0) as u64).min(Duration::from_millis(twin.config.max_fetch_wait_ms));
    let deadline = tokio::time::Instant::now() + wait;
    let mut lag_delay = 0;
    let mut not_leader: Vec<(String, i32)> = vec![];
    let parts = loop {
        let notified = twin.data.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let parts = {
            let mut core = twin.lock();
            let mut parts = vec![];
            for t in &req.topics {
                let topic = t.topic.0.to_string();
                for p in &t.partitions {
                    if not_leader.contains(&(topic.clone(), p.partition)) || core.take_not_leader(&topic, p.partition) {
                        not_leader.push((topic.clone(), p.partition));
                        parts.push(FetchPart {
                            topic: topic.clone(),
                            partition: p.partition,
                            op: None,
                            result: Ok(Outcome::Done),
                            error: NOT_LEADER_OR_FOLLOWER,
                        });
                        continue;
                    }
                    let mut max_records = (p.partition_max_bytes / 64).clamp(1, 5000);
                    if let Some((records, delay)) = core.lag(&topic) {
                        max_records = max_records.min(records);
                        lag_delay = lag_delay.max(delay);
                    }
                    let op = Op::FetchRequest {
                        tp: Tp { topic: topic.clone(), partition: p.partition },
                        offset: p.fetch_offset,
                        max_records,
                        isolation,
                    };
                    let result = core.peek(&op);
                    let error = result.as_ref().err().map(error_code).unwrap_or(0);
                    parts.push(FetchPart { topic: topic.clone(), partition: p.partition, op: Some(op), result, error });
                }
            }
            let ready = parts.iter().any(|p| {
                p.error != 0 || matches!(&p.result, Ok(Outcome::Fetch(f)) if !f.records.is_empty())
            });
            if ready || req.min_bytes <= 0 || tokio::time::Instant::now() >= deadline || parts.is_empty() {
                for p in &parts {
                    if let Some(op) = &p.op {
                        core.record(Source::Wire, op.clone(), &p.result);
                    }
                }
                Some(parts)
            } else {
                None
            }
        };
        if let Some(parts) = parts {
            break parts;
        }
        let _ = tokio::time::timeout_at(deadline, notified).await;
    };
    if lag_delay > 0 {
        tokio::time::sleep(Duration::from_millis(lag_delay as u64)).await;
    }
    let core = twin.lock();
    let mut responses: Vec<fetch_response::FetchableTopicResponse> = vec![];
    for p in parts {
        let mut data = fetch_response::PartitionData::default()
            .with_partition_index(p.partition)
            .with_error_code(p.error)
            .with_high_watermark(-1)
            .with_last_stable_offset(-1)
            .with_log_start_offset(0)
            .with_preferred_read_replica(BrokerId(-1));
        if p.error == NOT_LEADER_OR_FOLLOWER && version >= 12 {
            data.current_leader = fetch_response::LeaderIdAndEpoch::default()
                .with_leader_id(BrokerId(NODE_ID))
                .with_leader_epoch(core.leader_epoch(&p.topic, p.partition));
        }
        if let Ok(Outcome::Fetch(f)) = &p.result {
            let from = match &p.op {
                Some(Op::FetchRequest { offset, .. }) => *offset,
                _ => 0,
            };
            data.high_watermark = f.high_watermark;
            data.last_stable_offset = f.last_stable_offset;
            data.records = Some(encode_fetched(&core, &p.topic, p.partition, &f.records, f.next_offset, from));
            if isolation == Isolation::ReadCommitted {
                data.aborted_transactions = Some(vec![]);
            }
        }
        match responses.iter_mut().find(|r| r.topic.0.as_str() == p.topic) {
            Some(r) => r.partitions.push(data),
            None => responses.push(
                fetch_response::FetchableTopicResponse::default()
                    .with_topic(topic_name(&p.topic))
                    .with_topic_id(topic_id(&p.topic))
                    .with_partitions(vec![data]),
            ),
        }
    }
    FetchResponse::default().with_responses(responses)
}

fn list_offsets(twin: &Arc<Twin>, req: ListOffsetsRequest, version: i16) -> ListOffsetsResponse {
    let isolation = if version >= 2 && req.isolation_level == 1 { Isolation::ReadCommitted } else { Isolation::ReadUncommitted };
    let mut core = twin.lock();
    let topics = req
        .topics
        .iter()
        .map(|t| {
            let topic = t.name.0.to_string();
            let partitions = t
                .partitions
                .iter()
                .map(|p| {
                    let mut r = list_offsets_response::ListOffsetsPartitionResponse::default()
                        .with_partition_index(p.partition_index)
                        .with_timestamp(-1)
                        .with_offset(-1)
                        .with_leader_epoch(if version >= 4 { core.leader_epoch(&topic, p.partition_index) } else { -1 });
                    if core.take_not_leader(&topic, p.partition_index) {
                        r.error_code = NOT_LEADER_OR_FOLLOWER;
                        return r;
                    }
                    let tp = Tp { topic: topic.clone(), partition: p.partition_index };
                    let result = match p.timestamp {
                        -2 => core.call(Source::Wire, Op::ListOffsetsRequest { tp, spec: OffsetSpec::Earliest, isolation }),
                        -1 => core.call(Source::Wire, Op::ListOffsetsRequest { tp, spec: OffsetSpec::Latest, isolation }),
                        t if t >= 0 => {
                            // Offsets for a time: the first record at or after it. The
                            // contract states no timestamps, so this reads the twin's.
                            let latest =
                                core.call(Source::Wire, Op::ListOffsetsRequest { tp: tp.clone(), spec: OffsetSpec::Latest, isolation });
                            latest.map(|o| match o {
                                Outcome::Number(end) => {
                                    let stamps = core.timestamps.get(&(topic.clone(), p.partition_index));
                                    let found = stamps.and_then(|v| {
                                        v.iter().take(end.max(0) as usize).position(|s| *s >= t).map(|i| i as i64)
                                    });
                                    r.timestamp = found.map(|o| core.timestamp(&topic, p.partition_index, o)).unwrap_or(-1);
                                    Outcome::Number(found.unwrap_or(end))
                                }
                                other => other,
                            })
                        }
                        _ => Err(Failure::Kafka(KafkaError::InvalidRequest)),
                    };
                    match result {
                        Ok(Outcome::Number(o)) => r.offset = o,
                        Ok(_) => r.error_code = -1,
                        Err(f) => r.error_code = error_code(&f),
                    }
                    r
                })
                .collect();
            list_offsets_response::ListOffsetsTopicResponse::default().with_name(t.name.clone()).with_partitions(partitions)
        })
        .collect();
    ListOffsetsResponse::default().with_topics(topics)
}

// ---------------------------------------------------------------------------
// Groups and committed offsets
// ---------------------------------------------------------------------------

async fn join_group(twin: &Arc<Twin>, req: JoinGroupRequest, version: i16, client_id: &str, host: &str) -> JoinGroupResponse {
    let reply = group::join(
        twin,
        JoinRequest {
            group: req.group_id.0.to_string(),
            member_id: req.member_id.to_string(),
            instance_id: req.group_instance_id.as_ref().map(|i| i.to_string()),
            client_id: client_id.to_string(),
            client_host: format!("/{host}"),
            protocol_type: req.protocol_type.to_string(),
            protocols: req.protocols.iter().map(|p| (p.name.to_string(), p.metadata.clone())).collect(),
            session_timeout_ms: req.session_timeout_ms,
            requires_member_id: version >= 4,
        },
    )
    .await;
    JoinGroupResponse::default()
        .with_error_code(reply.error)
        .with_generation_id(reply.generation)
        .with_protocol_type(reply.protocol_type.as_deref().map(s))
        .with_protocol_name(reply.protocol_name.as_deref().map(s).or_else(|| (version < 7).then(|| s(""))))
        .with_leader(s(&reply.leader))
        .with_member_id(s(&reply.member_id))
        .with_members(
            reply
                .members
                .into_iter()
                .map(|(id, instance, meta)| {
                    join_group_response::JoinGroupResponseMember::default()
                        .with_member_id(s(&id))
                        .with_group_instance_id(instance.as_deref().map(s))
                        .with_metadata(meta)
                })
                .collect(),
        )
}

/// The consumer protocol's encoding of an assignment.
pub fn encode_assignment(tps: &[(String, i32)]) -> Bytes {
    let mut topics: Vec<consumer_protocol_assignment::TopicPartition> = vec![];
    for (t, p) in tps {
        match topics.iter_mut().find(|x| x.topic.0.as_str() == t) {
            Some(x) => x.partitions.push(*p),
            None => topics.push(
                consumer_protocol_assignment::TopicPartition::default().with_topic(topic_name(t)).with_partitions(vec![*p]),
            ),
        }
    }
    let a = ConsumerProtocolAssignment::default().with_assigned_partitions(topics);
    let mut buf = BytesMut::new();
    buf.put_i16(0);
    let _ = a.encode(&mut buf, 0);
    buf.freeze()
}

fn sync_group(twin: &Arc<Twin>, req: SyncGroupRequest) -> SyncGroupResponse {
    let group = req.group_id.0.to_string();
    let (code, tps) = group::sync(twin, &group, &req.member_id, req.generation_id);
    let (protocol_type, protocol_name) = {
        let core = twin.lock();
        let gw = core.groups.get(&group);
        (gw.and_then(|g| g.protocol_type.clone()), gw.and_then(|g| g.protocol_name.clone()))
    };
    let consumer = protocol_type.as_deref().is_none_or(|t| t == "consumer");
    SyncGroupResponse::default()
        .with_error_code(code)
        .with_protocol_type(protocol_type.as_deref().map(s))
        .with_protocol_name(protocol_name.as_deref().map(s))
        .with_assignment(if code == 0 && consumer { encode_assignment(&tps) } else { Bytes::new() })
}

fn leave_group(twin: &Arc<Twin>, req: LeaveGroupRequest, version: i16) -> LeaveGroupResponse {
    let group = req.group_id.0.to_string();
    if version < 3 {
        let codes = group::leave(twin, &group, &[req.member_id.to_string()]);
        return LeaveGroupResponse::default().with_error_code(codes[0]);
    }
    let ids: Vec<String> = req.members.iter().map(|m| m.member_id.to_string()).collect();
    let codes = group::leave(twin, &group, &ids);
    LeaveGroupResponse::default().with_members(
        req.members
            .iter()
            .zip(codes)
            .map(|(m, c)| {
                leave_group_response::MemberResponse::default()
                    .with_member_id(m.member_id.clone())
                    .with_group_instance_id(m.group_instance_id.clone())
                    .with_error_code(c)
            })
            .collect(),
    )
}

fn describe_groups(twin: &Arc<Twin>, req: DescribeGroupsRequest) -> DescribeGroupsResponse {
    let mut core = twin.lock();
    let groups = req
        .groups
        .iter()
        .map(|g| {
            let group = g.0.to_string();
            let view = core.call(Source::Wire, Op::DescribeGroupRequest { group: group.clone() });
            let gw = core.groups.get(&group);
            let mut d = describe_groups_response::DescribedGroup::default().with_group_id(g.clone());
            match view {
                Ok(Outcome::Group { members, .. }) => {
                    let state = if gw.is_some_and(|w| w.round.is_some()) {
                        "PreparingRebalance"
                    } else if members.is_empty() {
                        "Empty"
                    } else {
                        "Stable"
                    };
                    d.group_state = s(state);
                    d.protocol_type = s(gw.and_then(|w| w.protocol_type.clone()).as_deref().unwrap_or(""));
                    d.protocol_data = s(gw.and_then(|w| w.protocol_name.clone()).as_deref().unwrap_or(""));
                    d.members = members
                        .iter()
                        .map(|m| {
                            let wire = gw.and_then(|w| w.members.get(&m.member));
                            let tps: Vec<(String, i32)> = m.assigned.iter().map(|t| (t.topic.clone(), t.partition)).collect();
                            describe_groups_response::DescribedGroupMember::default()
                                .with_member_id(s(&m.member))
                                .with_client_id(s(wire.map(|w| w.client_id.as_str()).unwrap_or("")))
                                .with_client_host(s(wire.map(|w| w.client_host.as_str()).unwrap_or("")))
                                .with_member_assignment(encode_assignment(&tps))
                        })
                        .collect();
                }
                Ok(_) => d.error_code = -1,
                Err(f) => d.error_code = group::group_error(&f),
            }
            d
        })
        .collect();
    DescribeGroupsResponse::default().with_groups(groups)
}

fn list_groups(twin: &Arc<Twin>) -> ListGroupsResponse {
    let core = twin.lock();
    let groups = core
        .model
        .group_ids()
        .into_iter()
        .map(|g| {
            let t = core.groups.get(&g).and_then(|w| w.protocol_type.clone()).unwrap_or_else(|| "consumer".into());
            list_groups_response::ListedGroup::default()
                .with_group_id(GroupId(s(&g)))
                .with_protocol_type(s(&t))
                .with_group_state(s("Stable"))
                .with_group_type(s("classic"))
        })
        .collect();
    ListGroupsResponse::default().with_groups(groups)
}

fn offset_commit(twin: &Arc<Twin>, req: OffsetCommitRequest) -> OffsetCommitResponse {
    let offsets: Vec<OffsetEntry> = req
        .topics
        .iter()
        .flat_map(|t| {
            t.partitions.iter().map(|p| OffsetEntry {
                topic: t.name.0.to_string(),
                partition: p.partition_index,
                offset: p.committed_offset,
            })
        })
        .collect();
    let result = twin.lock().call(
        Source::Wire,
        Op::OffsetCommitRequest {
            group: req.group_id.0.to_string(),
            member: req.member_id.to_string(),
            generation: req.generation_id_or_member_epoch,
            offsets,
        },
    );
    let code = match &result {
        Ok(_) => 0,
        Err(f) => group::group_error(f),
    };
    OffsetCommitResponse::default().with_topics(
        req.topics
            .iter()
            .map(|t| {
                offset_commit_response::OffsetCommitResponseTopic::default().with_name(t.name.clone()).with_partitions(
                    t.partitions
                        .iter()
                        .map(|p| {
                            offset_commit_response::OffsetCommitResponsePartition::default()
                                .with_partition_index(p.partition_index)
                                .with_error_code(code)
                        })
                        .collect(),
                )
            })
            .collect(),
    )
}

fn offset_fetch(twin: &Arc<Twin>, req: OffsetFetchRequest) -> OffsetFetchResponse {
    let group = req.group_id.0.to_string();
    let mut core = twin.lock();
    let wanted: Vec<(String, Vec<i32>)> = match &req.topics {
        Some(ts) => ts.iter().map(|t| (t.name.0.to_string(), t.partition_indexes.clone())).collect(),
        None => {
            // Every partition with a committed offset: the topics the model has.
            core.model.topics().into_iter().map(|(t, n)| (t, (0..n).collect())).collect()
        }
    };
    let all = req.topics.is_none();
    let mut topics = vec![];
    for (topic, partitions) in wanted {
        let mut ps = vec![];
        for p in partitions {
            let result =
                core.call(Source::Wire, Op::OffsetFetchRequest { group: group.clone(), tp: Tp { topic: topic.clone(), partition: p } });
            let offset = match result {
                Ok(Outcome::Maybe(Some(o))) => o,
                _ => -1,
            };
            if all && offset < 0 {
                continue;
            }
            ps.push(
                offset_fetch_response::OffsetFetchResponsePartition::default()
                    .with_partition_index(p)
                    .with_committed_offset(offset)
                    .with_committed_leader_epoch(-1)
                    .with_metadata(Some(s(""))),
            );
        }
        if !ps.is_empty() || !all {
            topics.push(offset_fetch_response::OffsetFetchResponseTopic::default().with_name(topic_name(&topic)).with_partitions(ps));
        }
    }
    OffsetFetchResponse::default().with_topics(topics)
}

// ---------------------------------------------------------------------------
// Producer ids and transactions
// ---------------------------------------------------------------------------

fn init_producer_id(twin: &Arc<Twin>, req: InitProducerIdRequest) -> InitProducerIdResponse {
    let txid = req.transactional_id.as_ref().map(|t| t.0.to_string());
    let result = twin.lock().call(Source::Wire, Op::InitProducerIdRequest { transactional_id: txid });
    twin.data.notify_waiters();
    match result {
        Ok(Outcome::Producer { producer_id, producer_epoch }) => {
            InitProducerIdResponse::default().with_producer_id(ProducerId(producer_id)).with_producer_epoch(producer_epoch)
        }
        Ok(_) => InitProducerIdResponse::default().with_error_code(-1).with_producer_id(ProducerId(-1)).with_producer_epoch(-1),
        Err(f) => InitProducerIdResponse::default()
            .with_error_code(error_code(&f))
            .with_producer_id(ProducerId(-1))
            .with_producer_epoch(-1),
    }
}

fn add_partitions_to_txn(twin: &Arc<Twin>, req: AddPartitionsToTxnRequest, version: i16) -> AddPartitionsToTxnResponse {
    let tps: Vec<Tp> = req
        .v3_and_below_topics
        .iter()
        .flat_map(|t| t.partitions.iter().map(|p| Tp { topic: t.name.0.to_string(), partition: *p }))
        .collect();
    let result = twin.lock().call(
        Source::Wire,
        Op::AddPartitionsToTxnRequest {
            transactional_id: req.v3_and_below_transactional_id.0.to_string(),
            producer_id: req.v3_and_below_producer_id.0,
            producer_epoch: req.v3_and_below_producer_epoch,
            partitions: tps,
        },
    );
    let code = result.err().map(|f| error_code_for(&f, ApiKey::AddPartitionsToTxn, version)).unwrap_or(0);
    AddPartitionsToTxnResponse::default().with_results_by_topic_v3_and_below(
        req.v3_and_below_topics
            .iter()
            .map(|t| {
                add_partitions_to_txn_response::AddPartitionsToTxnTopicResult::default()
                    .with_name(t.name.clone())
                    .with_results_by_partition(
                        t.partitions
                            .iter()
                            .map(|p| {
                                add_partitions_to_txn_response::AddPartitionsToTxnPartitionResult::default()
                                    .with_partition_index(*p)
                                    .with_partition_error_code(code)
                            })
                            .collect(),
                    )
            })
            .collect(),
    )
}

fn txn_offset_commit(twin: &Arc<Twin>, req: TxnOffsetCommitRequest, version: i16) -> TxnOffsetCommitResponse {
    let offsets: Vec<OffsetEntry> = req
        .topics
        .iter()
        .flat_map(|t| {
            t.partitions.iter().map(|p| OffsetEntry {
                topic: t.name.0.to_string(),
                partition: p.partition_index,
                offset: p.committed_offset,
            })
        })
        .collect();
    let result = twin.lock().call(
        Source::Wire,
        Op::TxnOffsetCommitRequest {
            transactional_id: req.transactional_id.0.to_string(),
            producer_id: req.producer_id.0,
            producer_epoch: req.producer_epoch,
            group: req.group_id.0.to_string(),
            offsets,
        },
    );
    let code = result.err().map(|f| error_code_for(&f, ApiKey::TxnOffsetCommit, version)).unwrap_or(0);
    TxnOffsetCommitResponse::default().with_topics(
        req.topics
            .iter()
            .map(|t| {
                txn_offset_commit_response::TxnOffsetCommitResponseTopic::default().with_name(t.name.clone()).with_partitions(
                    t.partitions
                        .iter()
                        .map(|p| {
                            txn_offset_commit_response::TxnOffsetCommitResponsePartition::default()
                                .with_partition_index(p.partition_index)
                                .with_error_code(code)
                        })
                        .collect(),
                )
            })
            .collect(),
    )
}

/// Reads one length-prefixed response, for clients written in Rust.
pub fn split_frame(buf: &mut Bytes) -> Option<Bytes> {
    if buf.len() < 4 {
        return None;
    }
    let len = i32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if buf.len() < 4 + len {
        return None;
    }
    buf.advance(4);
    Some(buf.split_to(len))
}
