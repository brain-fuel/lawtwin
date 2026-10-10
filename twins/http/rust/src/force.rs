//! Forcing a state: records a test needs the service to hold, reached
//! through the contract's own operations (ref:DEC-force-through-operations).
//!
//! A forced state names records of each resource, by the record's name:
//!
//! ```json
//! { "reset": true,
//!   "resources": {
//!     "Pet": [ { "id": 7, "name": "Rex", "photoUrls": [] } ],
//!     "Label": [ { "parents": { "owner": "o", "repo": "r" }, "body": { "name": "bug", "color": "f00" } } ] } }
//! ```
//!
//! A record is reached by the resource's create, then its update or
//! replace for what the create does not set, and must then read back as
//! given; `body` sends the create's body as it is and takes what the create
//! answers. Everything runs on a copy of the model; if anything is refused,
//! or a record does not read back as given, the copy is dropped and the
//! twin is as it was.

use crate::history::{answer_json, call_json};
use crate::model::Model;
use lawtwin_wire::map::{Resource, TypeDef};
use lawtwin_wire::{Ty, WireMap};
use serde_json::{Map, Value, json};

/// Why a forced state was not reached.
pub enum Refused {
    /// The document is not a forced state.
    Malformed(String),
    /// No sequence of the contract's operations reaches it: the law, and why.
    Law { law: String, reason: String },
}

/// The operations that reached it: each operation, its arguments as the
/// history writes them, and its answer.
pub type Reached = Vec<(String, Vec<(String, Value)>, Value)>;

fn law_of(r: &Resource) -> String {
    r.laws.get("createdReadsBack").cloned().unwrap_or_else(|| format!("a created {} reads back", r.record.to_lowercase()))
}

/// Builds a value of `target` from a record, field by field: a field of the
/// same name is copied, and the others are left out.
fn shaped(map: &WireMap, target: &Ty, record_ty: &str, record: &Value) -> Value {
    let Ty::Named(name) = target.unmaybe() else { return record.clone() };
    if name == record_ty {
        return record.clone();
    }
    let (Some(TypeDef::Record { fields: to, .. }), Some(TypeDef::Record { fields: from, .. })) = (map.type_def(name), map.type_def(record_ty)) else {
        return record.clone();
    };
    let mut out = Map::new();
    for f in to {
        if let Some(src) = from.iter().find(|g| g.name == f.name && WireMap::ty(&g.ty).unmaybe() == WireMap::ty(&f.ty).unmaybe()) {
            if let Some(v) = record.get(&src.wire).filter(|v| !v.is_null()) {
                out.insert(f.wire.clone(), v.clone());
            }
        }
    }
    Value::Object(out)
}

/// Reaches a forced state on the model, which the caller has copied.
pub fn reach(map: &WireMap, model: &mut dyn Model, doc: &Value) -> Result<Reached, Refused> {
    let obj = doc.as_object().ok_or_else(|| Refused::Malformed("a forced state is a JSON object".into()))?;
    for k in obj.keys() {
        if k != "reset" && k != "resources" {
            return Err(Refused::Malformed(format!("a forced state has reset and resources, not {k}")));
        }
    }
    let mut done: Reached = vec![];
    let empty = Map::new();
    let resources = match obj.get("resources") {
        None => &empty,
        Some(Value::Object(m)) => m,
        Some(_) => return Err(Refused::Malformed("resources is an object of record names".into())),
    };
    for (name, items) in resources {
        let r = map
            .resources
            .iter()
            .find(|r| &r.record == name)
            .ok_or_else(|| Refused::Malformed(format!("{name} is not a resource of {}; it has {}", map.package, map.resources.iter().map(|r| r.record.as_str()).collect::<Vec<_>>().join(", "))))?;
        let items = items.as_array().ok_or_else(|| Refused::Malformed(format!("{name} is a list of records")))?;
        for item in items {
            reach_one(map, model, r, item, &mut done)?;
        }
    }
    Ok(done)
}

fn run(map: &WireMap, model: &mut dyn Model, op: &str, args: &Map<String, Value>, done: &mut Reached) -> Result<Value, Refused> {
    let (written, result) = call_json(model, map, op, args).map_err(Refused::Malformed)?;
    let answer = answer_json(map, op, &result);
    done.push((op.to_string(), written, answer.clone()));
    Ok(answer)
}

