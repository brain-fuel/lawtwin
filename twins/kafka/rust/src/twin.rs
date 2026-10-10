//! The twin: the model, its history, the twin clock, the seeded randomness,
//! and the little the wire needs beside the model (record timestamps, group
//! members' protocol metadata, leader epochs, active faults).
//!
//! Every change to the cluster's state is an operation of the contract's
//! `Broker` ability, run on the model and written to the history. Nothing
//! else holds cluster state: what the wire layer keeps is how to encode the
//! model's answers, never an answer of its own.

use crate::clock::{Clock, ClockMode, Random};
use crate::faults::{Active, Fault};
use crate::group::GroupWire;
use crate::history::{Answer, Entry, Event, Op, Outcome, Source};
use crate::model::{ClusterModel, Failure};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

/// How a twin is started.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// The seed of every random choice the twin makes.
    pub seed: u64,
    /// How the twin clock moves.
    pub clock: ClockMode,
    /// The twin clock at start, in milliseconds since the Unix epoch.
    pub clock_start_ms: i64,
    /// Where the Kafka listener binds.
    pub kafka_listen: String,
    /// Where the admin API binds.
    pub admin_listen: String,
    /// The host and port clients are told to connect to; by default the
    /// Kafka listener's.
    pub advertised: Option<String>,
    /// Whether a Metadata request for an unknown topic creates it, as
    /// Kafka's `auto.create.topics.enable` does.
    pub auto_create_topics: bool,
    /// The partitions of a topic created without a count.
    pub default_partitions: i32,
    /// How long, in real time, a rebalance waits for members to rejoin.
    pub rebalance_wait_ms: u64,
    /// The longest a fetch waits for records, in real time.
    pub max_fetch_wait_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            seed: 0,
            clock: ClockMode::Frozen,
            clock_start_ms: 1_767_225_600_000,
            kafka_listen: "127.0.0.1:9092".into(),
            admin_listen: "127.0.0.1:9644".into(),
            advertised: None,
            auto_create_topics: true,
            default_partitions: 1,
            rebalance_wait_ms: 10_000,
            max_fetch_wait_ms: 1_000,
        }
    }
}

/// The node id of the twin's one broker.
pub const NODE_ID: i32 = 1;

/// The twin's state: the model and everything around it.
pub struct Core {
    /// The contract's reference model.
    pub model: ClusterModel,
    /// What the twin has done.
    pub history: Vec<Entry>,
    /// The twin clock.
    pub clock: Clock,
    /// The seeded randomness.
    pub random: Random,
    /// The seed the twin was started or last reset with.
    pub seed: u64,
    /// The cluster id, from the seed.
    pub cluster_id: String,
    /// Each record's timestamp, by partition and offset. The contract's
    /// records carry none; the wire needs one.
    pub timestamps: HashMap<(String, i32), Vec<i64>>,
    /// The groups' members as the classic group protocol sees them.
    pub groups: BTreeMap<String, GroupWire>,
    /// Faults the twin is showing.
    pub faults: Vec<Active>,
    next_fault: u64,
    /// Each partition's leader epoch; a leader change raises it.
    pub leader_epochs: HashMap<(String, i32), i32>,
    /// Partitions whose next request is refused NOT_LEADER_OR_FOLLOWER.
    pub not_leader: BTreeSet<(String, i32)>,
}

impl Core {
    /// A twin with an empty cluster.
    pub fn new(config: &Config) -> Self {
        let mut random = Random::new(config.seed);
        let id = format!("{:016x}{:016x}", random.next_u64(), random.next_u64());
        Self {
            model: ClusterModel::start(),
            history: vec![],
            clock: Clock::new(config.clock, config.clock_start_ms),
            random,
            seed: config.seed,
            cluster_id: format!("lawtwin-{}", &id[..22]),
            timestamps: HashMap::new(),
            groups: BTreeMap::new(),
            faults: vec![],
            next_fault: 1,
            leader_epochs: HashMap::new(),
            not_leader: BTreeSet::new(),
        }
    }

    fn push(&mut self, source: Source, event: Event) {
        let seq = self.history.len() as u64 + 1;
        let at = self.clock.now_ms();
        self.history.push(Entry { seq, at, source, event });
    }

    /// Runs an operation of the contract on the model and writes it to the
    /// history.
    pub fn call(&mut self, source: Source, op: Op) -> Result<Outcome, Failure> {
        let result = crate::history::apply(&mut self.model, &op);
        let answer = Answer::of(&result);
        self.push(source, Event::Call { op, answer });
        result
    }

