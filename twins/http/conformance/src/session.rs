//! The scripted session: requests a core must answer as every other core
//! does. It walks each resource's laws, calls every other operation, sends
//! requests that break the document, forces states, raises every fault and
//! moves the clock, all drawn from one seed. Run on the Rust core with seed
//! 42, it is the golden session (`twins/http/shared/golden`); run with other
//! seeds, it gives histories to replay and check.

use crate::core::{TestTwin, wire_map};
use lawtwin_wire::call::Outgoing;
use lawtwin_wire::http::{Failure, Reply};
use lawtwin_wire::map::TypeDef;
use lawtwin_wire::traffic::Gen;
use lawtwin_wire::{Ty, WireMap};
use serde_json::{Map, Value, json};

/// The response headers a core must give as every other core does.
pub const COMPARED_HEADERS: &[&str] = &["Content-Type", "Location", "Retry-After", "Allow", "Date"];

fn body_text(b: &[u8]) -> Value {
    match std::str::from_utf8(b) {
        Ok(s) => json!(s),
        Err(_) => json!({ "hex": b.iter().map(|x| format!("{x:02x}")).collect::<String>() }),
    }
}

/// A response as the golden session writes it.
pub fn response_json(r: &Result<Reply, Failure>) -> Value {
    match r {
        Ok(reply) => {
            let mut headers = Map::new();
            for h in COMPARED_HEADERS {
                if let Some(v) = reply.header(h) {
                    headers.insert(h.to_string(), json!(v));
                }
            }
            json!({ "status": reply.status, "headers": headers, "body": body_text(&reply.body) })
        }
        Err(Failure::Timeout) => json!({ "failure": "timeout" }),
        Err(Failure::Closed(_)) => json!({ "failure": "closed" }),
        Err(Failure::Connect(_)) => json!({ "failure": "connect" }),
        Err(Failure::Malformed(_)) => json!({ "failure": "malformed" }),
    }
}

/// A request as the golden session writes it.
pub fn request_json(out: &Outgoing) -> Value {
    json!({ "method": out.method, "target": out.target, "headers": out.headers, "body": body_text(&out.body) })
}

