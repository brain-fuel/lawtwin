//! Values of a contract's types, and their JSON on the wire.
//!
//! [`V`] is a value in the contract's terms: a record is its constructor
//! with its fields in order, an enum its constructor, `Maybe` present or
//! not. A core turns it into its generated model's values and back; the
//! wire map says how it is written in JSON.

use crate::map::{TypeDef, WireMap};
use crate::ty::Ty;
use base64::Engine as _;
use serde_json::{Map, Value, json};

/// A value of one of the contract's types.
#[derive(Clone, Debug, PartialEq)]
pub enum V {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
    Bytes(Vec<u8>),
    Unit,
    Maybe(Option<Box<V>>),
    List(Vec<V>),
    /// A `KeyVal`, in key order.
    Map(Vec<(V, V)>),
    /// A constructor of a named type, with its fields in order.
    Data { ty: String, ctor: String, fields: Vec<V> },
}

impl V {
    /// The constructor's name, for a value of a named type.
    pub fn ctor(&self) -> Option<&str> {
        match self {
            V::Data { ctor, .. } => Some(ctor),
            _ => None,
        }
    }
}

/// Why a JSON value is not a value of the type: where, and what.
#[derive(Clone, Debug, PartialEq)]
pub struct Mismatch {
    pub at: String,
    pub reason: String,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.at.is_empty() { write!(f, "{}", self.reason) } else { write!(f, "{}: {}", self.at, self.reason) }
    }
}

fn mismatch(at: &str, reason: impl Into<String>) -> Mismatch {
    Mismatch { at: at.to_string(), reason: reason.into() }
}

fn join(at: &str, part: &str) -> String {
    if at.is_empty() { part.to_string() } else { format!("{at}.{part}") }
}

impl WireMap {
    /// Reads a JSON value as a value of the type, or says where it is not one.
    pub fn decode(&self, ty: &Ty, json: &Value, at: &str) -> Result<V, Mismatch> {
        match ty {
            Ty::Maybe(inner) => match json {
                Value::Null => Ok(V::Maybe(None)),
                other => Ok(V::Maybe(Some(Box::new(self.decode(inner, other, at)?)))),
            },
            Ty::Text => json.as_str().map(|s| V::Text(s.to_string())).ok_or_else(|| mismatch(at, "expected a string")),
            Ty::Int64 => match json {
                Value::Number(n) => n.as_i64().map(V::Int).ok_or_else(|| mismatch(at, "expected a whole number that fits in 64 bits")),
                _ => Err(mismatch(at, "expected an integer")),
            },
            Ty::Int32 => match json {
                Value::Number(n) => n
                    .as_i64()
                    .filter(|i| i32::try_from(*i).is_ok())
                    .map(V::Int)
                    .ok_or_else(|| mismatch(at, "expected a whole number that fits in 32 bits")),
                _ => Err(mismatch(at, "expected an integer")),
            },
            Ty::Float64 => json.as_f64().map(V::Float).ok_or_else(|| mismatch(at, "expected a number")),
            Ty::Bool => json.as_bool().map(V::Bool).ok_or_else(|| mismatch(at, "expected true or false")),
            Ty::Bytes => match json {
                Value::String(s) => Ok(V::Bytes(
                    base64::engine::general_purpose::STANDARD.decode(s).unwrap_or_else(|_| s.as_bytes().to_vec()),
                )),
                _ => Err(mismatch(at, "expected a string of bytes")),
            },
            Ty::Unit => Ok(V::Unit),
            Ty::List(item) => match json {
                Value::Array(items) => items
                    .iter()
                    .enumerate()
                    .map(|(i, v)| self.decode(item, v, &format!("{at}[{i}]")))
                    .collect::<Result<_, _>>()
                    .map(V::List),
                _ => Err(mismatch(at, "expected an array")),
            },
            Ty::KeyVal(k, v) => match json {
                Value::Object(map) => {
                    let mut out = vec![];
                    for (key, value) in map {
                        let kv = self.decode_text(k, key).map_err(|r| mismatch(&join(at, key), r))?;
                        out.push((kv, self.decode(v, value, &join(at, key))?));
                    }
                    out.sort_by(|a, b| order(&a.0, &b.0));
                    Ok(V::Map(out))
                }
                _ => Err(mismatch(at, "expected an object")),
            },
            Ty::Named(name) => self.decode_named(name, json, at),
        }
    }

