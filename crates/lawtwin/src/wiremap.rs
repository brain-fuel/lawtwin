//! `lawtwin-openapi.json`: how the package's names map back to the wire,
//! for the HTTP twin and for conformance runs. LawSpec never reads it.

use crate::api::{Api, KeySource};
use crate::ir::{Def, Types};
use serde_json::{Value, json};

/// The wire map of the package.
pub fn wire_map(
    package: &str,
    source: &str,
    title: &str,
    version: &str,
    types: &Types,
    api: &Api,
    unmodelled: &[String],
) -> Value {
    let ops: Vec<Value> = api
        .ops
        .iter()
        .map(|op| {
            json!({
                "operation": op.name,
                "operationId": op.operation_id,
                "ability": op.ability,
                "method": op.method.to_uppercase(),
                "path": op.path,
                "parameters": op.params.iter().map(|p| json!({
                    "name": p.name, "wire": p.wire, "in": p.location, "required": p.required, "type": p.ty.lawspec(),
                })).collect::<Vec<_>>(),
                "requestContent": op.body_content,
                "response": { "status": op.response_status, "content": op.response_content, "type": op.response.lawspec() },
                "failures": op.errors.iter().map(|(s, c)| json!({ "status": s, "constructor": c })).collect::<Vec<_>>(),
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
                "fields": fields.iter().map(|f| json!({ "name": f.name, "wire": f.wire, "type": f.ty.lawspec() })).collect::<Vec<_>>(),
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
    let resources: Vec<Value> = api
        .resources
        .iter()
        .map(|r| {
            let role = |i: Option<usize>| i.map(|i| api.ops[i].name.clone());
            json!({
                "record": r.record,
                "collection": r.collection_path,
                "item": r.item_path,
                "key": r.key_param,
                "keyField": r.key_field.as_ref().map(|(f, _)| f),
                "stateField": r.map_field,
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
                "update": role(r.patch),
            })
        })
        .collect();
    json!({
        "package": package,
        "source": { "file": source, "title": title, "version": version },
        "failures": api.errors.iter().map(|(s, c)| json!({ "status": s, "constructor": c })).collect::<Vec<_>>(),
        "operations": ops,
        "types": types_json,
        "resources": resources,
    })
}
