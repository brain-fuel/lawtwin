//! Every core answers the golden session as the Rust core did when it was
//! recorded: each response's status, compared headers and body, byte for
//! byte, and the history after it. `make golden-http` records it again
//! (LAWTWIN_WRITE_GOLDEN=1).

use lawtwin_http_conformance::core::{TestTwin, core_name};
use lawtwin_http_conformance::session::{replay_steps, script};
use lawtwin_http_conformance::{CONTRACTS, golden_dir};

const SEED: u64 = 42;
const ADVERTISED: &str = "lawtwin.golden:8080";

#[test]
fn every_core_answers_the_golden_session_byte_for_byte() {
    for contract in CONTRACTS {
        let steps_file = golden_dir().join(format!("{contract}-{SEED}.session.jsonl"));
        let history_file = golden_dir().join(format!("{contract}-{SEED}.history.jsonl"));
        if std::env::var("LAWTWIN_WRITE_GOLDEN").is_ok() {
            let twin = TestTwin::start_advertising(contract, SEED, ADVERTISED);
            let steps = script(&twin, SEED);
            let text: String = steps.iter().map(|s| serde_json::to_string(s).unwrap() + "\n").collect();
            std::fs::create_dir_all(golden_dir()).unwrap();
            std::fs::write(&steps_file, text).unwrap();
            std::fs::write(&history_file, twin.history_text()).unwrap();
            continue;
        }
        let steps: Vec<serde_json::Value> = std::fs::read_to_string(&steps_file)
            .unwrap_or_else(|e| panic!("{}: {e}; record it with make golden-http", steps_file.display()))
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let twin = TestTwin::start_advertising(contract, SEED, ADVERTISED);
        let differences = replay_steps(&twin, &steps);
        assert!(differences.is_empty(), "{}: {contract}: {} step(s) answered otherwise than the golden session: {:#}", core_name(), differences.len(), serde_json::Value::Array(differences.into_iter().take(5).collect()));
        let history = std::fs::read_to_string(&history_file).unwrap();
        assert_eq!(twin.history_text(), history, "{}: {contract}: the history differs from the golden one", core_name());
    }
}
