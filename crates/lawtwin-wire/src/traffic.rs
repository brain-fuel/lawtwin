//! Seeded values of the contract's types, valid and invalid, for peers
//! that send traffic and for conformance runs. The same seed draws the same
//! values in the same order.

use crate::map::{Operation, TypeDef, WireMap};
use crate::rng::Rng;
use crate::ty::Ty;
use serde_json::{Map, Value, json};

const WORDS: &[&str] = &[
    "amber", "birch", "cedar", "delta", "ember", "fjord", "garnet", "harbor", "iris", "juniper", "kestrel", "lumen",
    "maple", "nectar", "onyx", "pebble", "quartz", "raven", "sable", "tundra", "umber", "violet", "willow", "zephyr",
];

/// A generator of values for one contract.
pub struct Gen<'m> {
    pub map: &'m WireMap,
    pub rng: Rng,
}

/// A request that breaks the document on purpose, and how.
pub struct Invalid {
    pub what: String,
    /// The arguments, by name, with the broken one changed.
    pub args: Map<String, Value>,
    /// A raw body to send instead of the arguments' one.
    pub raw_body: Option<Vec<u8>>,
}

fn regex_origin(constraints: &[Value]) -> Option<&str> {
    constraints.iter().find(|c| c["check"] == "regex").and_then(|c| c["origin"].as_str())
}

fn bound(constraints: &[Value], check: &str) -> Option<f64> {
    constraints.iter().find(|c| c["check"] == check).and_then(|c| {
        c["bound"].as_f64().or_else(|| c["bound"].as_str().and_then(|s| s.parse().ok()))
    })
}

impl<'m> Gen<'m> {
    pub fn new(map: &'m WireMap, seed: u64) -> Gen<'m> {
        Gen { map, rng: Rng::new(seed) }
    }

    /// A word, from a small list, with a number.
    pub fn word(&mut self) -> String {
        let w = self.rng.pick(WORDS).copied().unwrap_or("x");
        format!("{w}-{}", self.rng.below(10_000))
    }

    /// A text that keeps the constraints a format or pattern puts on it.
    fn text(&mut self, constraints: &[Value]) -> String {
        let n = self.rng.below(28) + 1;
        let mut s = match regex_origin(constraints) {
            Some("format date-time") => format!("2026-{:02}-{:02}T{:02}:{:02}:{:02}Z", n % 12 + 1, n, n % 24, n * 2 % 60, n * 3 % 60),
            Some("format date") => format!("2026-{:02}-{:02}", n % 12 + 1, n),
            Some("format time") => format!("{:02}:{:02}:00", n % 24, n * 2 % 60),
            Some("format uuid") => format!("{:08x}-0000-4000-8000-{:012x}", self.rng.next_u64() as u32, self.rng.next_u64() >> 16),
            Some("format email") => format!("{}@example.com", self.word()),
            Some("format uri") | Some("format url") => format!("https://example.com/{}", self.word()),
            Some("format ipv4") => format!("10.0.{}.{}", n, self.rng.below(255)),
            Some("format hostname") => format!("{}.example.com", self.word()),
            Some("format byte") => "bGF3dHdpbg==".to_string(),
            _ => self.word(),
        };
        if let Some(max) = bound(constraints, "maxLength") {
            s.truncate(max as usize);
        }
        if let Some(min) = bound(constraints, "minLength") {
            while s.chars().count() < min as usize {
                s.push('x');
            }
        }
        s
    }

    fn integer(&mut self, constraints: &[Value], wide: bool) -> i64 {
        if let Some(values) = constraints.iter().find(|c| c["check"] == "oneOf").and_then(|c| c["values"].as_array()) {
            if let Some(v) = self.rng.pick(values).and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or(v.as_i64())) {
                return v;
            }
        }
        let lo = bound(constraints, "minimum").map(|b| b.ceil() as i64).unwrap_or(1);
        let hi = bound(constraints, "maximum").map(|b| b.floor() as i64).unwrap_or(if wide { 1_000_000_000 } else { 1000 });
        let ex_lo = constraints.iter().any(|c| c["check"] == "minimum" && c["exclusive"] == true) as i64;
        let ex_hi = constraints.iter().any(|c| c["check"] == "maximum" && c["exclusive"] == true) as i64;
        self.rng.between(lo + ex_lo, hi - ex_hi)
    }

    /// A value of the type, as JSON on the wire.
    pub fn value(&mut self, ty: &Ty, constraints: &[Value], depth: usize) -> Value {
        match ty {
            Ty::Maybe(inner) => {
                if depth > 3 || self.rng.chance(35) { Value::Null } else { self.value(inner, constraints, depth) }
            }
            Ty::Text => json!(self.text(constraints)),
            Ty::Int64 => json!(self.integer(constraints, true)),
            Ty::Int32 => json!(self.integer(constraints, false)),
            Ty::Float64 => json!(self.rng.below(100_000) as f64 / 100.0),
            Ty::Bool => json!(self.rng.chance(50)),
            Ty::Bytes => json!("bGF3dHdpbg=="),
            Ty::Unit => Value::Null,
            Ty::List(item) => {
                let lo = bound(constraints, "minItems").unwrap_or(0.0) as u64;
                let hi = bound(constraints, "maxItems").map(|b| b as u64).unwrap_or(lo + 2).max(lo);
                let n = if depth > 3 { lo } else { lo + self.rng.below(hi - lo + 1) };
                Value::Array((0..n).map(|_| self.value(item, &[], depth + 1)).collect())
            }
            Ty::KeyVal(_, v) => {
                let mut m = Map::new();
                for _ in 0..self.rng.below(3) {
                    let k = self.word();
                    m.insert(k, self.value(v, &[], depth + 1));
                }
                Value::Object(m)
            }
            Ty::Named(name) => self.named(name, depth),
        }
    }

