//! Leader hints must keep clients inside the recorder without erasing broker
//! answers or bypassing declared faults. The Produce fixture is the actual
//! response that let a librdkafka client escape during provider conformance.
//! ref:kafka-produce-leader-endpoints ref:kafka-fetch-leader-endpoints

use super::*;

async fn recorder() -> Arc<Recorder> {
    start(RecorderConfig {
        advertise: Some("proxy.lawtwin.test:29092".into()),
        clock: ClockMode::Frozen,
        ..Default::default()
    })
    .await
    .unwrap()
    .recorder
}

fn wire<R: Encodable>(key: ApiKey, version: i16, header: &ResponseHeader, value: &R) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    header.encode(&mut bytes, key.response_header_version(version)).unwrap();
    value.encode(&mut bytes, version).unwrap();
    bytes.to_vec()
}

fn produce() -> (Frame, ResponseHeader, ProduceResponse) {
    let frame: Frame = serde_json::from_str(include_str!("../tests/fixtures/produce-leader-v10.json")).unwrap();
    let (header, mut body) = crate::decode::split_response(frame.response.as_ref().unwrap(), ApiKey::Produce, 10).unwrap();
    let response = ProduceResponse::decode(&mut body, 10).unwrap();
    assert_eq!(response.responses[0].partition_responses[0].error_code, 6);
    assert_eq!(response.node_endpoints[0].host.as_str(), "localhost");
    (frame, header, response)
}

fn redirected(response: &mut ProduceResponse) {
    for endpoint in &mut response.node_endpoints {
        endpoint.host = StrBytes::from_static_str("proxy.lawtwin.test");
        endpoint.port = 29092;
    }
}

#[tokio::test]
async fn leader_hints_preserve_the_observed_produce_answer_and_extensions() {
    let rec = recorder().await;
    let (mut frame, mut header, mut response) = produce();
    header.unknown_tagged_fields.insert(11, Bytes::from_static(b"header-extension"));
    response.unknown_tagged_fields.insert(12, Bytes::from_static(b"response-extension"));
    response.node_endpoints[0].rack = Some(StrBytes::from_static_str("rack-a"));
    response.node_endpoints[0].unknown_tagged_fields.insert(13, Bytes::from_static(b"node-extension"));
    response.node_endpoints.push(response.node_endpoints[0].clone().with_node_id(BrokerId(2)).with_port(19094));
    frame.response = Some(wire(ApiKey::Produce, 10, &header, &response));
    let original = frame.response.clone();
    let (delivered, delay) = rec.respond(&mut frame);
    redirected(&mut response);
    assert_eq!(delivered, wire(ApiKey::Produce, 10, &header, &response));
    assert_eq!(delay, 0);
    assert_eq!(frame.response, original, "the original broker response is capture evidence");
    assert_eq!(frame.delivered, None, "address rewriting is not a declared fault");
}

#[tokio::test]
async fn leader_hints_still_apply_the_lost_acknowledgement_fault() {
    let rec = recorder().await;
    let (mut frame, header, mut response) = produce();
    response.responses[0].partition_responses.push(
        produce_response::PartitionProduceResponse::default().with_index(1).with_base_offset(11),
    );
    frame.response = Some(wire(ApiKey::Produce, 10, &header, &response));
    let original = frame.response.clone();
    rec.shadow.lock().raise(Fault::DuplicateDelivery { count: 1, topic: None }, None);
    let (delivered, _) = rec.respond(&mut frame);
    redirected(&mut response);
    response.responses[0].partition_responses[1].error_code = 7;
    response.responses[0].partition_responses[1].base_offset = -1;
    assert_eq!(delivered, wire(ApiKey::Produce, 10, &header, &response));
    assert_eq!(frame.response, original);
    assert_eq!(frame.delivered, Some(delivered));
    assert!(!rec.shadow.lock().take_duplicate("another-topic"), "the fault count was consumed");
}

#[tokio::test]
async fn leader_hints_preserve_fetch_data_and_leader_identity() {
    let rec = recorder().await;
    let (mut frame, header, _) = produce();
    let mut response = FetchResponse::default()
        .with_responses(vec![fetch_response::FetchableTopicResponse::default().with_partitions(vec![
            fetch_response::PartitionData::default().with_partition_index(4).with_error_code(6)
                .with_high_watermark(27).with_last_stable_offset(19)
                .with_current_leader(fetch_response::LeaderIdAndEpoch::default().with_leader_id(BrokerId(9)).with_leader_epoch(17)),
        ])])
        .with_node_endpoints(vec![fetch_response::NodeEndpoint::default().with_node_id(BrokerId(9))
            .with_host(StrBytes::from_static_str("real-broker.example")).with_port(19092)
            .with_rack(Some(StrBytes::from_static_str("rack-b")))]);
    response.unknown_tagged_fields.insert(21, Bytes::from_static(b"fetch-extension"));
    frame.key = ApiKey::Fetch as i16;
    frame.api = "Fetch".into();
    frame.version = 16;
    frame.response = Some(wire(ApiKey::Fetch, 16, &header, &response));
    let original = frame.response.clone();
    let (delivered, _) = rec.respond(&mut frame);
    response.node_endpoints[0].host = StrBytes::from_static_str("proxy.lawtwin.test");
    response.node_endpoints[0].port = 29092;
    assert_eq!(delivered, wire(ApiKey::Fetch, 16, &header, &response));
    assert_eq!(frame.response, original);
}

#[tokio::test]
async fn leader_hints_leave_legacy_responses_byte_identical() {
    let rec = recorder().await;
    let (mut frame, header, mut response) = produce();
    response.node_endpoints.clear();
    for topic in &mut response.responses {
        for partition in &mut topic.partition_responses {
            partition.current_leader = Default::default();
        }
    }
    frame.version = 9;
    frame.response = Some(wire(ApiKey::Produce, 9, &header, &response));
    let original = frame.response.clone().unwrap();
    let (delivered, _) = rec.respond(&mut frame);
    assert_eq!(delivered, original);
    assert_eq!(frame.response, Some(original));
    assert_eq!(frame.delivered, None);
}
