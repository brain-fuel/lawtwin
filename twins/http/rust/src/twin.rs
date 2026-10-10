//! The twin: one contract's model, its clock, seed, history and faults, and
//! how each HTTP request is answered.
//!
//! A request is read as a call of the contract (`lawtwin_wire::call`),
//! refused as the document says when it is not one, run on the model, and
//! answered with the status, headers and body the document gives the
//! model's answer. Active faults decide whether the request reaches the
//! model, and how and when the answer is sent; never what the model
//! answers.

use crate::V;
use crate::clock::{Clock, ClockMode};
use crate::faults::{Active, TwinFile, check};
use crate::history::{answer_json, control_entry, op_entry};
use crate::model::{Contract, Model, Outcome};
use lawtwin_wire::call::{Incoming, Refusal};
use lawtwin_wire::map::{Operation, Role};
use lawtwin_wire::rng::Rng;
use lawtwin_wire::schema::{self, Problem};
use lawtwin_wire::{Ty, WireMap};
use serde_json::{Map, Value, json};
use std::sync::{Mutex, MutexGuard};

/// When the twin clock starts: 2026-01-01T00:00:00Z.
pub const EPOCH_MS: i64 = 1_767_225_600_000;

/// The core's settings.
#[derive(Clone, Debug)]
pub struct Config {
    /// The contract it serves, by name (`petstore`) or package.
    pub contract: String,
    /// Where the API listens, `host:port`.
    pub listen: String,
    /// Where the admin API listens.
    pub admin_listen: String,
    /// The address clients reach, for `Location` headers and `/v1/twin`.
    pub advertised: Option<String>,
    pub seed: u64,
    pub clock: ClockMode,
    /// A deliberate defect, for showing that conformance and replay catch
    /// a twin that parts from its contract: `drop-field` or `skip-delete`.
    pub mutant: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            contract: String::new(),
            listen: "127.0.0.1:8080".into(),
            admin_listen: "127.0.0.1:9644".into(),
            advertised: None,
            seed: 0,
            clock: ClockMode::Frozen,
            mutant: None,
        }
    }
}

/// How a request is answered, which the server carries out.
#[derive(Debug)]
pub struct Plan {
    /// Real time to wait before answering.
    pub delay_ms: u64,
    pub action: Action,
}

#[derive(Debug)]
pub enum Action {
    Respond {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        /// Send the body this many bytes at a time, this many milliseconds apart.
        slow: Option<(usize, u64)>,
    },
    /// Send nothing; hold the connection this long, then close it.
    Hang(u64),
    /// Reset the connection.
    Reset,
}

/// What the active faults do to one request.
#[derive(Default)]
struct Effects {
    before: Option<(u64, String, Refuse)>,
    after: Option<(u64, String, Refuse)>,
    delay_ms: u64,
    slow: Option<(usize, u64)>,
}

#[derive(Clone, Debug)]
enum Refuse {
    Status { status: u16, retry_after: Option<i64> },
    Hang(u64),
    Reset,
}

/// The twin's mutable state.
pub struct Core {
    pub contract: Contract,
    pub map: WireMap,
    pub twin_file: TwinFile,
    pub model: Box<dyn Model>,
    pub seed: u64,
    pub rng: Rng,
    pub clock: Clock,
    pub history: Vec<Value>,
    pub faults: Vec<Active>,
    next_fault: u64,
    mutant: Option<String>,
    base_url: String,
}

/// The running twin.
pub struct Twin {
    core: Mutex<Core>,
    pub config: Config,
}

pub fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

/// An HTTP date (RFC 9110) for a time in milliseconds since the epoch.
pub fn http_date(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let weekday = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][days.rem_euclid(7) as usize];
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    let month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][(m - 1) as usize];
    format!("{weekday}, {d:02} {month} {y} {:02}:{:02}:{:02} GMT", rem / 3600, rem % 3600 / 60, rem % 60)
}

impl Twin {
    pub fn new(contract: Contract, config: Config) -> Result<Twin, String> {
        let map = WireMap::parse(contract.wire_map)?;
        let twin_file = TwinFile::parse(contract.twin_file)?;
        let host = config.advertised.clone().unwrap_or_else(|| config.listen.replace("0.0.0.0", "localhost"));
        let core = Core {
            contract,
            map,
            twin_file,
            model: (contract.start)(),
            seed: config.seed,
            rng: Rng::new(config.seed),
            clock: Clock::new(config.clock, EPOCH_MS),
            history: vec![],
            faults: vec![],
            next_fault: 1,
            mutant: config.mutant.clone(),
            base_url: format!("http://{host}"),
        };
        Ok(Twin { core: Mutex::new(core), config })
    }

