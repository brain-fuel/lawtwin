//! librdkafka (through the rdkafka crate) in the client scenarios: producing,
//! consuming, consumer groups with a rebalance, the idempotent producer,
//! transactions read committed, faults and forced states. Each scenario
//! asserts what the client observes, speaking to the broker at a bootstrap
//! address and to LawTwin's control plane at a URL, so it runs against a
//! twin core or, through the recorder, against a real broker. A namespace
//! prefixes every topic, group and transactional id, so that scenarios
//! share one real broker without meeting.

use crate::admin::Admin;
use rdkafka::ClientConfig;
use rdkafka::ClientContext;
use rdkafka::Message;
use rdkafka::consumer::{BaseConsumer, Consumer, ConsumerContext, Rebalance};
use rdkafka::producer::{BaseProducer, BaseRecord, DeliveryResult, Producer, ProducerContext};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The scenarios, by name, in the order the kit runs them.
pub const SCENARIOS: &[&str] = &["produce-consume", "group-rebalance", "idempotent", "transactions", "faults", "force"];

/// Where a scenario runs: the broker, the control plane and the namespace.
pub struct Ctx {
    bootstrap: String,
    admin: Admin,
    ns: String,
}

impl Ctx {
    /// A scenario's context.
    pub fn new(bootstrap: &str, admin_url: &str, ns: &str) -> Self {
        Self { bootstrap: bootstrap.into(), admin: Admin::new(admin_url), ns: ns.into() }
    }

    fn bootstrap(&self) -> String {
        self.bootstrap.clone()
    }

    fn n(&self, name: &str) -> String {
        format!("{}{name}", self.ns)
    }

    fn post(&self, path: &str, body: serde_json::Value) -> serde_json::Value {
        self.admin.post(path, body)
    }

    fn get(&self, path: &str) -> serde_json::Value {
        self.admin.get(path)
    }

    fn call(&self, method: &str, path: &str, body: Option<serde_json::Value>) -> (u16, serde_json::Value) {
        self.admin.call(method, path, body)
    }
}

/// Runs a scenario: what it saw, or why it failed.
pub fn run(scenario: &str, bootstrap: &str, admin_url: &str, ns: &str) -> Result<serde_json::Value, String> {
    let ctx = Ctx::new(bootstrap, admin_url, ns);
    let f: fn(&Ctx) = match scenario {
        "produce-consume" => produce_and_consume_in_order,
        "group-rebalance" => a_group_rebalances_and_every_partition_has_one_owner,
        "idempotent" => an_idempotent_producer_writes_a_lost_ack_once_and_a_plain_one_twice,
        "transactions" => transactions_are_read_committed,
        "faults" => faults_are_seen_by_the_client_and_the_twin_stays_lawful,
        "force" => a_forced_state_is_what_the_client_reads,
        other => return Err(format!("no scenario {other}")),
    };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&ctx))) {
        Ok(()) => Ok(json!({ "client": "librdkafka", "scenario": scenario, "ok": true })),
        Err(p) => Err(p
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "the scenario panicked".into())),
    }
}

fn producer(twin: &Ctx, extra: &[(&str, &str)]) -> BaseProducer<Deliveries> {
    let mut c = ClientConfig::new();
    c.set("bootstrap.servers", twin.bootstrap()).set("message.timeout.ms", "20000").set("linger.ms", "0");
    for (k, v) in extra {
        c.set(*k, *v);
    }
    c.create_with_context(Deliveries::default()).unwrap()
}

/// Each delivery's partition and offset, or why it failed.
type Outcomes = Arc<Mutex<Vec<Result<(i32, i64), String>>>>;

#[derive(Clone, Default)]
struct Deliveries(Outcomes);

impl ClientContext for Deliveries {}
impl ProducerContext for Deliveries {
    type DeliveryOpaque = ();
    fn delivery(&self, result: &DeliveryResult<'_>, _: ()) {
        let r = match result {
            Ok(m) => Ok((m.partition(), m.offset())),
            Err((e, _)) => Err(e.to_string()),
        };
        self.0.lock().unwrap().push(r);
    }
}

#[derive(Clone, Default)]
struct Rebalances(Arc<Mutex<Vec<String>>>);

impl ClientContext for Rebalances {}
impl ConsumerContext for Rebalances {
    fn post_rebalance(&self, _: &BaseConsumer<Self>, r: &Rebalance<'_>) {
        let what = match r {
            Rebalance::Assign(tpl) => format!("assign {}", tpl.count()),
            Rebalance::Revoke(tpl) => format!("revoke {}", tpl.count()),
            Rebalance::Error(e) => format!("error {e}"),
        };
        self.0.lock().unwrap().push(what);
    }
}

