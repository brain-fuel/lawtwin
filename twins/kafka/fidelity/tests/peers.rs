//! The simulated peers against the twin and, when `LAWTWIN_KAFKA` names
//! one, against a real broker: a producer peer's records are what the value
//! type says and are read back whole; a consumer peer reports exactly the
//! records an application wrote that break a law or the value type.

mod support;

use lawtwin_kafka::history::Isolation;
use lawtwin_kafka_conformance::core::TestTwin;
use lawtwin_kafka_fidelity::lawtype::{Format, Types};
use lawtwin_kafka_fidelity::peer::{ConsumeOptions, PeerFault, ProduceOptions, consume, produce};
use rdkafka::ClientConfig;
use rdkafka::admin::{AdminClient, AdminOptions, NewTopic, TopicReplication};
use rdkafka::client::DefaultClientContext;
use rdkafka::producer::{BaseProducer, BaseRecord, Producer};
use std::path::PathBuf;
use std::time::Duration;
use support::{real_kafka, unique};

fn orders() -> (Types, lawtwin_kafka_fidelity::lawtype::Ty) {
    let spec = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../examples/kafka/orders.lawspec");
    let types = Types::load(&spec).unwrap();
    let ty = types.parse_type("Order").unwrap();
    (types, ty)
}

fn create_topic(bootstrap: &str, topic: &str, partitions: i32) {
    let admin: AdminClient<DefaultClientContext> = ClientConfig::new().set("bootstrap.servers", bootstrap).create().unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let r = rt.block_on(admin.create_topics(&[NewTopic::new(topic, partitions, TopicReplication::Fixed(1))], &AdminOptions::new())).unwrap();
    assert!(r.iter().all(|x| x.is_ok()), "{r:?}");
    std::thread::sleep(Duration::from_millis(500));
}

