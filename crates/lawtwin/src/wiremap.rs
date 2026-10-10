//! `lawtwin-openapi.json`: how the package's names map back to the wire,
//! for the HTTP twin, its simulated peers and conformance runs. LawSpec
//! never reads it.

use crate::api::{Api, KeySource, Resource};
use crate::ir::{Check, Def, Ty, Types};
use crate::openapi::{self, Document};
use serde_json::{Map, Value, json};

/// The constraints of a value, through `Maybe`, as the wire map writes them.
fn constraints(ty: &Ty) -> Vec<Value> {
    let checks = match ty {
        Ty::Maybe(inner) => return constraints(inner),
        Ty::Scalar(_, checks) | Ty::List(_, checks) => checks,
        _ => return vec![],
    };
    checks
        .iter()
        .map(|c| match c {
            Check::Min { bound, exclusive } => json!({ "check": "minimum", "bound": bound, "exclusive": exclusive }),
            Check::Max { bound, exclusive } => json!({ "check": "maximum", "bound": bound, "exclusive": exclusive }),
            Check::MinLength(n) => json!({ "check": "minLength", "bound": n }),
            Check::MaxLength(n) => json!({ "check": "maxLength", "bound": n }),
            Check::Regex { regex, origin } => json!({ "check": "regex", "regex": regex, "origin": origin }),
            Check::Finite => json!({ "check": "finite" }),
            Check::MinItems(n) => json!({ "check": "minItems", "bound": n }),
            Check::MaxItems(n) => json!({ "check": "maxItems", "bound": n }),
            Check::OneOf(values) => json!({ "check": "oneOf", "values": values }),
        })
        .collect()
}

/// A schema with its local references written out, so a reader needs no
/// document. A reference met again inside itself is left as `{}`.
fn inline(doc: &Document, value: &Value, seen: &mut Vec<String>) -> Value {
    match value {
        Value::Object(map) => {
            if let Some(r) = map.get("$ref").and_then(Value::as_str) {
                if seen.iter().any(|s| s == r) || seen.len() > 16 {
                    return json!({});
                }
                let Ok(target) = doc.pointer(r) else {
                    return json!({});
                };
                seen.push(r.to_string());
                let out = inline(doc, target, seen);
                seen.pop();
                return out;
            }
            let mut out = Map::new();
            for (k, v) in map {
                if matches!(k.as_str(), "description" | "examples" | "externalDocs" | "title") {
                    continue;
                }
                out.insert(k.clone(), inline(doc, v, seen));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(|v| inline(doc, v, seen)).collect()),
        other => other.clone(),
    }
}

/// The documented response of an operation for a status, references followed.
fn response<'d>(doc: &'d Document, path: &str, method: &str, status: &str) -> Option<&'d Value> {
    let r = doc.root.get("paths")?.get(path)?.get(method)?.get("responses")?.get(status)?;
    doc.deref(r).ok()
}

/// A response's body, as the wire map writes it: its media type and schema.
fn body_of(doc: &Document, response: &Value) -> Value {
    let Some(content) = response.get("content").and_then(Value::as_object) else {
        return Value::Null;
    };
    match openapi::pick_content(content) {
        Some((media, item)) => json!({
            "content": media,
            "schema": item.get("schema").map(|s| inline(doc, s, &mut vec![])).unwrap_or(json!({})),
        }),
        None => Value::Null,
    }
}

/// A response's headers: each one's name, whether it is required, its
/// schema and its example.
fn headers_of(doc: &Document, response: &Value) -> Vec<Value> {
    let Some(headers) = response.get("headers").and_then(Value::as_object) else {
        return vec![];
    };
    headers
        .iter()
        .filter_map(|(name, h)| {
            let h = doc.deref(h).ok()?;
            let schema = h.get("schema").map(|s| inline(doc, s, &mut vec![])).unwrap_or(json!({ "type": "string" }));
            let example = h
                .get("example")
                .cloned()
                .or_else(|| h.get("schema").and_then(|s| s.get("example")).cloned())
                .unwrap_or(Value::Null);
            Some(json!({
                "name": name,
                "required": h.get("required").and_then(Value::as_bool).unwrap_or(false),
                "schema": schema,
                "example": example,
            }))
        })
        .collect()
}

