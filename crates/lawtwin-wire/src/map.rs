//! `lawtwin-openapi.json`, the wire map `lawtwin import openapi` writes
//! beside a contract package (docs/reference/openapi-import.md).

use crate::ty::Ty;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The document the contract was imported from.
#[derive(Clone, Debug, Deserialize)]
pub struct Source {
    pub file: String,
    pub title: String,
    pub version: String,
}

/// A failing status and the `ApiError` constructor it stands for.
#[derive(Clone, Debug, Deserialize)]
pub struct Failure {
    pub status: String,
    pub constructor: String,
}

/// A documented body: its media type and its schema, references written out.
#[derive(Clone, Debug, Deserialize)]
pub struct Body {
    pub content: String,
    pub schema: Value,
}

/// A failure an operation documents, with its body if it has one.
#[derive(Clone, Debug, Deserialize)]
pub struct OpFailure {
    pub status: String,
    pub constructor: String,
    #[serde(default)]
    pub body: Option<Body>,
}

/// A parameter of an operation, or its body (`in` is `body`).
#[derive(Clone, Debug, Deserialize)]
pub struct Param {
    pub name: String,
    pub wire: String,
    #[serde(rename = "in")]
    pub location: String,
    pub required: bool,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub constraints: Vec<Value>,
}

/// A documented response header.
#[derive(Clone, Debug, Deserialize)]
pub struct Header {
    pub name: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub schema: Value,
    #[serde(default)]
    pub example: Value,
}

/// An operation's success response.
#[derive(Clone, Debug, Deserialize)]
pub struct Response {
    pub status: String,
    pub content: Option<String>,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub headers: Vec<Header>,
}

/// One operation: a method on a path, an operation of one ability.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub operation: String,
    pub operation_id: Option<String>,
    pub ability: String,
    pub method: String,
    pub path: String,
    pub parameters: Vec<Param>,
    pub request_content: Option<String>,
    pub response: Response,
    pub failures: Vec<OpFailure>,
    pub modelled: bool,
}

impl Operation {
    /// The body parameter, if the operation takes one.
    pub fn body(&self) -> Option<&Param> {
        self.parameters.iter().find(|p| p.location == "body")
    }

    /// The success status, as a number (`2XX` reads as 200).
    pub fn success_status(&self) -> u16 {
        status_number(&self.response.status).unwrap_or(200)
    }

    /// The failure this operation documents for a constructor.
    pub fn failure(&self, constructor: &str) -> Option<&OpFailure> {
        self.failures.iter().find(|f| f.constructor == constructor)
    }

    /// Whether it documents a failing status.
    pub fn documents(&self, status: u16) -> bool {
        self.failures.iter().any(|f| status_matches(&f.status, status))
    }

    /// The status a request that breaks the document is refused with: 422
    /// when the operation documents it and not 400, otherwise 400.
    pub fn invalid_status(&self) -> u16 {
        if !self.documents(400) && self.documents(422) { 422 } else { 400 }
    }
}

/// Whether a documented status (`404`, `4XX`) covers a status.
pub fn status_matches(documented: &str, status: u16) -> bool {
    match documented.as_bytes() {
        [d, b'X' | b'x', b'X' | b'x'] => (*d as char).to_digit(10) == Some((status / 100) as u32),
        _ => documented.parse::<u16>().ok() == Some(status),
    }
}

/// A documented status as a number; `4XX` reads as 400.
pub fn status_number(documented: &str) -> Option<u16> {
    match documented.as_bytes() {
        [d, b'X' | b'x', b'X' | b'x'] => (*d as char).to_digit(10).map(|d| d as u16 * 100),
        _ => documented.parse().ok(),
    }
}

/// A field of a record.
#[derive(Clone, Debug, Deserialize)]
pub struct Field {
    pub name: String,
    pub wire: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default = "yes")]
    pub required: bool,
    #[serde(default)]
    pub constraints: Vec<Value>,
}

fn yes() -> bool {
    true
}

/// An enum's value: its constructor and its text on the wire.
#[derive(Clone, Debug, Deserialize)]
pub struct EnumValue {
    pub constructor: String,
    pub wire: String,
}

/// A `oneOf` alternative.
#[derive(Clone, Debug, Deserialize)]
pub struct Alternative {
    pub constructor: String,
    #[serde(rename = "type")]
    pub ty: String,
}

/// A named type of the contract.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind")]
pub enum TypeDef {
    #[serde(rename = "record")]
    Record {
        #[serde(rename = "type")]
        name: String,
        fields: Vec<Field>,
    },
    #[serde(rename = "enum")]
    Enum {
        #[serde(rename = "type")]
        name: String,
        values: Vec<EnumValue>,
    },
    #[serde(rename = "oneOf")]
    OneOf {
        #[serde(rename = "type")]
        name: String,
        alternatives: Vec<Alternative>,
    },
}

impl TypeDef {
    pub fn name(&self) -> &str {
        match self {
            TypeDef::Record { name, .. } | TypeDef::Enum { name, .. } | TypeDef::OneOf { name, .. } => name,
        }
    }
}

