//! The Rust core's codec against the shared golden frames
//! (`twins/kafka/shared/golden/`), which every core's codec tests read:
//! each request and response decodes, and encodes back to the same bytes.

use bytes::{Bytes, BytesMut};
use kafka_protocol::messages::*;
use kafka_protocol::protocol::{Decodable, Encodable};

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
}

fn round_trip<M: Decodable + Encodable>(bytes: &[u8], header: i16, version: i16, request: bool) -> Vec<u8> {
    let mut buf = Bytes::copy_from_slice(bytes);
    let mut out = BytesMut::new();
    if request {
        RequestHeader::decode(&mut buf, header).unwrap().encode(&mut out, header).unwrap();
    } else {
        ResponseHeader::decode(&mut buf, header).unwrap().encode(&mut out, header).unwrap();
    }
    M::decode(&mut buf, version).unwrap().encode(&mut out, version).unwrap();
    assert!(buf.is_empty(), "bytes left over");
    out.to_vec()
}

macro_rules! both {
    ($req:ty, $resp:ty, $f:expr, $key:expr) => {{
        let (rq, rs, v) = $f;
        (
            round_trip::<$req>(&rq, $key.request_header_version(v), v, true),
            round_trip::<$resp>(&rs, $key.response_header_version(v), v, false),
        )
    }};
}

#[test]
fn every_golden_frame_decodes_and_encodes_back_to_its_bytes() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../shared/golden/session-42.frames.jsonl");
    let text = std::fs::read_to_string(path).unwrap();
    let mut count = 0;
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let key = ApiKey::try_from(v["key"].as_i64().unwrap() as i16).unwrap();
        let version = v["version"].as_i64().unwrap() as i16;
        let rq = unhex(v["request"].as_str().unwrap());
        let rs = unhex(v["response"].as_str().unwrap());
        let f = (rq.clone(), rs.clone(), version);
        let (a, b) = match key {
            ApiKey::ApiVersions => both!(ApiVersionsRequest, ApiVersionsResponse, f, key),
            ApiKey::CreateTopics => both!(CreateTopicsRequest, CreateTopicsResponse, f, key),
            ApiKey::Metadata => both!(MetadataRequest, MetadataResponse, f, key),
            ApiKey::InitProducerId => both!(InitProducerIdRequest, InitProducerIdResponse, f, key),
            ApiKey::Produce => both!(ProduceRequest, ProduceResponse, f, key),
            ApiKey::AddPartitionsToTxn => both!(AddPartitionsToTxnRequest, AddPartitionsToTxnResponse, f, key),
            ApiKey::AddOffsetsToTxn => both!(AddOffsetsToTxnRequest, AddOffsetsToTxnResponse, f, key),
            ApiKey::TxnOffsetCommit => both!(TxnOffsetCommitRequest, TxnOffsetCommitResponse, f, key),
            ApiKey::EndTxn => both!(EndTxnRequest, EndTxnResponse, f, key),
            ApiKey::JoinGroup => both!(JoinGroupRequest, JoinGroupResponse, f, key),
            ApiKey::SyncGroup => both!(SyncGroupRequest, SyncGroupResponse, f, key),
            ApiKey::Heartbeat => both!(HeartbeatRequest, HeartbeatResponse, f, key),
            ApiKey::OffsetCommit => both!(OffsetCommitRequest, OffsetCommitResponse, f, key),
            ApiKey::OffsetFetch => both!(OffsetFetchRequest, OffsetFetchResponse, f, key),
            ApiKey::Fetch => both!(FetchRequest, FetchResponse, f, key),
            ApiKey::ListOffsets => both!(ListOffsetsRequest, ListOffsetsResponse, f, key),
            ApiKey::LeaveGroup => both!(LeaveGroupRequest, LeaveGroupResponse, f, key),
            other => panic!("no codec check for {other:?}"),
        };
        assert_eq!(a, rq, "{key:?} v{version} request");
        assert_eq!(b, rs, "{key:?} v{version} response");
        count += 1;
    }
    assert!(count > 20);
}