    fn decode_named(&self, name: &str, json: &Value, at: &str) -> Result<V, Mismatch> {
        if let Some(key) = self.key_record(name) {
            let obj = json.as_object().ok_or_else(|| mismatch(at, "expected an object"))?;
            let mut fields = vec![];
            for part in &key.fields {
                let v = obj.get(&part.name).ok_or_else(|| mismatch(&join(at, &part.name), "is required"))?;
                fields.push(self.decode(&WireMap::ty(&part.ty), v, &join(at, &part.name))?);
            }
            return Ok(V::Data { ty: name.into(), ctor: name.into(), fields });
        }
        match self.type_def(name) {
            Some(TypeDef::Record { fields: defs, .. }) => {
                let obj = json.as_object().ok_or_else(|| mismatch(at, format!("expected an object ({name})")))?;
                let mut fields = vec![];
                for f in defs {
                    let ty = WireMap::ty(&f.ty);
                    let here = join(at, &f.wire);
                    match obj.get(&f.wire) {
                        None if ty.is_maybe() && !f.required => fields.push(V::Maybe(None)),
                        None => return Err(mismatch(&here, "is required")),
                        Some(Value::Null) if !ty.is_maybe() => return Err(mismatch(&here, "must not be null")),
                        Some(v) => fields.push(self.decode(&ty, v, &here)?),
                    }
                }
                Ok(V::Data { ty: name.into(), ctor: name.into(), fields })
            }
            Some(TypeDef::Enum { values, .. }) => {
                let s = json.as_str().ok_or_else(|| mismatch(at, format!("expected one of {}", wires(values))))?;
                values
                    .iter()
                    .find(|v| v.wire == s)
                    .map(|v| V::Data { ty: name.into(), ctor: v.constructor.clone(), fields: vec![] })
                    .ok_or_else(|| mismatch(at, format!("{s:?} is not one of {}", wires(values))))
            }
            Some(TypeDef::OneOf { alternatives, .. }) => {
                for alt in alternatives {
                    if let Ok(v) = self.decode(&WireMap::ty(&alt.ty), json, at) {
                        return Ok(V::Data { ty: name.into(), ctor: alt.constructor.clone(), fields: vec![v] });
                    }
                }
                Err(mismatch(at, format!("matches no alternative of {name}")))
            }
            None => Err(mismatch(at, format!("the wire map has no type {name}"))),
        }
    }

    /// Reads a parameter's text (a path segment, a query value or a
    /// header) as a value of a scalar or enum type.
    pub fn decode_text(&self, ty: &Ty, text: &str) -> Result<V, String> {
        match ty {
            Ty::Maybe(inner) => Ok(V::Maybe(Some(Box::new(self.decode_text(inner, text)?)))),
            Ty::Text => Ok(V::Text(text.to_string())),
            Ty::Int64 => text.parse().map(V::Int).map_err(|_| format!("{text:?} is not an integer")),
            Ty::Int32 => text.parse::<i32>().map(|i| V::Int(i as i64)).map_err(|_| format!("{text:?} is not a 32-bit integer")),
            Ty::Float64 => text.parse().map(V::Float).map_err(|_| format!("{text:?} is not a number")),
            Ty::Bool => match text {
                "true" => Ok(V::Bool(true)),
                "false" => Ok(V::Bool(false)),
                _ => Err(format!("{text:?} is not true or false")),
            },
            Ty::Bytes => Ok(V::Bytes(text.as_bytes().to_vec())),
            Ty::Unit => Ok(V::Unit),
            Ty::List(item) => text.split(',').map(|t| self.decode_text(item, t)).collect::<Result<_, _>>().map(V::List),
            Ty::KeyVal(..) => Err("a map cannot be written as text".into()),
            Ty::Named(_) => self.decode(ty, &Value::String(text.into()), "").map_err(|m| m.reason),
        }
    }

    /// Writes a value of the type as JSON, as the document describes it. An
    /// absent optional property is left out; a nullable one is `null`.
    pub fn encode(&self, ty: &Ty, v: &V) -> Value {
        match (ty, v) {
            (Ty::Maybe(inner), V::Maybe(Some(x))) => self.encode(inner, x),
            (Ty::Maybe(_), V::Maybe(None)) => Value::Null,
            (_, V::Int(i)) => json!(i),
            (_, V::Float(f)) => json!(f),
            (_, V::Bool(b)) => json!(b),
            (_, V::Text(s)) => json!(s),
            (_, V::Bytes(b)) => json!(base64::engine::general_purpose::STANDARD.encode(b)),
            (_, V::Unit) => Value::Null,
            (Ty::List(item), V::List(xs)) => Value::Array(xs.iter().map(|x| self.encode(item, x)).collect()),
            (Ty::KeyVal(_, val), V::Map(entries)) => {
                let mut m = Map::new();
                for (k, x) in entries {
                    m.insert(text_of(k), self.encode(val, x));
                }
                Value::Object(m)
            }
            (Ty::Named(name), V::Data { ctor, fields, .. }) => self.encode_named(name, ctor, fields),
            (_, other) => self.encode_untyped(other),
        }
    }

