//! The declared fault set, read from the contract's twin file
//! (`contracts/<name>/lawtwin.json`), and the faults active in a twin.
//!
//! A fault is raised by name with parameters; both are checked against the
//! declaration, so a test can only ask for what the twin file says, and the
//! file says what each fault does to each law. ref:DEC-twin-file

use crate::clock::parse_duration;
use lawtwin_wire::WireMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// A parameter a fault declares.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParamDecl {
    pub name: String,
    /// duration, int32, boolean, operation or status.
    #[serde(rename = "type")]
    pub ty: String,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    pub summary: String,
}

/// A fault the twin file declares.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FaultDecl {
    pub name: String,
    pub summary: String,
    pub params: Vec<ParamDecl>,
    pub changes: String,
    /// Its effect on each law it touches; every other law holds.
    pub laws: BTreeMap<String, String>,
}

/// The twin file's content.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TwinFile {
    pub faults: Vec<FaultDecl>,
    #[serde(default)]
    pub force: Value,
}

/// The faults the HTTP core implements; a twin file may declare no others.
pub const IMPLEMENTED: &[&str] = &["latency", "server-error", "rate-limit", "timeout", "connection-reset", "slow-body"];

impl TwinFile {
    /// Reads a twin file; an empty one declares no faults.
    pub fn parse(text: &str) -> Result<TwinFile, String> {
        if text.trim().is_empty() {
            return Ok(TwinFile { faults: vec![], force: json!({}) });
        }
        let file: TwinFile = serde_json::from_str(text).map_err(|e| format!("the twin file: {e}"))?;
        for f in &file.faults {
            if !IMPLEMENTED.contains(&f.name.as_str()) {
                return Err(format!("the twin file declares {}, which the HTTP core does not implement", f.name));
            }
        }
        Ok(file)
    }

    /// The declaration of a fault.
    pub fn decl(&self, name: &str) -> Option<&FaultDecl> {
        self.faults.iter().find(|f| f.name == name)
    }
}

/// A fault, as active in the twin.
#[derive(Clone, Debug)]
pub struct Active {
    pub id: u64,
    pub name: String,
    /// Every declared parameter that has a value, defaults included.
    pub params: Map<String, Value>,
    /// When it ends on the twin clock, if raised for a time.
    pub until_ms: Option<i64>,
    /// How many more requests it answers, if it was raised with a count.
    pub remaining: Option<i64>,
    /// For rate-limit: when the current window started, and how many requests it let through.
    pub window_start_ms: i64,
    pub window_used: i64,
}

impl Active {
    /// As the admin API shows it.
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("id".into(), json!(self.id));
        m.insert("name".into(), json!(self.name));
        for (k, v) in &self.params {
            m.insert(k.clone(), v.clone());
        }
        if let Some(u) = self.until_ms {
            m.insert("untilMs".into(), json!(u));
        }
        if let Some(r) = self.remaining {
            m.insert("remaining".into(), json!(r));
        }
        Value::Object(m)
    }

    pub fn int(&self, name: &str) -> Option<i64> {
        self.params.get(name).and_then(Value::as_i64)
    }

    pub fn ms(&self, name: &str) -> Option<i64> {
        self.params.get(name).and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| parse_duration(s).ok())))
    }

    pub fn flag(&self, name: &str) -> bool {
        self.params.get(name).and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn operation(&self) -> Option<&str> {
        self.params.get("operation").and_then(Value::as_str)
    }

    /// Whether it touches a request for this operation.
    pub fn matches(&self, op: &str) -> bool {
        self.operation().is_none_or(|o| o == op)
    }
}

/// Checks a raised fault's parameters against its declaration, and gives
/// them with their defaults; `for` and `count` are kept apart.
pub fn check(decl: &FaultDecl, map: &WireMap, given: &Map<String, Value>) -> Result<(Map<String, Value>, Option<i64>, Option<i64>), String> {
    let mut out = Map::new();
    let (mut for_ms, mut count) = (None, None);
    for (k, _) in given {
        if !decl.params.iter().any(|p| &p.name == k) {
            return Err(format!("{} takes no parameter {k}; it takes {}", decl.name, decl.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")));
        }
    }
    for p in &decl.params {
        let Some(v) = given.get(&p.name).cloned().or_else(|| p.default.clone()) else {
            if p.required {
                return Err(format!("{} needs {}", decl.name, p.name));
            }
            continue;
        };
        let v = typed(&p.ty, &v, map).map_err(|e| format!("{}: {} {e}", decl.name, p.name))?;
        match p.name.as_str() {
            "for" => for_ms = v.as_i64(),
            "count" => count = v.as_i64(),
            _ => {
                out.insert(p.name.clone(), v);
            }
        }
    }
    if decl.name == "server-error" {
        let s = out.get("status").and_then(Value::as_i64).unwrap_or(500);
        if !(500..600).contains(&s) {
            return Err(format!("server-error: status {s} is not 500 to 599"));
        }
    }
    Ok((out, for_ms, count))
}

fn typed(ty: &str, v: &Value, map: &WireMap) -> Result<Value, String> {
    match ty {
        "duration" => match v {
            Value::Number(n) => n.as_i64().filter(|n| *n >= 0).map(|n| json!(n)).ok_or("is not a duration".into()),
            Value::String(s) => parse_duration(s).map(|_| json!(s)),
            _ => Err("is not a duration such as 500ms or 5s".into()),
        },
        "int32" | "status" => {
            let n = v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).ok_or("is not a whole number")?;
            if i32::try_from(n).is_err() || n < 0 {
                return Err(format!("{n} is out of range"));
            }
            Ok(json!(n))
        }
        "boolean" => match v {
            Value::Bool(b) => Ok(json!(b)),
            Value::String(s) if s == "true" || s == "false" => Ok(json!(s == "true")),
            Value::Number(n) if n.as_i64() == Some(0) || n.as_i64() == Some(1) => Ok(json!(n.as_i64() == Some(1))),
            _ => Err("is not true or false".into()),
        },
        "operation" => {
            let s = v.as_str().ok_or("is not an operation name")?;
            if map.op(s).is_none() {
                return Err(format!("{s} is not an operation of {}", map.package));
            }
            Ok(json!(s))
        }
        _ => Ok(v.clone()),
    }
}
