//! HTTP requests as calls of the contract's operations, and the answers
//! as the model gives them.
//!
//! A twin reads a request into a [`Call`]: the operation its method and
//! path name, and each argument as a value of its type. A peer, a recorder
//! and a replay go the other way: from an operation and its arguments to a
//! request, and from a response to the answer `{"ok": ...}` or
//! `{"error": "<ApiError>"}` the model would give.

use crate::map::{Operation, WireMap, status_matches};
use crate::schema;
use crate::ty::Ty;
use crate::value::V;
use serde_json::{Map, Value, json};

/// A request as it arrives, before it is read.
pub struct Incoming<'a> {
    pub method: &'a str,
    /// The path, without the query.
    pub path: &'a str,
    /// The query, without the `?`.
    pub query: &'a str,
    pub headers: &'a [(String, String)],
    pub body: &'a [u8],
}

impl Incoming<'_> {
    /// A header's value, by name in any case.
    pub fn header(&self, name: &str) -> Option<&str> {
        header(self.headers, name)
    }
}

/// A header's value, by name in any case.
pub fn header<'h>(headers: &'h [(String, String)], name: &str) -> Option<&'h str> {
    headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

/// A request read as a call of an operation.
#[derive(Clone, Debug)]
pub struct Call {
    /// The operation's name in the contract.
    pub op: String,
    /// Each argument by its name in the contract, as JSON on the wire
    /// (`null` for an absent optional one), in the operation's order.
    pub args: Vec<(String, Value)>,
    /// The same arguments as values of their types.
    pub values: Vec<V>,
}

/// Why a request is not a call of the contract.
#[derive(Clone, Debug, PartialEq)]
pub enum Refusal {
    /// No operation has the path; or none has the method, with the methods it allows.
    NoRoute { allowed: Vec<String> },
    /// The request breaks the document: the status it is refused with, and why.
    Invalid { op: String, status: u16, reason: String },
}

/// Percent-decoding, with `+` as a space when `plus` (query strings).
pub fn percent_decode(text: &str, plus: bool) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                match std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()).ok_or(()) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                        continue;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b'+' if plus => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encoding of a path segment or query value.
pub fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The query's pairs, decoded.
pub fn query_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (percent_decode(k, true), percent_decode(v, true)),
            None => (percent_decode(p, true), String::new()),
        })
        .collect()
}

/// Matches a path against a template such as `/pet/{petId}`, giving each
/// parameter's decoded value; how many literal segments matched ranks it.
fn match_path(template: &str, path: &str) -> Option<(usize, Vec<(String, String)>)> {
    let t: Vec<&str> = template.trim_end_matches('/').split('/').collect();
    let p: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    if t.len() != p.len() {
        return None;
    }
    let mut params = vec![];
    let mut literal = 0;
    for (ts, ps) in t.iter().zip(&p) {
        if let Some(name) = ts.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            if ps.is_empty() {
                return None;
            }
            params.push((name.to_string(), percent_decode(ps, false)));
        } else if ts == ps {
            literal += 1;
        } else {
            return None;
        }
    }
    Some((literal, params))
}

impl WireMap {
    /// The operation a method and path name, with the path's parameters by
    /// their wire names. A literal segment wins over a parameter.
    pub fn route(&self, method: &str, path: &str) -> Result<(&Operation, Vec<(String, String)>), Refusal> {
        let mut best: Option<(usize, &Operation, Vec<(String, String)>)> = None;
        let mut allowed = vec![];
        let mut best_any: Option<usize> = None;
        for op in &self.operations {
            if let Some((rank, params)) = match_path(&op.path, path) {
                if best_any.is_none_or(|b| rank > b) {
                    best_any = Some(rank);
                    allowed.clear();
                }
                if Some(rank) == best_any {
                    allowed.push(op.method.clone());
                }
                if op.method.eq_ignore_ascii_case(method) && best.as_ref().is_none_or(|(r, ..)| rank > *r) {
                    best = Some((rank, op, params));
                }
            }
        }
        match best {
            Some((_, op, params)) => Ok((op, params)),
            None => Err(Refusal::NoRoute { allowed }),
        }
    }