/// A producer peer feeds a topic with orders in transactions, some aborted
/// and some poisoned; a consumer peer reads it committed and reports
/// exactly the poisoned records that were committed.
fn producer_and_consumer_peers(bootstrap: &str, ns: &str) {
    let topic = format!("{ns}orders");
    create_topic(bootstrap, &topic, 3);
    let (types, ty) = orders();
    let o = ProduceOptions {
        bootstrap: bootstrap.into(),
        topic: topic.clone(),
        count: Some(60),
        seed: 7,
        keys: Some(5),
        headers: vec![("source".into(), "lawtwin-peer".into())],
        value_type: Some((types.clone(), ty.clone())),
        transactional_id: Some(format!("{ns}feeder")),
        txn_size: 10,
        idempotent: true,
        faults: vec![PeerFault::Abort(0.3), PeerFault::Poison(0.1)],
        ..Default::default()
    };
    let (sent, summary) = produce(&o).unwrap();
    assert_eq!(summary["failed"], 0, "{summary}");
    assert!(summary["transactions"]["aborted"].as_u64().unwrap() >= 1, "the seed aborts a transaction: {summary}");
    assert!(sent.iter().any(|s| s.faults.contains(&"poison".to_string())), "the seed poisons a record");
    // The same seed sends the same records.
    let again = lawtwin_kafka_fidelity::peer::produce(&ProduceOptions { topic: format!("{topic}-again"), transactional_id: Some(format!("{ns}feeder-2")), ..o.clone() });
    if let Ok((again, _)) = again {
        let values = |v: &[lawtwin_kafka_fidelity::peer::Sent]| v.iter().map(|s| (s.key.clone(), s.value.clone())).collect::<Vec<_>>();
        assert_eq!(values(&sent), values(&again), "a seed fixes the records");
    }
    let committed: Vec<_> = sent.iter().filter(|s| s.outcome.as_deref() == Some("committed")).collect();
    let poisoned = committed.iter().filter(|s| s.faults.contains(&"poison".to_string())).count();
    for s in &committed {
        if s.faults.is_empty() {
            let bytes = serde_json::to_vec(&s.value).unwrap();
            assert!(types.validate(&ty, &bytes, Format::Json).is_empty(), "a generated value is an Order: {}", s.value);
        }
    }
    let (reads, report) = consume(&ConsumeOptions {
        bootstrap: bootstrap.into(),
        topic: topic.clone(),
        group: format!("{ns}auditors"),
        isolation: Isolation::ReadCommitted,
        value_type: Some((types, ty)),
        expect: Some(committed.len() as u64),
        duration: Duration::from_secs(60),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(reads.len(), committed.len(), "{report:#}");
    let violations = report["violations"].as_array().unwrap();
    assert_eq!(violations.len(), poisoned, "exactly the committed poisoned records: {report:#}");
    assert!(violations.iter().all(|v| v["law"] == "every value is a Order"), "{report:#}");
}

/// An application writes a record whose value breaks the type, and a
/// transaction it aborts; a consumer peer reading committed reports the
/// record and never sees the aborted ones.
fn consumer_peer_checks_an_application(bootstrap: &str, ns: &str) {
    let topic = format!("{ns}payments");
    create_topic(bootstrap, &topic, 1);
    let app: BaseProducer = ClientConfig::new()
        .set("bootstrap.servers", bootstrap)
        .set("transactional.id", format!("{ns}app"))
        .create()
        .unwrap();
    app.init_transactions(Duration::from_secs(30)).unwrap();
    let good = r#"{"id":"ord-000001","customer":"ada","amount":120,"currency":"EUR","lines":[{"sku":"ABC-1234","quantity":2}],"note":null}"#;
    let bad = r#"{"id":"ord-000002","customer":"ada","amount":0,"currency":"EUR","lines":[{"sku":"ABC-1234","quantity":2}],"note":null}"#;
    let send = |v: &str| app.send(BaseRecord::<str, str>::to(&topic).key("ada").payload(v)).map_err(|(e, _)| e).unwrap();
    app.begin_transaction().unwrap();
    send(good);
    send(bad);
    app.commit_transaction(Duration::from_secs(30)).unwrap();
    app.begin_transaction().unwrap();
    send("never read");
    app.flush(Duration::from_secs(30)).unwrap();
    app.abort_transaction(Duration::from_secs(30)).unwrap();
    app.begin_transaction().unwrap();
    send(good);
    app.commit_transaction(Duration::from_secs(30)).unwrap();
    let (types, ty) = orders();
    let (reads, report) = consume(&ConsumeOptions {
        bootstrap: bootstrap.into(),
        topic,
        group: format!("{ns}checker"),
        value_type: Some((types, ty)),
        expect: Some(3),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(reads.len(), 3, "{report:#}");
    let violations = report["violations"].as_array().unwrap();
    assert_eq!(violations.len(), 1, "{report:#}");
    assert_eq!(violations[0]["law"], "every value is a Order");
    assert_eq!(violations[0]["offset"], 1);
    assert!(violations[0]["detail"].as_str().unwrap().contains("value.amount"), "{report:#}");
}

#[test]
fn producer_and_consumer_peers_against_the_twin() {
    let twin = TestTwin::start_core(None, 31);
    producer_and_consumer_peers(&twin.bootstrap(), "");
    twin.assert_lawful();
}

#[test]
fn consumer_peer_checks_an_application_against_the_twin() {
    let twin = TestTwin::start_core(None, 32);
    consumer_peer_checks_an_application(&twin.bootstrap(), "");
    twin.assert_lawful();
}

#[test]
fn producer_and_consumer_peers_against_real_kafka() {
    let Some(kafka) = real_kafka("the peers against real Kafka") else { return };
    producer_and_consumer_peers(&kafka, &unique("peers"));
}

#[test]
fn consumer_peer_checks_an_application_against_real_kafka() {
    let Some(kafka) = real_kafka("the consumer peer against real Kafka") else { return };
    consumer_peer_checks_an_application(&kafka, &unique("peers"));
}
