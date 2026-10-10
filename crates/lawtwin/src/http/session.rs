//! A session of seeded traffic against an HTTP service: the simulated
//! client that `lawtwin peer http call` and `lawtwin conform http` run.
//!
//! For each resource of the contract it walks the sequences the contract's
//! laws are about (create, read, list, update, replace, delete, read), with
//! values drawn from the seed; on request it also sends requests that break
//! the document, retries and duplicates writes, and hangs up before
//! answers. Every answer is read as the model's (`{"ok": ...}` or
//! `{"error": ...}`) and written as a history entry, so the contract's laws
//! are checked over what the service did, with the same check a twin runs
//! over its own history.

use lawtwin_wire::call::{Incoming, Outgoing, Reading};
use lawtwin_wire::history::op_entry;
use lawtwin_wire::http::{self, Endpoint, Failure};
use lawtwin_wire::map::{Operation, Resource, TypeDef};
use lawtwin_wire::traffic::Gen;
use lawtwin_wire::{Ty, WireMap, laws};
use serde_json::{Map, Value, json};
use std::time::Duration;

/// What to send.
#[derive(Clone, Debug)]
pub struct Options {
    pub seed: u64,
    /// How many times each resource's sequence runs.
    pub rounds: usize,
    /// Also send requests that break the document.
    pub invalid: bool,
    /// Also send each successful create again, as a client that lost the answer does.
    pub retries: bool,
    /// Also send each create twice at once.
    pub duplicates: bool,
    /// Also hang up on a create before its answer, then look for what it did.
    pub timeouts: bool,
    /// Also call every operation that is not part of a resource.
    pub all_operations: bool,
    /// How long to wait for each answer.
    pub timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            seed: 0,
            rounds: 3,
            invalid: false,
            retries: false,
            duplicates: false,
            timeouts: false,
            all_operations: false,
            timeout: Duration::from_secs(10),
        }
    }
}

/// What a session saw.
pub struct Session<'m> {
    pub map: &'m WireMap,
    pub ep: Endpoint,
    pub draw: Gen<'m>,
    pub opts: Options,
    /// The calls answered, as history entries.
    pub entries: Vec<Value>,
    /// Every exchange: the request, the status, the answer, and what in it breaks the document.
    pub exchanges: Vec<Value>,
    /// What breaks the document, besides the laws.
    pub problems: Vec<Value>,
    /// Behaviour the contract forbids that is not a law over a history: an
    /// invalid request accepted, a failure of the server.
    pub findings: Vec<Value>,
}