/// A resource's entry: its paths, key, state fields, operations and laws.
fn resource_json(r: &Resource, api: &Api, laws: &[(String, &'static str, String)]) -> Value {
    let role = |i: Option<usize>| i.map(|i| api.ops[i].name.clone());
    let mut law_titles = Map::new();
    for (record, kind, title) in laws {
        if *record == r.record {
            law_titles.insert(kind.to_string(), json!(title));
        }
    }
    let key_type = match &r.key_record {
        Some(kr) => json!({
            "type": kr,
            "fields": r.parents.iter().map(|(n, t)| json!({ "name": n, "type": t.lawspec() }))
                .chain(std::iter::once(json!({ "name": r.key_param, "type": r.key_ty.lawspec() })))
                .collect::<Vec<_>>(),
        }),
        None => json!({ "type": r.key_ty.lawspec() }),
    };
    json!({
        "record": r.record,
        "collection": r.collection_path,
        "item": r.item_path,
        "key": r.key_param,
        "keyField": r.key_field.as_ref().map(|(f, _)| f),
        "keyType": key_type,
        "parents": r.parents.iter().map(|(n, t)| json!({ "name": n, "type": t.lawspec() })).collect::<Vec<_>>(),
        "stateField": r.map_field,
        "counterField": r.counter_field,
        "keys": match r.create.as_ref().map(|c| &c.key) {
            Some(KeySource::Body { .. }) => "client",
            Some(KeySource::BodyOrAssigned { .. }) => "client or assigned",
            Some(KeySource::BodyIfGiven { .. }) => "client",
            Some(KeySource::Assigned) => "assigned",
            None => "none",
        },
        "read": api.ops[r.read].name,
        "create": r.create.as_ref().map(|c| api.ops[c.op].name.clone()),
        "delete": role(r.delete),
        "list": role(r.list),
        "replace": role(r.replace),
        "replaceByBodyKey": r.replace_by_body_key,
        "update": role(r.patch),
        "laws": law_titles,
    })
}

/// The wire map of the package.
#[allow(clippy::too_many_arguments)]
pub fn wire_map(
    doc: &Document,
    package: &str,
    source: &str,
    title: &str,
    version: &str,
    types: &Types,
    api: &Api,
    unmodelled: &[String],
    laws: &[(String, &'static str, String)],
) -> Value {
    let ops: Vec<Value> = api
        .ops
        .iter()
        .map(|op| {
            let success = response(doc, &op.path, &op.method, &op.response_status);
            json!({
                "operation": op.name,
                "operationId": op.operation_id,
                "ability": op.ability,
                "method": op.method.to_uppercase(),
                "path": op.path,
                "parameters": op.params.iter().map(|p| {
                    let mut v = json!({
                        "name": p.name, "wire": p.wire, "in": p.location, "required": p.required, "type": p.ty.lawspec(),
                    });
                    let c = constraints(&p.ty);
                    if !c.is_empty() {
                        v["constraints"] = Value::Array(c);
                    }
                    v
                }).collect::<Vec<_>>(),
                "requestContent": op.body_content,
                "response": {
                    "status": op.response_status,
                    "content": op.response_content,
                    "type": op.response.lawspec(),
                    "headers": success.map(|r| headers_of(doc, r)).unwrap_or_default(),
                },
                "failures": op.errors.iter().map(|(s, c)| json!({
                    "status": s,
                    "constructor": c,
                    "body": response(doc, &op.path, &op.method, s).map(|r| body_of(doc, r)).unwrap_or(Value::Null),
                })).collect::<Vec<_>>(),
                "modelled": !unmodelled.contains(&op.name),
            })
        })
        .collect();
    let types_json: Vec<Value> = types
        .defs
        .iter()
        .map(|d| match d {
            Def::Record { name, fields, wire, .. } => json!({
                "type": name, "schema": wire, "kind": "record",
                "fields": fields.iter().map(|f| {
                    let mut v = json!({ "name": f.name, "wire": f.wire, "type": f.ty.lawspec(), "required": f.required });
                    let c = constraints(&f.ty);
                    if !c.is_empty() {
                        v["constraints"] = Value::Array(c);
                    }
                    v
                }).collect::<Vec<_>>(),
            }),
            Def::Enum { name, ctors, wire, .. } => json!({
                "type": name, "schema": wire, "kind": "enum",
                "values": ctors.iter().map(|(c, w)| json!({ "constructor": c, "wire": w })).collect::<Vec<_>>(),
            }),
            Def::Union { name, alts, wire, .. } => json!({
                "type": name, "schema": wire, "kind": "oneOf",
                "alternatives": alts.iter().map(|(c, t)| json!({ "constructor": c, "type": t.lawspec() })).collect::<Vec<_>>(),
            }),
        })
        .collect();
    let resources: Vec<Value> = api.resources.iter().map(|r| resource_json(r, api, laws)).collect();
    // The model's State, field by field, in its constructor's order.
    let mut state = vec![];
    for r in &api.resources {
        let k = r.key_record.clone().unwrap_or_else(|| r.key_ty.lawspec());
        state.push(json!({ "field": r.map_field, "type": format!("KeyVal {k} {}", r.record) }));
        if let Some(c) = &r.counter_field {
            state.push(json!({ "field": c, "type": r.key_ty.lawspec() }));
        }
    }
    json!({
        "package": package,
        "source": { "file": source, "title": title, "version": version },
        "failures": api.errors.iter().map(|(s, c)| json!({ "status": s, "constructor": c })).collect::<Vec<_>>(),
        "operations": ops,
        "types": types_json,
        "resources": resources,
        "state": state,
    })
}
