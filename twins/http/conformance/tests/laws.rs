//! The scripted session with several seeds, on each contract: the core's
//! history replays through the reference model with no divergence, the
//! contract's laws hold over it and over the replayed state, and the core's
//! own law check agrees. A history changed by one answer is caught by both.

use lawtwin_http_conformance::CONTRACTS;
use lawtwin_http_conformance::core::{TestTwin, wire_map};
use lawtwin_http_conformance::session::script;

#[test]
fn every_history_replays_and_keeps_the_laws() {
    for contract in CONTRACTS {
        for seed in [1, 2, 3, 99] {
            let twin = TestTwin::start(contract, seed);
            script(&twin, seed);
            twin.assert_lawful();
        }
    }
}

#[test]
fn a_changed_answer_is_caught_by_the_replay_and_the_laws() {
    for contract in CONTRACTS {
        let twin = TestTwin::start(contract, 5);
        script(&twin, 5);
        let map = wire_map(contract);
        let c = lawtwin_http::contracts::find(contract).unwrap();
        let mut entries = twin.history();
        // The first read that found a record now says it is gone.
        let read_ops: Vec<String> = map.resources.iter().map(|r| r.read.clone()).collect();
        let i = entries
            .iter()
            .position(|e| e["op"].as_str().is_some_and(|o| read_ops.iter().any(|r| r == o)) && e["answer"].get("ok").is_some())
            .expect("a read that found a record");
        entries[i]["answer"] = serde_json::json!({ "error": "NotFound" });
        let replayed = lawtwin_http::history::replay(&map, c.start, &entries).unwrap();
        assert_eq!(replayed.divergences.len(), 1, "{contract}: {:#?}", replayed.divergences);
        assert!(!lawtwin_wire::laws::check_history(&map, &entries).is_empty(), "{contract}: the laws miss the changed answer");
        // The core's replay endpoint says the same.
        let body: String = entries.iter().map(|e| serde_json::to_string(e).unwrap() + "\n").collect();
        let (status, v) = twin.admin("POST", "/v1/history/replay", &body);
        assert_eq!(status, 200);
        assert_eq!(v["divergences"].as_array().unwrap().len(), 1);
        assert!(!v["violations"]["history"].as_array().unwrap().is_empty());
    }
}

#[test]
fn the_same_seed_gives_the_same_history_byte_for_byte() {
    for contract in CONTRACTS {
        let runs: Vec<String> = [11, 11, 12]
            .into_iter()
            .map(|seed| {
                let twin = TestTwin::start(contract, seed);
                script(&twin, seed);
                twin.history_text()
            })
            .collect();
        assert!(runs[0].len() > 2000);
        assert_eq!(runs[0], runs[1], "{contract}: two runs with seed 11 differ");
        assert_ne!(runs[0], runs[2], "{contract}: another seed gives the same history");
    }
}