    pub fn lock(&self) -> MutexGuard<'_, Core> {
        self.core.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Answers one request.
    pub fn handle(&self, method: &str, target: &str, headers: &[(String, String)], body: &[u8]) -> Plan {
        let mut core = self.lock();
        core.expire();
        core.answer(method, target, headers, body)
    }
}

fn json_body(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap_or_default()
}

impl Core {
    fn now(&self) -> i64 {
        self.clock.now_ms()
    }

    /// Writes an entry at the end of the history.
    pub fn record(&mut self, mut entry: Value) {
        entry["seq"] = json!(self.history.len() as u64 + 1);
        self.history.push(entry);
    }

    pub fn control(&mut self, source: &str, control: &str, detail: Value) {
        let at = self.now();
        self.record(control_entry(0, at, source, control, detail));
    }

    /// Ends the faults whose time on the twin clock is up.
    pub fn expire(&mut self) {
        let now = self.now();
        let (ended, kept): (Vec<Active>, Vec<Active>) = std::mem::take(&mut self.faults).into_iter().partition(|f| f.until_ms.is_some_and(|u| u <= now));
        self.faults = kept;
        for f in ended {
            self.control("fault", "fault ended", json!({ "id": f.id, "name": f.name, "why": "its time is up" }));
        }
    }

    /// Starts again from an empty state, with this seed.
    pub fn reset(&mut self, seed: u64) {
        self.model = (self.contract.start)();
        self.seed = seed;
        self.rng = Rng::new(seed);
        self.clock = Clock::new(self.clock.mode(), EPOCH_MS);
        self.history.clear();
        self.faults.clear();
        self.next_fault = 1;
    }