    /// Reads a request as a call of the contract, or refuses it as the
    /// document says: a missing or mistyped parameter, or a body that is
    /// not JSON or not of its type.
    pub fn read_request(&self, req: &Incoming) -> Result<Call, Refusal> {
        let (op, path_params) = self.route(req.method, req.path)?;
        let invalid = |reason: String| Refusal::Invalid { op: op.operation.clone(), status: op.invalid_status(), reason };
        let query = query_pairs(req.query);
        let cookies: Vec<(String, String)> = req
            .header("cookie")
            .map(|c| {
                c.split(';')
                    .filter_map(|p| p.trim().split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let mut args = vec![];
        let mut values = vec![];
        for p in &op.parameters {
            let ty = WireMap::ty(&p.ty);
            let value = match p.location.as_str() {
                "body" => self.read_body(op, &ty, p.required, req).map_err(invalid)?,
                location => {
                    let texts: Vec<String> = match location {
                        "path" => path_params.iter().filter(|(k, _)| *k == p.wire).map(|(_, v)| v.clone()).collect(),
                        "query" => query.iter().filter(|(k, _)| *k == p.wire).map(|(_, v)| v.clone()).collect(),
                        "header" => req.header(&p.wire).map(|v| vec![v.to_string()]).unwrap_or_default(),
                        _ => cookies.iter().filter(|(k, _)| *k == p.wire).map(|(_, v)| v.clone()).collect(),
                    };
                    if texts.is_empty() {
                        if ty.is_maybe() {
                            V::Maybe(None)
                        } else if let Ty::List(_) = ty {
                            if p.required {
                                return Err(invalid(format!("the {location} parameter {} is required", p.wire)));
                            }
                            V::List(vec![])
                        } else {
                            return Err(invalid(format!("the {location} parameter {} is required", p.wire)));
                        }
                    } else {
                        let read = |t: &Ty| -> Result<V, Refusal> {
                            match t {
                                Ty::List(item) => {
                                    let mut xs = vec![];
                                    for text in &texts {
                                        for part in text.split(',').filter(|s| !s.is_empty()) {
                                            xs.push(self.decode_text(item, part).map_err(|e| invalid(format!("{}: {e}", p.wire)))?);
                                        }
                                    }
                                    Ok(V::List(xs))
                                }
                                t => self.decode_text(t, &texts[0]).map_err(|e| invalid(format!("the {location} parameter {}: {e}", p.wire))),
                            }
                        };
                        match &ty {
                            Ty::Maybe(inner) => V::Maybe(Some(Box::new(read(inner)?))),
                            t => read(t)?,
                        }
                    }
                }
            };
            args.push((p.name.clone(), self.encode(&ty, &value)));
            values.push(value);
        }
        Ok(Call { op: op.operation.clone(), args, values })
    }

    fn read_body(&self, op: &Operation, ty: &Ty, required: bool, req: &Incoming) -> Result<V, String> {
        if req.body.iter().all(u8::is_ascii_whitespace) {
            return if ty.is_maybe() || !required {
                Ok(if ty.is_maybe() { V::Maybe(None) } else { V::Unit })
            } else {
                Err("the request body is required".into())
            };
        }
        let media = op.request_content.as_deref().unwrap_or("application/json");
        let inner = ty.unmaybe();
        let wrap = |v: V| if ty.is_maybe() { V::Maybe(Some(Box::new(v))) } else { v };
        if media.contains("json") {
            if let Some(ct) = req.header("content-type") {
                if !ct.to_ascii_lowercase().contains("json") {
                    return Err(format!("the request body must be {media}, not {ct}"));
                }
            }
            let json: Value = serde_json::from_slice(req.body).map_err(|e| format!("the request body is not JSON: {e}"))?;
            return self.decode(inner, &json, "body").map(wrap).map_err(|m| m.to_string());
        }
        if *inner == Ty::Bytes {
            return Ok(wrap(V::Bytes(req.body.to_vec())));
        }
        if *inner == Ty::Text {
            return Ok(wrap(V::Text(String::from_utf8_lossy(req.body).into_owned())));
        }
        if media == "application/x-www-form-urlencoded" {
            let mut obj = Map::new();
            for (k, v) in query_pairs(&String::from_utf8_lossy(req.body)) {
                obj.insert(k, Value::String(v));
            }
            let json = self.form_to_json(inner, obj);
            return self.decode(inner, &json, "body").map(wrap).map_err(|m| m.to_string());
        }
        let json: Value = serde_json::from_slice(req.body).map_err(|e| format!("the request body is not {media}: {e}"))?;
        self.decode(inner, &json, "body").map(wrap).map_err(|m| m.to_string())
    }

    /// A form's text fields, typed by the record they fill.
    fn form_to_json(&self, ty: &Ty, obj: Map<String, Value>) -> Value {
        let Ty::Named(name) = ty else { return Value::Object(obj) };
        let Some(crate::map::TypeDef::Record { fields, .. }) = self.type_def(name) else {
            return Value::Object(obj);
        };
        let mut out = Map::new();
        for (k, v) in obj {
            let typed = fields
                .iter()
                .find(|f| f.wire == k)
                .and_then(|f| self.decode_text(&WireMap::ty(&f.ty), v.as_str().unwrap_or("")).ok())
                .map(|x| self.encode_untyped(&x))
                .unwrap_or(v);
            out.insert(k, typed);
        }
        Value::Object(out)
    }

    /// The request for a call: the operation and its arguments by name, as
    /// JSON on the wire. An argument left out, or `null`, is not sent.
    pub fn build_request(&self, op: &Operation, args: &Map<String, Value>) -> Outgoing {
        let mut path = op.path.clone();
        let mut query = vec![];
        let mut headers = vec![];
        let mut body = None;
        for p in &op.parameters {
            let Some(v) = args.get(&p.name).filter(|v| !v.is_null()) else { continue };
            match p.location.as_str() {
                "path" => path = path.replace(&format!("{{{}}}", p.wire), &percent_encode(&json_text(v))),
                "query" => match v {
                    Value::Array(items) => {
                        for i in items {
                            query.push(format!("{}={}", percent_encode(&p.wire), percent_encode(&json_text(i))));
                        }
                    }
                    v => query.push(format!("{}={}", percent_encode(&p.wire), percent_encode(&json_text(v)))),
                },
                "header" => headers.push((p.wire.clone(), json_text(v))),
                "cookie" => headers.push(("Cookie".into(), format!("{}={}", p.wire, json_text(v)))),
                _ => {
                    let media = op.request_content.clone().unwrap_or_else(|| "application/json".into());
                    let bytes = match (media.contains("json"), v) {
                        (false, Value::String(s)) => s.as_bytes().to_vec(),
                        _ => serde_json::to_vec(v).unwrap_or_default(),
                    };
                    headers.push(("Content-Type".into(), media));
                    body = Some(bytes);
                }
            }
        }
        let target = if query.is_empty() { path } else { format!("{path}?{}", query.join("&")) };
        Outgoing { method: op.method.clone(), target, headers, body: body.unwrap_or_default() }
    }

    /// Reads a response as the answer the model would give: `{"ok": value}`
    /// for the success status with a body of its type, `{"error": "<ApiError>"}`
    /// for a failing status. What breaks the document is listed beside it.
    pub fn read_response(&self, op: &Operation, status: u16, headers: &[(String, String)], body: &[u8]) -> Reading {
        let mut problems = vec![];
        if (200..300).contains(&status) {
            if !status_matches(&op.response.status, status) {
                problems.push(format!("{} answered {status}, not its documented {}", op.operation, op.response.status));
            }
            let ty = WireMap::ty(&op.response.ty);
            let answer = if ty == Ty::Unit {
                Value::Null
            } else {
                match serde_json::from_slice::<Value>(body) {
                    Ok(json) => match self.decode(&ty, &json, "") {
                        Ok(v) => self.encode(&ty, &v),
                        Err(m) => {
                            problems.push(format!("the {status} body of {} is not a {}: {m}", op.operation, op.response.ty));
                            json
                        }
                    },
                    Err(_) if matches!(ty, Ty::Text) => Value::String(String::from_utf8_lossy(body).into_owned()),
                    Err(e) => {
                        problems.push(format!("the {status} body of {} is not JSON: {e}", op.operation));
                        Value::Null
                    }
                }
            };
            if let Some(ct) = &op.response.content {
                match header(headers, "content-type") {
                    Some(given) if !media_matches(ct, given) => {
                        problems.push(format!("{} answered Content-Type {given}, not {ct}", op.operation))
                    }
                    None if !body.is_empty() => problems.push(format!("{} answered without a Content-Type", op.operation)),
                    _ => {}
                }
            }
            for h in op.response.headers.iter().filter(|h| h.required) {
                if header(headers, &h.name).is_none() {
                    problems.push(format!("{} answered without its required header {}", op.operation, h.name));
                }
            }
            return Reading { answer: json!({ "ok": answer }), status, problems };
        }
        let ctor = op
            .failures
            .iter()
            .find(|f| f.status.parse::<u16>().ok() == Some(status))
            .or_else(|| op.failures.iter().find(|f| status_matches(&f.status, status)))
            .map(|f| f.constructor.clone())
            .or_else(|| self.failure_constructor(status).map(String::from))
            .unwrap_or_else(|| format!("Status{status}"));
        let documented = op.documents(status);
        if !documented {
            problems.push(format!("{} answered {status}, which it does not document", op.operation));
        }
        if let Some(b) = op.failures.iter().find(|f| status_matches(&f.status, status)).and_then(|f| f.body.as_ref()) {
            match serde_json::from_slice::<Value>(body) {
                Ok(json) => {
                    for v in schema::violations(&b.schema, &json, "") {
                        problems.push(format!("the {status} body of {}: {v}", op.operation));
                    }
                }
                Err(_) => problems.push(format!("the {status} body of {} is not JSON", op.operation)),
            }
        }
        Reading { answer: json!({ "error": ctor }), status, problems }
    }
}

fn media_matches(documented: &str, given: &str) -> bool {
    let given = given.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    let documented = documented.to_ascii_lowercase();
    given == documented || (documented.contains("json") && given.contains("json")) || documented.ends_with("/*")
}

/// A JSON scalar as the text a path, query or header carries.
pub fn json_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Array(items) => items.iter().map(json_text).collect::<Vec<_>>().join(","),
        other => other.to_string(),
    }
}

/// A request to send.
#[derive(Clone, Debug)]
pub struct Outgoing {
    pub method: String,
    /// The path and query.
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// A response read as the model's answer.
#[derive(Clone, Debug)]
pub struct Reading {
    /// `{"ok": value}` or `{"error": "<ApiError>"}`.
    pub answer: Value,
    pub status: u16,
    /// What in the response breaks the document.
    pub problems: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn petstore() -> WireMap {
        WireMap::parse(include_str!("../../../contracts/petstore/lawtwin-openapi.json")).unwrap()
    }

