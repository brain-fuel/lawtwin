//! Record and replay: a session of a real client against a real broker,
//! recorded through the recorder, replays through the Rust core with no
//! divergence; a core that ignores idempotent dedupe is caught.

mod support;

use lawtwin_kafka_conformance::admin::Admin;
use lawtwin_kafka_fidelity::frame::frames_text;
use lawtwin_kafka_fidelity::replay::{replay_frames, replay_trace};
use rdkafka::ClientConfig;
use rdkafka::Message;
use rdkafka::consumer::{BaseConsumer, CommitMode, Consumer};
use rdkafka::producer::{BaseProducer, BaseRecord, Producer};
use serde_json::json;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};
use support::{Recording, real_kafka, unique};

fn producer(bootstrap: &str, extra: &[(&str, &str)]) -> BaseProducer {
    let mut c = ClientConfig::new();
    c.set("bootstrap.servers", bootstrap).set("linger.ms", "0").set("message.timeout.ms", "30000");
    for (k, v) in extra {
        c.set(*k, *v);
    }
    c.create().unwrap()
}

fn send(p: &BaseProducer, topic: &str, partition: i32, value: &str) {
    p.send(BaseRecord::to(topic).partition(partition).key("k").payload(value)).map_err(|(e, _)| e).unwrap();
}

/// A session with an idempotent producer whose acknowledgements are lost
/// twice, a committed and an aborted transaction, and a group that reads
/// committed and commits its offsets.
fn session(rec: &Recording, ns: &str) {
    let admin = Admin::new(&rec.admin_url());
    let topic = format!("{ns}orders");
    admin.post("/v1/state", json!({ "topics": [{ "name": topic, "partitions": 2 }] }));
    admin.post("/v1/faults/duplicate-delivery", json!({ "count": 2, "topic": topic }));
    let idempotent = producer(&rec.bootstrap(), &[("enable.idempotence", "true")]);
    for i in 0..6 {
        send(&idempotent, &topic, i % 2, &format!("idem-{i}"));
        idempotent.flush(Duration::from_secs(30)).unwrap();
    }
    assert!(admin.get("/v1/faults")["active"].as_array().unwrap().is_empty(), "both acknowledgements were lost");
    let tx = producer(&rec.bootstrap(), &[("transactional.id", &format!("{ns}tx"))]);
    tx.init_transactions(Duration::from_secs(30)).unwrap();
    tx.begin_transaction().unwrap();
    for i in 0..3 {
        send(&tx, &topic, 0, &format!("committed-{i}"));
    }
    tx.commit_transaction(Duration::from_secs(30)).unwrap();
    tx.begin_transaction().unwrap();
    for i in 0..2 {
        send(&tx, &topic, 0, &format!("aborted-{i}"));
    }
    tx.flush(Duration::from_secs(30)).unwrap();
    tx.abort_transaction(Duration::from_secs(30)).unwrap();
    drop(tx);
    let consumer: BaseConsumer = ClientConfig::new()
        .set("bootstrap.servers", rec.bootstrap())
        .set("group.id", format!("{ns}readers"))
        .set("auto.offset.reset", "earliest")
        .set("enable.auto.commit", "false")
        .set("isolation.level", "read_committed")
        .set("fetch.wait.max.ms", "100")
        .create()
        .unwrap();
    consumer.subscribe(&[&topic]).unwrap();
    let mut values = vec![];
    let deadline = Instant::now() + Duration::from_secs(30);
    while values.len() < 9 && Instant::now() < deadline {
        if let Some(Ok(m)) = consumer.poll(Duration::from_millis(100)) {
            values.push(String::from_utf8_lossy(m.payload().unwrap_or_default()).to_string());
        }
    }
    assert_eq!(values.len(), 9, "{values:?}");
    assert!(!values.iter().any(|v| v.starts_with("aborted")), "{values:?}");
    consumer.commit_consumer_state(CommitMode::Sync).unwrap();
    consumer.unsubscribe();
    drop(consumer);
    std::thread::sleep(Duration::from_millis(500));
}

fn mutant_core() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let target = root.join("target/mutant");
    let status = Command::new(env!("CARGO"))
        .args(["build", "--release", "-p", "lawtwin-kafka", "--features", "mutant-ignore-dedupe", "--target-dir"])
        .arg(&target)
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(status.success(), "building the mutant core");
    target.join("release/lawtwin-kafka")
}

#[test]
fn a_recorded_session_replays_through_the_rust_core_and_a_mutant_is_caught() {
    let Some(kafka) = real_kafka("record and replay") else { return };
    let rec = Recording::start(&kafka);
    session(&rec, &unique("replay"));
    let r = &rec.running.recorder;
    assert!(r.divergences().is_empty(), "the real broker's answers are the contract's: {:#?}", r.divergences());
    assert!(r.violations().is_empty(), "the real broker's history is lawful: {:#?}", r.violations());
    let frames = r.frames();
    assert!(frames.len() > 20, "a session of {} frames", frames.len());

    // Through the Rust core, written to a trace and read back.
    let dir = std::env::temp_dir().join(unique("lawtwin-trace"));
    std::fs::create_dir_all(&dir).unwrap();
    let prefix = dir.join("session");
    std::fs::write(dir.join("session.frames.jsonl"), frames_text(&frames)).unwrap();
    let report = replay_trace(&prefix, "rust").unwrap();
    assert_eq!(report.answered, report.requests, "{report:#?}");
    assert!(report.stalled.is_empty(), "{report:#?}");
    assert!(report.divergences.is_empty(), "the Rust core answers as the broker did: {:#?}", report.divergences);
    assert!(report.violations.is_empty(), "{:#?}", report.violations);

    // A core that ignores idempotent dedupe writes the retries again.
    let mutant = mutant_core();
    let report = replay_frames(&frames, Some(mutant.to_str().unwrap())).unwrap();
    assert!(
        report.divergences.iter().any(|d| d.api == "Produce"),
        "the mutant must be caught at a produce: {:#?}",
        report.divergences
    );
}
