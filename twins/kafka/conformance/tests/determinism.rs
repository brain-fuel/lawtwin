//! The same seed and the same requests give the same history, byte for
//! byte; another seed gives other member ids; a mutated history is caught by
//! both the replay and the law checks.

use lawtwin_kafka_conformance::core::TestTwin;
use lawtwin_kafka_conformance::session::script;

#[test]
fn the_same_seed_and_requests_give_the_same_history_byte_for_byte() {
    let runs: Vec<(String, Vec<String>)> = [42, 42, 43]
        .into_iter()
        .map(|seed| {
            let twin = TestTwin::start(seed);
            let (members, _) = script(&twin);
            twin.assert_lawful();
            (twin.history_text(), members)
        })
        .collect();
    assert!(runs[0].0.len() > 1000);
    assert_eq!(runs[0].0, runs[1].0, "two runs with seed 42 differ");
    assert_eq!(runs[0].1, runs[1].1);
    assert_ne!(runs[0].1, runs[2].1, "the member id comes from the seed");
    assert_ne!(runs[0].0, runs[2].0);
}

#[test]
fn a_history_replays_and_a_mutated_one_is_caught() {
    let twin = TestTwin::start(7);
    script(&twin);
    let text = twin.history_text();
    let entries = lawtwin_kafka::admin::parse_history(&text).unwrap();
    let (_, divergences) = lawtwin_kafka::history::replay(&entries);
    assert!(divergences.is_empty());
    // Change one produce's answer: the replay and the law check both notice.
    let mutated = text.replacen(r#""answer":{"ok":8}"#, r#""answer":{"ok":9}"#, 1);
    assert_ne!(mutated, text);
    let entries = lawtwin_kafka::admin::parse_history(&mutated).unwrap();
    assert_eq!(lawtwin_kafka::history::replay(&entries).1.len(), 1);
    assert!(!lawtwin_kafka::laws::check_history(&entries).is_empty());
}
