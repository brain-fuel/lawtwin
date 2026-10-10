//! Every core answers the golden session byte for byte, and writes the
//! golden history. With LAWTWIN_WRITE_GOLDEN set, the session is recorded
//! from the core under test instead, which must be the reference core.

use lawtwin_kafka_conformance::core::TestTwin;
use lawtwin_kafka_conformance::golden::{self, ADVERTISED, SEED, api_name};
use lawtwin_kafka_conformance::session::{Client, script};

#[test]
fn the_core_answers_the_golden_session_byte_for_byte() {
    let twin = TestTwin::start_advertising(SEED, ADVERTISED);
    if std::env::var_os("LAWTWIN_WRITE_GOLDEN").is_some() {
        let (_, frames) = script(&twin);
        twin.assert_lawful();
        golden::write(&frames, &twin.history_text());
        return;
    }
    let (frames, history) = golden::read();
    assert!(frames.len() > 20);
    let mut c = Client::connect(&twin);
    for (i, f) in frames.iter().enumerate() {
        let got = c.exchange(&f.request);
        assert!(
            got == f.response,
            "frame {} ({} v{}): the core's response differs from the golden one\n  golden: {}\n  core:   {}",
            i + 1,
            api_name(f.key),
            f.version,
            f.response.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            got.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
    }
    let got = twin.history_text();
    if got != history {
        let line = got.lines().zip(history.lines()).position(|(a, b)| a != b).map(|i| i + 1);
        panic!("the core's history differs from the golden one at line {line:?}");
    }
    twin.assert_lawful();
}

#[test]
fn the_scripted_session_gives_the_golden_frames() {
    if std::env::var_os("LAWTWIN_WRITE_GOLDEN").is_some() {
        return;
    }
    let twin = TestTwin::start_advertising(SEED, ADVERTISED);
    let (_, frames) = script(&twin);
    let (golden, history) = golden::read();
    assert_eq!(frames.len(), golden.len());
    for (i, (a, b)) in frames.iter().zip(&golden).enumerate() {
        assert_eq!(a.request, b.request, "frame {}: the script's request changed; run make golden", i + 1);
        assert_eq!(a.response, b.response, "frame {} ({}): the response differs", i + 1, api_name(a.key));
    }
    assert_eq!(twin.history_text(), history);
}
