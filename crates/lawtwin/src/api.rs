//! An OpenAPI document's operations, their failures, and the resources the
//! importer recognises among them.

use crate::ir::{Def, Field, Scalar, Ty, Types};
use crate::names::{self, Namespace};
use crate::openapi::{self, Document};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Param {
    pub wire: String,
    /// path, query, header, cookie or body.
    pub location: String,
    pub name: String,
    pub ty: Ty,
    pub required: bool,
}

#[derive(Clone, Debug)]
pub struct Op {
    pub name: String,
    pub operation_id: Option<String>,
    pub method: String,
    pub path: String,
    pub ability: String,
    pub params: Vec<Param>,
    pub body_content: Option<String>,
    pub response: Ty,
    pub response_status: String,
    pub response_content: Option<String>,
    /// Each documented failing status and the ApiError constructor it raises.
    pub errors: Vec<(String, String)>,
    pub summary: String,
}

impl Op {
    pub fn body(&self) -> Option<&Param> {
        self.params.iter().find(|p| p.location == "body")
    }

    pub fn path_params(&self) -> Vec<&Param> {
        self.params
            .iter()
            .filter(|p| p.location == "path")
            .collect()
    }

    pub fn documents(&self, ctor: &str) -> bool {
        self.errors.iter().any(|(_, c)| c == ctor)
    }

    /// The constructor for refusing a request that breaks its constraints.
    pub fn invalid_error(&self) -> &'static str {
        if self.documents("UnprocessableEntity") {
            "UnprocessableEntity"
        } else {
            "BadRequest"
        }
    }

    /// The constructor for a key that is already taken.
    pub fn conflict_error(&self) -> Option<&'static str> {
        ["Conflict", "UnprocessableEntity", "BadRequest"]
            .into_iter()
            .find(|c| self.documents(c))
    }
}

/// Where a created resource's key comes from.
#[derive(Clone, Debug)]
pub enum KeySource {
    /// A required field of the body.
    Body { field: String },
    /// An optional field of the body; when absent, the service assigns one.
    BodyOrAssigned { field: String },
    /// An optional field of the body, which a create must give.
    BodyIfGiven { field: String },
    /// The service assigns it.
    Assigned,
}

#[derive(Clone, Debug)]
pub struct Create {
    pub op: usize,
    pub key: KeySource,
}

#[derive(Clone, Debug)]
pub struct Resource {
    /// The record the service keeps, such as Pet.
    pub record: String,
    pub collection_path: String,
    pub item_path: String,
    /// Path parameters of the collection, by LawSpec name, with their types.
    pub parents: Vec<(String, Ty)>,
    pub key_param: String,
    pub key_ty: Ty,
    /// The record's field holding the key, and whether it is a Maybe.
    pub key_field: Option<(String, bool)>,
    /// The key type of the state's KeyVal: the key's own type, or a record of
    /// the parents and the key.
    pub key_record: Option<String>,
    pub map_field: String,
    pub counter_field: Option<String>,
    pub read: usize,
    pub create: Option<Create>,
    pub delete: Option<usize>,
    pub list: Option<usize>,
    /// A PUT on the item, or a PUT or PATCH on the collection whose body
    /// carries the key.
    pub replace: Option<usize>,
    pub replace_by_body_key: bool,
    pub patch: Option<usize>,
}

pub struct Api {
    pub ops: Vec<Op>,
    pub abilities: Vec<String>,
    pub errors: Vec<(String, String)>,
    pub resources: Vec<Resource>,
    pub notes: Vec<String>,
}