fn consumer(twin: &Ctx, group: &str, extra: &[(&str, &str)]) -> BaseConsumer<Rebalances> {
    let mut c = ClientConfig::new();
    c.set("bootstrap.servers", twin.bootstrap())
        .set("group.id", twin.n(group))
        .set("auto.offset.reset", "earliest")
        .set("enable.auto.commit", "false")
        .set("session.timeout.ms", "10000")
        .set("heartbeat.interval.ms", "500")
        .set("fetch.wait.max.ms", "100");
    for (k, v) in extra {
        c.set(*k, *v);
    }
    c.create_with_context(Rebalances::default()).unwrap()
}

fn send(twin: &Ctx, p: &BaseProducer<Deliveries>, topic: &str, key: &str, value: &str, partition: Option<i32>) {
    let topic = twin.n(topic);
    let mut r = BaseRecord::to(&topic).key(key).payload(value);
    if let Some(p) = partition {
        r = r.partition(p);
    }
    p.send(r).map_err(|(e, _)| e).unwrap();
}

fn flush(p: &BaseProducer<Deliveries>) {
    p.flush(Duration::from_secs(30)).unwrap();
}

/// Polls until `want` records arrived or the time is up; the records by partition.
fn drain<C: ConsumerContext>(c: &BaseConsumer<C>, want: usize, secs: u64) -> BTreeMap<i32, Vec<(i64, String)>> {
    let mut got: BTreeMap<i32, Vec<(i64, String)>> = BTreeMap::new();
    let deadline = Instant::now() + Duration::from_secs(secs);
    while got.values().map(Vec::len).sum::<usize>() < want && Instant::now() < deadline {
        if let Some(Ok(m)) = c.poll(Duration::from_millis(100)) {
            let v = String::from_utf8_lossy(m.payload().unwrap_or_default()).to_string();
            got.entry(m.partition()).or_default().push((m.offset(), v));
        }
    }
    got
}

fn create(twin: &Ctx, topic: &str, partitions: i32) {
    twin.post("/v1/state", json!({ "topics": [{ "name": twin.n(topic), "partitions": partitions }] }));
}

fn produce_and_consume_in_order(twin: &Ctx) {
    create(twin, "orders", 3);
    let p = producer(twin, &[("acks", "all")]);
    for i in 0..30 {
        send(twin, &p, "orders", &format!("key-{}", i % 7), &format!("v{i}"), None);
    }
    flush(&p);
    let delivered = p.context().0.lock().unwrap().clone();
    assert_eq!(delivered.len(), 30);
    assert!(delivered.iter().all(|d| d.is_ok()), "{delivered:?}");
    let c = consumer(twin, "readers", &[]);
    c.subscribe(&[&twin.n("orders")]).unwrap();
    let got = drain(&c, 30, 30);
    assert_eq!(got.values().map(Vec::len).sum::<usize>(), 30, "{got:?}");
    for records in got.values() {
        let offsets: Vec<i64> = records.iter().map(|r| r.0).collect();
        assert!(offsets.windows(2).all(|w| w[1] == w[0] + 1), "dense and in order: {offsets:?}");
    }
    // Records with one key went to one partition, as the murmur2 partitioner says.
    let mut owner: BTreeMap<String, i32> = BTreeMap::new();
    for (p, records) in &got {
        for (_, v) in records {
            let i: usize = v[1..].parse().unwrap();
            let k = format!("key-{}", i % 7);
            assert_eq!(*owner.entry(k).or_insert(*p), *p);
        }
    }
    c.commit_consumer_state(rdkafka::consumer::CommitMode::Sync).unwrap();
    drop(c);
}