fn reach_one(map: &WireMap, model: &mut dyn Model, r: &Resource, item: &Value, done: &mut Reached) -> Result<(), Refused> {
    let law = law_of(r);
    let create = r.create.as_deref().and_then(|c| map.op(c)).ok_or_else(|| Refused::Law {
        law: law.clone(),
        reason: format!("the contract has no create for {}, so no operation reaches one", r.record),
    })?;
    let (parents, record, body) = match item {
        Value::Object(m) if m.contains_key("record") || m.contains_key("body") || m.contains_key("parents") => {
            (m.get("parents").cloned().unwrap_or(json!({})), m.get("record").cloned(), m.get("body").cloned())
        }
        other => (json!({}), Some(other.clone()), None),
    };
    let mut args = Map::new();
    for p in &r.parents {
        let v = parents.get(&p.name).cloned().ok_or_else(|| Refused::Malformed(format!("a {} needs parents.{}", r.record, p.name)))?;
        args.insert(p.name.clone(), v);
    }
    if let Some(rec) = &record {
        map.decode(&Ty::Named(r.record.clone()), rec, &r.record).map_err(|m| Refused::Malformed(format!("not a {}: {m}", r.record)))?;
    }
    let body_param = create.body().ok_or_else(|| Refused::Malformed(format!("{} takes no body", create.operation)))?;
    let sent = match (&body, &record) {
        (Some(b), _) => b.clone(),
        (None, Some(rec)) => shaped(map, &WireMap::ty(&body_param.ty), &r.record, rec),
        (None, None) => return Err(Refused::Malformed(format!("a {} needs record or body", r.record))),
    };
    args.insert(body_param.name.clone(), sent);
    let answer = run(map, model, &create.operation, &args, done)?;
    let Some(created) = answer.get("ok").cloned() else {
        return Err(Refused::Law { law, reason: format!("{} refused it with {}", create.operation, answer) });
    };
    let Some(record) = record else { return Ok(()) };
    let key_wire = r.key_field.as_deref().and_then(|kf| match map.type_def(&r.record) {
        Some(TypeDef::Record { fields, .. }) => fields.iter().find(|f| f.name == kf).map(|f| f.wire.clone()),
        _ => None,
    });
    let key = key_wire.as_ref().and_then(|w| created.get(w).cloned()).filter(|k| !k.is_null());
    let Some(key) = key else { return Ok(()) };
    let mut record = record;
    if let (Some(w), Value::Object(m)) = (&key_wire, &mut record) {
        if m.get(w).is_none_or(Value::is_null) {
            m.insert(w.clone(), key.clone());
        }
    }
    let mut key_args = Map::new();
    for p in &r.parents {
        key_args.insert(p.name.clone(), args[&p.name].clone());
    }
    key_args.insert(r.key.clone(), key.clone());
    let read = |model: &mut dyn Model, done: &mut Reached| run(map, model, &r.read, &key_args, done);
    let mut now = if created == record { created.clone() } else { Value::Null };
    if now != record {
        if let Some(update) = r.update.as_deref().and_then(|u| map.op(u)) {
            let mut a = key_args.clone();
            if let Some(b) = update.body() {
                a.insert(b.name.clone(), shaped(map, &WireMap::ty(&b.ty), &r.record, &record));
            }
            if let Some(v) = run(map, model, &update.operation, &a, done)?.get("ok") {
                now = v.clone();
            }
        } else if let Some(replace) = r.replace.as_deref().and_then(|u| map.op(u)) {
            let mut a = if r.replace_by_body_key { Map::new() } else { key_args.clone() };
            if let Some(b) = replace.body() {
                let mut rec = record.clone();
                if let Some(w) = &key_wire {
                    rec[w] = key.clone();
                }
                a.insert(b.name.clone(), shaped(map, &WireMap::ty(&b.ty), &r.record, &rec));
            }
            run(map, model, &replace.operation, &a, done)?;
        }
    }
    let _ = now;
    let back = read(model, done)?;
    if back.get("ok") != Some(&record) {
        let found = back.get("ok").cloned().unwrap_or(back.clone());
        let differs: Vec<String> = match (&record, &found) {
            (Value::Object(want), Value::Object(got)) => want
                .iter()
                .filter(|(k, v)| got.get(*k) != Some(v))
                .map(|(k, v)| format!("{k} is {} rather than {v}", got.get(k).unwrap_or(&Value::Null)))
                .chain(got.keys().filter(|k| !want.contains_key(*k)).map(|k| format!("{k} is {} rather than absent", got[k])))
                .collect(),
            _ => vec![format!("it reads back as {found}")],
        };
        return Err(Refused::Law {
            law,
            reason: format!("the contract's operations reach the {} {key} only with {}", r.record, differs.join(", ")),
        });
    }
    Ok(())
}