    /// A literal segment wins over a parameter, and an unknown method lists the allowed ones.
    #[test]
    fn routes() {
        let map = petstore();
        assert_eq!(map.route("GET", "/pet/findByStatus").unwrap().0.operation, "findPetsByStatus");
        assert_eq!(map.route("GET", "/pet/7").unwrap().0.operation, "getPetById");
        match map.route("PATCH", "/pet/7") {
            Err(Refusal::NoRoute { allowed }) => assert!(allowed.contains(&"GET".to_string())),
            other => panic!("{other:?}"),
        }
    }

    /// A built request reads back as the same call.
    #[test]
    fn builds_and_reads_requests() {
        let map = petstore();
        let op = map.op("deletePet").unwrap();
        let args: Map<String, Value> = serde_json::from_value(json!({ "petId": 9, "apiKey": "k" })).unwrap();
        let out = map.build_request(op, &args);
        assert_eq!(out.target, "/pet/9");
        let call = map
            .read_request(&Incoming { method: "DELETE", path: "/pet/9", query: "", headers: &out.headers, body: &[] })
            .unwrap();
        assert_eq!(call.args, vec![("petId".to_string(), json!(9)), ("apiKey".to_string(), json!("k"))]);
        let bad = map.read_request(&Incoming { method: "GET", path: "/pet/x", query: "", headers: &[], body: &[] });
        assert!(matches!(bad, Err(Refusal::Invalid { status: 400, .. })));
    }
}