    /// Raises a declared fault.
    pub fn raise(&mut self, name: &str, params: &Map<String, Value>) -> Result<Value, String> {
        let decl = self.twin_file.decl(name).ok_or_else(|| {
            format!("{name} is not a fault the twin file declares; it declares {}", self.twin_file.faults.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", "))
        })?;
        let (params, for_ms, count) = check(decl, &self.map, params)?;
        let now = self.now();
        let f = Active {
            id: self.next_fault,
            name: name.to_string(),
            params,
            until_ms: for_ms.map(|ms| now + ms),
            remaining: count,
            window_start_ms: now,
            window_used: 0,
        };
        self.next_fault += 1;
        let shown = f.to_json();
        self.control("fault", "fault raised", shown.clone());
        self.faults.push(f);
        Ok(shown)
    }

    /// Clears the active faults with this name, or all.
    pub fn clear(&mut self, name: Option<&str>) -> usize {
        let (gone, kept): (Vec<Active>, Vec<Active>) = std::mem::take(&mut self.faults).into_iter().partition(|f| name.is_none_or(|n| f.name == n));
        self.faults = kept;
        for f in &gone {
            self.control("fault", "fault cleared", json!({ "id": f.id, "name": f.name }));
        }
        gone.len()
    }

    /// What the active faults do to a request for this operation.
    fn effects(&mut self, op: &str) -> Effects {
        let now = self.now();
        let mut e = Effects::default();
        let mut used_up = vec![];
        for i in 0..self.faults.len() {
            if !self.faults[i].matches(op) {
                continue;
            }
            let p = self.faults[i].int("probability").unwrap_or(100);
            // A draw is made only for a fault that may let a request by, so
            // faults that always apply leave the sequence alone.
            if p < 100 && !self.rng.chance(p.max(0) as u64) {
                continue;
            }
            let f = &mut self.faults[i];
            let refuse = match f.name.as_str() {
                "latency" => {
                    let jitter = f.ms("jitter").unwrap_or(0).max(0) as u64;
                    let extra = if jitter > 0 { self.rng.below(jitter + 1) } else { 0 };
                    let f = &self.faults[i];
                    e.delay_ms += f.ms("delay").unwrap_or(0).max(0) as u64 + extra;
                    continue;
                }
                "slow-body" => {
                    e.slow = Some((f.int("chunk").unwrap_or(16).max(1) as usize, f.ms("delay").unwrap_or(100).max(0) as u64));
                    continue;
                }
                "rate-limit" => {
                    let window = f.ms("window").unwrap_or(1000).max(1);
                    if now >= f.window_start_ms + window {
                        f.window_start_ms += (now - f.window_start_ms) / window * window;
                        f.window_used = 0;
                    }
                    if f.window_used < f.int("limit").unwrap_or(0) {
                        f.window_used += 1;
                        continue;
                    }
                    let left = f.window_start_ms + window - now;
                    Refuse::Status { status: 429, retry_after: Some(((left + 999) / 1000).max(1)) }
                }
                "server-error" => Refuse::Status { status: f.int("status").unwrap_or(500) as u16, retry_after: None },
                "timeout" => Refuse::Hang(f.ms("hold").unwrap_or(30_000).max(0) as u64),
                "connection-reset" => Refuse::Reset,
                _ => continue,
            };
            let blocked = e.before.is_some();
            let slot = if f.flag("applied") { &mut e.after } else { &mut e.before };
            if slot.is_none() && !blocked {
                *slot = Some((f.id, f.name.clone(), refuse));
                if let Some(r) = f.remaining.as_mut() {
                    *r -= 1;
                    if *r <= 0 {
                        used_up.push(f.id);
                    }
                }
            }
        }
        for id in used_up {
            if let Some(pos) = self.faults.iter().position(|f| f.id == id) {
                let f = self.faults.remove(pos);
                self.control("fault", "fault ended", json!({ "id": f.id, "name": f.name, "why": "used up" }));
            }
        }
        e
    }

    /// A body for a failing status: the document's, when it gives one.
    fn failure_body(&self, op: Option<&Operation>, status: u16, message: &str) -> (Vec<(String, String)>, Vec<u8>) {
        let documented = op.and_then(|o| o.failures.iter().find(|f| lawtwin_wire::map::status_matches(&f.status, status)));
        let problem = Problem { status, reason: reason(status), message };
        let (media, value) = match documented.and_then(|f| f.body.as_ref()) {
            Some(b) => (b.content.clone(), schema::instance(&b.schema, &problem)),
            None => ("application/json".into(), json!({ "message": message })),
        };
        (vec![("Content-Type".into(), media)], json_body(&value))
    }

    fn respond(&self, status: u16, mut headers: Vec<(String, String)>, body: Vec<u8>) -> Action {
        headers.insert(0, ("Date".into(), http_date(self.now())));
        Action::Respond { status, headers, body, slow: None }
    }

    fn refusal(&mut self, op: &Operation, fault: (u64, String, Refuse), applied: bool) -> Action {
        let (id, name, refuse) = fault;
        let detail = json!({ "id": id, "name": name, "op": op.operation, "applied": applied });
        self.control("fault", "fault answered", detail);
        match refuse {
            Refuse::Status { status, retry_after } => {
                let (mut headers, body) = self.failure_body(Some(op), status, &format!("lawtwin: the fault {name} answered {status}"));
                if let Some(s) = retry_after {
                    headers.push(("Retry-After".into(), s.to_string()));
                }
                self.respond(status, headers, body)
            }
            Refuse::Hang(ms) => Action::Hang(ms),
            Refuse::Reset => Action::Reset,
        }
    }

    fn answer(&mut self, method: &str, target: &str, headers: &[(String, String)], body: &[u8]) -> Plan {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let incoming = Incoming { method, path, query, headers, body };
        let call = match self.map.read_request(&incoming) {
            Ok(c) => c,
            Err(Refusal::NoRoute { allowed }) => {
                let (status, extra) = if allowed.is_empty() {
                    (404, vec![])
                } else {
                    (405, vec![("Allow".to_string(), allowed.join(", "))])
                };
                let message = format!("lawtwin: {} has no operation at {method} {path}", self.map.package);
                let (mut h, b) = self.failure_body(None, status, &message);
                h.extend(extra);
                return Plan { delay_ms: 0, action: self.respond(status, h, b) };
            }
            Err(Refusal::Invalid { op, status, reason: why }) => {
                let at = self.now();
                self.record(control_entry(0, at, "wire", "request refused", json!({ "op": op, "status": status, "reason": why })));
                let o = self.map.op(&op).cloned();
                let (h, b) = self.failure_body(o.as_ref(), status, &why);
                return Plan { delay_ms: 0, action: self.respond(status, h, b) };
            }
        };
        let op = self.map.op(&call.op).cloned().expect("a routed operation");
        let effects = self.effects(&op.operation);
        if let Some(f) = effects.before.clone() {
            let action = self.refusal(&op, f, false);
            return Plan { delay_ms: effects.delay_ms, action };
        }
        let skip = self.mutant.as_deref() == Some("skip-delete") && matches!(self.map.role(&op.operation), Some((_, Role::Delete)));
        let result = if skip { Ok(V::Unit) } else { self.model.call(&op.operation, call.values.clone()) };
        let at = self.now();
        let answer = answer_json(&self.map, &op.operation, &result);
        self.record(op_entry(0, at, "wire", &op.operation, &call.args, answer));
        if let Some(f) = effects.after.clone() {
            let action = self.refusal(&op, f, true);
            return Plan { delay_ms: effects.delay_ms, action };
        }
        let mut action = match result {
            Ok(v) => self.success(&op, &call.args, &v),
            Err(Outcome::Failure(ctor)) => {
                let status = self.map.failure_status(&ctor);
                let message = if ctor == "Unmodelled" {
                    format!("lawtwin: {} does not model {} yet; write it in src/handwritten.lawspec", self.map.package, op.operation)
                } else {
                    format!("{ctor}: {}", reason(status))
                };
                let (h, b) = self.failure_body(Some(&op), status, &message);
                self.respond(status, h, b)
            }
            Err(Outcome::Defect(d)) => {
                self.control("wire", "model defect", json!({ "op": op.operation, "defect": d }));
                let (h, b) = self.failure_body(None, 500, &format!("lawtwin: a defect in the generated model: {d}"));
                self.respond(500, h, b)
            }
        };
        if let (Action::Respond { slow, .. }, Some(s)) = (&mut action, effects.slow) {
            *slow = Some(s);
        }
        Plan { delay_ms: effects.delay_ms, action }
    }

    /// The success response: the document's status, its media type, the
    /// answer as JSON, and the headers it documents.
    fn success(&mut self, op: &Operation, args: &[(String, Value)], v: &V) -> Action {
        let status = op.success_status();
        let ty = WireMap::ty(&op.response.ty);
        let mut headers = vec![];
        let mut body = vec![];
        if ty != Ty::Unit && status != 204 {
            let mut json = self.map.encode(&ty, v);
            if self.mutant.as_deref() == Some("drop-field") {
                drop_last_field(&mut json);
            }
            let media = op.response.content.clone().unwrap_or_else(|| "application/json".into());
            body = match (&json, media.contains("json")) {
                (Value::String(s), false) => s.clone().into_bytes(),
                _ => json_body(&json),
            };
            headers.push(("Content-Type".into(), media));
        }
        for h in &op.response.headers {
            if h.name.eq_ignore_ascii_case("location") {
                if let Some(loc) = self.location(op, args, v) {
                    headers.push((h.name.clone(), loc));
                    continue;
                }
            }
            if h.required {
                let value = match &h.example {
                    Value::String(s) => s.clone(),
                    Value::Null => match h.schema.get("type").and_then(Value::as_str) {
                        Some("integer") | Some("number") => self.rng.below(1000).to_string(),
                        _ => format!("lawtwin-{}", self.rng.below(1_000_000)),
                    },
                    other => other.to_string(),
                };
                headers.push((h.name.clone(), value));
            }
        }
        self.respond(status, headers, body)
    }

    /// Where a created record now is: the item path, with the record's key.
    fn location(&self, op: &Operation, args: &[(String, Value)], v: &V) -> Option<String> {
        let (r, role) = self.map.role(&op.operation)?;
        if role != Role::Create {
            return None;
        }
        let json = self.map.encode(&WireMap::ty(&op.response.ty), v);
        let key_wire = match self.map.type_def(&r.record)? {
            lawtwin_wire::map::TypeDef::Record { fields, .. } => fields.iter().find(|f| Some(&f.name) == r.key_field.as_ref())?.wire.clone(),
            _ => return None,
        };
        let key = json.get(&key_wire)?;
        let item_op = self.map.op(&r.read)?;
        let mut path = r.item.clone();
        for p in item_op.parameters.iter().filter(|p| p.location == "path") {
            let value = if p.name == r.key { key.clone() } else { args.iter().find(|(n, _)| *n == p.name)?.1.clone() };
            path = path.replace(&format!("{{{}}}", p.wire), &lawtwin_wire::call::percent_encode(&lawtwin_wire::call::json_text(&value)));
        }
        Some(format!("{}{path}", self.base_url))
    }
}

fn drop_last_field(v: &mut Value) {
    match v {
        Value::Object(m) => {
            let last = m.keys().next_back().cloned();
            if let Some(k) = last {
                m.shift_remove(&k);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(drop_last_field),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The twin clock's start reads as an HTTP date.
    #[test]
    fn dates() {
        assert_eq!(http_date(EPOCH_MS), "Thu, 01 Jan 2026 00:00:00 GMT");
        assert_eq!(http_date(EPOCH_MS + 86_400_000 * 59 + 3_723_000), "Sun, 01 Mar 2026 01:02:03 GMT");
    }
}
