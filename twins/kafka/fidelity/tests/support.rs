//! What the fidelity tests share: the real broker they need, and a
//! recorder in front of it.

#![allow(dead_code)]

use lawtwin_kafka::clock::ClockMode;
use lawtwin_kafka_fidelity::record::{self, RecorderConfig};

/// The real broker the tests run against, from `LAWTWIN_KAFKA`
/// (`make fidelity-test` starts apache/kafka in podman and sets it). A test
/// that needs one is skipped without it, unless `LAWTWIN_REQUIRE_KAFKA` is
/// set.
pub fn real_kafka(test: &str) -> Option<String> {
    match std::env::var("LAWTWIN_KAFKA").ok().filter(|v| !v.is_empty()) {
        Some(b) => Some(b),
        None if std::env::var_os("LAWTWIN_REQUIRE_KAFKA").is_some() => panic!("{test} needs a real broker: set LAWTWIN_KAFKA"),
        None => {
            eprintln!("skipping {test}: set LAWTWIN_KAFKA to a real broker's bootstrap address");
            None
        }
    }
}

/// A name no other run has used.
pub fn unique(prefix: &str) -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    format!("{prefix}-{:x}-", t % 0xffff_ffff_ffff)
}

/// A recorder in front of a broker, on its own runtime.
pub struct Recording {
    /// The runtime it runs on.
    pub runtime: tokio::runtime::Runtime,
    /// The recorder.
    pub running: record::Running,
}

impl Recording {
    /// Starts one, with a frozen clock.
    pub fn start(upstream: &str) -> Self {
        let runtime = lawtwin_kafka::runtime().unwrap();
        let running = runtime
            .block_on(record::start(RecorderConfig { upstream: upstream.into(), clock: ClockMode::Frozen, ..Default::default() }))
            .unwrap();
        Self { runtime, running }
    }

    /// The bootstrap address clients use.
    pub fn bootstrap(&self) -> String {
        self.running.kafka.to_string()
    }

    /// The control plane's URL.
    pub fn admin_url(&self) -> String {
        format!("http://{}", self.running.admin)
    }
}