/// An ApiError constructor for a status.
fn status_ctor(status: &str) -> String {
    let named = match status {
        "400" => "BadRequest",
        "401" => "Unauthorized",
        "402" => "PaymentRequired",
        "403" => "Forbidden",
        "404" => "NotFound",
        "405" => "MethodNotAllowed",
        "406" => "NotAcceptable",
        "408" => "RequestTimeout",
        "409" => "Conflict",
        "410" => "Gone",
        "411" => "LengthRequired",
        "412" => "PreconditionFailed",
        "413" => "ContentTooLarge",
        "415" => "UnsupportedMediaType",
        "416" => "RangeNotSatisfiable",
        "418" => "ImATeapot",
        "422" => "UnprocessableEntity",
        "423" => "Locked",
        "428" => "PreconditionRequired",
        "429" => "TooManyRequests",
        "451" => "UnavailableForLegalReasons",
        "500" => "InternalServerError",
        "501" => "NotImplemented",
        "502" => "BadGateway",
        "503" => "ServiceUnavailable",
        "504" => "GatewayTimeout",
        "4XX" | "4xx" => "ClientError",
        "5XX" | "5xx" => "ServerError",
        "3XX" | "3xx" => "Redirection",
        _ => "",
    };
    if named.is_empty() {
        format!("Status{status}")
    } else {
        named.to_string()
    }
}

fn is_success(status: &str) -> bool {
    status.starts_with('2')
}

/// Whether a path parameter names a record's field: the same name, or the
/// resource's name and then the field (`petId` and `id`, `milestone_number`
/// and `number`), or any name ending in id for `id`.
pub fn param_matches_field(param: &str, field: &str, record: &str) -> bool {
    let norm = |s: &str| {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase()
    };
    let (p, f, r) = (norm(param), norm(field), norm(record));
    p == f || p == format!("{r}{f}") || (f == "id" && p.ends_with("id"))
}