/// A request read back from the golden session.
pub fn request_of(v: &Value) -> Outgoing {
    Outgoing {
        method: v["method"].as_str().unwrap_or("GET").into(),
        target: v["target"].as_str().unwrap_or("/").into(),
        headers: v["headers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|h| Some((h[0].as_str()?.to_string(), h[1].as_str()?.to_string())))
            .collect(),
        body: v["body"].as_str().unwrap_or("").as_bytes().to_vec(),
    }
}

struct Script<'t, 'm> {
    twin: &'t TestTwin,
    map: &'m WireMap,
    draw: Gen<'m>,
    steps: Vec<Value>,
}

impl Script<'_, '_> {
    fn api(&mut self, out: Outgoing) -> Option<Reply> {
        let r = self.twin.send(&out);
        self.steps.push(json!({ "step": "api", "request": request_json(&out), "response": response_json(&r) }));
        r.ok()
    }

    fn call(&mut self, op: &str, args: &Map<String, Value>) -> Option<Value> {
        let o = self.map.op(op)?.clone();
        let out = self.map.build_request(&o, args);
        let r = self.api(out)?;
        Some(self.map.read_response(&o, r.status, &r.headers, &r.body).answer)
    }

    fn admin(&mut self, method: &str, path: &str, body: Value) -> Value {
        let text = if body.is_null() { String::new() } else { body.to_string() };
        let (status, answer) = self.twin.admin(method, path, &text);
        self.steps.push(json!({ "step": "admin", "method": method, "path": path, "body": body, "response": { "status": status, "body": answer } }));
        answer
    }

    fn wire_of(&self, record: &str, field: &str) -> Option<String> {
        match self.map.type_def(record) {
            Some(TypeDef::Record { fields, .. }) => fields.iter().find(|f| f.name == field).map(|f| f.wire.clone()),
            _ => None,
        }
    }

    fn args_with(&mut self, op: &str, base: &Map<String, Value>) -> Map<String, Value> {
        let o = self.map.op(op).unwrap().clone();
        let mut a = self.draw.args(&o);
        for (k, v) in base {
            a.insert(k.clone(), v.clone());
        }
        a
    }
}

fn body_record(map: &WireMap, op: &str) -> Option<(String, String)> {
    let o = map.op(op)?;
    let b = o.body()?;
    match WireMap::ty(&b.ty).unmaybe() {
        Ty::Named(n) => Some((b.name.clone(), n.clone())),
        _ => None,
    }
}

/// Runs the scripted session against a core serving its contract, and
/// gives every step with what the core answered.
pub fn script(twin: &TestTwin, seed: u64) -> Vec<Value> {
    let map = wire_map(&twin.contract);
    let mut s = Script { twin, map: &map, draw: Gen::new(&map, seed), steps: vec![] };
    let resources = map.resources.clone();
    let mut first_create: Option<(String, Map<String, Value>)> = None;
    let mut a_read: Option<(String, Map<String, Value>)> = None;
    for r in &resources {
        let Some(create) = r.create.clone() else { continue };
        let mut parents = Map::new();
        for p in &r.parents {
            let v = s.draw.value(&WireMap::ty(&p.ty), &[], 0);
            parents.insert(p.name.clone(), v);
        }
        let mut args = s.args_with(&create, &parents);
        if let (Some((body, record)), Some(kf)) = (body_record(&map, &create), r.key_field.as_deref()) {
            if r.keys == "client" {
                if let Some(w) = s.wire_of(&record, kf) {
                    if args[&body].get(&w).is_none_or(Value::is_null) {
                        let word = s.draw.word();
                        args[&body][&w] = json!(word);
                    }
                }
            }
        }
        if first_create.is_none() {
            first_create = Some((create.clone(), args.clone()));
        }
        let Some(answer) = s.call(&create, &args) else { continue };
        let key = r.key_field.as_deref().and_then(|kf| s.wire_of(&r.record, kf)).and_then(|w| answer["ok"].get(&w).cloned());
        let Some(key) = key.filter(|k| !k.is_null()) else { continue };
        let mut place = parents.clone();
        place.insert(r.key.clone(), key.clone());
        let read = s.args_with(&r.read, &place);
        a_read.get_or_insert((r.read.clone(), read.clone()));
        s.call(&r.read, &read);
        if let Some(list) = &r.list {
            let a = s.args_with(list, &parents);
            s.call(list, &a);
        }
        for change in [&r.update, &r.replace].into_iter().flatten() {
            let mut a = s.args_with(change, &place);
            if r.replace_by_body_key && Some(change) == r.replace.as_ref() {
                if let (Some((body, record)), Some(kf)) = (body_record(&map, change), r.key_field.as_deref()) {
                    if let Some(w) = s.wire_of(&record, kf) {
                        a[&body][&w] = key.clone();
                    }
                }
            }
            s.call(change, &a);
            s.call(&r.read, &read);
        }
        s.call(&create, &args);
        if let Some(delete) = &r.delete {
            let a = s.args_with(delete, &place);
            s.call(delete, &a);
            s.call(&r.read, &read);
        }
    }
    // Every operation that is not part of a resource.
    for op in map.operations.clone() {
        if map.role(&op.operation).is_none() {
            let a = s.draw.args(&op);
            s.call(&op.operation, &a);
        }
    }
    // Requests that break the document.
    for op in map.operations.clone() {
        let valid = s.draw.args(&op);
        for bad in s.draw.invalid(&op, &valid) {
            let mut out = map.build_request(&op, &bad.args);
            if let Some(raw) = bad.raw_body {
                out.body = raw;
            }
            s.api(out);
        }
    }
    // No operation at the path; no operation with the method.
    s.api(Outgoing { method: "GET".into(), target: "/lawtwin/no-such-path".into(), headers: vec![], body: vec![] });
    if let Some((read, args)) = a_read.clone() {
        let mut out = map.build_request(map.op(&read).unwrap(), &args);
        out.method = "TRACE".into();
        s.api(out);
    }
    // Forced states: one the operations reach, and one they do not.
    if let Some(r) = resources.iter().find(|r| r.create.is_some()) {
        let create = r.create.clone().unwrap();
        let mut parents = Map::new();
        for p in &r.parents {
            let v = s.draw.value(&WireMap::ty(&p.ty), &[], 0);
            parents.insert(p.name.clone(), v);
        }
        let a = s.args_with(&create, &parents);
        let (body_name, _) = body_record(&map, &create).unwrap_or_default();
        let body = a.get(&body_name).cloned().unwrap_or(json!({}));
        s.admin("POST", "/v1/state", json!({ "resources": { &r.record: [{ "parents": parents, "body": body }] } }));
        s.admin("GET", "/v1/state", Value::Null);
        // The same record twice, and a record with a key the service would not give it.
        let item = json!({ "parents": parents, "body": body });
        s.admin("POST", "/v1/state", json!({ "resources": { &r.record: [item.clone(), item] } }));
    }
    for r in &resources {
        if let Some(kf) = r.key_field.clone() {
            let mut record = s.draw.value(&Ty::Named(r.record.clone()), &[], 0);
            if let Some(w) = s.wire_of(&r.record, &kf) {
                record[&w] = json!(987_654);
                if r.keys == "client" {
                    record[&w] = json!(format!("forced-{}", s.draw.word()));
                }
            }
            let mut parents = Map::new();
            for p in &r.parents {
                parents.insert(p.name.clone(), json!("forced"));
            }
            s.admin("POST", "/v1/state", json!({ "resources": { &r.record: [{ "parents": parents, "record": record }] } }));
        }
    }
    // Every fault, as a client sees it.
    let (probe_op, probe) = a_read.unwrap_or_else(|| {
        let op = map.operations[0].clone();
        let a = s.draw.args(&op);
        (op.operation, a)
    });
    let probe_out = map.build_request(map.op(&probe_op).unwrap(), &probe);
    s.admin("POST", "/v1/faults/rate-limit", json!({ "limit": 1, "window": "10s" }));
    s.api(probe_out.clone());
    s.api(probe_out.clone());
    s.admin("POST", "/v1/clock/advance", json!({ "by": "4s" }));
    s.api(probe_out.clone());
    s.admin("POST", "/v1/clock/advance", json!({ "by": "6s" }));
    s.api(probe_out.clone());
    s.admin("DELETE", "/v1/faults/rate-limit", Value::Null);
    s.admin("POST", "/v1/faults/server-error", json!({ "status": 503, "count": 1 }));
    s.api(probe_out.clone());
    s.api(probe_out.clone());
    s.admin("POST", "/v1/faults/server-error", json!({ "probability": 50, "operation": probe_op }));
    for _ in 0..6 {
        s.api(probe_out.clone());
    }
    s.admin("DELETE", "/v1/faults", Value::Null);
    s.admin("POST", "/v1/faults/latency", json!({ "delay": "5ms", "jitter": "5ms", "for": "1s" }));
    s.api(probe_out.clone());
    s.admin("POST", "/v1/clock/advance", json!({ "by": "1s" }));
    s.admin("POST", "/v1/faults/slow-body", json!({ "chunk": 7, "delay": "1ms" }));
    s.api(probe_out.clone());
    s.admin("DELETE", "/v1/faults/slow-body", Value::Null);
    s.admin("POST", "/v1/faults/timeout", json!({ "hold": "50ms", "count": 1 }));
    s.api(probe_out.clone());
    s.admin("POST", "/v1/faults/connection-reset", json!({ "count": 1 }));
    s.api(probe_out.clone());
    if let Some((create, args)) = first_create {
        s.admin("POST", "/v1/faults/server-error", json!({ "applied": true, "count": 1, "operation": create }));
        let out = map.build_request(map.op(&create).unwrap(), &args);
        s.api(out);
    }
    s.admin("GET", "/v1/faults", Value::Null);
    s.admin("POST", "/v1/faults/no-such-fault", json!({}));
    s.admin("POST", "/v1/faults/latency", json!({ "delay": "soon" }));
    s.admin("GET", "/v1/state", Value::Null);
    s.admin("GET", "/v1/laws", Value::Null);
    s.steps
}

/// Sends a recorded session's steps to a core, and gives each step whose
/// answer differs: the step, what was recorded, and what came.
pub fn replay_steps(twin: &TestTwin, steps: &[Value]) -> Vec<Value> {
    let mut out = vec![];
    for (i, step) in steps.iter().enumerate() {
        let got = match step["step"].as_str() {
            Some("api") => response_json(&twin.send(&request_of(&step["request"]))),
            _ => {
                let body = if step["body"].is_null() { String::new() } else { step["body"].to_string() };
                let (status, answer) = twin.admin(step["method"].as_str().unwrap_or("GET"), step["path"].as_str().unwrap_or("/"), &body);
                json!({ "status": status, "body": answer })
            }
        };
        if got != step["response"] {
            out.push(json!({ "step": i + 1, "sent": step.get("request").or(step.get("path")), "recorded": step["response"], "answered": got }));
        }
    }
    out
}
