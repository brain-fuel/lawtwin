//! OpenAPI schemas as LawSpec types.
//!
//! Objects become records, string enums sum types, `oneOf` and `anyOf`
//! sums with a constructor per alternative, arrays `List`, maps `KeyVal
//! Text`, and nullable or optional values `Maybe`. A schema's constraints
//! (bounds, lengths, patterns, formats, item counts) do not change its type:
//! they are kept as checks, and become the package's validation definitions
//! and named refinements, so a value that breaks them can still be sent and
//! refused, as a server would refuse it.

use crate::names::{self, Namespace};
use crate::openapi::Document;
use crate::regex;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scalar {
    Text,
    Int32,
    Int64,
    Float64,
    Bool,
    Bytes,
    Unit,
    /// A free-form JSON value, held as its JSON text.
    Json,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Check {
    Min {
        bound: String,
        exclusive: bool,
    },
    Max {
        bound: String,
        exclusive: bool,
    },
    MinLength(u64),
    MaxLength(u64),
    /// A portable regex, and where it came from (`pattern` or a format).
    Regex {
        regex: String,
        origin: String,
    },
    Finite,
    MinItems(u64),
    MaxItems(u64),
    OneOf(Vec<String>),
}

#[derive(Clone, Debug)]
pub enum Ty {
    Scalar(Scalar, Vec<Check>),
    List(Box<Ty>, Vec<Check>),
    Maybe(Box<Ty>),
    Map(Box<Ty>),
    Named(String),
}

impl Ty {
    pub fn text() -> Ty {
        Ty::Scalar(Scalar::Text, vec![])
    }

    pub fn unit() -> Ty {
        Ty::Scalar(Scalar::Unit, vec![])
    }

    pub fn maybe(self) -> Ty {
        match self {
            Ty::Maybe(_) => self,
            t => Ty::Maybe(Box::new(t)),
        }
    }

    /// The same LawSpec type, whatever the checks.
    pub fn same_shape(&self, other: &Ty) -> bool {
        match (self, other) {
            (Ty::Scalar(a, _), Ty::Scalar(b, _)) => a.lawspec() == b.lawspec(),
            (Ty::List(a, _), Ty::List(b, _))
            | (Ty::Maybe(a), Ty::Maybe(b))
            | (Ty::Map(a), Ty::Map(b)) => a.same_shape(b),
            (Ty::Named(a), Ty::Named(b)) => a == b,
            _ => false,
        }
    }

    pub fn lawspec(&self) -> String {
        match self {
            Ty::Scalar(s, _) => s.lawspec().to_string(),
            Ty::List(t, _) => format!("List {}", t.atom()),
            Ty::Maybe(t) => format!("Maybe {}", t.atom()),
            Ty::Map(t) => format!("KeyVal Text {}", t.atom()),
            Ty::Named(n) => n.clone(),
        }
    }

    /// The type with its named types qualified, as `model.Pet`.
    pub fn qualified(&self, q: &str) -> String {
        match self {
            Ty::Scalar(s, _) => s.lawspec().to_string(),
            Ty::List(t, _) => format!("List {}", t.qualified_atom(q)),
            Ty::Maybe(t) => format!("Maybe {}", t.qualified_atom(q)),
            Ty::Map(t) => format!("KeyVal Text {}", t.qualified_atom(q)),
            Ty::Named(n) => format!("{q}.{n}"),
        }
    }

    pub fn qualified_atom(&self, q: &str) -> String {
        match self {
            Ty::Scalar(..) | Ty::Named(_) => self.qualified(q),
            _ => format!("({})", self.qualified(q)),
        }
    }

    /// The type, parenthesised when it is an application.
    pub fn atom(&self) -> String {
        match self {
            Ty::Scalar(..) | Ty::Named(_) => self.lawspec(),
            _ => format!("({})", self.lawspec()),
        }
    }

    pub fn is_unit(&self) -> bool {
        matches!(self, Ty::Scalar(Scalar::Unit, _))
    }

    /// A key a KeyVal may use: anything but floats and free-form JSON.
    pub fn keyable(&self) -> bool {
        matches!(self, Ty::Scalar(s, _) if !matches!(s, Scalar::Float64 | Scalar::Unit))
    }
}

impl Scalar {
    pub fn lawspec(&self) -> &'static str {
        match self {
            Scalar::Text | Scalar::Json => "Text",
            Scalar::Int32 => "Int32",
            Scalar::Int64 => "Int64",
            Scalar::Float64 => "Float64",
            Scalar::Bool => "Bool",
            Scalar::Bytes => "Bytes",
            Scalar::Unit => "Unit",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Field {
    pub wire: String,
    pub name: String,
    pub ty: Ty,
    pub doc: String,
}

#[derive(Clone, Debug)]
pub enum Def {
    Record {
        name: String,
        fields: Vec<Field>,
        doc: String,
        wire: String,
    },
    Enum {
        name: String,
        ctors: Vec<(String, String)>,
        doc: String,
        wire: String,
    },
    Union {
        name: String,
        alts: Vec<(String, Ty)>,
        doc: String,
        wire: String,
    },
}

impl Def {
    pub fn name(&self) -> &str {
        match self {
            Def::Record { name, .. } | Def::Enum { name, .. } | Def::Union { name, .. } => name,
        }
    }
}

pub struct Types<'d> {
    pub doc: &'d Document,
    pub defs: Vec<Def>,
    index: BTreeMap<String, usize>,
    refs: BTreeMap<String, Ty>,
    in_progress: Vec<String>,
    pub upper: Namespace,
    pub notes: Vec<String>,
}

enum Kind {
    Record,
    Enum,
    Union,
    Alias,
}

impl<'d> Types<'d> {
    pub fn new(doc: &'d Document) -> Self {
        Types {
            doc,
            defs: vec![],
            index: BTreeMap::new(),
            refs: BTreeMap::new(),
            in_progress: vec![],
            upper: Namespace::with_reserved_types(),
            notes: vec![],
        }
    }

    pub fn def(&self, name: &str) -> Option<&Def> {
        self.index.get(name).map(|&i| &self.defs[i])
    }

    pub fn record_fields(&self, name: &str) -> Option<&[Field]> {
        match self.def(name) {
            Some(Def::Record { fields, .. }) => Some(fields),
            _ => None,
        }
    }

    fn push(&mut self, def: Def) {
        let name = def.name().to_string();
        self.index.insert(name, self.defs.len());
        self.defs.push(def);
    }

    /// The schema without its nullability, and whether it was nullable.
    fn strip_null(&self, schema: &Value) -> (Value, bool) {
        let mut s = schema.clone();
        let mut nullable = false;
        if let Some(o) = s.as_object_mut() {
            if o.get("nullable").and_then(Value::as_bool) == Some(true) {
                nullable = true;
                o.remove("nullable");
            }
            if let Some(Value::Array(types)) = o.get("type").cloned() {
                let rest: Vec<Value> = types
                    .iter()
                    .filter(|t| t.as_str() != Some("null"))
                    .cloned()
                    .collect();
                if rest.len() < types.len() {
                    nullable = true;
                }
                match rest.len() {
                    0 => {
                        o.insert("type".into(), Value::String("null".into()));
                    }
                    1 => {
                        o.insert("type".into(), rest[0].clone());
                    }
                    _ => {
                        o.insert("type".into(), Value::Array(rest));
                    }
                }
            }
            for key in ["oneOf", "anyOf"] {
                if let Some(Value::Array(alts)) = o.get(key).cloned() {
                    let is_null = |a: &Value| {
                        a.get("type").and_then(Value::as_str) == Some("null")
                            || a.get("const") == Some(&Value::Null)
                            || a.get("enum")
                                .and_then(Value::as_array)
                                .is_some_and(|e| e.len() == 1 && e[0].is_null())
                    };
                    let rest: Vec<Value> = alts.iter().filter(|a| !is_null(a)).cloned().collect();
                    if rest.len() < alts.len() {
                        nullable = true;
                        o.insert(key.into(), Value::Array(rest));
                    }
                }
            }
            if let Some(Value::Array(values)) = o.get("enum").cloned() {
                let rest: Vec<Value> = values.iter().filter(|v| !v.is_null()).cloned().collect();
                if rest.len() < values.len() {
                    nullable = true;
                    o.insert("enum".into(), Value::Array(rest));
                }
            }
        }
        (s, nullable)
    }

    fn kind(&self, schema: &Value) -> Kind {
        let o = match schema.as_object() {
            Some(o) => o,
            None => return Kind::Alias,
        };
        if o.contains_key("$ref") {
            return Kind::Alias;
        }
        if let Some(Value::Array(all)) = o.get("allOf") {
            if all.len() == 1 && !o.contains_key("properties") {
                return Kind::Alias;
            }
            return Kind::Record;
        }
        for key in ["oneOf", "anyOf"] {
            if let Some(Value::Array(alts)) = o.get(key) {
                if alts.len() == 1 {
                    return Kind::Alias;
                }
                return if alts.iter().all(|a| self.string_values(a).is_some()) {
                    Kind::Enum
                } else {
                    Kind::Union
                };
            }
        }
        if self.string_values(schema).is_some() {
            return Kind::Enum;
        }
        let ty = o.get("type").and_then(Value::as_str);
        let props = o
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|p| !p.is_empty());
        let closed = o.get("additionalProperties") == Some(&Value::Bool(false));
        if (ty == Some("object") || ty.is_none()) && (props || (closed && ty == Some("object"))) {
            return Kind::Record;
        }
        Kind::Alias
    }

    /// The values of a string enum or string const, after references.
    fn string_values(&self, schema: &Value) -> Option<Vec<String>> {
        let schema = self.doc.deref(schema).ok()?;
        if let Some(Value::Array(values)) = schema.get("enum") {
            let strings: Vec<String> = values
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if !strings.is_empty()
                && strings.len() == values.iter().filter(|v| !v.is_null()).count()
            {
                return Some(strings);
            }
        }
        schema
            .get("const")
            .and_then(Value::as_str)
            .map(|s| vec![s.to_string()])
    }

    /// The type of a schema met at `ctx`, which names it when it is an inline
    /// object, enum or union.
    pub fn ty(&mut self, schema: &Value, ctx: &str) -> Ty {
        let (inner, nullable) = self.strip_null(schema);
        let t = self.ty_nonnull(&inner, ctx);
        if nullable { t.maybe() } else { t }
    }

    fn ty_nonnull(&mut self, schema: &Value, ctx: &str) -> Ty {
        let o = match schema.as_object() {
            Some(o) => o.clone(),
            None => return Ty::Scalar(Scalar::Json, vec![]),
        };
        if let Some(r) = o.get("$ref").and_then(Value::as_str) {
            return self.ref_ty(r);
        }
        match self.kind(schema) {
            Kind::Record => {
                let name = self.upper.claim(ctx, "Object");
                return Ty::Named(self.record(schema, name, ctx));
            }
            Kind::Enum => {
                // An inline enum with the values of one already made is that enum.
                let values = self.enum_values(schema);
                if let Some(Def::Enum { name, .. }) = self.defs.iter().find(|d| matches!(d, Def::Enum { ctors, .. } if ctors.iter().map(|(_, w)| w).eq(values.iter()))) {
                    return Ty::Named(name.clone());
                }
                let name = self.upper.claim(ctx, "Object");
                return Ty::Named(self.enumeration(schema, name, ctx));
            }
            Kind::Union => {
                let name = self.upper.claim(ctx, "Object");
                return Ty::Named(self.union(schema, name, ctx));
            }
            Kind::Alias => {}
        }
        if let Some(Value::Array(all)) = o.get("allOf") {
            return self.ty(&all[0], ctx);
        }
        for key in ["oneOf", "anyOf"] {
            if let Some(Value::Array(alts)) = o.get(key) {
                if let Some(first) = alts.first() {
                    return self.ty(first, ctx);
                }
            }
        }
        let ty = match o.get("type") {
            Some(Value::String(t)) => Some(t.as_str()),
            Some(Value::Array(_)) => {
                self.notes.push(format!(
                    "{ctx}: a value of several types is kept as JSON text"
                ));
                return Ty::Scalar(Scalar::Json, vec![]);
            }
            _ => None,
        };
        match ty {
            Some("array") => {
                let item = match o.get("items") {
                    Some(items) => self.ty(items, &format!("{ctx}Item")),
                    None => Ty::Scalar(Scalar::Json, vec![]),
                };
                let mut checks = vec![];
                if let Some(n) = o.get("minItems").and_then(Value::as_u64) {
                    checks.push(Check::MinItems(n));
                }
                if let Some(n) = o.get("maxItems").and_then(Value::as_u64) {
                    checks.push(Check::MaxItems(n));
                }
                Ty::List(Box::new(item), checks)
            }
            Some("object") | None
                if o.contains_key("additionalProperties") || ty == Some("object") =>
            {
                match o.get("additionalProperties") {
                    Some(ap @ Value::Object(_)) => {
                        Ty::Map(Box::new(self.ty(ap, &format!("{ctx}Value"))))
                    }
                    _ => Ty::Scalar(Scalar::Json, vec![]),
                }
            }
            Some("string") => self.string(&o, ctx),
            Some("integer") => {
                let s = if o.get("format").and_then(Value::as_str) == Some("int32") {
                    Scalar::Int32
                } else {
                    Scalar::Int64
                };
                let mut checks = self.bounds(&o, false);
                if let Some(Value::Array(values)) = o.get("enum") {
                    checks.push(Check::OneOf(
                        values
                            .iter()
                            .filter_map(|v| v.as_i64().map(|n| n.to_string()))
                            .collect(),
                    ));
                }
                Ty::Scalar(s, checks)
            }
            Some("number") => {
                let mut checks = vec![Check::Finite];
                checks.extend(self.bounds(&o, true));
                Ty::Scalar(Scalar::Float64, checks)
            }
            Some("boolean") => Ty::Scalar(Scalar::Bool, vec![]),
            Some("null") => Ty::unit(),
            _ => Ty::Scalar(Scalar::Json, vec![]),
        }
    }

    fn string(&mut self, o: &Map<String, Value>, ctx: &str) -> Ty {
        let format = o.get("format").and_then(Value::as_str).unwrap_or("");
        if format == "binary" {
            return Ty::Scalar(Scalar::Bytes, vec![]);
        }
        let mut checks = vec![];
        if let Some(n) = o.get("minLength").and_then(Value::as_u64) {
            checks.push(Check::MinLength(n));
        }
        if let Some(n) = o.get("maxLength").and_then(Value::as_u64) {
            checks.push(Check::MaxLength(n));
        }
        if let Some(p) = o.get("pattern").and_then(Value::as_str) {
            match regex::portable(p) {
                Ok(r) => checks.push(Check::Regex {
                    regex: r,
                    origin: "pattern".into(),
                }),
                Err(why) => self
                    .notes
                    .push(format!("{ctx}: the pattern {p} is not checked: {why}")),
            }
        }
        if let Some(r) = regex::format_regex(format) {
            checks.push(Check::Regex {
                regex: r.into(),
                origin: format!("format {format}"),
            });
        }
        Ty::Scalar(Scalar::Text, checks)
    }

    fn bounds(&self, o: &Map<String, Value>, float: bool) -> Vec<Check> {
        let literal = |v: &Value| -> Option<String> {
            let n = v.as_f64()?;
            Some(if float {
                if n.fract() == 0.0 {
                    format!("{n:.1}")
                } else {
                    format!("{n}")
                }
            } else if n.fract() == 0.0 {
                format!("{}", n as i64)
            } else {
                return None;
            })
        };
        let mut out = vec![];
        let exclusive_flag = |k: &str| o.get(k).and_then(Value::as_bool).unwrap_or(false);
        if let Some(b) = o.get("minimum").and_then(literal) {
            out.push(Check::Min {
                bound: b,
                exclusive: exclusive_flag("exclusiveMinimum"),
            });
        }
        if let Some(b) = o
            .get("exclusiveMinimum")
            .filter(|v| v.is_number())
            .and_then(literal)
        {
            out.push(Check::Min {
                bound: b,
                exclusive: true,
            });
        }
        if let Some(b) = o.get("maximum").and_then(literal) {
            out.push(Check::Max {
                bound: b,
                exclusive: exclusive_flag("exclusiveMaximum"),
            });
        }
        if let Some(b) = o
            .get("exclusiveMaximum")
            .filter(|v| v.is_number())
            .and_then(literal)
        {
            out.push(Check::Max {
                bound: b,
                exclusive: true,
            });
        }
        out
    }

    fn ref_ty(&mut self, reference: &str) -> Ty {
        if let Some(t) = self.refs.get(reference) {
            return t.clone();
        }
        let target = match self.doc.pointer(reference) {
            Ok(v) => v.clone(),
            Err(e) => {
                self.notes.push(e);
                return Ty::Scalar(Scalar::Json, vec![]);
            }
        };
        let base = reference.rsplit('/').next().unwrap_or("Schema");
        let ctx = names::upper_camel(base);
        let (inner, nullable) = self.strip_null(&target);
        let named = |name: String| {
            if nullable {
                Ty::Named(name).maybe()
            } else {
                Ty::Named(name)
            }
        };
        match self.kind(&inner) {
            Kind::Record => {
                let name = self.upper.claim(&ctx, "Schema");
                self.refs.insert(reference.into(), named(name.clone()));
                self.record(&inner, name.clone(), base);
                named(name)
            }
            Kind::Enum => {
                let name = self.upper.claim(&ctx, "Schema");
                self.refs.insert(reference.into(), named(name.clone()));
                self.enumeration(&inner, name.clone(), base);
                named(name)
            }
            Kind::Union => {
                let name = self.upper.claim(&ctx, "Schema");
                self.refs.insert(reference.into(), named(name.clone()));
                self.union(&inner, name.clone(), base);
                named(name)
            }
            Kind::Alias => {
                if self.in_progress.iter().any(|r| r == reference) {
                    self.notes.push(format!(
                        "{reference} refers to itself through no object; it is kept as JSON text"
                    ));
                    return Ty::Scalar(Scalar::Json, vec![]);
                }
                self.in_progress.push(reference.into());
                let t = self.ty_nonnull(&inner, &ctx);
                self.in_progress.pop();
                let t = if nullable { t.maybe() } else { t };
                self.refs.insert(reference.into(), t.clone());
                t
            }
        }
    }

    /// Properties and required names of an object schema, through `allOf`.
    fn properties(
        &self,
        schema: &Value,
        props: &mut Vec<(String, Value)>,
        required: &mut Vec<String>,
        depth: usize,
    ) {
        let schema = match self.doc.deref(schema) {
            Ok(s) => s,
            Err(_) => return,
        };
        if depth > 16 {
            return;
        }
        if let Some(Value::Array(all)) = schema.get("allOf") {
            for part in all {
                self.properties(part, props, required, depth + 1);
            }
        }
        if let Some(p) = schema.get("properties").and_then(Value::as_object) {
            for (k, v) in p {
                match props.iter_mut().find(|(n, _)| n == k) {
                    Some(slot) => slot.1 = v.clone(),
                    None => props.push((k.clone(), v.clone())),
                }
            }
        }
        if let Some(Value::Array(r)) = schema.get("required") {
            required.extend(r.iter().filter_map(|v| v.as_str().map(String::from)));
        }
    }

    /// A record under a name already claimed.
    fn record(&mut self, schema: &Value, name: String, wire: &str) -> String {
        // Reserve the slot so recursive references see the name.
        self.push(Def::Record {
            name: name.clone(),
            fields: vec![],
            doc: String::new(),
            wire: wire.into(),
        });
        let mut props = vec![];
        let mut required = vec![];
        self.properties(schema, &mut props, &mut required, 0);
        let mut taken = Namespace::default();
        let mut fields = vec![];
        for (wire_name, prop) in props {
            let field_name = taken.claim(&names::value_name(&wire_name), "_");
            let ctx = format!("{name}{}", names::upper_camel(&wire_name));
            let mut ty = self.ty(&prop, &ctx);
            if !required.contains(&wire_name) {
                ty = ty.maybe();
            }
            let doc = self
                .doc
                .deref(&prop)
                .ok()
                .and_then(|p| p.get("description"))
                .and_then(Value::as_str)
                .map(names::summary)
                .unwrap_or_default();
            fields.push(Field {
                wire: wire_name,
                name: field_name,
                ty,
                doc,
            });
        }
        let doc = schema
            .get("description")
            .or_else(|| schema.get("title"))
            .and_then(Value::as_str)
            .map(names::summary)
            .unwrap_or_default();
        let i = self.index[&name];
        self.defs[i] = Def::Record {
            name: name.clone(),
            fields,
            doc,
            wire: wire.into(),
        };
        name
    }

    fn enum_values(&self, schema: &Value) -> Vec<String> {
        let mut values = vec![];
        let alts: Vec<Value> = ["oneOf", "anyOf"]
            .iter()
            .find_map(|k| schema.get(*k).and_then(Value::as_array).cloned())
            .unwrap_or_else(|| vec![schema.clone()]);
        for alt in &alts {
            for v in self.string_values(alt).unwrap_or_default() {
                if !values.contains(&v) {
                    values.push(v);
                }
            }
        }
        values
    }

    fn enumeration(&mut self, schema: &Value, name: String, wire: &str) -> String {
        let values = self.enum_values(schema);
        let ctors = values
            .into_iter()
            .map(|v| {
                let base = names::upper_camel(if v.is_empty() { "empty" } else { &v });
                let ctor = if self.upper.contains(&base) {
                    self.upper.claim(&format!("{name}{base}"), "Value")
                } else {
                    self.upper.claim(&base, "Value")
                };
                (ctor, v)
            })
            .collect();
        let doc = schema
            .get("description")
            .and_then(Value::as_str)
            .map(names::summary)
            .unwrap_or_default();
        self.push(Def::Enum {
            name: name.clone(),
            ctors,
            doc,
            wire: wire.into(),
        });
        name
    }

    fn union(&mut self, schema: &Value, name: String, wire: &str) -> String {
        self.push(Def::Union {
            name: name.clone(),
            alts: vec![],
            doc: String::new(),
            wire: wire.into(),
        });
        let alts_schema: Vec<Value> = ["oneOf", "anyOf"]
            .iter()
            .find_map(|k| schema.get(*k).and_then(Value::as_array).cloned())
            .unwrap_or_default();
        let mut alts = vec![];
        for (i, alt) in alts_schema.iter().enumerate() {
            let label = match alt.get("$ref").and_then(Value::as_str) {
                Some(r) => names::upper_camel(r.rsplit('/').next().unwrap_or("Option")),
                None => match alt.get("type").and_then(Value::as_str) {
                    Some(t) => names::upper_camel(t),
                    None => format!("Option{}", i + 1),
                },
            };
            let ty = self.ty(alt, &format!("{name}{label}"));
            let ctor = self.upper.claim(&format!("{name}{label}"), "Value");
            alts.push((ctor, ty));
        }
        let doc = schema
            .get("description")
            .and_then(Value::as_str)
            .map(names::summary)
            .unwrap_or_default();
        let i = self.index[&name];
        self.defs[i] = Def::Union {
            name: name.clone(),
            alts,
            doc,
            wire: wire.into(),
        };
        name
    }
}