    /// Runs a read of the contract without writing it to the history: a
    /// fetch's probes while it waits for records.
    pub fn peek(&mut self, op: &Op) -> Result<Outcome, Failure> {
        crate::history::apply(&mut self.model, op)
    }

    /// Writes an operation that was run with `peek` to the history.
    pub fn record(&mut self, source: Source, op: Op, result: &Result<Outcome, Failure>) {
        self.push(source, Event::Call { op, answer: Answer::of(result) });
    }

    /// Writes a control-plane event to the history.
    pub fn control(&mut self, source: Source, control: &str, detail: Value) {
        self.push(source, Event::Control { control: control.into(), detail });
    }

    /// A member id: the client id and 64 seeded random bits.
    pub fn member_id(&mut self, client_id: &str) -> String {
        let client = if client_id.is_empty() { "member" } else { client_id };
        format!("{client}-{:016x}", self.random.next_u64())
    }

    /// Raises a fault.
    pub fn raise(&mut self, fault: Fault, lasting_ms: Option<i64>) -> Active {
        let id = self.next_fault;
        self.next_fault += 1;
        let until_ms = lasting_ms.map(|ms| self.clock.now_ms().saturating_add(ms));
        if let Fault::LeaderChange { topic, partition } = &fault {
            for (name, count) in self.model.topics() {
                if topic.as_ref().is_some_and(|t| t != &name) {
                    continue;
                }
                for p in 0..count {
                    if partition.is_some_and(|q| q != p) {
                        continue;
                    }
                    *self.leader_epochs.entry((name.clone(), p)).or_insert(0) += 1;
                    self.not_leader.insert((name.clone(), p));
                }
            }
        }
        let active = Active { id, fault, until_ms };
        self.control(Source::Fault, "fault raised", serde_json::to_value(&active).unwrap_or_default());
        if !matches!(active.fault, Fault::LeaderChange { .. }) {
            self.faults.push(active.clone());
        }
        active
    }

    /// Clears the faults with this name, or every fault.
    pub fn clear(&mut self, name: Option<&str>) -> usize {
        let before = self.faults.len();
        let kept: Vec<Active> = self
            .faults
            .drain(..)
            .filter(|a| name.is_some_and(|n| fault_name(&a.fault) != n))
            .collect();
        self.faults = kept;
        if name.is_none() || name == Some("leader-change") {
            self.not_leader.clear();
        }
        let cleared = before - self.faults.len();
        self.control(Source::Fault, "fault cleared", json!({ "name": name, "cleared": cleared }));
        cleared
    }

    /// Ends the faults whose time is up.
    pub fn expire_faults(&mut self) {
        let now = self.clock.now_ms();
        let ended: Vec<Active> = self.faults.iter().filter(|a| a.until_ms.is_some_and(|u| u <= now)).cloned().collect();
        if ended.is_empty() {
            return;
        }
        self.faults.retain(|a| !a.until_ms.is_some_and(|u| u <= now));
        for a in ended {
            self.control(Source::Fault, "fault ended", json!({ "id": a.id, "name": fault_name(&a.fault) }));
        }
    }

    /// Whether the broker is unreachable.
    pub fn unavailable(&self) -> bool {
        self.faults.iter().any(|a| matches!(a.fault, Fault::BrokerUnavailable))
    }

    /// The real-time delay for a response to this request type.
    pub fn latency_ms(&self, api: &str) -> i64 {
        self.faults
            .iter()
            .filter_map(|a| match &a.fault {
                Fault::RequestLatency { delay_ms, api: only } if only.as_deref().is_none_or(|o| o == api) => Some(*delay_ms),
                _ => None,
            })
            .max()
            .unwrap_or(0)
    }

    /// The consumer-lag fault for a topic: the most records per fetch, and
    /// the delay.
    pub fn lag(&self, topic: &str) -> Option<(i32, i64)> {
        self.faults.iter().find_map(|a| match &a.fault {
            Fault::ConsumerLag { records, delay_ms, topic: only } if only.as_deref().is_none_or(|o| o == topic) => {
                Some((*records, *delay_ms))
            }
            _ => None,
        })
    }

