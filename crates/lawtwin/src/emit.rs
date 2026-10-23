//! The LawSpec package: the model unit, the facade unit with the abilities,
//! their laws and the reference handler, and the units left to the author.

use crate::api::{self, Api, Create, KeySource, Op, Resource, Role};
use crate::ir::{Check, Def, Field, Scalar, Ty, Types};
use crate::names::{self, Namespace, text_literal};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// The package's units, as LawSpec source.
pub struct Package {
    /// The model unit: types, constraints, State and each recognised operation's definition.
    pub model: String,
    /// The facade unit: the Service resource, the abilities with their laws, and the reference handler.
    pub facade: String,
    /// The hand-written unit's first version, with a stub for each unmodelled operation.
    pub handwritten: String,
    /// The hand-written laws unit's first version.
    pub laws: String,
    /// Operations answered by the hand-written unit.
    pub unmodelled: Vec<String>,
    /// How many laws were generated.
    pub law_count: usize,
    /// Each generated law of a resource: the record, the law's kind and its title.
    pub resource_laws: Vec<(String, &'static str, String)>,
}

/// The state variable of every model definition; no parameter can take
/// this name, as only reserved words get a trailing underscore.
const STATE: &str = "current_";

/// Writes a package from the types and the operations.
pub struct Emitter<'a, 'd> {
    /// The package's root unit name, such as `lawtwin.petstore`.
    pub unit: String,
    /// The converted schemas.
    pub types: &'a Types<'d>,
    /// The operations and resources.
    pub api: &'a Api,
    /// The document, as the generated files name it.
    pub source: String,
    helpers: String,
    emitted: BTreeSet<String>,
    has_checks_memo: BTreeMap<String, bool>,
    refinements: BTreeMap<String, String>,
    refinement_names: Namespace,
    after: BTreeSet<&'static str>,
    defaults_failed: BTreeSet<String>,
}

fn check_expr(check: &Check, v: &str) -> String {
    match check {
        Check::Min { bound, exclusive } => {
            format!("{v} {} {bound}", if *exclusive { ">" } else { ">=" })
        }
        Check::Max { bound, exclusive } => {
            format!("{v} {} {bound}", if *exclusive { "<" } else { "<=" })
        }
        Check::MinLength(n) | Check::MinItems(n) => format!("prelude.length {v} >= {n}"),
        Check::MaxLength(n) | Check::MaxItems(n) => format!("prelude.length {v} <= {n}"),
        Check::Regex { regex, .. } => format!("{v} matches regex {}", text_literal(regex)),
        Check::Finite => format!("prelude.isFinite {v}"),
        Check::OneOf(values) => {
            if values.is_empty() {
                "true".into()
            } else {
                format!(
                    "({})",
                    values
                        .iter()
                        .map(|n| format!("{v} = {n}"))
                        .collect::<Vec<_>>()
                        .join(" || ")
                )
            }
        }
    }
}

fn conj(parts: Vec<String>) -> Option<String> {
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" && "))
    }
}

impl<'a, 'd> Emitter<'a, 'd> {
    /// An emitter for the package `unit`.
    pub fn new(unit: &str, types: &'a Types<'d>, api: &'a Api, source: &str) -> Self {
        let mut refinement_names = Namespace::default();
        for d in &types.defs {
            refinement_names.reserve(d.name());
        }
        for (_, c) in &api.errors {
            refinement_names.reserve(c);
        }
        Emitter {
            unit: unit.into(),
            types,
            api,
            source: source.into(),
            helpers: String::new(),
            emitted: BTreeSet::new(),
            has_checks_memo: BTreeMap::new(),
            refinements: BTreeMap::new(),
            refinement_names,
            after: BTreeSet::new(),
            defaults_failed: BTreeSet::new(),
        }
    }

    // ------------------------------------------------------------------
    // Validation
    // ------------------------------------------------------------------

    fn has_checks(&mut self, ty: &Ty, visiting: &mut Vec<String>) -> bool {
        match ty {
            Ty::Scalar(_, c) => !c.is_empty(),
            Ty::List(t, c) => !c.is_empty() || self.has_checks(t, visiting),
            Ty::Maybe(t) | Ty::Map(t) => self.has_checks(t, visiting),
            Ty::Named(n) => {
                if let Some(&b) = self.has_checks_memo.get(n) {
                    return b;
                }
                if visiting.contains(n) {
                    // A record that holds itself is not checked through itself.
                    return false;
                }
                visiting.push(n.clone());
                let inner: Vec<Ty> = match self.types.def(n) {
                    Some(Def::Record { fields, .. }) => {
                        fields.iter().map(|f| f.ty.clone()).collect()
                    }
                    Some(Def::Union { alts, .. }) => alts.iter().map(|(_, t)| t.clone()).collect(),
                    _ => vec![],
                };
                let b = inner.iter().any(|t| self.has_checks(t, visiting));
                visiting.pop();
                if visiting.is_empty() {
                    self.has_checks_memo.insert(n.clone(), b);
                }
                b
            }
        }
    }

    fn checked(&mut self, ty: &Ty) -> bool {
        self.has_checks(ty, &mut vec![])
    }

    /// A Boolean expression that `expr`, of type `ty`, keeps its schema's
    /// constraints; None when it has none.
    fn validator(&mut self, ty: &Ty, expr: &str, depth: usize) -> Option<String> {
        if !self.checked(ty) {
            return None;
        }
        match ty {
            Ty::Scalar(_, checks) => conj(checks.iter().map(|c| check_expr(c, expr)).collect()),
            Ty::Maybe(t) => {
                let v = format!("present{depth}");
                let inner = self.validator(t, &v, depth + 1)?;
                Some(format!(
                    "(first matching case of {expr} : when Absent then true when Present {v} then {inner} end case)"
                ))
            }
            Ty::List(t, checks) => {
                let mut parts: Vec<String> = checks.iter().map(|c| check_expr(c, expr)).collect();
                if self.checked(t) {
                    let helper = self.each_helper(t);
                    parts.push(format!("{helper} {expr}"));
                }
                conj(parts)
            }
            Ty::Map(t) => {
                let helper = self.each_helper(t);
                Some(format!("{helper} (prelude.values {expr})"))
            }
            Ty::Named(n) => {
                let helper = self.named_validator(n);
                Some(format!("{helper} {expr}"))
            }
        }
    }

    /// `validEach...`: every item of a list keeps its constraints.
    fn each_helper(&mut self, item: &Ty) -> String {
        let name = format!(
            "validEach{}",
            names::upper_camel(&item.lawspec().replace(' ', "_"))
        );
        let name = if let Ty::Scalar(_, checks) = item {
            // Scalars with different checks need different helpers.
            let digest: u32 = format!("{checks:?}")
                .bytes()
                .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
            format!("{name}{}", digest % 100000)
        } else {
            name
        };
        if self.emitted.insert(name.clone()) {
            let body = self
                .validator(item, "item", 0)
                .unwrap_or_else(|| "true".into());
            let _ = write!(
                self.helpers,
                "function {name} (items :: List {}) -> Bool is\n  first matching case of items :\n  when Nil then true\n  when Cons item rest then {body} && {name} rest\n  end case\nend function\n\n",
                item.atom()
            );
        }
        name
    }

