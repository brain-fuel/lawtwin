//! The history format every twin writes (docs/reference/control-plane.md):
//! an operation of the contract as the call it is, with its arguments by
//! name and the model's answer, or a control-plane event. Peers, recorders
//! and replays write traces in the same terms, so one law check reads all
//! of them. ref:DEC-history-as-ability-calls

use serde_json::{Map, Value, json};

/// The fields an entry has besides its arguments.
const RESERVED: &[&str] = &["seq", "at", "source", "op", "answer", "control", "detail"];

/// An operation's entry: position, time, source, the operation, each
/// argument by name, and the answer.
pub fn op_entry(seq: u64, at: i64, source: &str, op: &str, args: &[(String, Value)], answer: Value) -> Value {
    let mut m = Map::new();
    m.insert("seq".into(), json!(seq));
    m.insert("at".into(), json!(at));
    m.insert("source".into(), json!(source));
    m.insert("op".into(), json!(op));
    for (k, v) in args {
        let key = if RESERVED.contains(&k.as_str()) { format!("{k}_") } else { k.clone() };
        m.insert(key, v.clone());
    }
    m.insert("answer".into(), answer);
    Value::Object(m)
}

/// A control-plane event's entry.
pub fn control_entry(seq: u64, at: i64, source: &str, control: &str, detail: Value) -> Value {
    json!({ "seq": seq, "at": at, "source": source, "control": control, "detail": detail })
}

/// An argument of an entry, by the operation's parameter name.
pub fn arg<'e>(entry: &'e Value, name: &str) -> &'e Value {
    let key = if RESERVED.contains(&name) { format!("{name}_") } else { name.to_string() };
    entry.get(&key).unwrap_or(&Value::Null)
}

/// A history read from JSON lines or a JSON array.
pub fn parse(text: &str) -> Result<Vec<Value>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(vec![]);
    }
    if t.starts_with('[') {
        return serde_json::from_str(t).map_err(|e| format!("not a history: {e}"));
    }
    t.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| serde_json::from_str(l).map_err(|e| format!("line {}: {e}", i + 1)))
        .collect()
}

