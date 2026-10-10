//! Documented bodies that the contract does not type: the schemas of
//! failure responses, as the wire map carries them. A twin builds a body
//! that keeps the schema; a peer checks that a body keeps it.

use serde_json::{Map, Value, json};

/// What a failure body says: its status and why.
pub struct Problem<'a> {
    pub status: u16,
    pub reason: &'a str,
    pub message: &'a str,
}

fn types_of(schema: &Value) -> Vec<String> {
    match schema.get("type") {
        Some(Value::String(t)) => vec![t.clone()],
        Some(Value::Array(ts)) => ts.iter().filter_map(|t| t.as_str().map(String::from)).collect(),
        _ => {
            if schema.get("properties").is_some() {
                vec!["object".into()]
            } else {
                vec![]
            }
        }
    }
}

/// A value that keeps the schema: every required property, and the
/// properties a failure body usually carries (`message`, `status`, `code`,
/// `error`), filled from the problem.
pub fn instance(schema: &Value, problem: &Problem) -> Value {
    build(schema, problem, "", 0)
}

fn build(schema: &Value, p: &Problem, name: &str, depth: usize) -> Value {
    if depth > 6 {
        return Value::Null;
    }
    if let Some(alts) = schema.get("oneOf").or_else(|| schema.get("anyOf")).and_then(Value::as_array) {
        if let Some(first) = alts.first() {
            return build(first, p, name, depth + 1);
        }
    }
    if let Some(parts) = schema.get("allOf").and_then(Value::as_array) {
        let mut out = Map::new();
        for part in parts {
            if let Value::Object(m) = build(part, p, name, depth + 1) {
                out.extend(m);
            }
        }
        return Value::Object(out);
    }
    if let Some(Value::Array(values)) = schema.get("enum") {
        return values.first().cloned().unwrap_or(Value::Null);
    }
    if let Some(c) = schema.get("const") {
        return c.clone();
    }
    let types = types_of(schema);
    let ty = types.iter().find(|t| *t != "null").map(String::as_str).unwrap_or("object");
    match ty {
        "object" => {
            let mut out = Map::new();
            let required: Vec<&str> =
                schema.get("required").and_then(Value::as_array).map(|r| r.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                for (k, s) in props {
                    let usual = matches!(k.as_str(), "message" | "status" | "code" | "error" | "detail" | "title" | "type");
                    if required.contains(&k.as_str()) || usual {
                        out.insert(k.clone(), build(s, p, k, depth + 1));
                    }
                }
            } else if depth == 0 {
                out.insert("message".into(), json!(p.message));
            }
            Value::Object(out)
        }
        "array" => json!([]),
        "integer" | "number" => match name {
            "status" | "code" => json!(p.status),
            _ => json!(0),
        },
        "boolean" => json!(false),
        "string" => match name {
            "message" | "detail" | "error" => json!(p.message),
            "title" => json!(p.reason),
            "status" | "code" => json!(p.status.to_string()),
            "documentation_url" | "url" | "type" => json!("https://lawtwin.dev/http-twin"),
            _ => json!(""),
        },
        _ => Value::Null,
    }
}

/// Where a value breaks the schema, if anywhere. It checks types, required
/// properties, enums, items and alternatives: the shape of the body, not
/// every keyword JSON Schema has.
pub fn violations(schema: &Value, value: &Value, at: &str) -> Vec<String> {
    let mut out = vec![];
    check(schema, value, at, &mut out, 0);
    out
}

fn check(schema: &Value, value: &Value, at: &str, out: &mut Vec<String>, depth: usize) {
    if depth > 12 || !schema.is_object() {
        return;
    }
    let here = if at.is_empty() { "the body".to_string() } else { at.to_string() };
    if let Some(alts) = schema.get("oneOf").or_else(|| schema.get("anyOf")).and_then(Value::as_array) {
        if !alts.is_empty() && !alts.iter().any(|a| violations_at(a, value, at, depth).is_empty()) {
            out.push(format!("{here} matches none of its alternatives"));
        }
        return;
    }
    if let Some(parts) = schema.get("allOf").and_then(Value::as_array) {
        for part in parts {
            check(part, value, at, out, depth + 1);
        }
    }
    if value.is_null() && (schema.get("nullable") == Some(&json!(true)) || types_of(schema).iter().any(|t| t == "null")) {
        return;
    }
    if let Some(Value::Array(values)) = schema.get("enum") {
        if !values.contains(value) {
            out.push(format!("{here} is {value}, not one of its values"));
        }
    }
    let types = types_of(schema);
    if !types.is_empty() {
        let fits = types.iter().any(|t| match t.as_str() {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => true,
        });
        if !fits {
            out.push(format!("{here} is {}, not {}", kind(value), types.join(" or ")));
            return;
        }
    }
    if let Value::Object(obj) = value {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for r in required.iter().filter_map(Value::as_str) {
                if !obj.contains_key(r) {
                    out.push(format!("{} is required", join(at, r)));
                }
            }
        }
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for (k, s) in props {
                if let Some(v) = obj.get(k) {
                    check(s, v, &join(at, k), out, depth + 1);
                }
            }
        }
    }
    if let (Value::Array(items), Some(item)) = (value, schema.get("items")) {
        for (i, v) in items.iter().enumerate() {
            check(item, v, &format!("{here}[{i}]"), out, depth + 1);
        }
    }
}

fn violations_at(schema: &Value, value: &Value, at: &str, depth: usize) -> Vec<String> {
    let mut out = vec![];
    check(schema, value, at, &mut out, depth + 1);
    out
}

fn join(at: &str, k: &str) -> String {
    if at.is_empty() { k.to_string() } else { format!("{at}.{k}") }
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A built body keeps its schema, and a broken one is caught.
    #[test]
    fn builds_and_checks() {
        let schema = json!({ "type": "object", "required": ["message", "documentation_url"],
            "properties": { "message": { "type": "string" }, "documentation_url": { "type": "string" },
                            "errors": { "type": "array", "items": { "type": "object" } } } });
        let body = instance(&schema, &Problem { status: 422, reason: "Unprocessable Entity", message: "invalid" });
        assert_eq!(body["message"], "invalid");
        assert!(violations(&schema, &body, "").is_empty());
        assert_eq!(violations(&schema, &json!({ "message": 3 }), "").len(), 2);
    }
}