impl<'m> Session<'m> {
    pub fn new(map: &'m WireMap, ep: Endpoint, opts: Options) -> Session<'m> {
        Session { map, ep, draw: Gen::new(map, opts.seed), opts, entries: vec![], exchanges: vec![], problems: vec![], findings: vec![] }
    }

    /// The contract's laws over what the session saw.
    pub fn violations(&self) -> Vec<Value> {
        laws::check_history(self.map, &self.entries)
    }

    /// Sends a call, reads its answer, and writes both down. `None` when no
    /// answer came.
    pub fn call(&mut self, op: &Operation, args: &Map<String, Value>) -> Option<Reading> {
        let out = self.map.build_request(op, args);
        self.send(op, out, None)
    }

    fn send(&mut self, op: &Operation, out: Outgoing, what: Option<&str>) -> Option<Reading> {
        let result = http::send(&self.ep, &out, self.opts.timeout);
        let seq = self.exchanges.len() + 1;
        match result {
            Ok(reply) => {
                let reading = self.map.read_response(op, reply.status, &reply.headers, &reply.body);
                // A request sent broken is judged by what it was refused with, below.
                for p in reading.problems.iter().filter(|_| what.is_none()) {
                    self.problems.push(json!({ "exchange": seq, "op": op.operation, "problem": p }));
                }
                let (path, query) = out.target.split_once('?').unwrap_or((&out.target, ""));
                let incoming = Incoming { method: &out.method, path, query, headers: &out.headers, body: &out.body };
                if what.is_none() {
                    if let Ok(call) = self.map.read_request(&incoming) {
                        let n = self.entries.len() as u64 + 1;
                        self.entries.push(op_entry(n, 0, "wire", &op.operation, &call.args, reading.answer.clone()));
                    }
                }
                self.exchanges.push(json!({
                    "exchange": seq, "op": op.operation, "method": out.method, "target": out.target,
                    "status": reply.status, "answer": reading.answer, "sent": what,
                }));
                Some(reading)
            }
            Err(f) => {
                self.exchanges.push(json!({ "exchange": seq, "op": op.operation, "method": out.method, "target": out.target, "failure": f.to_string(), "sent": what }));
                self.findings.push(json!({ "exchange": seq, "op": op.operation, "finding": format!("{} {}: {f}", out.method, out.target) }));
                None
            }
        }
    }

    fn wire_of(&self, record: &str, field: &str) -> Option<String> {
        match self.map.type_def(record) {
            Some(TypeDef::Record { fields, .. }) => fields.iter().find(|f| f.name == field).map(|f| f.wire.clone()),
            _ => None,
        }
    }

    /// Runs every resource's sequence, `rounds` times, and what else was asked.
    pub fn run(&mut self) {
        let resources = self.map.resources.clone();
        for _ in 0..self.opts.rounds {
            for r in &resources {
                self.resource(r);
            }
            if self.opts.all_operations {
                let ops: Vec<Operation> = self.map.operations.iter().filter(|o| self.map.role(&o.operation).is_none()).cloned().collect();
                for op in ops {
                    let args = self.draw.args(&op);
                    self.call(&op, &args);
                }
            }
            if self.opts.invalid {
                let ops = self.map.operations.clone();
                for op in ops {
                    self.invalid(&op);
                }
            }
        }
    }

    fn invalid(&mut self, op: &Operation) {
        let valid = self.draw.args(op);
        for bad in self.draw.invalid(op, &valid) {
            let mut out = self.map.build_request(op, &bad.args);
            if let Some(raw) = &bad.raw_body {
                out.body = raw.clone();
            }
            let Some(reading) = self.send(op, out, Some(&bad.what)) else { continue };
            let s = reading.status;
            if s == 501 {
                // The operation is not served at all; there is nothing to refuse.
                continue;
            }
            if (200..300).contains(&s) {
                self.findings.push(json!({ "op": op.operation, "finding": format!("{} accepted a request that breaks the document ({}), answering {s}", op.operation, bad.what) }));
            } else if s >= 500 {
                self.findings.push(json!({ "op": op.operation, "finding": format!("{} failed with {s} on a request that breaks the document ({}), rather than refusing it", op.operation, bad.what) }));
            } else if s != 404 && !op.documents(s) && s != op.invalid_status() {
                self.problems.push(json!({ "op": op.operation, "problem": format!("{} refused a request that breaks the document ({}) with {s}, which it does not document", op.operation, bad.what) }));
            }
        }
    }

    /// The arguments that place a record: its parents, and its key.
    fn place(&self, r: &Resource, parents: &Map<String, Value>, key: &Value) -> Map<String, Value> {
        let mut a = parents.clone();
        a.insert(r.key.clone(), key.clone());
        a
    }

    fn with(&mut self, op_name: Option<&str>, base: &Map<String, Value>) -> Option<(Operation, Map<String, Value>)> {
        let op = self.map.op(op_name?)?.clone();
        let mut args = self.draw.args(&op);
        for (k, v) in base {
            args.insert(k.clone(), v.clone());
        }
        Some((op, args))
    }

    fn resource(&mut self, r: &Resource) {
        let Some(create) = r.create.as_deref().and_then(|c| self.map.op(c)).cloned() else { return };
        let mut parents = Map::new();
        for p in &r.parents {
            let v = self.draw.value(&WireMap::ty(&p.ty), &[], 0);
            parents.insert(p.name.clone(), v);
        }
        let mut args = self.draw.args(&create);
        for (k, v) in &parents {
            args.insert(k.clone(), v.clone());
        }
        // A key the client gives is always given, so a create can succeed.
        if let (Some(body), Some(kf)) = (create.body(), r.key_field.as_deref()) {
            if r.keys == "client" {
                if let Some(w) = self.wire_of(&body_record(&body.ty), kf) {
                    if args[&body.name].get(&w).is_none_or(Value::is_null) {
                        let word = self.draw.word();
                        args[&body.name][&w] = json!(word);
                    }
                }
            }
        }
        if self.opts.timeouts {
            let out = self.map.build_request(&create, &args);
            let _ = http::send_and_hang_up(&self.ep, &out, Duration::from_millis(1));
            self.exchanges.push(json!({ "op": create.operation, "method": out.method, "target": out.target, "sent": "hung up before the answer" }));
            // Give the request time to land before anything looks for it.
            std::thread::sleep(Duration::from_millis(50));
        }
        let created = if self.opts.duplicates {
            let out = self.map.build_request(&create, &args);
            let (ep, t) = (self.ep.clone(), self.opts.timeout);
            let twin = out.clone();
            let other = std::thread::spawn(move || http::send(&ep, &twin, t));
            let first = self.send(&create, out, None);
            let second = other.join().unwrap_or(Err(Failure::Closed("the thread failed".into())));
            if let Ok(reply) = second {
                let reading = self.map.read_response(&create, reply.status, &reply.headers, &reply.body);
                if reply.status >= 500 {
                    self.findings.push(json!({ "op": create.operation, "finding": format!("a duplicate {} failed with {}", create.operation, reply.status) }));
                }
                self.exchanges.push(json!({ "op": create.operation, "status": reply.status, "answer": reading.answer, "sent": "the same create, at the same time" }));
            }
            first
        } else {
            self.call(&create, &args)
        };
        let Some(created) = created else { return };
        let Some(record) = created.answer.get("ok").cloned() else { return };
        let key = r
            .key_field
            .as_deref()
            .and_then(|kf| self.wire_of(&r.record, kf))
            .and_then(|w| record.get(&w).cloned())
            .filter(|k| !k.is_null());
        let Some(key) = key else { return };
        let place = self.place(r, &parents, &key);
        if let Some((op, a)) = self.with(Some(&r.read), &place) {
            self.call(&op, &a);
        }
        if let Some((op, a)) = self.with(r.list.as_deref(), &parents) {
            self.call(&op, &a);
        }
        if self.opts.retries {
            if let Some(reading) = self.call(&create, &args) {
                if reading.status >= 500 {
                    self.findings.push(json!({ "op": create.operation, "finding": format!("a retried {} failed with {}", create.operation, reading.status) }));
                }
            }
            if let Some((op, a)) = self.with(Some(&r.read), &place) {
                self.call(&op, &a);
            }
        }
        if let Some((op, a)) = self.with(r.update.as_deref(), &place) {
            self.call(&op, &a);
            if let Some((op, a)) = self.with(Some(&r.read), &place) {
                self.call(&op, &a);
            }
        }
        if let Some((op, mut a)) = self.with(r.replace.as_deref(), &place) {
            if r.replace_by_body_key {
                if let (Some(body), Some(kf)) = (op.body(), r.key_field.as_deref()) {
                    if let Some(w) = self.wire_of(&body_record(&body.ty), kf) {
                        a[&body.name][&w] = key.clone();
                    }
                }
            }
            self.call(&op, &a);
            if let Some((op, a)) = self.with(Some(&r.read), &place) {
                self.call(&op, &a);
            }
        }
        if let Some((op, a)) = self.with(r.delete.as_deref(), &place) {
            self.call(&op, &a);
            if let Some((op, a)) = self.with(Some(&r.read), &place) {
                self.call(&op, &a);
            }
        }
    }

    /// The session's report.
    pub fn report(&self, target: &str) -> Value {
        let violations = self.violations();
        let checked: Vec<&String> = self.map.resources.iter().flat_map(|r| r.laws.values()).collect();
        json!({
            "contract": self.map.package,
            "target": target,
            "seed": self.opts.seed,
            "requests": self.exchanges.len(),
            "calls": self.entries.len(),
            "laws": checked,
            "violations": violations,
            "findings": self.findings,
            "problems": self.problems,
            "holds": violations.is_empty() && self.findings.is_empty(),
        })
    }
}

/// The record a body's type names.
fn body_record(ty: &str) -> String {
    match WireMap::ty(ty).unmaybe() {
        Ty::Named(n) => n.clone(),
        _ => String::new(),
    }
}
