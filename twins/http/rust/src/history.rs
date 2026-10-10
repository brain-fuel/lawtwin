//! The twin's history: every operation of the contract the twin performed,
//! with its arguments by name and what the model answered, in order, and
//! the control-plane events between them.
//!
//! An operation is written as the ability call it is (`addPet`,
//! `issuesCreateLabel`), its arguments as JSON on the wire, so a history can
//! be replayed through a fresh model, checked against the contract's laws,
//! and compared byte for byte with another run's. ref:DEC-history-as-ability-calls

use crate::V;
use crate::model::{Model, Outcome};
use lawtwin_wire::WireMap;
use serde_json::{Map, Value, json};

pub use lawtwin_wire::history::{arg, control_entry, op_entry, parse};

/// The answer the model gave, as the history writes it.
pub fn answer_json(map: &WireMap, op: &str, result: &Result<V, Outcome>) -> Value {
    match result {
        Ok(v) => {
            let ty = map.op(op).map(|o| WireMap::ty(&o.response.ty)).unwrap_or(lawtwin_wire::Ty::Unit);
            json!({ "ok": map.encode(&ty, v) })
        }
        Err(Outcome::Failure(c)) => json!({ "error": c }),
        Err(Outcome::Defect(d)) => json!({ "defect": d }),
    }
}

/// Runs an operation with its arguments given by name as JSON on the wire;
/// an argument left out is absent. Gives the arguments as the history
/// writes them, and the model's result.
pub fn call_json(model: &mut dyn Model, map: &WireMap, op: &str, args: &Map<String, Value>) -> Result<(Vec<(String, Value)>, Result<V, Outcome>), String> {
    let o = map.op(op).ok_or_else(|| format!("{op} is not an operation of {}", map.package))?;
    let mut written = vec![];
    let mut values = vec![];
    for p in &o.parameters {
        let ty = WireMap::ty(&p.ty);
        let given = args.get(&p.name).cloned().unwrap_or(Value::Null);
        let v = map.decode(&ty, &given, &p.name).map_err(|m| format!("{op}: {m}"))?;
        written.push((p.name.clone(), map.encode(&ty, &v)));
        values.push(v);
    }
    Ok((written, model.call(op, values)))
}

/// What a replay found.
pub struct Replayed {
    pub operations: usize,
    pub divergences: Vec<Value>,
    pub model: Box<dyn Model>,
    /// The replayed history, with the answers the fresh model gave.
    pub entries: Vec<Value>,
}

/// Replays a history's operations, in order, through a fresh model, and
/// reports each answer that differs from the recorded one. Control entries
/// are skipped.
pub fn replay(map: &WireMap, start: fn() -> Box<dyn Model>, entries: &[Value]) -> Result<Replayed, String> {
    let mut model = start();
    let mut divergences = vec![];
    let mut operations = 0;
    let mut out = vec![];
    for e in entries {
        let Some(op) = e.get("op").and_then(Value::as_str) else { continue };
        let o = map.op(op).ok_or_else(|| format!("entry {}: {op} is not an operation of {}", e["seq"], map.package))?;
        let mut args = Map::new();
        for p in &o.parameters {
            args.insert(p.name.clone(), arg(e, &p.name).clone());
        }
        let (written, result) = call_json(model.as_mut(), map, op, &args).map_err(|m| format!("entry {}: {m}", e["seq"]))?;
        operations += 1;
        let answer = answer_json(map, op, &result);
        if e.get("answer") != Some(&answer) {
            divergences.push(json!({ "seq": e["seq"], "op": op, "recorded": e.get("answer").cloned().unwrap_or(Value::Null), "replayed": answer }));
        }
        out.push(op_entry(e["seq"].as_u64().unwrap_or(0), e["at"].as_i64().unwrap_or(0), "replay", op, &written, answer));
    }
    Ok(Replayed { operations, divergences, model, entries: out })
}
