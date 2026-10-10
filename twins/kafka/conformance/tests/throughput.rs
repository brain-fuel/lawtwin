//! How fast a core answers a real client over the wire: librdkafka produces
//! records in batches and a group reads them back. Not part of the kit's
//! verdict; run it with `cargo test --release -p lawtwin-kafka-conformance
//! --test throughput -- --ignored --nocapture`.

use lawtwin_kafka_conformance::core::TestTwin;
use rdkafka::ClientConfig;
use rdkafka::consumer::{BaseConsumer, Consumer};
use rdkafka::producer::{BaseProducer, BaseRecord, Producer};
use std::time::{Duration, Instant};

#[test]
#[ignore]
fn records_per_second_over_the_wire() {
    let n: usize = std::env::var("LAWTWIN_RECORDS").ok().and_then(|s| s.parse().ok()).unwrap_or(200_000);
    let twin = TestTwin::start(1);
    twin.post("/v1/state", serde_json::json!({ "topics": [{ "name": "bench", "partitions": 8 }] }));
    let p: BaseProducer = ClientConfig::new()
        .set("bootstrap.servers", twin.bootstrap())
        .set("linger.ms", "5")
        .set("batch.num.messages", "10000")
        .set("queue.buffering.max.messages", "1000000")
        .create()
        .unwrap();
    let value = vec![b'x'; 100];
    let start = Instant::now();
    for i in 0..n {
        let key = (i % 1024).to_string();
        loop {
            match p.send(BaseRecord::to("bench").key(&key).payload(&value)) {
                Ok(()) => break,
                Err((_, _)) => {
                    p.poll(Duration::from_millis(1));
                }
            }
        }
    }
    p.flush(Duration::from_secs(120)).unwrap();
    let produced = start.elapsed();
    let c: BaseConsumer = ClientConfig::new()
        .set("bootstrap.servers", twin.bootstrap())
        .set("group.id", "bench")
        .set("auto.offset.reset", "earliest")
        .create()
        .unwrap();
    c.subscribe(&["bench"]).unwrap();
    let start = Instant::now();
    let mut read = 0;
    while read < n && start.elapsed() < Duration::from_secs(300) {
        if let Some(Ok(_)) = c.poll(Duration::from_millis(100)) {
            read += 1;
        }
    }
    let consumed = start.elapsed();
    assert_eq!(read, n);
    println!(
        "produced {n} records in {:.2}s ({:.0}/s); consumed them in {:.2}s ({:.0}/s)",
        produced.as_secs_f64(),
        n as f64 / produced.as_secs_f64(),
        consumed.as_secs_f64(),
        n as f64 / consumed.as_secs_f64()
    );
}