/// A field of a key record, or a parent of a resource.
#[derive(Clone, Debug, Deserialize)]
pub struct KeyPart {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

/// The type a resource is kept under: its key's own type, or a record of
/// the parents and the key.
#[derive(Clone, Debug, Deserialize)]
pub struct KeyType {
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub fields: Vec<KeyPart>,
}

/// A resource the importer recognised.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    pub record: String,
    pub collection: String,
    pub item: String,
    pub key: String,
    pub key_field: Option<String>,
    pub key_type: KeyType,
    #[serde(default)]
    pub parents: Vec<KeyPart>,
    pub state_field: String,
    pub counter_field: Option<String>,
    pub keys: String,
    pub read: String,
    pub create: Option<String>,
    pub delete: Option<String>,
    pub list: Option<String>,
    pub replace: Option<String>,
    #[serde(default)]
    pub replace_by_body_key: bool,
    pub update: Option<String>,
    /// Each generated law, by kind (`createdReadsBack`, `deletedIsNotFound`,
    /// `listHoldsCreated`, `updateChangesOnlyGivenFields`, `replacedReadsBack`).
    #[serde(default)]
    pub laws: BTreeMap<String, String>,
}

/// A field of the model's State.
#[derive(Clone, Debug, Deserialize)]
pub struct StateField {
    pub field: String,
    #[serde(rename = "type")]
    pub ty: String,
}

/// The wire map of a contract.
#[derive(Clone, Debug, Deserialize)]
pub struct WireMap {
    pub package: String,
    pub source: Source,
    pub failures: Vec<Failure>,
    pub operations: Vec<Operation>,
    pub types: Vec<TypeDef>,
    pub resources: Vec<Resource>,
    #[serde(default)]
    pub state: Vec<StateField>,
}

impl WireMap {
    /// Reads a wire map from its JSON text.
    pub fn parse(text: &str) -> Result<WireMap, String> {
        serde_json::from_str(text).map_err(|e| format!("not a wire map: {e}"))
    }

    /// Reads `lawtwin-openapi.json` from a contract directory, or a wire map file.
    pub fn load(path: &Path) -> Result<WireMap, String> {
        let file = if path.is_dir() { path.join("lawtwin-openapi.json") } else { path.to_path_buf() };
        let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        WireMap::parse(&text).map_err(|e| format!("{}: {e}", file.display()))
    }

    /// The operation of that name.
    pub fn op(&self, name: &str) -> Option<&Operation> {
        self.operations.iter().find(|o| o.operation == name)
    }

    /// The named type of that name.
    pub fn type_def(&self, name: &str) -> Option<&TypeDef> {
        self.types.iter().find(|t| t.name() == name)
    }

    /// The key record of that name, as a resource keeps it.
    pub fn key_record(&self, name: &str) -> Option<&KeyType> {
        self.resources.iter().map(|r| &r.key_type).find(|k| k.ty == name && !k.fields.is_empty())
    }

    /// The status a failure constructor stands for (`Unmodelled` is 501).
    pub fn failure_status(&self, constructor: &str) -> u16 {
        self.failures
            .iter()
            .find(|f| f.constructor == constructor)
            .and_then(|f| status_number(&f.status))
            .unwrap_or(if constructor == "Unmodelled" { 501 } else { 500 })
    }

    /// The failure constructor for a status, if the contract has one.
    pub fn failure_constructor(&self, status: u16) -> Option<&str> {
        self.failures
            .iter()
            .find(|f| f.status.parse::<u16>().ok() == Some(status))
            .or_else(|| self.failures.iter().find(|f| status_matches(&f.status, status)))
            .map(|f| f.constructor.as_str())
    }

    /// The resource whose operation this is, and its part in it.
    pub fn role(&self, op: &str) -> Option<(&Resource, Role)> {
        self.resources.iter().find_map(|r| {
            let is = |o: &Option<String>| o.as_deref() == Some(op);
            let role = if r.read == op {
                Role::Read
            } else if is(&r.create) {
                Role::Create
            } else if is(&r.delete) {
                Role::Delete
            } else if is(&r.list) {
                Role::List
            } else if is(&r.replace) {
                Role::Replace
            } else if is(&r.update) {
                Role::Update
            } else {
                return None;
            };
            Some((r, role))
        })
    }

    /// The type of a parameter or response, read.
    pub fn ty(text: &str) -> Ty {
        Ty::parse(text).unwrap_or(Ty::Text)
    }
}

/// The part an operation plays in a resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Read,
    Create,
    Delete,
    List,
    Replace,
    Update,
}

/// Finds a contract's directory: a directory given as is, else
/// `contracts/<name>` under the current directory or `LAWTWIN_CONTRACTS`,
/// where the name may be the package (`lawtwin.petstore`) or its last word.
pub fn find_contract(name: &str) -> Result<PathBuf, String> {
    let direct = PathBuf::from(name);
    if direct.join("lawtwin-openapi.json").exists() || (direct.is_file() && name.ends_with(".json")) {
        return Ok(direct);
    }
    let short = name.rsplit('.').next().unwrap_or(name);
    let roots: Vec<PathBuf> = std::env::var("LAWTWIN_CONTRACTS")
        .ok()
        .map(PathBuf::from)
        .into_iter()
        .chain([PathBuf::from("contracts")])
        .collect();
    for root in roots {
        let dir = root.join(short);
        if dir.join("lawtwin-openapi.json").exists() {
            return Ok(dir);
        }
    }
    Err(format!("no imported contract {name}: give its directory, or run from the repository (contracts/<name>), or set LAWTWIN_CONTRACTS"))
}
