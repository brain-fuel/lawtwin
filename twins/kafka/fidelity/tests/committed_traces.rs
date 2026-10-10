//! The traces the 0.1.0 conformance run recorded from apache/kafka 4.3.1,
//! kept beside its report, replay through the Rust core: every scenario
//! with no divergence, except a group's rebalance, where Kafka's classic
//! protocol lets the client's assignor choose the partitions and the
//! contract's coordinator chooses its own (DEC-kafka-divergence-group-assignment,
//! DEC-kafka-divergence-rebalance-heartbeat). No broker is needed.

use lawtwin_kafka_fidelity::replay::replay_trace;
use std::path::PathBuf;

fn traces() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/reports/0.1.0/traces")
}

fn replay(client: &str, scenario: &str) -> lawtwin_kafka_fidelity::replay::ReplayReport {
    let prefix = traces().join(client).join(scenario);
    let report = replay_trace(&prefix, "rust").unwrap_or_else(|e| panic!("{client} {scenario}: {e:#}"));
    assert_eq!(report.answered + report.skipped, report.requests, "{client} {scenario}: {report:#?}");
    assert!(report.stalled.is_empty(), "{client} {scenario}: {report:#?}");
    assert!(report.violations.is_empty(), "{client} {scenario}: {:#?}", report.violations);
    report
}

#[test]
fn every_recorded_scenario_but_a_rebalance_replays_with_no_divergence() {
    for client in ["librdkafka", "kafkajs", "franz-go", "java"] {
        for scenario in ["produce-consume", "idempotent", "transactions", "faults", "force"] {
            let report = replay(client, scenario);
            assert!(report.divergences.is_empty(), "{client} {scenario}: {:#?}", report.divergences);
        }
    }
}

#[test]
fn a_recorded_rebalance_differs_only_in_the_group_protocol() {
    for client in ["librdkafka", "kafkajs", "franz-go", "java"] {
        let report = replay(client, "group-rebalance");
        for d in &report.divergences {
            assert!(
                ["SyncGroup", "Heartbeat", "DescribeGroups"].contains(&d.api.as_str()),
                "{client}: a divergence outside the group protocol: {d:#?}"
            );
        }
    }
}
