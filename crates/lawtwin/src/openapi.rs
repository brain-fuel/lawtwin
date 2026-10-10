//! Reading an OpenAPI 3.0 or 3.1 document, in JSON or YAML, and following
//! its local references.

use serde_json::{Map, Value};
use std::path::Path;

pub struct Document {
    pub root: Value,
    /// 3.1 writes nullability in `type`; 3.0 in `nullable`.
    pub v31: bool,
    pub title: String,
    pub version: String,
}

impl Document {
    pub fn load(path: &Path) -> Result<Document, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Document::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Document, String> {
        let root: Value = if text.trim_start().starts_with('{') {
            serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?
        } else {
            serde_yaml::from_str(text).map_err(|e| format!("not YAML: {e}"))?
        };
        let version = root.get("openapi").and_then(Value::as_str).ok_or(
            "no `openapi` field: this reads OpenAPI 3.0 and 3.1 (Swagger 2.0 is not supported)",
        )?;
        if !(version.starts_with("3.0") || version.starts_with("3.1")) {
            return Err(format!(
                "OpenAPI {version} is not supported; 3.0 and 3.1 are"
            ));
        }
        let info = root.get("info");
        let title = info
            .and_then(|i| i.get("title"))
            .and_then(Value::as_str)
            .unwrap_or("API")
            .to_string();
        let api_version = info
            .and_then(|i| i.get("version"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Ok(Document {
            v31: version.starts_with("3.1"),
            root,
            title,
            version: api_version,
        })
    }

    /// The value a local reference names, such as `#/components/schemas/Pet`.
    pub fn pointer(&self, reference: &str) -> Result<&Value, String> {
        let local = reference.strip_prefix('#').ok_or_else(|| {
            format!("the reference {reference} is not local; only #/... references are followed")
        })?;
        self.root
            .pointer(local)
            .ok_or_else(|| format!("the reference {reference} names nothing"))
    }

    /// A value with its references followed (for parameters, bodies and
    /// responses; schemas follow theirs as they are converted).
    pub fn deref<'a>(&'a self, mut value: &'a Value) -> Result<&'a Value, String> {
        for _ in 0..32 {
            match value.get("$ref").and_then(Value::as_str) {
                Some(r) => value = self.pointer(r)?,
                None => return Ok(value),
            }
        }
        Err("a reference chain is longer than 32".into())
    }

    pub fn paths(&self) -> Vec<(&str, &Map<String, Value>)> {
        self.root
            .get("paths")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_object().map(|o| (k.as_str(), o)))
                    .collect()
            })
            .unwrap_or_default()
    }
}

pub const METHODS: &[&str] = &[
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// The media type to read a body or response from: JSON first, then any
/// `+json`, then form, text and binary, then whatever is first.
pub fn pick_content(content: &Map<String, Value>) -> Option<(&str, &Value)> {
    let order: [&dyn Fn(&str) -> bool; 5] = [
        &|m| m == "application/json",
        &|m| m.ends_with("+json") || m.ends_with("/json"),
        &|m| m == "application/x-www-form-urlencoded" || m == "multipart/form-data",
        &|m| m.starts_with("text/"),
        &|_| true,
    ];
    for test in order {
        if let Some((k, v)) = content.iter().find(|(k, _)| test(k)) {
            return Some((k.as_str(), v));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_yaml_and_json() {
        let y =
            Document::parse("openapi: 3.1.0\ninfo: {title: T, version: '1'}\npaths: {}\n").unwrap();
        assert!(y.v31);
        let j =
            Document::parse(r#"{"openapi":"3.0.3","info":{"title":"T","version":"2"},"paths":{}}"#)
                .unwrap();
        assert!(!j.v31);
        assert_eq!(j.version, "2");
        assert!(Document::parse(r#"{"swagger":"2.0"}"#).is_err());
    }
}