impl Api {
    pub fn build(
        doc: &Document,
        types: &mut Types,
        include: &dyn Fn(&str, &str, &[String]) -> bool,
    ) -> Result<Api, String> {
        let mut ops = vec![];
        let mut lower = Namespace::default();
        let mut abilities: Vec<String> = vec![];
        let mut ability_names = Namespace::default();
        let mut tag_ability: BTreeMap<String, String> = BTreeMap::new();
        let mut notes = vec![];
        // Failures are named before any schema, so a schema called NotFound
        // becomes NotFoundSchema rather than clash with the failure.
        for status in [
            "400", "401", "402", "403", "404", "405", "406", "408", "409", "410", "411", "412",
            "413", "415", "416", "418", "422", "423", "428", "429", "451", "500", "501", "502",
            "503", "504", "4XX", "5XX", "3XX",
        ] {
            types.upper.reserve(&status_ctor(status));
        }
        types.upper.reserve("Unmodelled");
        for (path, item) in doc.paths() {
            let shared: Vec<Value> = item
                .get("parameters")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for method in openapi::METHODS {
                let Some(op) = item.get(*method) else {
                    continue;
                };
                let tags: Vec<String> = op
                    .get("tags")
                    .and_then(Value::as_array)
                    .map(|t| {
                        t.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                if !include(path, method, &tags) {
                    continue;
                }
                let tag = tags.first().cloned().unwrap_or_else(|| "api".into());
                let ability = tag_ability
                    .entry(tag.clone())
                    .or_insert_with(|| {
                        let base = names::upper_camel(&tag);
                        let base = base.strip_suffix("Api").unwrap_or(&base).to_string();
                        let name = ability_names.claim(&format!("{base}Api"), "");
                        types.upper.reserve(&name);
                        abilities.push(name.clone());
                        name
                    })
                    .clone();
                let operation_id = op
                    .get("operationId")
                    .and_then(Value::as_str)
                    .map(String::from);
                let wanted = match &operation_id {
                    Some(id) => names::value_name(id),
                    None => names::value_name(&format!("{method} {path}")),
                };
                let name = lower.claim(&wanted, "Op");
                let ctx = names::upper_camel(&name);
                // Parameters: the operation's override the path's, by name and place.
                let mut raw: Vec<Value> = vec![];
                for p in shared.iter().chain(
                    op.get("parameters")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten(),
                ) {
                    let p = doc.deref(p)?.clone();
                    let key = (p.get("name").cloned(), p.get("in").cloned());
                    raw.retain(|q| (q.get("name").cloned(), q.get("in").cloned()) != key);
                    raw.push(p);
                }
                // Path parameters first, in the order the path names them.
                let order = |p: &Value| -> (usize, usize) {
                    let place = p.get("in").and_then(Value::as_str).unwrap_or("");
                    let name = p.get("name").and_then(Value::as_str).unwrap_or("");
                    match place {
                        "path" => (0, path.find(&format!("{{{name}}}")).unwrap_or(0)),
                        "query" => (1, 0),
                        "header" => (2, 0),
                        _ => (3, 0),
                    }
                };
                raw.sort_by_key(order);
                let mut taken = Namespace::default();
                let mut params = vec![];
                for p in &raw {
                    let wire = p
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("param")
                        .to_string();
                    let location = p
                        .get("in")
                        .and_then(Value::as_str)
                        .unwrap_or("query")
                        .to_string();
                    let required = location == "path"
                        || p.get("required").and_then(Value::as_bool).unwrap_or(false);
                    let schema = p.get("schema").cloned().or_else(|| {
                        p.get("content")
                            .and_then(Value::as_object)
                            .and_then(openapi::pick_content)
                            .and_then(|(_, m)| m.get("schema").cloned())
                    });
                    let mut ty = match schema {
                        Some(s) => types.ty(&s, &format!("{ctx}{}", names::upper_camel(&wire))),
                        None => Ty::text(),
                    };
                    if !required {
                        ty = ty.maybe();
                    }
                    let name = taken.claim(&names::value_name(&wire), "_");
                    params.push(Param {
                        wire,
                        location,
                        name,
                        ty,
                        required,
                    });
                }
                let mut body_content = None;
                if let Some(rb) = op.get("requestBody") {
                    let rb = doc.deref(rb)?;
                    if let Some((media, m)) = rb
                        .get("content")
                        .and_then(Value::as_object)
                        .and_then(openapi::pick_content)
                    {
                        let required = rb.get("required").and_then(Value::as_bool).unwrap_or(false);
                        let mut ty = match m.get("schema") {
                            Some(s)
                                if media == "application/octet-stream"
                                    && s.get("$ref").is_none() =>
                            {
                                Ty::Scalar(Scalar::Bytes, vec![])
                            }
                            Some(s) => types.ty(s, &format!("{ctx}Body")),
                            None => Ty::Scalar(Scalar::Bytes, vec![]),
                        };
                        if !required {
                            ty = ty.maybe();
                        }
                        let name = taken.claim("body", "_");
                        params.push(Param {
                            wire: "body".into(),
                            location: "body".into(),
                            name,
                            ty,
                            required,
                        });
                        body_content = Some(media.to_string());
                    }
                }
                // The success response: 200, then 201, then any 2xx, then 2XX.
                let responses = op
                    .get("responses")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                let mut statuses: Vec<&String> =
                    responses.keys().filter(|s| is_success(s)).collect();
                statuses.sort_by_key(|s| match s.as_str() {
                    "200" => 0,
                    "201" => 1,
                    "2XX" | "2xx" => 3,
                    _ => 2,
                });
                let (mut response, mut response_status, mut response_content) =
                    (Ty::unit(), "200".to_string(), None);
                if let Some(status) = statuses.first() {
                    response_status = status.to_string();
                    let r = doc.deref(&responses[*status])?;
                    if let Some((media, m)) = r
                        .get("content")
                        .and_then(Value::as_object)
                        .and_then(openapi::pick_content)
                    {
                        response_content = Some(media.to_string());
                        response = match m.get("schema") {
                            Some(s) => types.ty(s, &format!("{ctx}Response")),
                            None => Ty::Scalar(Scalar::Bytes, vec![]),
                        };
                    }
                }
                let mut errors = vec![];
                for status in responses.keys() {
                    if is_success(status) || status == "default" || status.starts_with('1') {
                        continue;
                    }
                    errors.push((status.clone(), status_ctor(status)));
                }
                if responses.contains_key("default") {
                    notes.push(format!(
                        "{name}: the `default` response is not a typed failure"
                    ));
                }
                let summary = op
                    .get("summary")
                    .or_else(|| op.get("description"))
                    .and_then(Value::as_str)
                    .map(names::summary)
                    .unwrap_or_default();
                ops.push(Op {
                    name,
                    operation_id,
                    method: method.to_string(),
                    path: path.to_string(),
                    ability,
                    params,
                    body_content,
                    response,
                    response_status,
                    response_content,
                    errors,
                    summary,
                });
            }
        }
        if ops.is_empty() {
            return Err("no operation was selected".into());
        }
        // The failures: every documented one, and three the model needs.
        let mut errors: Vec<(String, String)> = vec![];
        for op in &ops {
            for (status, ctor) in &op.errors {
                if !errors.iter().any(|(_, c)| c == ctor) {
                    errors.push((status.clone(), ctor.clone()));
                }
            }
        }
        for (status, ctor) in [
            ("400", "BadRequest"),
            ("404", "NotFound"),
            ("501", "Unmodelled"),
        ] {
            if !errors.iter().any(|(_, c)| c == ctor) {
                errors.push((status.into(), ctor.into()));
            }
        }
        errors.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, ctor) in &errors {
            types.upper.reserve(ctor);
        }
        let mut api = Api {
            ops,
            abilities,
            errors,
            resources: vec![],
            notes,
        };
        api.recognise(types);
        Ok(api)
    }

    fn recognise(&mut self, types: &mut Types) {
        let mut state_names = Namespace::default();
        state_names.reserve("unused");
        for (gi, get) in self.ops.iter().enumerate() {
            if get.method != "get" {
                continue;
            }
            let Some(last) = get.path.rsplit('/').next() else {
                continue;
            };
            if !(last.starts_with('{') && last.ends_with('}')) {
                continue;
            }
            let Ty::Named(record) = &get.response else {
                continue;
            };
            let Some(fields) = types.record_fields(record) else {
                continue;
            };
            let key_wire = &last[1..last.len() - 1];
            let Some(key) = get
                .params
                .iter()
                .find(|p| p.location == "path" && p.wire == key_wire)
            else {
                continue;
            };
            if !key.ty.keyable() {
                continue;
            }
            let collection = get.path[..get.path.len() - last.len() - 1].to_string();
            if self.resources.iter().any(|r| r.item_path == get.path) {
                continue;
            }
            let parents: Vec<(String, Ty)> = get
                .path_params()
                .into_iter()
                .filter(|p| p.wire != key_wire)
                .map(|p| (p.name.clone(), p.ty.clone()))
                .collect();
            let key_field = fields
                .iter()
                .find(|f| {
                    param_matches_field(key_wire, &f.wire, record)
                        && (f.ty.same_shape(&key.ty) || f.ty.same_shape(&key.ty.clone().maybe()))
                })
                .map(|f| (f.name.clone(), matches!(f.ty, Ty::Maybe(_))));
            let at = |method: &str, path: &str| {
                self.ops
                    .iter()
                    .position(|o| o.method == method && o.path == path)
            };
            let returns = |i: usize, ty: &Ty| self.ops[i].response.same_shape(ty);
            let record_ty = Ty::Named(record.clone());
            // Create: a POST on the collection with a record body.
            let create = at("post", &collection).and_then(|ci| {
                let body = self.ops[ci].body()?;
                let body_record = match &body.ty {
                    Ty::Named(n) => n.clone(),
                    Ty::Maybe(t) => match &**t {
                        Ty::Named(n) => n.clone(),
                        _ => return None,
                    },
                    _ => return None,
                };
                let body_fields = types.record_fields(&body_record)?;
                if !(returns(ci, &record_ty) || self.ops[ci].response.is_unit()) {
                    return None;
                }
                let from_body = body_fields.iter().find(|f| {
                    param_matches_field(key_wire, &f.wire, record)
                        || key_field.as_ref().is_some_and(|(k, _)| *k == f.name)
                });
                let integer = matches!(key.ty, Ty::Scalar(Scalar::Int32 | Scalar::Int64, _));
                let source = match from_body {
                    Some(f) if f.ty.same_shape(&key.ty) => KeySource::Body {
                        field: f.name.clone(),
                    },
                    Some(f) if f.ty.same_shape(&key.ty.clone().maybe()) && integer => {
                        KeySource::BodyOrAssigned {
                            field: f.name.clone(),
                        }
                    }
                    Some(f) if f.ty.same_shape(&key.ty.clone().maybe()) => KeySource::BodyIfGiven {
                        field: f.name.clone(),
                    },
                    _ if integer && key_field.is_some() => KeySource::Assigned,
                    _ => return None,
                };
                Some(Create {
                    op: ci,
                    key: source,
                })
            });
            let delete = at("delete", &get.path);
            let list = at("get", &collection).filter(
                |&li| matches!(&self.ops[li].response, Ty::List(t, _) if t.same_shape(&record_ty)),
            );
            // Replace: PUT on the item, or PUT or PATCH on the collection
            // with the key in a body of the record's own type.
            let mut replace_by_body_key = false;
            let replace = at("put", &get.path)
                .filter(|&pi| {
                    self.ops[pi].body().is_some_and(|b| {
                        matches!(&b.ty, Ty::Named(_))
                            || matches!(&b.ty, Ty::Maybe(t) if matches!(&**t, Ty::Named(_)))
                    })
                })
                .or_else(|| {
                    let found = ["put", "patch"]
                        .iter()
                        .find_map(|m| at(m, &collection))
                        .filter(|&pi| {
                            self.ops[pi].body().is_some_and(|b| {
                                b.ty.same_shape(&record_ty)
                                    || b.ty.same_shape(&record_ty.clone().maybe())
                            }) && key_field.is_some()
                                && parents.is_empty()
                        });
                    replace_by_body_key = found.is_some();
                    found
                })
                .filter(|&pi| returns(pi, &record_ty) || self.ops[pi].response.is_unit());
            let patch = at("patch", &get.path).filter(|&pi| {
                let op = &self.ops[pi];
                let body_ok = match op.body().map(|b| &b.ty) {
                    Some(Ty::Named(n)) => types.record_fields(n).is_some(),
                    Some(Ty::Maybe(b)) => {
                        matches!(&**b, Ty::Named(n) if types.record_fields(n).is_some())
                    }
                    _ => false,
                };
                body_ok && (returns(pi, &record_ty) || op.response.is_unit())
            });
            let replace = if patch.is_some() && replace == patch {
                None
            } else {
                replace
            };
            let base = names::lower_camel(record);
            let map_field = state_names.claim(&format!("{base}s"), "ByKey");
            let counter_field = match &create {
                Some(Create {
                    key: KeySource::Assigned | KeySource::BodyOrAssigned { .. },
                    ..
                }) => Some(state_names.claim(&format!("next{record}Key"), "_")),
                _ => None,
            };
            let key_record = if parents.is_empty() {
                None
            } else {
                Some(types.upper.claim(&format!("{record}Key"), "Value"))
            };
            self.resources.push(Resource {
                record: record.clone(),
                collection_path: collection,
                item_path: get.path.clone(),
                parents,
                key_param: key.name.clone(),
                key_ty: key.ty.clone(),
                key_field,
                key_record,
                map_field,
                counter_field,
                read: gi,
                create,
                delete,
                list,
                replace,
                replace_by_body_key,
                patch,
            });
        }
        let _ = &types.defs;
    }

    /// The resource an operation belongs to, and the part it plays.
    pub fn role(&self, op: usize) -> Option<(usize, Role)> {
        for (ri, r) in self.resources.iter().enumerate() {
            let role = if r.read == op {
                Role::Read
            } else if r.create.as_ref().is_some_and(|c| c.op == op) {
                Role::Create
            } else if r.delete == Some(op) {
                Role::Delete
            } else if r.list == Some(op) {
                Role::List
            } else if r.replace == Some(op) {
                Role::Replace
            } else if r.patch == Some(op) {
                Role::Patch
            } else {
                continue;
            };
            return Some((ri, role));
        }
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Read,
    Create,
    Delete,
    List,
    Replace,
    Patch,
}

/// A record's field by name.
pub fn field<'a>(fields: &'a [Field], name: &str) -> Option<&'a Field> {
    fields.iter().find(|f| f.name == name)
}

pub fn is_record(types: &Types, ty: &Ty) -> Option<String> {
    match ty {
        Ty::Named(n) if matches!(types.def(n), Some(Def::Record { .. })) => Some(n.clone()),
        _ => None,
    }
}
