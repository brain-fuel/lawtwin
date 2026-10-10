//! A scripted session: one connection, one request at a time, every kind
//! of request the twin answers, each answer checked. Two runs with the same
//! seed must write the same history; the frames of a run on the reference
//! core are the golden session in `twins/kafka/shared/golden/`, which every
//! core must answer byte for byte.

use crate::core::TestTwin;
use bytes::{Bytes, BytesMut};
use kafka_protocol::messages::*;
use kafka_protocol::protocol::{Decodable, Encodable, HeaderVersion, Request, StrBytes};
use kafka_protocol::records::{Compression, Record, RecordBatchDecoder, RecordBatchEncoder, RecordEncodeOptions, TimestampType};
use std::io::{Read, Write};
use std::net::TcpStream;

/// One request and its response, as sent and received on the wire: the
/// header and body of each, without the length prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The request's API key.
    pub key: i16,
    /// Its version.
    pub version: i16,
    /// The request's bytes.
    pub request: Vec<u8>,
    /// The response's bytes.
    pub response: Vec<u8>,
}

/// A client that sends typed requests one at a time and keeps every frame.
pub struct Client {
    stream: TcpStream,
    correlation: i32,
    /// The frames so far.
    pub frames: Vec<Frame>,
}

impl Client {
    /// A connection to the core's Kafka port.
    pub fn connect(twin: &TestTwin) -> Self {
        Self { stream: TcpStream::connect(twin.kafka).unwrap(), correlation: 0, frames: vec![] }
    }

    /// Sends bytes as one request and gives the response's bytes.
    pub fn exchange(&mut self, request: &[u8]) -> Vec<u8> {
        self.stream.write_all(&(request.len() as i32).to_be_bytes()).unwrap();
        self.stream.write_all(request).unwrap();
        let mut len = [0u8; 4];
        self.stream.read_exact(&mut len).unwrap();
        let mut body = vec![0u8; i32::from_be_bytes(len) as usize];
        self.stream.read_exact(&mut body).unwrap();
        body
    }

    /// Sends a typed request and gives the typed response.
    pub fn send<R: Request>(&mut self, req: R, version: i16) -> R::Response {
        self.correlation += 1;
        let header = RequestHeader::default()
            .with_request_api_key(R::KEY)
            .with_request_api_version(version)
            .with_correlation_id(self.correlation)
            .with_client_id(Some(StrBytes::from_static_str("scripted")));
        let mut buf = BytesMut::new();
        header.encode(&mut buf, R::header_version(version)).unwrap();
        req.encode(&mut buf, version).unwrap();
        let raw = self.exchange(&buf);
        self.frames.push(Frame { key: R::KEY, version, request: buf.to_vec(), response: raw.clone() });
        let mut body = Bytes::from(raw);
        let header = ResponseHeader::decode(&mut body, R::Response::header_version(version)).unwrap();
        assert_eq!(header.correlation_id, self.correlation);
        R::Response::decode(&mut body, version).unwrap()
    }
}

/// A protocol string.
pub fn s(x: &str) -> StrBytes {
    StrBytes::from_string(x.to_string())
}

/// A v2 record batch.
pub fn batch(pid: i64, epoch: i16, seq: i32, values: &[&str], transactional: bool, compression: Compression) -> Bytes {
    let records: Vec<Record> = values
        .iter()
        .enumerate()
        .map(|(i, v)| Record {
            transactional,
            control: false,
            delete_horizon: false,
            partition_leader_epoch: 0,
            producer_id: pid,
            producer_epoch: epoch,
            timestamp_type: TimestampType::Creation,
            offset: i as i64,
            sequence: seq + i as i32,
            timestamp: 1_767_225_600_000 + i as i64,
            key: Some(Bytes::from(format!("k{i}"))),
            value: Some(Bytes::from(v.to_string())),
            headers: [(s("h"), Some(Bytes::from_static(b"\x00\x01binary")))].into_iter().collect(),
        })
        .collect();
    let mut buf = BytesMut::new();
    RecordBatchEncoder::encode(&mut buf, records.iter(), &RecordEncodeOptions { version: 2, compression }).unwrap();
    buf.freeze()
}

/// A produce request to one partition: the error code and base offset.
pub fn produce(c: &mut Client, topic: &str, partition: i32, txid: Option<&str>, records: Bytes) -> (i16, i64) {
    let req = ProduceRequest::default()
        .with_transactional_id(txid.map(|t| TransactionalId(s(t))))
        .with_acks(-1)
        .with_timeout_ms(1000)
        .with_topic_data(vec![produce_request::TopicProduceData::default().with_name(TopicName(s(topic))).with_partition_data(vec![
            produce_request::PartitionProduceData::default().with_index(partition).with_records(Some(records)),
        ])]);
    let r = c.send(req, 9);
    let p = &r.responses[0].partition_responses[0];
    (p.error_code, p.base_offset)
}