    fn named(&mut self, name: &str, depth: usize) -> Value {
        match self.map.type_def(name).cloned() {
            Some(TypeDef::Record { fields, .. }) => {
                let mut m = Map::new();
                for f in &fields {
                    let ty = WireMap::ty(&f.ty);
                    let v = self.value(&ty, &f.constraints, depth + 1);
                    if v.is_null() && !f.required {
                        continue;
                    }
                    m.insert(f.wire.clone(), v);
                }
                Value::Object(m)
            }
            Some(TypeDef::Enum { values, .. }) => {
                json!(self.rng.pick(&values).map(|v| v.wire.clone()).unwrap_or_default())
            }
            Some(TypeDef::OneOf { alternatives, .. }) => match self.rng.pick(&alternatives).cloned() {
                Some(a) => self.value(&WireMap::ty(&a.ty), &[], depth + 1),
                None => Value::Null,
            },
            None => Value::Null,
        }
    }

    /// Arguments for an operation, by name: every required one, and some
    /// optional ones.
    pub fn args(&mut self, op: &Operation) -> Map<String, Value> {
        let mut m = Map::new();
        for p in &op.parameters {
            let ty = WireMap::ty(&p.ty);
            let v = if p.location == "body" && ty.is_maybe() {
                self.value(ty.unmaybe(), &p.constraints, 0)
            } else if p.location == "header" && !p.required {
                Value::Null
            } else {
                self.value(&ty, &p.constraints, 0)
            };
            m.insert(p.name.clone(), v);
        }
        m
    }

    /// Requests that break the document, built from valid arguments: a body
    /// that is not JSON, a required field left out, a field of the wrong
    /// type, a value that breaks a constraint, and a path parameter of the
    /// wrong type.
    pub fn invalid(&mut self, op: &Operation, valid: &Map<String, Value>) -> Vec<Invalid> {
        let mut out = vec![];
        if let Some(p) = op.parameters.iter().find(|p| p.location == "path" && matches!(WireMap::ty(&p.ty), Ty::Int64 | Ty::Int32)) {
            let mut args = valid.clone();
            args.insert(p.name.clone(), json!("not-a-number"));
            out.push(Invalid { what: format!("the path parameter {} is not an integer", p.wire), args, raw_body: None });
        }
        let Some(body) = op.body() else { return out };
        if !op.request_content.as_deref().unwrap_or("application/json").contains("json") {
            return out;
        }
        out.push(Invalid { what: "the body is not JSON".into(), args: valid.clone(), raw_body: Some(b"{not json".to_vec()) });
        let Ty::Named(name) = WireMap::ty(&body.ty).unmaybe().clone() else { return out };
        let Some(TypeDef::Record { fields, .. }) = self.map.type_def(&name).cloned() else { return out };
        let sent = valid.get(&body.name).cloned().unwrap_or(json!({}));
        if let Some(f) = fields.iter().find(|f| f.required && !WireMap::ty(&f.ty).is_maybe()) {
            let mut b = sent.clone();
            if let Some(o) = b.as_object_mut() {
                o.remove(&f.wire);
            }
            let mut args = valid.clone();
            args.insert(body.name.clone(), b);
            out.push(Invalid { what: format!("the required field {} is left out", f.wire), args, raw_body: None });
        }
        if let Some(f) = fields.iter().find(|f| !matches!(WireMap::ty(&f.ty).unmaybe(), Ty::Text | Ty::Unit)) {
            let mut b = sent.clone();
            b[&f.wire] = json!("wrong-type");
            if WireMap::ty(&f.ty).unmaybe() == &Ty::Text {
                b[&f.wire] = json!(17);
            }
            let mut args = valid.clone();
            args.insert(body.name.clone(), b);
            out.push(Invalid { what: format!("the field {} has the wrong type", f.wire), args, raw_body: None });
        }
        if let Some(f) = fields.iter().find(|f| regex_origin(&f.constraints).is_some()) {
            let mut b = sent.clone();
            b[&f.wire] = json!("not valid: no format or pattern matches this");
            let mut args = valid.clone();
            args.insert(body.name.clone(), b);
            out.push(Invalid { what: format!("the field {} breaks its {}", f.wire, regex_origin(&f.constraints).unwrap_or("pattern")), args, raw_body: None });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same seed draws the same arguments, and they read as a call.
    #[test]
    fn seeded_and_valid() {
        let map = WireMap::parse(include_str!("../../../contracts/github/lawtwin-openapi.json")).unwrap();
        let op = map.op("issuesCreateMilestone").unwrap();
        let a = Gen::new(&map, 7).args(op);
        assert_eq!(a, Gen::new(&map, 7).args(op));
        let out = map.build_request(op, &a);
        let (path, query) = out.target.split_once('?').unwrap_or((&out.target, ""));
        let call = map
            .read_request(&crate::call::Incoming { method: &out.method, path, query, headers: &out.headers, body: &out.body })
            .unwrap();
        assert_eq!(call.op, "issuesCreateMilestone");
        assert!(!Gen::new(&map, 7).invalid(op, &a).is_empty());
    }
}
