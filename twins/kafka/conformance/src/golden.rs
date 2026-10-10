//! The golden session: the scripted session's frames and history, recorded
//! on the reference core with seed 42 and a frozen clock, kept in
//! `twins/kafka/shared/golden/`. Every core must answer each request with
//! the same bytes and write the same history, byte for byte, and every
//! core's codec tests read the same frames.
//!
//! `session-42.frames.jsonl` holds one frame per line: the API key and its
//! name, the version, and the request and response as hex (each the header
//! and body, without the length prefix). `session-42.history.jsonl` is the
//! history the admin API gives after the session.

use crate::session::Frame;
use std::path::PathBuf;

/// The seed of the golden session.
pub const SEED: u64 = 42;

/// The address the core advertises in the golden session.
pub const ADVERTISED: &str = "lawtwin.golden:9092";

/// The directory of the shared fixtures.
pub fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../shared/golden")
}

fn frames_path() -> PathBuf {
    dir().join(format!("session-{SEED}.frames.jsonl"))
}

fn history_path() -> PathBuf {
    dir().join(format!("session-{SEED}.history.jsonl"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex")).collect()
}

/// The name of an API key.
pub fn api_name(key: i16) -> String {
    kafka_protocol::messages::ApiKey::try_from(key).map(|k| format!("{k:?}")).unwrap_or_else(|_| key.to_string())
}

/// Writes the golden session.
pub fn write(frames: &[Frame], history: &str) {
    std::fs::create_dir_all(dir()).unwrap();
    let mut text = String::new();
    for (i, f) in frames.iter().enumerate() {
        let line = serde_json::json!({
            "n": i + 1,
            "api": api_name(f.key),
            "key": f.key,
            "version": f.version,
            "request": hex(&f.request),
            "response": hex(&f.response),
        });
        text.push_str(&line.to_string());
        text.push('\n');
    }
    std::fs::write(frames_path(), text).unwrap();
    std::fs::write(history_path(), history).unwrap();
}

/// Reads the golden session: its frames and its history.
pub fn read() -> (Vec<Frame>, String) {
    let text = std::fs::read_to_string(frames_path()).expect("the golden session; write it with make golden");
    let frames = text
        .lines()
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            Frame {
                key: v["key"].as_i64().unwrap() as i16,
                version: v["version"].as_i64().unwrap() as i16,
                request: unhex(v["request"].as_str().unwrap()),
                response: unhex(v["response"].as_str().unwrap()),
            }
        })
        .collect();
    (frames, std::fs::read_to_string(history_path()).unwrap())
}
