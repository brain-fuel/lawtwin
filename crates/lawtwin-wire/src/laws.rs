//! The contract's laws, checked over a twin's history and state.
//!
//! The importer writes, for each resource, laws over sequences of its
//! operations: a created record reads back, a deleted one is not found, the
//! list holds a created one, an update changes only the fields it gives, a
//! replaced one reads back. The history checks mirror them over every
//! sequence of operations a history holds, with the contract's own titles,
//! and check only the laws the contract states. The state check is the
//! invariant they rest on: a record is kept under its own key.

use crate::V;
use crate::history::arg;
use crate::map::{Resource, Role, TypeDef};
use crate::{Ty, WireMap};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn violation(law: &str, detail: String, seq: Option<&Value>) -> Value {
    let mut v = json!({ "law": law, "detail": detail });
    if let Some(s) = seq {
        v["seq"] = s.clone();
    }
    v
}

/// The wire name of a record's field, by its LawSpec name.
fn wire_of(map: &WireMap, record: &str, field: &str) -> Option<String> {
    match map.type_def(record) {
        Some(TypeDef::Record { fields, .. }) => fields.iter().find(|f| f.name == field).map(|f| f.wire.clone()),
        _ => None,
    }
}

/// The key of the record an entry concerns, as JSON: its parents and key.
fn key_of(map: &WireMap, r: &Resource, role: Role, e: &Value) -> Option<Value> {
    let mut parts: Vec<Value> = r.parents.iter().map(|p| arg(e, &p.name).clone()).collect();
    let from_record = |rec: &Value| -> Option<Value> {
        let wire = wire_of(map, &r.record, r.key_field.as_deref()?)?;
        rec.get(&wire).filter(|v| !v.is_null()).cloned()
    };
    let key = match role {
        Role::Create => {
            let answer = e.get("answer")?.get("ok")?;
            from_record(answer).or_else(|| {
                let op = map.op(e["op"].as_str()?)?;
                from_record(arg(e, &op.body()?.name))
            })?
        }
        Role::List => return Some(Value::Array(parts)),
        Role::Replace if r.replace_by_body_key => {
            let op = map.op(e["op"].as_str()?)?;
            from_record(arg(e, &op.body()?.name))?
        }
        _ => arg(e, &r.key).clone(),
    };
    parts.push(key);
    Some(Value::Array(parts))
}

