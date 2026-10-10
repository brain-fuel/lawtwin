//! librdkafka (through the rdkafka crate) against the twin: each scenario of
//! `rdkafka_scenarios` runs against a fresh core, and the test then checks
//! the contract's laws over the core's history and replays it.

use lawtwin_kafka_conformance::core::TestTwin;
use lawtwin_kafka_conformance::rdkafka_scenarios::run;

fn scenario(name: &str, seed: u64) {
    let twin = TestTwin::start(seed);
    if let Err(e) = run(name, &twin.bootstrap(), &twin.admin_url(), "") {
        panic!("librdkafka {name}: {e}");
    }
    twin.assert_lawful();
}

#[test]
fn produce_and_consume_in_order() {
    scenario("produce-consume", 1);
}

#[test]
fn a_group_rebalances_and_every_partition_has_one_owner() {
    scenario("group-rebalance", 2);
}

#[test]
fn an_idempotent_producer_writes_a_lost_ack_once_and_a_plain_one_twice() {
    scenario("idempotent", 3);
}

#[test]
fn transactions_are_read_committed() {
    scenario("transactions", 4);
}

#[test]
fn faults_are_seen_by_the_client_and_the_twin_stays_lawful() {
    scenario("faults", 5);
}

#[test]
fn a_forced_state_is_what_the_client_reads() {
    scenario("force", 6);
}