/// The script: every kind of request the twin answers, in a fixed order,
/// checking each answer. It gives the member ids the core chose, and the
/// frames, which are the golden session when the core is the reference.
pub fn script(twin: &TestTwin) -> (Vec<String>, Vec<Frame>) {
    let mut c = Client::connect(twin);
    let mut seen = vec![];
    let versions = c.send(ApiVersionsRequest::default().with_client_software_name(s("scripted")).with_client_software_version(s("1")), 3);
    assert_eq!(versions.error_code, 0);
    let created = c.send(
        CreateTopicsRequest::default().with_timeout_ms(1000).with_topics(vec![
            create_topics_request::CreatableTopic::default().with_name(TopicName(s("det"))).with_num_partitions(2).with_replication_factor(1),
        ]),
        5,
    );
    assert_eq!(created.topics[0].error_code, 0);
    let md = c.send(MetadataRequest::default().with_topics(None), 9);
    assert_eq!(md.topics.len(), 1);
    // An idempotent producer, with a retry the broker recognises.
    let id = c.send(InitProducerIdRequest::default().with_transactional_id(None).with_transaction_timeout_ms(60000).with_producer_id(ProducerId(-1)).with_producer_epoch(-1), 2);
    let pid = id.producer_id.0;
    assert_eq!(id.error_code, 0, "{id:?}");
    for (i, compression) in [Compression::None, Compression::Gzip, Compression::Snappy, Compression::Lz4, Compression::Zstd].into_iter().enumerate() {
        let (code, base) = produce(&mut c, "det", 0, None, batch(pid, 0, 2 * i as i32, &["a", "b"], false, compression));
        assert_eq!((code, base), (0, 2 * i as i64), "{compression:?}");
    }
    let retry = produce(&mut c, "det", 0, None, batch(pid, 0, 8, &["a", "b"], false, Compression::None));
    assert_eq!(retry, (0, 8), "a retry gets its first offset");
    assert_eq!(produce(&mut c, "det", 0, None, batch(pid, 0, 20, &["gap"], false, Compression::None)).0, 45);
    // A transaction that commits records and offsets.
    let tx = c.send(InitProducerIdRequest::default().with_transactional_id(Some(TransactionalId(s("tx")))).with_transaction_timeout_ms(60000), 2);
    let (tpid, tepoch) = (tx.producer_id.0, tx.producer_epoch);
    let add = c.send(
        AddPartitionsToTxnRequest::default()
            .with_v3_and_below_transactional_id(TransactionalId(s("tx")))
            .with_v3_and_below_producer_id(ProducerId(tpid))
            .with_v3_and_below_producer_epoch(tepoch)
            .with_v3_and_below_topics(vec![add_partitions_to_txn_request::AddPartitionsToTxnTopic::default().with_name(TopicName(s("det"))).with_partitions(vec![1])]),
        3,
    );
    assert_eq!(add.results_by_topic_v3_and_below[0].results_by_partition[0].partition_error_code, 0);
    assert_eq!(produce(&mut c, "det", 1, Some("tx"), batch(tpid, tepoch, 0, &["t1", "t2"], true, Compression::None)), (0, 0));
    let ao = c.send(
        AddOffsetsToTxnRequest::default().with_transactional_id(TransactionalId(s("tx"))).with_producer_id(ProducerId(tpid)).with_producer_epoch(tepoch).with_group_id(GroupId(s("g"))),
        2,
    );
    assert_eq!(ao.error_code, 0);
    let to = c.send(
        TxnOffsetCommitRequest::default()
            .with_transactional_id(TransactionalId(s("tx")))
            .with_group_id(GroupId(s("g")))
            .with_producer_id(ProducerId(tpid))
            .with_producer_epoch(tepoch)
            .with_topics(vec![txn_offset_commit_request::TxnOffsetCommitRequestTopic::default().with_name(TopicName(s("det"))).with_partitions(vec![
                txn_offset_commit_request::TxnOffsetCommitRequestPartition::default().with_partition_index(0).with_committed_offset(4),
            ])]),
        2,
    );
    assert_eq!(to.topics[0].partitions[0].error_code, 0);
    let end = c.send(EndTxnRequest::default().with_transactional_id(TransactionalId(s("tx"))).with_producer_id(ProducerId(tpid)).with_producer_epoch(tepoch).with_committed(true), 2);
    assert_eq!(end.error_code, 0);
    // A group of one, with a member id the twin chooses from its seed.
    let join = |c: &mut Client, member: &str| {
        c.send(
            JoinGroupRequest::default()
                .with_group_id(GroupId(s("g")))
                .with_session_timeout_ms(10000)
                .with_rebalance_timeout_ms(10000)
                .with_member_id(s(member))
                .with_protocol_type(s("consumer"))
                .with_protocols(vec![join_group_request::JoinGroupRequestProtocol::default().with_name(s("range")).with_metadata({
                    let mut b = BytesMut::new();
                    b.extend_from_slice(&0i16.to_be_bytes());
                    ConsumerProtocolSubscription::default().with_topics(vec![s("det")]).encode(&mut b, 0).unwrap();
                    b.freeze()
                })]),
            5,
        )
    };
    let first = join(&mut c, "");
    assert_eq!(first.error_code, 79, "MEMBER_ID_REQUIRED");
    let member = first.member_id.to_string();
    seen.push(member.clone());
    let joined = join(&mut c, &member);
    assert_eq!(joined.error_code, 0);
    assert_eq!(joined.leader.as_str(), member);
    let sync = c.send(SyncGroupRequest::default().with_group_id(GroupId(s("g"))).with_generation_id(joined.generation_id).with_member_id(s(&member)), 3);
    assert_eq!(sync.error_code, 0);
    let mut a = sync.assignment.clone();
    let _ = bytes::Buf::get_i16(&mut a);
    let assignment = ConsumerProtocolAssignment::decode(&mut a, 0).unwrap();
    assert_eq!(assignment.assigned_partitions[0].partitions, vec![0, 1]);
    let hb = c.send(HeartbeatRequest::default().with_group_id(GroupId(s("g"))).with_generation_id(joined.generation_id).with_member_id(s(&member)), 3);
    assert_eq!(hb.error_code, 0);
    let commit = c.send(
        OffsetCommitRequest::default()
            .with_group_id(GroupId(s("g")))
            .with_generation_id_or_member_epoch(joined.generation_id)
            .with_member_id(s(&member))
            .with_topics(vec![offset_commit_request::OffsetCommitRequestTopic::default().with_name(TopicName(s("det"))).with_partitions(vec![
                offset_commit_request::OffsetCommitRequestPartition::default().with_partition_index(1).with_committed_offset(2),
            ])]),
        7,
    );
    assert_eq!(commit.topics[0].partitions[0].error_code, 0);
    let fetched = c.send(
        OffsetFetchRequest::default().with_group_id(GroupId(s("g"))).with_topics(Some(vec![
            offset_fetch_request::OffsetFetchRequestTopic::default().with_name(TopicName(s("det"))).with_partition_indexes(vec![0, 1]),
        ])),
        5,
    );
    let offsets: Vec<i64> = fetched.topics[0].partitions.iter().map(|p| p.committed_offset).collect();
    assert_eq!(offsets, vec![4, 2], "the transaction's offset, and the member's");
    // Read committed: the transaction's records, then past its marker.
    let fetch = c.send(
        FetchRequest::default().with_max_wait_ms(0).with_min_bytes(1).with_max_bytes(1 << 20).with_isolation_level(1).with_topics(vec![
            fetch_request::FetchTopic::default().with_topic(TopicName(s("det"))).with_partitions(vec![
                fetch_request::FetchPartition::default().with_partition(1).with_fetch_offset(0).with_partition_max_bytes(1 << 20),
                fetch_request::FetchPartition::default().with_partition(0).with_fetch_offset(0).with_partition_max_bytes(1 << 20),
            ]),
        ]),
        11,
    );
    let p1 = &fetch.responses[0].partitions[0];
    assert_eq!((p1.high_watermark, p1.last_stable_offset), (3, 3));
    let mut records = p1.records.clone().unwrap();
    let sets = RecordBatchDecoder::decode_all(&mut records).unwrap();
    let values: Vec<String> = sets.iter().flat_map(|s| s.records.iter()).map(|r| String::from_utf8_lossy(r.value.as_ref().unwrap()).to_string()).collect();
    assert_eq!(values, vec!["t1", "t2"]);
    let last = sets.last().unwrap();
    assert!(last.records.is_empty(), "an empty batch moves the position past the marker");
    let p0 = &fetch.responses[0].partitions[1];
    let mut records = p0.records.clone().unwrap();
    let sets = RecordBatchDecoder::decode_all(&mut records).unwrap();
    assert_eq!(sets.iter().map(|s| s.records.len()).sum::<usize>(), 10);
    let lo = c.send(
        ListOffsetsRequest::default().with_replica_id(BrokerId(-1)).with_topics(vec![list_offsets_request::ListOffsetsTopic::default().with_name(TopicName(s("det"))).with_partitions(vec![
            list_offsets_request::ListOffsetsPartition::default().with_partition_index(0).with_timestamp(-1),
        ])]),
        5,
    );
    assert_eq!(lo.topics[0].partitions[0].offset, 10);
    let leave = c.send(
        LeaveGroupRequest::default().with_group_id(GroupId(s("g"))).with_members(vec![leave_group_request::MemberIdentity::default().with_member_id(s(&member))]),
        3,
    );
    assert_eq!(leave.members[0].error_code, 0);
    (seen, c.frames)
}