#[derive(Clone)]
enum Known {
    /// The record a read must give, and the kind of law that says so.
    Record(Value, &'static str),
    /// A read must fail with NotFound.
    Gone,
}

/// Checks the laws over a history: each violation with the law's title.
pub fn check_history(map: &WireMap, entries: &[Value]) -> Vec<Value> {
    let mut out = vec![];
    let mut known: BTreeMap<(String, String), Known> = BTreeMap::new();
    for e in entries {
        let Some(op) = e.get("op").and_then(Value::as_str) else { continue };
        let Some((r, role)) = map.role(op) else { continue };
        let Some(key) = key_of(map, r, role, e) else { continue };
        let slot = (r.record.clone(), key.to_string());
        let answer = e.get("answer").cloned().unwrap_or(Value::Null);
        let ok = answer.get("ok");
        let law = |kind: &str| r.laws.get(kind).cloned();
        match role {
            Role::Create => {
                if let Some(rec) = ok {
                    known.insert(slot, Known::Record(rec.clone(), "createdReadsBack"));
                }
            }
            Role::Read => match known.get(&slot) {
                Some(Known::Record(rec, kind)) => {
                    if let Some(title) = law(kind) {
                        if ok != Some(rec) {
                            out.push(violation(&title, format!("{op} {key} answered {answer}, not the record {rec}"), e.get("seq")));
                        }
                    }
                }
                Some(Known::Gone) => {
                    if let Some(title) = law("deletedIsNotFound") {
                        if answer != json!({ "error": "NotFound" }) {
                            out.push(violation(&title, format!("{op} {key} answered {answer} after a delete, not NotFound"), e.get("seq")));
                        }
                    }
                }
                None => {}
            },
            Role::Delete => {
                if ok.is_some() || answer == json!({ "error": "NotFound" }) {
                    known.insert(slot, Known::Gone);
                }
            }
            Role::Replace => match ok {
                Some(rec) if !rec.is_null() => {
                    known.insert(slot, Known::Record(rec.clone(), "replacedReadsBack"));
                }
                Some(_) => {
                    known.remove(&slot);
                }
                None => {}
            },
            Role::Update => {
                let Some(after) = ok else { continue };
                if let (Some(Known::Record(before, _)), Some(title)) = (known.get(&slot), law("updateChangesOnlyGivenFields")) {
                    for problem in update_problems(map, r, op, e, before, after) {
                        out.push(violation(&title, format!("{op} {key}: {problem}"), e.get("seq")));
                    }
                }
                known.insert(slot, Known::Record(after.clone(), "updateChangesOnlyGivenFields"));
            }
            Role::List => {
                let (Some(Value::Array(listed)), Some(title)) = (ok, law("listHoldsCreated")) else { continue };
                let prefix = key.as_array().cloned().unwrap_or_default();
                for ((rec_name, k), kn) in &known {
                    let Known::Record(rec, _) = kn else { continue };
                    let Ok(Value::Array(parts)) = serde_json::from_str::<Value>(k) else { continue };
                    if *rec_name == r.record && parts.len() == prefix.len() + 1 && parts[..prefix.len()] == prefix[..] && !listed.contains(rec) {
                        out.push(violation(&title, format!("{op} {key} does not hold the record {rec}"), e.get("seq")));
                    }
                }
            }
        }
    }
    out
}

/// What an update changed that its body did not give, or did not change
/// that it gave.
fn update_problems(map: &WireMap, r: &Resource, op: &str, e: &Value, before: &Value, after: &Value) -> Vec<String> {
    let mut out = vec![];
    let Some(o) = map.op(op) else { return out };
    let Some(body_param) = o.body() else { return out };
    let body = arg(e, &body_param.name);
    let body_fields = match WireMap::ty(&body_param.ty).unmaybe() {
        Ty::Named(n) => match map.type_def(n) {
            Some(TypeDef::Record { fields, .. }) => fields.clone(),
            _ => vec![],
        },
        _ => vec![],
    };
    let Some(TypeDef::Record { fields, .. }) = map.type_def(&r.record) else { return out };
    for f in fields {
        let given = body_fields
            .iter()
            .find(|b| b.name == f.name && WireMap::ty(&b.ty).unmaybe() == WireMap::ty(&f.ty).unmaybe())
            .and_then(|b| body.get(&b.wire))
            .filter(|v| !v.is_null());
        let now = after.get(&f.wire).unwrap_or(&Value::Null);
        match given {
            Some(v) if now != v => out.push(format!("{} is {now}, not the {v} it was given", f.wire)),
            None if Some(f.name.as_str()) != r.key_field.as_deref() && now != before.get(&f.wire).unwrap_or(&Value::Null) => {
                out.push(format!("{} changed from {} to {now}, though the update did not give it", f.wire, before.get(&f.wire).unwrap_or(&Value::Null)))
            }
            _ => {}
        }
    }
    out
}

/// Checks the state: every record is kept under its own key.
pub fn check_state(map: &WireMap, state: &V) -> Vec<Value> {
    let mut out = vec![];
    let V::Data { fields, .. } = state else { return out };
    for (layout, value) in map.state.iter().zip(fields) {
        let Some(r) = map.resources.iter().find(|r| r.state_field == layout.field) else { continue };
        let (V::Map(entries), Some(kf)) = (value, &r.key_field) else { continue };
        let Some(title) = r.laws.get("createdReadsBack") else { continue };
        let Some(TypeDef::Record { fields: defs, .. }) = map.type_def(&r.record) else { continue };
        let Some(index) = defs.iter().position(|f| &f.name == kf) else { continue };
        for (k, rec) in entries {
            let own = match k {
                V::Data { fields, .. } if !r.parents.is_empty() => fields.last().cloned(),
                other => Some(other.clone()),
            };
            let V::Data { fields: rf, .. } = rec else { continue };
            let held = match rf.get(index) {
                Some(V::Maybe(Some(x))) => Some((**x).clone()),
                Some(V::Maybe(None)) => None,
                other => other.cloned(),
            };
            if held != own {
                out.push(violation(title, format!("a {} is kept under {} but its {kf} is {:?}", r.record, crate::value::text_of(k), held), None));
            }
        }
    }
    out
}