    /// `valid<Type>`: a record or union keeps its schema's constraints.
    fn named_validator(&mut self, n: &str) -> String {
        let name = format!("valid{n}");
        if self.emitted.insert(name.clone()) {
            let body = match self.types.def(n).cloned() {
                Some(Def::Record { fields, .. }) => {
                    let mut parts = vec![];
                    for f in &fields {
                        if let Some(v) = self.validator(&f.ty, &format!("value.{}", f.name), 0) {
                            parts.push(v);
                        }
                    }
                    conj(parts).unwrap_or_else(|| "true".into())
                }
                Some(Def::Union { alts, .. }) => {
                    let mut arms = String::new();
                    for (ctor, t) in &alts {
                        let v = self
                            .validator(t, "payload", 0)
                            .unwrap_or_else(|| "true".into());
                        let _ = write!(arms, " | {ctor} payload -> {v}");
                    }
                    format!("first matching case of value :{arms} end case")
                }
                _ => "true".into(),
            };
            let _ = write!(
                self.helpers,
                "-- Whether a {n} keeps its schema's constraints.\nfunction {name} (value :: {n}) -> Bool is\n  {body}\nend function\n\n"
            );
        }
        name
    }

    /// A named refinement of a scalar with checks, for law inputs.
    fn refinement(&mut self, wanted: &str, ty: &Ty) -> Option<String> {
        let Ty::Scalar(s, checks) = ty else {
            return None;
        };
        if checks.is_empty() {
            return None;
        }
        let pred = conj(checks.iter().map(|c| check_expr(c, "v")).collect())?;
        let key = format!("{}|{pred}", s.lawspec());
        if let Some(n) = self.refinements.get(&key) {
            return Some(n.clone());
        }
        let name = self
            .refinement_names
            .claim(&names::upper_camel(wanted), "Value");
        self.refinements.insert(key, name.clone());
        Some(name)
    }

    // ------------------------------------------------------------------
    // Defaults
    // ------------------------------------------------------------------

    fn default_of(&mut self, ty: &Ty, visiting: &mut Vec<String>) -> Option<String> {
        Some(match ty {
            Ty::Scalar(s, _) => match s {
                Scalar::Text => "\"\"".into(),
                Scalar::Json => "\"null\"".into(),
                Scalar::Int32 | Scalar::Int64 => "0".into(),
                Scalar::Float64 => "0.0".into(),
                Scalar::Bool => "false".into(),
                Scalar::Bytes => "(prelude.utf8 \"\")".into(),
                Scalar::Unit => "unitValue".into(),
            },
            Ty::Maybe(_) => "Absent".into(),
            Ty::List(..) => "[]".into(),
            Ty::Map(_) => "(prelude.keyValOf [])".into(),
            Ty::Named(n) => {
                if self.defaults_failed.contains(n) || visiting.contains(n) {
                    return None;
                }
                match self.types.def(n).cloned()? {
                    Def::Enum { ctors, .. } => ctors.first()?.0.clone(),
                    Def::Union { alts, .. } => {
                        visiting.push(n.clone());
                        let (ctor, t) = alts.first()?.clone();
                        let d = self.default_of(&t, visiting);
                        visiting.pop();
                        format!("({ctor} {})", d?)
                    }
                    Def::Record { fields, .. } => {
                        let name = format!("default{n}");
                        if !self.emitted.contains(&name) {
                            visiting.push(n.clone());
                            let mut args = vec![];
                            for f in &fields {
                                match self.default_of(&f.ty, visiting) {
                                    Some(d) => args.push(format!("({d})")),
                                    None => {
                                        visiting.pop();
                                        self.defaults_failed.insert(n.clone());
                                        return None;
                                    }
                                }
                            }
                            visiting.pop();
                            self.emitted.insert(name.clone());
                            let _ = write!(
                                self.helpers,
                                "-- A {n} with every field empty, for what a request does not say.\nfunction {name} (u :: Unit) -> {n} is {n} record{{{}}} end function\n\n",
                                fields
                                    .iter()
                                    .zip(args.iter())
                                    .map(|(f, v)| format!("{} = {v}", f.name))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            );
                        }
                        format!("({name} unitValue)")
                    }
                }
            }
        })
    }