fn a_group_rebalances_and_every_partition_has_one_owner(twin: &Ctx) {
    create(twin, "work", 4);
    let a = consumer(twin, "workers", &[]);
    a.subscribe(&[&twin.n("work")]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while a.assignment().unwrap().count() < 4 && Instant::now() < deadline {
        a.poll(Duration::from_millis(100));
    }
    assert_eq!(a.assignment().unwrap().count(), 4);
    let b = consumer(twin, "workers", &[]);
    b.subscribe(&[&twin.n("work")]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        a.poll(Duration::from_millis(50));
        b.poll(Duration::from_millis(50));
        let (x, y) = (a.assignment().unwrap(), b.assignment().unwrap());
        if x.count() == 2 && y.count() == 2 {
            let xs: BTreeSet<i32> = x.elements().iter().map(|e| e.partition()).collect();
            let ys: BTreeSet<i32> = y.elements().iter().map(|e| e.partition()).collect();
            assert!(xs.is_disjoint(&ys));
            break;
        }
        assert!(Instant::now() < deadline, "no split: {} and {}", x.count(), y.count());
    }
    let view = twin.get("/v1/state");
    let group = &view["groups"][0];
    assert_eq!(group["members"].as_array().unwrap().len(), 2);
    drop(b);
    let deadline = Instant::now() + Duration::from_secs(30);
    while a.assignment().unwrap().count() < 4 {
        a.poll(Duration::from_millis(50));
        assert!(Instant::now() < deadline, "a never got the partitions back");
    }
    assert!(a.context().0.lock().unwrap().len() >= 3);
    drop(a);
}

fn an_idempotent_producer_writes_a_lost_ack_once_and_a_plain_one_twice(twin: &Ctx) {
    create(twin, "pay", 1);
    twin.post("/v1/faults/duplicate-delivery", json!({ "count": 2 }));
    let p = producer(twin, &[("enable.idempotence", "true")]);
    for i in 0..5 {
        send(twin, &p, "pay", "k", &format!("idem-{i}"), Some(0));
        flush(&p);
    }
    let c = consumer(twin, "audit", &[]);
    c.subscribe(&[&twin.n("pay")]).unwrap();
    let got = drain(&c, 5, 20);
    let values: Vec<&str> = got[&0].iter().map(|r| r.1.as_str()).collect();
    assert_eq!(values, vec!["idem-0", "idem-1", "idem-2", "idem-3", "idem-4"], "written once each");
    assert!(twin.get("/v1/faults")["active"].as_array().unwrap().is_empty(), "both acks were lost");
    // Without idempotence, a lost acknowledgement means the record is written twice.
    twin.post("/v1/faults/duplicate-delivery", json!({ "count": 1 }));
    let plain = producer(twin, &[("enable.idempotence", "false"), ("acks", "all"), ("retries", "5")]);
    send(twin, &plain, "pay", "k", "plain", Some(0));
    flush(&plain);
    let more = drain(&c, 2, 20);
    let values: Vec<&str> = more.get(&0).map(|v| v.iter().map(|r| r.1.as_str()).collect()).unwrap_or_default();
    assert_eq!(values, vec!["plain", "plain"]);
    drop(c);
}

fn transactions_are_read_committed(twin: &Ctx) {
    create(twin, "ledger", 2);
    let p = producer(twin, &[("transactional.id", &twin.n("ledger-writer"))]);
    p.init_transactions(Duration::from_secs(20)).unwrap();
    p.begin_transaction().unwrap();
    for i in 0..3 {
        send(twin, &p, "ledger", "acct", &format!("committed-{i}"), Some(0));
    }
    p.commit_transaction(Duration::from_secs(20)).unwrap();
    p.begin_transaction().unwrap();
    for i in 0..2 {
        send(twin, &p, "ledger", "acct", &format!("aborted-{i}"), Some(0));
    }
    flush(&p);
    p.abort_transaction(Duration::from_secs(20)).unwrap();
    p.begin_transaction().unwrap();
    send(twin, &p, "ledger", "acct", "committed-3", Some(0));
    p.commit_transaction(Duration::from_secs(20)).unwrap();

    let committed = consumer(twin, "rc", &[("isolation.level", "read_committed")]);
    committed.subscribe(&[&twin.n("ledger")]).unwrap();
    let got = drain(&committed, 4, 20);
    let values: Vec<&str> = got[&0].iter().map(|r| r.1.as_str()).collect();
    assert_eq!(values, vec!["committed-0", "committed-1", "committed-2", "committed-3"]);
    let uncommitted = consumer(twin, "ru", &[("isolation.level", "read_uncommitted")]);
    uncommitted.subscribe(&[&twin.n("ledger")]).unwrap();
    let got = drain(&uncommitted, 6, 20);
    assert_eq!(got[&0].len(), 6, "read uncommitted sees the aborted records too");
    // A second instance with the same transactional id fences the first.
    let zombie_fence = producer(twin, &[("transactional.id", &twin.n("ledger-writer"))]);
    zombie_fence.init_transactions(Duration::from_secs(20)).unwrap();
    p.begin_transaction().unwrap();
    send(twin, &p, "ledger", "acct", "zombie", Some(0));
    let fenced = p.commit_transaction(Duration::from_secs(20));
    assert!(fenced.is_err(), "the zombie must be fenced");
    drop(committed);
    drop(uncommitted);
}

fn faults_are_seen_by_the_client_and_the_twin_stays_lawful(twin: &Ctx) {
    create(twin, "f", 1);
    let p = producer(twin, &[("acks", "all")]);
    send(twin, &p, "f", "k", "before", Some(0));
    flush(&p);

    // Request latency: a produce takes at least the delay.
    twin.post("/v1/faults/request-latency", json!({ "delay": "400ms", "api": "Produce" }));
    let t = Instant::now();
    send(twin, &p, "f", "k", "slow", Some(0));
    flush(&p);
    assert!(t.elapsed() >= Duration::from_millis(400), "{:?}", t.elapsed());
    twin.call("DELETE", "/v1/faults/request-latency", None);

    // A leader change: the client refreshes its metadata and its retry succeeds.
    twin.post("/v1/faults/leader-change", json!({ "topic": twin.n("f") }));
    send(twin, &p, "f", "k", "after-leader-change", Some(0));
    flush(&p);
    let epoch = twin.get("/v1/state")["topics"][0]["partitions"][0]["leaderEpoch"].as_i64().unwrap();
    assert_eq!(epoch, 1);

    // The broker is unavailable until the twin clock passes the fault.
    twin.post("/v1/faults/broker-unavailable", json!({ "for": "5s" }));
    send(twin, &p, "f", "k", "while-down", Some(0));
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(p.context().0.lock().unwrap().len(), 3, "nothing is delivered while the broker is down");
    twin.post("/v1/clock/advance", json!({ "by": "5s" }));
    flush(&p);
    let deliveries = p.context().0.lock().unwrap().clone();
    assert_eq!(deliveries.len(), 4);
    assert!(deliveries.iter().all(|d| d.is_ok()), "{deliveries:?}");

    // Consumer lag: one record per fetch.
    twin.post("/v1/faults/consumer-lag", json!({ "records": 1, "delay": "50ms" }));
    let c = consumer(twin, "lagging", &[]);
    c.subscribe(&[&twin.n("f")]).unwrap();
    let got = drain(&c, 4, 30);
    assert_eq!(got[&0].len(), 4);
    let fetches = twin.get("/v1/history?limit=100000").as_array().unwrap().iter()
        .filter(|e| e["op"] == "fetchRequest" && e["answer"]["ok"]["records"].as_array().is_some_and(|r| !r.is_empty()))
        .map(|e| e["answer"]["ok"]["records"].as_array().unwrap().len())
        .collect::<Vec<_>>();
    assert!(fetches.iter().all(|n| *n == 1), "{fetches:?}");
    twin.call("DELETE", "/v1/faults", None);

    // A rebalance storm: the consumer sees more rebalances, each a new generation.
    let before = c.context().0.lock().unwrap().len();
    twin.post("/v1/faults/rebalance-storm", json!({ "rounds": 2 }));
    let deadline = Instant::now() + Duration::from_secs(30);
    while twin.get("/v1/faults")["active"].as_array().unwrap().iter().any(|f| f["name"] == "rebalance-storm") {
        c.poll(Duration::from_millis(50));
        assert!(Instant::now() < deadline, "the storm never passed");
    }
    for _ in 0..20 {
        c.poll(Duration::from_millis(50));
    }
    assert!(c.context().0.lock().unwrap().len() >= before + 2, "{:?}", c.context().0.lock().unwrap());
    drop(c);
}

fn a_forced_state_is_what_the_client_reads(twin: &Ctx) {
    let (status, refused) = twin.call(
        "POST",
        "/v1/state",
        Some(json!({ "topics": [{ "name": twin.n("orders"), "partitions": 2 }], "records": [{ "topic": twin.n("orders"), "partition": 5, "records": [{ "value": "x" }] }] })),
    );
    assert_eq!(status, 422);
    assert_eq!(refused["law"], "a send to a partition the topic lacks is refused");
    twin.post(
        "/v1/state",
        json!({
            "topics": [{ "name": twin.n("orders"), "partitions": 1 }],
            "records": [{ "topic": twin.n("orders"), "partition": 0, "records": [
                { "key": "a", "value": "r0" }, { "key": "a", "value": "r1" }, { "key": "a", "value": "r2" }, { "key": "a", "value": "r3" } ] }],
            "committed": [{ "group": twin.n("resume"), "topic": twin.n("orders"), "partition": 0, "offset": 2 }]
        }),
    );
    let c = consumer(twin, "resume", &[]);
    c.subscribe(&[&twin.n("orders")]).unwrap();
    let got = drain(&c, 2, 20);
    let values: Vec<&str> = got[&0].iter().map(|r| r.1.as_str()).collect();
    assert_eq!(values, vec!["r2", "r3"], "the consumer resumes at the forced committed offset");
    drop(c);
}