    fn encode_named(&self, name: &str, ctor: &str, fields: &[V]) -> Value {
        if let Some(key) = self.key_record(name) {
            let mut m = Map::new();
            for (part, v) in key.fields.iter().zip(fields) {
                m.insert(part.name.clone(), self.encode(&WireMap::ty(&part.ty), v));
            }
            return Value::Object(m);
        }
        match self.type_def(name) {
            Some(TypeDef::Record { fields: defs, .. }) => {
                let mut m = Map::new();
                for (f, v) in defs.iter().zip(fields) {
                    if matches!(v, V::Maybe(None)) && !f.required {
                        continue;
                    }
                    m.insert(f.wire.clone(), self.encode(&WireMap::ty(&f.ty), v));
                }
                Value::Object(m)
            }
            Some(TypeDef::Enum { values, .. }) => {
                values.iter().find(|v| v.constructor == ctor).map(|v| json!(v.wire)).unwrap_or(json!(ctor))
            }
            Some(TypeDef::OneOf { alternatives, .. }) => {
                match (alternatives.iter().find(|a| a.constructor == ctor), fields.first()) {
                    (Some(a), Some(v)) => self.encode(&WireMap::ty(&a.ty), v),
                    _ => Value::Null,
                }
            }
            None => self.encode_untyped(&V::Data { ty: name.into(), ctor: ctor.into(), fields: fields.to_vec() }),
        }
    }

    /// A value written without its type, for diagnostics.
    pub fn encode_untyped(&self, v: &V) -> Value {
        match v {
            V::Int(i) => json!(i),
            V::Float(f) => json!(f),
            V::Bool(b) => json!(b),
            V::Text(s) => json!(s),
            V::Bytes(b) => json!(base64::engine::general_purpose::STANDARD.encode(b)),
            V::Unit => Value::Null,
            V::Maybe(None) => Value::Null,
            V::Maybe(Some(x)) => self.encode_untyped(x),
            V::List(xs) => Value::Array(xs.iter().map(|x| self.encode_untyped(x)).collect()),
            V::Map(entries) => Value::Array(
                entries.iter().map(|(k, x)| json!({ "key": self.encode_untyped(k), "value": self.encode_untyped(x) })).collect(),
            ),
            V::Data { ty, ctor, fields } => self.encode_named(ty, ctor, fields),
        }
    }
}

fn wires(values: &[crate::map::EnumValue]) -> String {
    values.iter().map(|v| format!("{:?}", v.wire)).collect::<Vec<_>>().join(", ")
}

/// A key written as text, for a JSON object's property names.
pub fn text_of(v: &V) -> String {
    match v {
        V::Text(s) => s.clone(),
        V::Int(i) => i.to_string(),
        V::Float(f) => f.to_string(),
        V::Bool(b) => b.to_string(),
        V::Maybe(Some(x)) => text_of(x),
        V::Data { ctor, fields, .. } if fields.is_empty() => ctor.clone(),
        V::Data { fields, .. } => fields.iter().map(text_of).collect::<Vec<_>>().join("/"),
        other => format!("{other:?}"),
    }
}

/// The order LawSpec keeps keys in: integers by value, text by its bytes,
/// records field by field.
pub fn order(a: &V, b: &V) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    match (a, b) {
        (V::Int(x), V::Int(y)) => x.cmp(y),
        (V::Float(x), V::Float(y)) => x.partial_cmp(y).unwrap_or(Equal),
        (V::Bool(x), V::Bool(y)) => x.cmp(y),
        (V::Text(x), V::Text(y)) => x.cmp(y),
        (V::Bytes(x), V::Bytes(y)) => x.cmp(y),
        (V::Maybe(None), V::Maybe(None)) => Equal,
        (V::Maybe(None), V::Maybe(Some(_))) => Less,
        (V::Maybe(Some(_)), V::Maybe(None)) => Greater,
        (V::Maybe(Some(x)), V::Maybe(Some(y))) => order(x, y),
        (V::List(x), V::List(y)) => {
            for (p, q) in x.iter().zip(y) {
                let o = order(p, q);
                if o != Equal {
                    return o;
                }
            }
            x.len().cmp(&y.len())
        }
        (V::Data { fields: x, ctor: c, .. }, V::Data { fields: y, ctor: d, .. }) => c.cmp(d).then_with(|| {
            for (p, q) in x.iter().zip(y) {
                let o = order(p, q);
                if o != Equal {
                    return o;
                }
            }
            Equal
        }),
        _ => Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn petstore() -> WireMap {
        WireMap::parse(include_str!("../../../contracts/petstore/lawtwin-openapi.json")).unwrap()
    }

    /// A pet reads from JSON and writes back as it was sent.
    #[test]
    fn round_trips_a_record() {
        let map = petstore();
        let pet = json!({ "id": 7, "name": "Rex", "photoUrls": ["a"], "status": "sold", "tags": [{ "id": 1, "name": "t" }] });
        let v = map.decode(&Ty::parse("Pet").unwrap(), &pet, "").unwrap();
        assert_eq!(map.encode(&Ty::parse("Pet").unwrap(), &v), pet);
    }

    /// A missing required field, a wrong type and an unknown enum value are each refused with where they are.
    #[test]
    fn says_where_a_value_breaks_its_type() {
        let map = petstore();
        let pet = Ty::parse("Pet").unwrap();
        assert_eq!(map.decode(&pet, &json!({ "photoUrls": [] }), "body").unwrap_err().at, "body.name");
        assert_eq!(map.decode(&pet, &json!({ "name": "x", "photoUrls": [], "id": "7" }), "body").unwrap_err().at, "body.id");
        assert!(map.decode(&pet, &json!({ "name": "x", "photoUrls": [], "status": "lost" }), "").is_err());
    }
}