    /// The stored record built from a request body: a copy when the body is
    /// the record, otherwise field by field, with the key put in its field.
    fn build_from_body(
        &mut self,
        record: &str,
        body_ty: &Ty,
        body: &str,
        key: Option<(&str, bool, &str)>,
    ) -> Option<String> {
        let key_update = |base: String| match key {
            Some((field, maybe, k)) => format!(
                "{base} with {field} = {}",
                if maybe {
                    format!("Present {k}")
                } else {
                    k.to_string()
                }
            ),
            None => base,
        };
        if body_ty.same_shape(&Ty::Named(record.into())) {
            return Some(match key {
                Some(_) => format!("({})", key_update(body.into())),
                None => body.into(),
            });
        }
        let body_fields: Vec<Field> = api::is_record(self.types, body_ty)
            .and_then(|b| self.types.record_fields(&b).map(|f| f.to_vec()))?;
        let fields: Vec<Field> = self.types.record_fields(record)?.to_vec();
        let mut args = vec![];
        for f in &fields {
            if let Some((kf, maybe, k)) = key {
                if kf == f.name {
                    args.push(if maybe {
                        format!("(Present {k})")
                    } else {
                        k.to_string()
                    });
                    continue;
                }
            }
            let from = api::field(&body_fields, &f.name).cloned();
            let arg = match from {
                Some(b) if b.ty.same_shape(&f.ty) => format!("{body}.{}", b.name),
                Some(b)
                    if b.ty.same_shape(&f.ty.clone().maybe()) && !matches!(f.ty, Ty::Maybe(_)) =>
                {
                    let d = self.default_of(&f.ty, &mut vec![])?;
                    format!(
                        "(first matching case of {body}.{} : when Absent then {d} when Present given then given end case)",
                        b.name
                    )
                }
                Some(b) if f.ty.same_shape(&b.ty.clone().maybe()) => {
                    format!("(Present {body}.{})", b.name)
                }
                _ => self.default_of(&f.ty, &mut vec![])?,
            };
            args.push(if arg.contains(' ') && !arg.starts_with('(') {
                format!("({arg})")
            } else {
                arg
            });
        }
        Some(if args.is_empty() {
            record.to_string()
        } else {
            format!(
                "({record} record{{{}}})",
                fields
                    .iter()
                    .zip(args)
                    .map(|(f, v)| format!("{} = {v}", f.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
    }

    // ------------------------------------------------------------------
    // The model unit
    // ------------------------------------------------------------------

    fn key_expr(&self, r: &Resource, parents: &[String], key: &str) -> String {
        match &r.key_record {
            Some(kr) => format!(
                "({kr} record{{{}, {} = {key}}})",
                r.parents
                    .iter()
                    .zip(parents)
                    .map(|((n, _), v)| format!("{n} = {v}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                r.key_param
            ),
            None => key.to_string(),
        }
    }

    fn after_helper(&mut self, ty: &Ty) -> &'static str {
        let (name, max, t) = match ty {
            Ty::Scalar(Scalar::Int32, _) => ("after32", "2147483647", "Int32"),
            _ => ("after64", "9223372036854775807", "Int64"),
        };
        if self.after.insert(name) {
            let _ = write!(
                self.helpers,
                "-- The key after k, or k at the end of its type.\nfunction {name} (k :: {t}) -> {t} is if k < {max} then k + 1 else k end if\nend function\n\n"
            );
        }
        name
    }

    fn param_sig(op: &Op) -> String {
        op.params
            .iter()
            .map(|p| format!("({} :: {}) ", p.name, p.ty.lawspec()))
            .collect()
    }

    /// The validation guard of an operation's parameters and body.
    fn guard(&mut self, op: &Op) -> Option<String> {
        let mut parts = vec![];
        for p in &op.params {
            if let Some(v) = self.validator(&p.ty, &p.name, 0) {
                parts.push(v);
            }
        }
        conj(parts)
    }

    /// The body of a model definition for an operation's role, or None
    /// when the importer cannot model it.
    fn model_body(&mut self, oi: usize, ri: usize, role: Role) -> Option<String> {
        let op = self.api.ops[oi].clone();
        let r = self.api.resources[ri].clone();
        let record_ty = Ty::Named(r.record.clone());
        let parents: Vec<String> = r.parents.iter().map(|(n, _)| n.clone()).collect();
        let map = format!("{STATE}.{}", r.map_field);
        let respond = |found: &str| -> Option<String> {
            if op.response.same_shape(&record_ty) {
                Some(found.to_string())
            } else if op.response.is_unit() {
                Some("unitValue".into())
            } else {
                None
            }
        };
        let guard = self.guard(&op);
        let invalid = op.invalid_error();
        let guarded = |body: String| match &guard {
            Some(g) => format!("if !({g}) then raise {invalid} else\n  {body}\nend if"),
            None => body,
        };
        let key_param = op
            .params
            .iter()
            .find(|p| p.name == r.key_param && p.location == "path")
            .map(|p| p.name.clone());
        let body = match role {
            Role::Read => {
                let k = self.key_expr(&r, &parents, key_param.as_deref()?);
                format!(
                    "first matching case of prelude.lookup {k} {map} :\n  when Absent then raise NotFound\n  when Present found then Pair {} {STATE}\n  end case",
                    respond("found")?
                )
            }
            Role::Delete => {
                let k = self.key_expr(&r, &parents, key_param.as_deref()?);
                format!(
                    "first matching case of prelude.lookup {k} {map} :\n  when Absent then raise NotFound\n  when Present found then Pair {} ({STATE} with {} = prelude.delete {k} {map})\n  end case",
                    respond("found")?,
                    r.map_field
                )
            }
            Role::List => {
                let values = if parents.is_empty() {
                    format!("(prelude.values {map})")
                } else {
                    format!(
                        "(model{}Under {} (prelude.entries {map}))",
                        names::upper_camel(&r.map_field),
                        parents.join(" ")
                    )
                };
                if !parents.is_empty() {
                    self.under_helper(&r);
                }
                format!("Pair {values} {STATE}")
            }
            Role::Create => {
                let Create { key, .. } = r.create.clone()?;
                let body_param = op.body()?.clone();
                let (sent, unwrap) = match &body_param.ty {
                    Ty::Maybe(inner) => (
                        "sent".to_string(),
                        Some((*inner.clone(), body_param.name.clone())),
                    ),
                    _ => (body_param.name.clone(), None),
                };
                let sent_ty = unwrap
                    .as_ref()
                    .map(|(t, _)| t.clone())
                    .unwrap_or(body_param.ty.clone());
                let counter = r.counter_field.clone();
                let key_value = match &key {
                    KeySource::Body { field } => format!("{sent}.{field}"),
                    KeySource::BodyOrAssigned { field } => {
                        format!(
                            "(first matching case of {sent}.{field} : when Present given then given when Absent then {STATE}.{} end case)",
                            counter.clone()?
                        )
                    }
                    KeySource::Assigned => format!("{STATE}.{}", counter.clone()?),
                    KeySource::BodyIfGiven { .. } => "given".to_string(),
                };
                let key_field = r.key_field.clone();
                let stored = self.build_from_body(
                    &r.record,
                    &sent_ty,
                    &sent,
                    key_field.as_ref().map(|(f, m)| (f.as_str(), *m, "key")),
                )?;
                let k = self.key_expr(&r, &parents, "key");
                let mut next = format!(
                    "{STATE} with {} = prelude.put {k} stored {map}",
                    r.map_field
                );
                if let Some(c) = &counter {
                    let after = self.after_helper(&r.key_ty);
                    let _ = write!(
                        next,
                        ", {c} = (if key >= {STATE}.{c} then {after} key else {STATE}.{c} end if)"
                    );
                }
                let answer = format!("Pair {} ({next})", respond("stored")?);
                let mut inner = format!("let key = {key_value} in\n  let stored = {stored} in\n  ");
                match op.conflict_error() {
                    Some(err) => {
                        let _ = write!(
                            inner,
                            "first matching case of prelude.lookup {k} {map} :\n  when Present existing then raise {err}\n  when Absent then {answer}\n  end case"
                        );
                    }
                    None => inner.push_str(&answer),
                }
                inner.push_str("\n  end let\n  end let");
                if let KeySource::BodyIfGiven { field } = &key {
                    inner = format!(
                        "first matching case of {sent}.{field} :\n  when Absent then raise {invalid}\n  when Present given then\n  {inner}\n  end case"
                    );
                }
                match unwrap {
                    Some((_, name)) => format!(
                        "first matching case of {name} :\n  when Absent then raise {invalid}\n  when Present sent then\n  {inner}\n  end case"
                    ),
                    None => inner,
                }
            }
            Role::Replace => {
                let body_param = op.body()?.clone();
                let (sent, sent_ty, wrap) = match &body_param.ty {
                    Ty::Maybe(inner) => ("sent".to_string(), *inner.clone(), true),
                    t => (body_param.name.clone(), t.clone(), false),
                };
                let key_field = r.key_field.clone();
                let stored = self.build_from_body(
                    &r.record,
                    &sent_ty,
                    &sent,
                    key_field.as_ref().map(|(f, m)| (f.as_str(), *m, "key")),
                )?;
                let k = self.key_expr(&r, &parents, "key");
                let update = format!(
                    "first matching case of prelude.lookup {k} {map} :\n  when Absent then raise NotFound\n  when Present old then\n      let stored = {stored} in\n      Pair {} ({STATE} with {} = prelude.put {k} stored {map})\n      end let\n  end case",
                    respond("stored")?,
                    r.map_field
                );
                let keyed = if r.replace_by_body_key {
                    let (kf, maybe) = key_field?;
                    if maybe {
                        format!(
                            "first matching case of {sent}.{kf} :\n  when Absent then raise {invalid}\n  when Present key then\n  {update}\n  end case"
                        )
                    } else {
                        format!("let key = {sent}.{kf} in\n  {update}\nend let")
                    }
                } else {
                    format!("let key = {} in\n  {update}\nend let", key_param?)
                };
                if wrap {
                    format!(
                        "first matching case of {} :\n  when Absent then raise {invalid}\n  when Present sent then\n  {keyed}\n  end case",
                        body_param.name
                    )
                } else {
                    keyed
                }
            }
            Role::Patch => {
                let body_param = op.body()?.clone();
                let (patch_ty, maybe_body) = match &body_param.ty {
                    Ty::Maybe(inner) => (*inner.clone(), true),
                    t => (t.clone(), false),
                };
                let assignments = self.patch_assignments(&r, &patch_ty, "change", "old")?;
                let k = self.key_expr(&r, &parents, key_param.as_deref()?);
                let patched = if assignments.is_empty() {
                    "old".to_string()
                } else {
                    format!("(old with {})", assignments.join(", "))
                };
                let patched = if maybe_body {
                    format!(
                        "(first matching case of {} : when Absent then old when Present change then {patched} end case)",
                        body_param.name
                    )
                } else {
                    format!("(let change = {} in {patched} end let)", body_param.name)
                };
                format!(
                    "first matching case of prelude.lookup {k} {map} :\n  when Absent then raise NotFound\n  when Present old then\n      let stored = {patched} in\n      Pair {} ({STATE} with {} = prelude.put {k} stored {map})\n      end let\n  end case",
                    respond("stored")?,
                    r.map_field
                )
            }
        };
        Some(guarded(body))
    }

    /// `field = value` for each field of the record a patch body names.
    fn patch_assignments(
        &self,
        r: &Resource,
        patch_ty: &Ty,
        change: &str,
        old: &str,
    ) -> Option<Vec<String>> {
        let patch_fields = self
            .types
            .record_fields(&api::is_record(self.types, patch_ty)?)?;
        let fields = self.types.record_fields(&r.record)?;
        let mut out = vec![];
        for f in fields {
            if r.key_field.as_ref().is_some_and(|(k, _)| *k == f.name) {
                continue;
            }
            let Some(p) = api::field(patch_fields, &f.name) else {
                continue;
            };
            let value = if p.ty.same_shape(&f.ty) && matches!(p.ty, Ty::Maybe(_)) {
                // An absent field and a null one read the same: both leave it.
                format!(
                    "(first matching case of {change}.{} : when Absent then {old}.{} when Present given then Present given end case)",
                    p.name, f.name
                )
            } else if p.ty.same_shape(&f.ty) {
                format!("{change}.{}", p.name)
            } else if p.ty.same_shape(&f.ty.clone().maybe()) && !matches!(f.ty, Ty::Maybe(_)) {
                format!(
                    "(first matching case of {change}.{} : when Absent then {old}.{} when Present given then given end case)",
                    p.name, f.name
                )
            } else if f.ty.same_shape(&p.ty.clone().maybe()) {
                format!("(Present {change}.{})", p.name)
            } else {
                continue;
            };
            out.push(format!("{} = {value}", f.name));
        }
        Some(out)
    }

    fn under_helper(&mut self, r: &Resource) {
        let name = format!("model{}Under", names::upper_camel(&r.map_field));
        if !self.emitted.insert(name.clone()) {
            return;
        }
        let kr = r.key_record.clone().unwrap();
        let params: String = r
            .parents
            .iter()
            .map(|(n, t)| format!("({n} :: {}) ", t.lawspec()))
            .collect();
        let args: String = r.parents.iter().map(|(n, _)| format!("{n} ")).collect();
        let test = r
            .parents
            .iter()
            .map(|(n, _)| format!("entry.key.{n} = {n}"))
            .collect::<Vec<_>>()
            .join(" && ");
        let _ = write!(
            self.helpers,
            "-- The {}s under one parent, in key order.\nfunction {name} {params}(entries :: List (Entry {kr} {})) -> List {} is\n  first matching case of entries :\n  when Nil then []\n  when Cons entry rest then if {test} then Cons entry.value ({name} {args}rest) else {name} {args}rest end if\n  end case\nend function\n\n",
            r.record, r.record, r.record
        );
    }

    fn contains_helper(&mut self, record: &str) -> String {
        let name = format!("contains{record}");
        if self.emitted.insert(name.clone()) {
            let _ = write!(
                self.helpers,
                "-- Whether a list holds a {record}.\nfunction {name} (wanted :: {record}) (items :: List {record}) -> Bool is\n  first matching case of items :\n  when Nil then false\n  when Cons item rest then item = wanted || {name} wanted rest\n  end case\nend function\n\n"
            );
        }
        name
    }

    // ------------------------------------------------------------------
    // Everything
    // ------------------------------------------------------------------

    /// Write every unit of the package.
    pub fn package(mut self) -> Package {
        let unit = self.unit.clone();
        let mut model = String::new();
        let header = format!(
            "-- Generated by `lawtwin import openapi` from\n-- {}.\n-- Do not edit: the importer writes this file again on every import. Write\n-- behaviour the importer cannot model in handwritten.lawspec, and laws in\n-- laws.lawspec.\n",
            self.source
        );
        let _ = writeln!(model, "unit {unit}.model\n\n{header}");
        // Failures.
        model.push_str("-- What a request fails with: one constructor for each failing status the\n-- document names, and BadRequest, NotFound and Unmodelled, which the\n-- model needs. Unmodelled is an operation's answer until its behaviour is\n-- written in handwritten.lawspec.\ntype ApiError is one of\n");
        for (status, ctor) in &self.api.errors {
            let note = if ctor == "Unmodelled" {
                "the model does not answer this operation yet".to_string()
            } else {
                format!("HTTP {status}")
            };
            let _ = writeln!(model, "  | {ctor} -- {note}");
        }
        model.push_str("end type\n\n");
        // Types.
        model.push_str("-- ---------------------------------------------------------------------------\n-- Schemas\n-- ---------------------------------------------------------------------------\n\n");
        for d in &self.types.defs {
            match d {
                Def::Record {
                    name,
                    fields,
                    doc,
                    wire,
                } => {
                    if !doc.is_empty() {
                        let _ = writeln!(model, "-- {doc}");
                    }
                    let _ = writeln!(model, "-- Schema `{wire}`.\ntype {name} is a record{{");
                    for f in fields {
                        let mut notes = vec![];
                        if f.wire != f.name {
                            notes.push(format!("`{}`", f.wire));
                        }
                        if !f.doc.is_empty() {
                            notes.push(f.doc.clone());
                        }
                        let note = if notes.is_empty() {
                            String::new()
                        } else {
                            format!(" -- {}", notes.join(": "))
                        };
                        let _ = writeln!(model, "    {} :: {}{note}", f.name, f.ty.lawspec());
                    }
                    model.push_str("}\nend type\n\n");
                }
                Def::Enum {
                    name,
                    ctors,
                    doc,
                    wire,
                } => {
                    if !doc.is_empty() {
                        let _ = writeln!(model, "-- {doc}");
                    }
                    let _ = writeln!(model, "-- Schema `{wire}`.\ntype {name} is one of");
                    for (c, w) in ctors {
                        let _ = writeln!(model, "  | {c} -- {}", text_literal(w));
                    }
                    model.push_str("end type\n\n");
                }
                Def::Union {
                    name,
                    alts,
                    doc,
                    wire,
                } => {
                    if !doc.is_empty() {
                        let _ = writeln!(model, "-- {doc}");
                    }
                    let _ = writeln!(
                        model,
                        "-- Schema `{wire}`: one of its alternatives.\ntype {name} is one of"
                    );
                    for (c, t) in alts {
                        let _ = writeln!(model, "  | {c} of type {}", t.lawspec());
                    }
                    model.push_str("end type\n\n");
                }
            }
        }
        // Key records.
        for r in &self.api.resources {
            if let Some(kr) = &r.key_record {
                let _ = writeln!(
                    model,
                    "-- Where a {} is kept: {}.\ntype {kr} is a record{{",
                    r.record, r.item_path
                );
                for (n, t) in &r.parents {
                    let _ = writeln!(model, "    {n} :: {}", t.lawspec());
                }
                let _ = writeln!(
                    model,
                    "    {} :: {}\n}}\nend type\n",
                    r.key_param,
                    r.key_ty.lawspec()
                );
            }
        }
        // State.
        model.push_str("-- ---------------------------------------------------------------------------\n-- The service's state\n-- ---------------------------------------------------------------------------\n\n");
        model.push_str("-- What the service holds: each resource the importer recognised, by key,\n-- and the next key for those whose keys the service assigns.\ntype State is a record{\n");
        let mut starts = vec![];
        for r in &self.api.resources {
            let k = r.key_record.clone().unwrap_or_else(|| r.key_ty.lawspec());
            let _ = writeln!(
                model,
                "    {} :: KeyVal {k} {} -- {}",
                r.map_field, r.record, r.item_path
            );
            starts.push("(prelude.keyValOf [])".to_string());
            if let Some(c) = &r.counter_field {
                let _ = writeln!(model, "    {c} :: {}", r.key_ty.lawspec());
                starts.push("1".into());
            }
        }
        model.push_str("}\nend type\n\n");
        let _ = write!(
            model,
            "-- The state of a new service: nothing held.\nfunction emptyState (u :: Unit) -> State is State record{{{}}} end function\n\n",
            self.api
                .resources
                .iter()
                .flat_map(
                    |r| std::iter::once(r.map_field.as_str()).chain(r.counter_field.as_deref())
                )
                .zip(starts)
                .map(|(f, v)| format!("{f} = {v}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        // Model definitions.
        let mut defs = String::new();
        let mut modelled: BTreeMap<String, String> = BTreeMap::new();
        for oi in 0..self.api.ops.len() {
            let Some((ri, role)) = self.api.role(oi) else {
                continue;
            };
            let op = self.api.ops[oi].clone();
            if let Some(body) = self.model_body(oi, ri, role) {
                let def_name = format!("model{}", names::upper_camel(&op.name));
                let (kind, fails) = if body.contains("raise ") {
                    ("procedure", " fails with ApiError")
                } else {
                    ("function", "")
                };
                let _ = write!(
                    defs,
                    "-- {} {}: {}\n{kind} {def_name} {}({STATE} :: State) -> Pair {} State{fails} is\n  {body}\nend {kind}\n\n",
                    op.method.to_uppercase(),
                    op.path,
                    format!("{role:?}").to_lowercase(),
                    Self::param_sig(&op),
                    op.response.atom()
                );
                modelled.insert(op.name.clone(), def_name);
            }
        }
        // Law inputs need refinements; laws are built before the model is
        // finished so their helpers land in it.
        let (laws_by_ability, top_laws, law_count, resource_laws) = self.laws();
        // Refinements, validation and helpers.
        if !self.refinements.is_empty() {
            model.push_str("-- ---------------------------------------------------------------------------\n-- Constraints\n-- ---------------------------------------------------------------------------\n\n");
            model.push_str(
                "-- Named refinements of constrained values, for laws to draw valid inputs.\n",
            );
            let mut refs: Vec<(&String, &String)> = self.refinements.iter().collect();
            refs.sort_by(|a, b| a.1.cmp(b.1));
            for (key, name) in refs {
                let (scalar, pred) = key.split_once('|').unwrap();
                let _ = writeln!(
                    model,
                    "refinement {name} is (v :: {scalar} where {pred}) end refinement"
                );
            }
            model.push('\n');
        }
        // Validators of every body, so a twin can refuse what the document refuses.
        for op in self.api.ops.clone() {
            for p in &op.params {
                if let Some(n) = api::is_record(
                    self.types,
                    match &p.ty {
                        Ty::Maybe(t) => t,
                        t => t,
                    },
                ) {
                    if self.checked(&Ty::Named(n.clone())) {
                        self.named_validator(&n);
                    }
                }
            }
        }
        model.push_str(&std::mem::take(&mut self.helpers));
        model.push_str("-- ---------------------------------------------------------------------------\n-- What each recognised operation does\n-- ---------------------------------------------------------------------------\n\n");
        model.push_str(&defs);
        // The facade.
        let mut facade = String::new();
        let _ = writeln!(facade, "unit {unit}\n\n{header}");
        let mut imported: Vec<String> = self
            .types
            .defs
            .iter()
            .map(|d| d.name().to_string())
            .collect();
        imported.extend(
            self.api
                .resources
                .iter()
                .filter_map(|r| r.key_record.clone()),
        );
        imported.push("ApiError".into());
        imported.push("State".into());
        let mut refinement_names: Vec<String> = self.refinements.values().cloned().collect();
        refinement_names.sort();
        imported.extend(refinement_names);
        let _ = writeln!(
            facade,
            "import {unit}.model as model ({})\nimport {unit}.handwritten as handwritten\n",
            imported.join(", ")
        );
        let _ = writeln!(
            facade,
            "-- A connection to the service. Every native handler of a case is made with\n-- the case's one Service, so what one operation changes the next one sees.\nhandle Service\n\nconnectService :: Unit -> Service\ndisconnectService :: Service -> Unit\n\nresource Service is\n  acquire is connectService unitValue end acquire\n  release service is disconnectService service end release\nend resource\n"
        );
        for ability in &self.api.abilities {
            let _ = writeln!(
                facade,
                "-- ---------------------------------------------------------------------------\n-- {ability}\n-- ---------------------------------------------------------------------------\n\nability {ability} with Service is"
            );
            for op in self.api.ops.iter().filter(|o| &o.ability == ability) {
                let _ = writeln!(
                    facade,
                    "  -- {} {}{}",
                    op.method.to_uppercase(),
                    op.path,
                    if op.summary.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", op.summary)
                    }
                );
                let args: Vec<String> = op.params.iter().map(|p| p.ty.lawspec()).collect();
                let sig = if args.is_empty() {
                    op.response.lawspec()
                } else {
                    format!("{} -> {}", args.join(" -> "), op.response.lawspec())
                };
                let _ = writeln!(facade, "  {} :: {sig} fails with ApiError", op.name);
            }
            if let Some(laws) = laws_by_ability.get(ability) {
                facade.push_str("laws\n");
                facade.push_str(laws);
                facade.push_str("end laws\n");
            }
            facade.push_str("end ability\n\n");
        }
        facade.push_str(&top_laws);
        let _ = writeln!(
            facade,
            "-- ---------------------------------------------------------------------------\n-- The reference model\n-- ---------------------------------------------------------------------------\n\n-- Every operation, answered from one state: by the model's definition of\n-- it when the importer recognised what it does, otherwise by the\n-- hand-written unit.\nhandler reference for {} with state state :: State start model.emptyState unitValue is",
            self.api.abilities.join(", ")
        );
        let mut unmodelled = vec![];
        for op in &self.api.ops {
            match modelled.get(&op.name) {
                Some(d) => {
                    let _ = writeln!(facade, "  operation {} by model.{d} end operation", op.name);
                }
                None => {
                    let _ = writeln!(
                        facade,
                        "  operation {} by handwritten.{} end operation",
                        op.name, op.name
                    );
                    unmodelled.push(op.name.clone());
                }
            }
        }
        facade.push_str("end handler\nend unit\n");
        // The hand-written unit.
        let mut hand = String::new();
        let _ = writeln!(
            hand,
            "unit {unit}.handwritten\n\n-- Written once by `lawtwin import openapi`, and yours from then on: the\n-- importer never writes this file again. Each definition answers one\n-- operation the importer could not model. Replace its body with what the\n-- service does, over the model's State; until then it fails with\n-- Unmodelled. An operation added to the document later needs a definition\n-- here, and the importer names it. The model's types are written\n-- qualified, as model.Pet, so a new import does not break this unit.\n\nimport {unit}.model as model (ApiError, State)\n"
        );
        for name in &unmodelled {
            let op = self.api.ops.iter().find(|o| &o.name == name).unwrap();
            let _ = write!(
                hand,
                "-- HAND-WRITTEN: {} {}{}\nprocedure {} {}({STATE} :: State) -> Pair {} State fails with ApiError is\n  raise Unmodelled\nend procedure\n\n",
                op.method.to_uppercase(),
                op.path,
                if op.summary.is_empty() {
                    String::new()
                } else {
                    format!(": {}", op.summary)
                },
                op.name,
                op.params
                    .iter()
                    .map(|p| format!("({} :: {}) ", p.name, p.ty.qualified("model")))
                    .collect::<String>(),
                op.response.qualified_atom("model")
            );
        }
        if unmodelled.is_empty() {
            hand.push_str("-- Every operation is modelled; this unit has nothing to answer yet.\nfunction nothingHandWritten (u :: Unit) -> Unit is unitValue end function\n");
        }
        let laws = format!(
            "unit {unit}.laws\n\n-- HAND-WRITTEN LAWS. Written once by `lawtwin import openapi`, and yours\n-- from then on: the importer never writes this file again. State here what\n-- the document cannot say: how filters, pagination or state changes behave.\n-- The generated laws are in the abilities of {unit}.\n--\n-- For example:\n--\n--   law \"|a found pet has the status asked for| is\n--     definition :\n--       for all (status :: model.PetStatus) => ... end for all\n--     end definition\n--   end law\n\nimport {unit} as api\nimport {unit}.model as model (ApiError, State)\n\n-- A placeholder, so the unit is not empty; delete it once a law is here.\nfunction noHandWrittenLawsYet (u :: Unit) -> Unit is unitValue end function\nend unit\n"
        );
        model.push_str("end unit\n");
        hand.push_str("end unit\n");
        Package {
            model,
            facade,
            handwritten: hand,
            laws,
            unmodelled,
            law_count,
            resource_laws,
        }
    }

    // ------------------------------------------------------------------
    // Laws
    // ------------------------------------------------------------------

    #[allow(clippy::type_complexity)]
    fn laws(
        &mut self,
    ) -> (
        BTreeMap<String, String>,
        String,
        usize,
        Vec<(String, &'static str, String)>,
    ) {
        let mut by_ability: BTreeMap<String, String> = BTreeMap::new();
        let mut top = String::new();
        let mut count = 0;
        let mut titled = vec![];
        let mut law_names = Namespace::default();
        for ri in 0..self.api.resources.len() {
            let r = self.api.resources[ri].clone();
            for (title, ops, text) in self.resource_laws(&r) {
                let title = law_names.claim(&title, &format!(" at {}", r.item_path));
                let text = text.replace("@TITLE@", &title);
                count += 1;
                titled.push((r.record.clone(), law_kind(&title), title.clone()));
                let abilities: BTreeSet<&String> =
                    ops.iter().map(|&o| &self.api.ops[o].ability).collect();
                if abilities.len() == 1 {
                    let a = abilities.into_iter().next().unwrap().clone();
                    by_ability.entry(a).or_default().push_str(&indent(&text, 2));
                } else {
                    top.push_str(&text);
                    top.push('\n');
                }
            }
        }
        (by_ability, top, count, titled)
    }

    /// Call arguments of an operation: the values given by name, `Absent`
    /// for optional ones, and a quantified input for each other required one.
    fn call(
        &self,
        op: &Op,
        given: &BTreeMap<String, String>,
        extra: &mut Vec<(String, String)>,
        scope: &mut Namespace,
    ) -> String {
        let mut args = vec![op.name.clone()];
        for p in &op.params {
            if let Some(v) = given.get(&p.name) {
                args.push(paren(v));
            } else if matches!(p.ty, Ty::Maybe(_)) {
                args.push("Absent".into());
            } else {
                let v = scope.claim(&p.name, "Input");
                extra.push((v.clone(), p.ty.lawspec()));
                args.push(v);
            }
        }
        args.join(" ")
    }

    /// Inputs that set a body's constrained top-level fields to valid
    /// values, and the expression of the body with them in place.
    fn valid_body(
        &mut self,
        body_ty: &Ty,
        body: &str,
        scope: &mut Namespace,
        inputs: &mut Vec<(String, String)>,
    ) -> String {
        let Some(record) = api::is_record(self.types, body_ty) else {
            return body.to_string();
        };
        let fields = self.types.record_fields(&record).unwrap().to_vec();
        let mut sets = vec![];
        for f in &fields {
            let (inner, maybe) = match &f.ty {
                Ty::Maybe(t) => ((**t).clone(), true),
                t => (t.clone(), false),
            };
            if let Some(rn) =
                self.refinement(&format!("{record}{}", names::upper_camel(&f.name)), &inner)
            {
                let v = scope.claim(&format!("{}Value", f.name.trim_end_matches('_')), "_");
                inputs.push((v.clone(), rn));
                sets.push(format!(
                    "{} = {}",
                    f.name,
                    if maybe { format!("Present {v}") } else { v }
                ));
            }
        }
        if sets.is_empty() {
            body.to_string()
        } else {
            format!("({body} with {})", sets.join(", "))
        }
    }

    fn resource_laws(&mut self, r: &Resource) -> Vec<(String, Vec<usize>, String)> {
        let mut out = vec![];
        let Some(create) = r.create.clone() else {
            return out;
        };
        let ops = &self.api.ops;
        let create_op = ops[create.op].clone();
        let read_op = ops[r.read].clone();
        let noun = names::upper_camel(&r.record);
        let noun = split_words(&noun);
        let record_ty = Ty::Named(r.record.clone());
        let create_returns = create_op.response.same_shape(&record_ty);
        // The key of what was created: from the body, or from the response.
        let key_from_body = matches!(
            create.key,
            KeySource::Body { .. } | KeySource::BodyIfGiven { .. }
        );
        if !key_from_body && !(create_returns && r.key_field.is_some()) {
            return out;
        }
        let body_param = create_op.body().unwrap().clone();
        let body_inner = match &body_param.ty {
            Ty::Maybe(t) => (**t).clone(),
            t => t.clone(),
        };

        // The shared opening: inputs, the body sent, and the create.
        let opening = |this: &mut Self,
                       scope: &mut Namespace,
                       inputs: &mut Vec<(String, String)>,
                       given: &mut BTreeMap<String, String>|
         -> String {
            for (n, t) in &r.parents {
                let v = scope.claim(n, "Input");
                let ty = this
                    .refinement(&format!("{}{}", r.record, names::upper_camel(n)), t)
                    .unwrap_or_else(|| t.lawspec());
                inputs.push((v.clone(), ty));
                given.insert(n.clone(), v);
            }
            let body = scope.claim("body", "_");
            inputs.push((body.clone(), body_inner.lawspec()));
            let mut sent_expr = this.valid_body(&body_inner, &body, scope, inputs);
            if let KeySource::BodyIfGiven { field } = &create.key {
                let v = scope.claim("keyGiven", "_");
                inputs.push((v.clone(), r.key_ty.lawspec()));
                sent_expr = format!("({sent_expr} with {field} = Present {v})");
            }
            let sent = "sent".to_string();
            let mut g = given.clone();
            g.insert(
                body_param.name.clone(),
                if matches!(body_param.ty, Ty::Maybe(_)) {
                    format!("Present {sent}")
                } else {
                    sent.clone()
                },
            );
            let call = this.call(&create_op, &g, inputs, scope);
            format!(
                "let {sent} = {sent_expr} in\n       first matching case of prelude.attempt ({call}) :\n       when Left failure then true\n       when Right created then\n"
            )
        };
        let with_key = |inner: &str| -> String {
            match (&create.key, &r.key_field) {
                (KeySource::Body { field }, _) => {
                    format!("let key = sent.{field} in {inner} end let")
                }
                (KeySource::BodyIfGiven { field }, _) => {
                    format!(
                        "first matching case of sent.{field} : when Absent then false when Present key then {inner} end case"
                    )
                }
                (_, Some((kf, true))) => {
                    format!(
                        "first matching case of created.{kf} : when Absent then false when Present key then {inner} end case"
                    )
                }
                (_, Some((kf, false))) => format!("let key = created.{kf} in {inner} end let"),
                _ => "false".into(),
            }
        };
        let law = |inputs: &[(String, String)], body: &str| -> String {
            let quantified: String = inputs
                .iter()
                .map(|(v, t)| format!("({v} :: {t}) "))
                .collect();
            format!(
                "law \"|@TITLE@| is\n  definition :\n    for all {quantified}=>\n      ({body}) = true end for all\n  end definition\nend law\n"
            )
        };
        let read_call = |this: &Self,
                         given: &BTreeMap<String, String>,
                         scope: &mut Namespace,
                         inputs: &mut Vec<(String, String)>|
         -> String {
            let mut g = given.clone();
            g.insert(r.key_param.clone(), "key".into());
            this.call(&read_op, &g, inputs, scope)
        };
        let fresh_scope = || {
            let mut s = Namespace::default();
            for n in [
                "created", "key", "failure", "found", "listed", "before", "after", "changed",
                "replaced", "gone", "item", "rest", "sent",
            ] {
                s.reserve(n);
            }
            s
        };

        // 1. A created resource reads back.
        if create_returns {
            let mut scope = fresh_scope();
            let mut inputs = vec![];
            let mut given = BTreeMap::new();
            let open = opening(self, &mut scope, &mut inputs, &mut given);
            let read = read_call(self, &given, &mut scope, &mut inputs);
            let body = format!(
                "{open}           {}\n       end case\n       end let",
                with_key(&format!("prelude.attempt ({read}) = Right created"))
            );
            out.push((
                format!("a created {noun} reads back"),
                vec![create.op, r.read],
                law(&inputs, &body),
            ));
        }
        // 2. A deleted resource is not found.
        if let Some(di) = r.delete {
            let delete_op = self.api.ops[di].clone();
            let mut scope = fresh_scope();
            let mut inputs = vec![];
            let mut given = BTreeMap::new();
            let open = opening(self, &mut scope, &mut inputs, &mut given);
            let mut g = given.clone();
            g.insert(r.key_param.clone(), "key".into());
            let delete = self.call(&delete_op, &g, &mut inputs, &mut scope);
            let read = read_call(self, &given, &mut scope, &mut inputs);
            let inner = format!(
                "(let gone = prelude.attempt ({delete}) in prelude.attempt ({read}) = Left NotFound end let)"
            );
            let body = format!(
                "{open}           {}\n       end case\n       end let",
                with_key(&inner)
            );
            out.push((
                format!("a deleted {noun} is not found"),
                vec![create.op, di, r.read],
                law(&inputs, &body),
            ));
        }
        // 3. The list holds a created resource.
        if let (Some(li), true) = (r.list, create_returns) {
            let list_op = self.api.ops[li].clone();
            let contains = self.contains_helper(&r.record);
            let mut scope = fresh_scope();
            let mut inputs = vec![];
            let mut given = BTreeMap::new();
            let open = opening(self, &mut scope, &mut inputs, &mut given);
            let list = self.call(&list_op, &given, &mut inputs, &mut scope);
            let body = format!(
                "{open}           first matching case of prelude.attempt ({list}) :\n           when Left failure then false\n           when Right listed then model.{contains} created listed\n           end case\n       end case\n       end let"
            );
            out.push((
                format!("the list holds a created {noun}"),
                vec![create.op, li],
                law(&inputs, &body),
            ));
        }
        // 4. An update changes only the fields it gives.
        if let Some(pi) = r.patch {
            let patch_op = self.api.ops[pi].clone();
            let patch_param = patch_op.body().unwrap().clone();
            let patch_ty = match &patch_param.ty {
                Ty::Maybe(t) => (**t).clone(),
                t => t.clone(),
            };
            let fields = self.types.record_fields(&r.record).unwrap().to_vec();
            let assigned: BTreeMap<String, String> = self
                .patch_assignments(r, &patch_ty, "change", "before")
                .unwrap_or_default()
                .into_iter()
                .filter_map(|a| {
                    a.split_once(" = ")
                        .map(|(f, v)| (f.to_string(), v.to_string()))
                })
                .collect();
            let mut checks = vec![];
            for f in &fields {
                if r.key_field.as_ref().is_some_and(|(k, _)| *k == f.name) {
                    continue;
                }
                let expected = assigned
                    .get(&f.name)
                    .cloned()
                    .unwrap_or_else(|| format!("before.{}", f.name));
                checks.push(format!("after.{} = {expected}", f.name));
            }
            if !checks.is_empty() {
                let mut scope = fresh_scope();
                let mut inputs = vec![];
                let mut given = BTreeMap::new();
                let open = opening(self, &mut scope, &mut inputs, &mut given);
                let change = scope.claim("change", "_");
                inputs.push((change.clone(), patch_ty.lawspec()));
                let mut g = given.clone();
                g.insert(r.key_param.clone(), "key".into());
                g.insert(
                    patch_param.name.clone(),
                    if matches!(patch_param.ty, Ty::Maybe(_)) {
                        format!("Present {change}")
                    } else {
                        change.clone()
                    },
                );
                let patch = self.call(&patch_op, &g, &mut inputs, &mut scope);
                let read = read_call(self, &given, &mut scope, &mut inputs);
                let checks = checks
                    .join("\n                   && ")
                    .replace("change.", &format!("{change}."));
                let inner = format!(
                    "first matching case of prelude.attempt ({read}) :\n           when Left failure then false\n           when Right before then\n               first matching case of prelude.attempt ({patch}) :\n               when Left failure then true\n               when Right changed then\n                   first matching case of prelude.attempt ({read}) :\n                   when Left failure then false\n                   when Right after then {checks}\n                   end case\n               end case\n           end case"
                );
                let body = format!(
                    "{open}           {}\n       end case\n       end let",
                    with_key(&inner)
                );
                out.push((
                    format!("an update changes only the fields it gives to a {noun}"),
                    vec![create.op, pi, r.read],
                    law(&inputs, &body),
                ));
            }
        }
        // 5. A replaced resource reads back as replaced.
        if let Some(pi) = r.replace {
            let replace_op = self.api.ops[pi].clone();
            if replace_op.response.same_shape(&record_ty) {
                let body_param2 = replace_op.body().unwrap().clone();
                let body2_ty = match &body_param2.ty {
                    Ty::Maybe(t) => (**t).clone(),
                    t => t.clone(),
                };
                let mut scope = fresh_scope();
                let mut inputs = vec![];
                let mut given = BTreeMap::new();
                let open = opening(self, &mut scope, &mut inputs, &mut given);
                let replacement = scope.claim("replacement", "_");
                inputs.push((replacement.clone(), body2_ty.lawspec()));
                let mut sent2 = self.valid_body(&body2_ty, &replacement, &mut scope, &mut inputs);
                if r.replace_by_body_key {
                    if let Some((kf, maybe)) = &r.key_field {
                        sent2 = format!(
                            "({sent2} with {kf} = {})",
                            if *maybe { "Present key" } else { "key" }
                        );
                    }
                }
                let mut g = given.clone();
                g.insert(r.key_param.clone(), "key".into());
                g.insert(
                    body_param2.name.clone(),
                    if matches!(body_param2.ty, Ty::Maybe(_)) {
                        format!("Present {sent2}")
                    } else {
                        sent2
                    },
                );
                let replace = self.call(&replace_op, &g, &mut inputs, &mut scope);
                let read = read_call(self, &given, &mut scope, &mut inputs);
                let inner = format!(
                    "first matching case of prelude.attempt ({replace}) :\n           when Left failure then true\n           when Right replaced then prelude.attempt ({read}) = Right replaced\n           end case"
                );
                let body = format!(
                    "{open}           {}\n       end case\n       end let",
                    with_key(&inner)
                );
                out.push((
                    format!("a replaced {noun} reads back as replaced"),
                    vec![create.op, pi, r.read],
                    law(&inputs, &body),
                ));
            }
        }
        out
    }
}

/// The kind of a resource law, from its title, as the wire map names it.
fn law_kind(title: &str) -> &'static str {
    if title.starts_with("a created ") {
        "createdReadsBack"
    } else if title.starts_with("a deleted ") {
        "deletedIsNotFound"
    } else if title.starts_with("the list holds ") {
        "listHoldsCreated"
    } else if title.starts_with("an update changes only ") {
        "updateChangesOnlyGivenFields"
    } else {
        "replacedReadsBack"
    }
}

fn paren(v: &str) -> String {
    if v.contains(' ') && !(v.starts_with('(') && v.ends_with(')')) {
        format!("({v})")
    } else {
        v.to_string()
    }
}

fn indent(text: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    text.lines()
        .map(|l| {
            if l.is_empty() {
                "\n".to_string()
            } else {
                format!("{pad}{l}\n")
            }
        })
        .collect()
}

/// `NullableSimpleUser` to `nullable simple user`.
fn split_words(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            out.push(' ');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}