    /// Whether this produce loses its acknowledgement; it uses one count of
    /// the duplicate-delivery fault.
    pub fn take_duplicate(&mut self, topic: &str) -> bool {
        let found = self.faults.iter().position(|a| {
            matches!(&a.fault, Fault::DuplicateDelivery { count, topic: only } if *count > 0 && only.as_deref().is_none_or(|o| o == topic))
        });
        let Some(i) = found else { return false };
        if let Fault::DuplicateDelivery { count, .. } = &mut self.faults[i].fault {
            *count -= 1;
            if *count == 0 {
                self.faults.remove(i);
            }
        }
        true
    }

    /// Whether this partition's next request is refused NOT_LEADER_OR_FOLLOWER;
    /// the refusal is used up.
    pub fn take_not_leader(&mut self, topic: &str, partition: i32) -> bool {
        self.not_leader.remove(&(topic.to_string(), partition))
    }

    /// The leader epoch of a partition.
    pub fn leader_epoch(&self, topic: &str, partition: i32) -> i32 {
        self.leader_epochs.get(&(topic.to_string(), partition)).copied().unwrap_or(0)
    }

    /// Keeps the timestamps of records written at `base`.
    pub fn stamp(&mut self, topic: &str, partition: i32, base: i64, stamps: &[i64]) {
        let v = self.timestamps.entry((topic.to_string(), partition)).or_default();
        let base = base.max(0) as usize;
        if v.len() < base {
            let fill = v.last().copied().unwrap_or(-1);
            v.resize(base, fill);
        }
        for (i, t) in stamps.iter().enumerate() {
            if base + i < v.len() {
                continue; // a duplicate: the first write's timestamps stay
            }
            v.push(*t);
        }
    }

    /// A record's timestamp.
    pub fn timestamp(&self, topic: &str, partition: i32, offset: i64) -> i64 {
        self.timestamps
            .get(&(topic.to_string(), partition))
            .and_then(|v| v.get(offset.max(0) as usize).copied())
            .unwrap_or(self.clock.now_ms())
    }
}

/// A fault's declared name.
pub fn fault_name(f: &Fault) -> &'static str {
    match f {
        Fault::BrokerUnavailable => "broker-unavailable",
        Fault::LeaderChange { .. } => "leader-change",
        Fault::ConsumerLag { .. } => "consumer-lag",
        Fault::DuplicateDelivery { .. } => "duplicate-delivery",
        Fault::RebalanceStorm { .. } => "rebalance-storm",
        Fault::RequestLatency { .. } => "request-latency",
    }
}

/// A running twin, shared by its listeners.
pub struct Twin {
    core: Mutex<Core>,
    /// Woken whenever records may have become readable.
    pub data: tokio::sync::Notify,
    /// How it was started.
    pub config: Config,
    /// The address the Kafka listener is bound to.
    pub kafka_addr: Mutex<Option<std::net::SocketAddr>>,
    /// Bumped by `reset`, so open connections from before it close.
    pub epoch: std::sync::atomic::AtomicU64,
}

impl Twin {
    /// A twin with an empty cluster.
    pub fn new(config: Config) -> Arc<Self> {
        Arc::new(Self {
            core: Mutex::new(Core::new(&config)),
            data: tokio::sync::Notify::new(),
            config,
            kafka_addr: Mutex::new(None),
            epoch: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// The twin's state, locked. Never held across an await.
    pub fn lock(&self) -> MutexGuard<'_, Core> {
        self.core.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Starts again from an empty cluster, with a seed.
    pub fn reset(&self, seed: u64) {
        let mut config = self.config.clone();
        config.seed = seed;
        let mut core = self.lock();
        *core = Core::new(&config);
        core.control(Source::Coordinator, "reset", json!({ "seed": seed }));
        self.epoch.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        drop(core);
        self.data.notify_waiters();
    }

    /// The host and port clients are told to connect to.
    pub fn advertised(&self) -> (String, i32) {
        let given = self.config.advertised.clone().or_else(|| self.kafka_addr.lock().unwrap().map(|a| a.to_string()));
        let text = given.unwrap_or_else(|| self.config.kafka_listen.clone());
        let (host, port) = match text.rsplit_once(':') {
            Some((h, p)) => (h.trim_matches(['[', ']']).to_string(), p.parse().unwrap_or(9092)),
            None => (text, 9092),
        };
        // A listener on every address cannot be connected to as such.
        let host = if host == "0.0.0.0" || host == "::" { "localhost".to_string() } else { host };
        (host, port)
    }
}
