//! Compiles every imported OpenAPI contract into the HTTP twin's Rust core.
//!
//! Each contract under `contracts/` that has a wire map
//! (`lawtwin-openapi.json`) and a generated Rust test project is a contract
//! the core can serve: the registry of compiled contracts
//! (ref:DEC-http-contract-registry). LawSpec generates each project as a crate
//! of its own, all named alike, so this script brings their sources in as
//! modules instead, one per contract, with `crate::` paths moved under it.
//!
//! The generated `reference` handler answers each operation through a typed
//! definition of the model, and panics with the `ApiError` it raises, which
//! suits a law but not a server. For each operation this script writes a
//! case of `Model::call` that runs the same typed definition, with the same
//! argument conversions, over a state the twin owns, and gives the raised
//! failure back as a value. Nothing is restated: the code the twin runs is
//! the code the laws check (ref:DEC-embed-generated-model).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

struct Op {
    name: String,
    function: String,
    conversions: Vec<String>,
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let contracts = manifest.join("../../../contracts").canonicalize().expect("the contracts directory");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", contracts.display());
    let mut names: Vec<String> = std::fs::read_dir(&contracts)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("lawtwin-openapi.json").exists() && e.path().join("test/rust/src/lawspec_abilities.rs").exists())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut text = String::from("// Written by build.rs: every imported contract the core serves.\n\n");
    let mut registry = String::new();
    for name in &names {
        let dir = contracts.join(name);
        println!("cargo:rerun-if-changed={}", dir.join("lawtwin-openapi.json").display());
        println!("cargo:rerun-if-changed={}", dir.join("lawtwin.json").display());
        println!("cargo:rerun-if-changed={}", dir.join("test/rust/src").display());
        let module = embed(name, &dir, &out).unwrap_or_else(|e| panic!("embedding the contract {name}: {e}"));
        text.push_str(&module);
        writeln!(
            registry,
            "        Contract {{ name: {name:?}, package: {name}::PACKAGE, version: {name}::VERSION, wire_map: {name}::WIRE_MAP, twin_file: {name}::TWIN_FILE, start: {name}::start }},"
        )
        .unwrap();
    }
    writeln!(
        text,
        "/// Every contract compiled into the core, by name.\npub fn registry() -> Vec<Contract> {{\n    vec![\n{registry}    ]\n}}"
    )
    .unwrap();
    std::fs::write(out.join("contracts.rs"), text).unwrap();
}

fn read(p: &Path) -> Result<String, String> {
    std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))
}

/// Copies one contract's generated sources under OUT_DIR and writes its module.
fn embed(name: &str, dir: &Path, out: &Path) -> Result<String, String> {
    let src = dir.join("test/rust/src");
    let package_json = read(&dir.join("lawspec-package.json"))?;
    let field = |key: &str| -> Result<String, String> {
        let at = package_json.find(&format!("\"{key}\"")).ok_or(format!("no {key} in lawspec-package.json"))?;
        let rest = &package_json[at + key.len() + 2..];
        let start = rest.find('"').ok_or("a malformed lawspec-package.json")? + 1;
        let end = start + rest[start..].find('"').ok_or("a malformed lawspec-package.json")?;
        Ok(rest[start..end].to_string())
    };
    let package = field("name")?;
    let version = field("version")?;
    let prefix = format!("crate::contracts::{name}::");
    let modules_rs = read(&src.join("lawspec_modules.rs"))?;
    let mut module = format!("#[allow(warnings, clippy::all, non_snake_case, unused)]\npub mod {name} {{\n");
    let mut path_attr = None;
    for line in modules_rs.lines() {
        let line = line.trim();
        if let Some(p) = line.strip_prefix("#[path = \"").and_then(|l| l.strip_suffix("\"]")) {
            path_attr = Some(p.to_string());
        } else if let (Some(m), Some(p)) = (line.strip_prefix("pub mod ").and_then(|l| l.strip_suffix(';')), path_attr.take()) {
            let from = src.join(&p);
            let to = out.join(name).join(&p);
            std::fs::create_dir_all(to.parent().unwrap()).map_err(|e| e.to_string())?;
            let body = read(&from)?.replace("crate::", &prefix);
            std::fs::write(&to, body).map_err(|e| e.to_string())?;
            writeln!(module, "    #[path = {:?}]\n    pub mod {m};", to.display().to_string()).unwrap();
        }
    }
    let abilities = read(&src.join("lawspec_abilities.rs"))?;
    let unit = package.replace('.', "_");
    let (ops, start_fn, state_ty) = bind(&abilities, &unit)?;
    let twin_file = dir.join("lawtwin.json");
    let twin_file = if twin_file.exists() { format!("include_str!({:?})", twin_file.display().to_string()) } else { "\"\"".into() };
    write!(
        module,
        r#"
    use self::lawspec_runtime as ls;
    use self::lawspec_definitions::typed;
    use crate::model::{{Model as _, Outcome}};
    use crate::V;

    pub const PACKAGE: &str = {package:?};
    pub const VERSION: &str = {version:?};
    pub const WIRE_MAP: &str = include_str!({wire:?});
    pub const TWIN_FILE: &str = {twin_file};

    fn to_ls(v: V) -> ls::Value {{
        match v {{
            V::Int(i) => <i64 as ls::fast::Dyn>::to_dyn(&i),
            V::Float(f) => ls::Value::Float64(f),
            V::Bool(b) => ls::Value::Bool(b),
            V::Text(s) => ls::Value::Text(s),
            V::Bytes(b) => ls::Value::Bytes(b),
            V::Unit => ls::Value::Unit,
            V::Maybe(o) => ls::Value::Maybe(o.map(|x| Box::new(to_ls(*x)))),
            V::List(xs) => ls::Value::List(xs.into_iter().map(to_ls).collect()),
            V::Map(entries) => ls::Value::Data(
                ls::fast::KEYVAL.into(),
                vec![ls::Value::List(
                    entries.into_iter().map(|(k, x)| ls::Value::Data(ls::fast::ENTRY.into(), vec![to_ls(k), to_ls(x)])).collect(),
                )],
            ),
            V::Data {{ ty, ctor, fields }} => {{
                ls::Value::Data(format!("{{PACKAGE}}.model::type::{{ty}}::{{ctor}}"), fields.into_iter().map(to_ls).collect())
            }}
        }}
    }}

    fn from_ls(v: ls::Value) -> V {{
        match v {{
            ls::Value::Integer(n) => V::Int(<i64 as ls::fast::Dyn>::from_dyn(ls::Value::Integer(n)).unwrap_or(i64::MAX)),
            ls::Value::Float64(f) => V::Float(f),
            ls::Value::Float32(f) => V::Float(f as f64),
            ls::Value::Bool(b) => V::Bool(b),
            ls::Value::Text(s) => V::Text(s),
            ls::Value::Bytes(b) => V::Bytes(b),
            ls::Value::Unit | ls::Value::Null | ls::Value::Undefined => V::Unit,
            ls::Value::Maybe(o) | ls::Value::Optional(o) | ls::Value::Nullable(o) => V::Maybe(o.map(|x| Box::new(from_ls(*x)))),
            ls::Value::List(xs) => V::List(xs.into_iter().map(from_ls).collect()),
            ls::Value::Data(tag, mut fields) if tag == ls::fast::KEYVAL && fields.len() == 1 => {{
                let entries = match fields.pop() {{ Some(ls::Value::List(es)) => es, _ => vec![] }};
                V::Map(entries.into_iter().filter_map(|e| match e {{
                    ls::Value::Data(_, kv) if kv.len() == 2 => {{
                        let mut kv = kv.into_iter();
                        Some((from_ls(kv.next().unwrap()), from_ls(kv.next().unwrap())))
                    }}
                    _ => None,
                }}).collect())
            }}
            ls::Value::Data(tag, fields) => {{
                let mut parts = tag.rsplit("::");
                let ctor = parts.next().unwrap_or("").to_string();
                let ty = parts.next().unwrap_or("").to_string();
                V::Data {{ ty, ctor, fields: fields.into_iter().map(from_ls).collect() }}
            }}
            other => V::Text(format!("{{other:?}}")),
        }}
    }}

    /// Runs one generated definition, giving a raised ApiError back as a value.
    fn run<T>(body: impl FnOnce() -> ls::Result<T>) -> Result<T, Outcome> {{
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {{
            Ok(Ok(value)) => Ok(value),
            Ok(Err(message)) => Err(Outcome::Defect(message)),
            Err(payload) => match payload.downcast::<ls::Failure>() {{
                Ok(failure) => match &failure.value {{
                    ls::Value::Data(tag, _) => Err(Outcome::Failure(tag.rsplit("::").next().unwrap_or("").to_string())),
                    other => Err(Outcome::Defect(format!("{{}} raised {{other:?}}", failure.ability))),
                }},
                Err(other) => Err(Outcome::Defect(crate::model::panic_text(&other))),
            }},
        }}
    }}

    /// The contract's reference model, over a state the twin owns.
    #[derive(Clone)]
    pub struct Model {{
        state: typed::{state_ty},
    }}

    pub fn start() -> Box<dyn crate::model::Model> {{
        Box::new(Model {{ state: typed::{start_fn}(()).expect("the contract's start state") }})
    }}

    impl crate::model::Model for Model {{
        fn call(&mut self, op: &str, args: Vec<V>) -> Result<V, Outcome> {{
            let mut args = args.into_iter().map(to_ls);
            let mut arg = move || args.next().ok_or_else(|| Outcome::Defect("too few arguments".into()));
            let state = self.state.clone();
            match op {{
"#,
        wire = dir.join("lawtwin-openapi.json").display().to_string(),
    )
    .unwrap();
    for op in &ops {
        writeln!(module, "                {:?} => {{", op.name).unwrap();
        let mut converted = vec![];
        for (i, c) in op.conversions.iter().enumerate() {
            writeln!(module, "                    let a{i} = arg()?;").unwrap();
            let c = c.replace("crate::", &prefix);
            let c = c
                .split_once("ls::fast::Public>::from_native(")
                .map(|(head, _)| format!("{head}ls::fast::Dyn>::from_dyn(a{i})?"))
                .ok_or_else(|| format!("{}: an argument conversion {c} this script does not know", op.name))?;
            converted.push(c);
        }
        let function = op.function.replace("crate::", &prefix);
        let args = if converted.is_empty() { String::new() } else { format!(", {}", converted.join(", ")) };
        writeln!(module, "                    let pair = run(move || {function}(state{args}))?;").unwrap();
        writeln!(module, "                    let (result, next) = typed::pair_fields(pair);").unwrap();
        writeln!(module, "                    self.state = next;").unwrap();
        writeln!(module, "                    Ok(from_ls(ls::fast::Dyn::to_dyn(&result)))").unwrap();
        writeln!(module, "                }}").unwrap();
    }
    writeln!(
        module,
        r#"                other => Err(Outcome::Defect(format!("the contract has no operation {{other}}"))),
            }}
        }}

        fn state(&self) -> V {{
            from_ls(ls::fast::Dyn::to_dyn(&self.state))
        }}

        fn boxed(&self) -> Box<dyn crate::model::Model> {{
            Box::new(self.clone())
        }}
    }}
}}
"#
    )
    .unwrap();
    Ok(module)
}

/// Each operation of the generated `reference` handler, its start state's
/// definition, and its state's type.
fn bind(text: &str, unit: &str) -> Result<(Vec<Op>, String, String), String> {
    let module = text.find(&format!("pub mod {unit} {{")).ok_or(format!("no module {unit}"))?;
    let end = text[module + 1..].find("\npub mod ").map(|e| module + 1 + e).unwrap_or(text.len());
    let body = &text[module..end];
    let state_ty = body
        .split_once("state: std::sync::Mutex<crate::lawspec_definitions::typed::")
        .and_then(|(_, r)| r.split_once(">,"))
        .map(|(t, _)| t.to_string())
        .ok_or("no typed state in the reference handler")?;
    let start_fn = body
        .split_once("Ok(crate::lawspec_definitions::typed::")
        .and_then(|(_, r)| r.split_once('('))
        .map(|(f, _)| f.to_string())
        .ok_or("no start state in the reference handler")?;
    let mut ops = vec![];
    for block in body.split("    impl super::").skip(1) {
        let Some((header, rest)) = block.split_once(" for Reference {\n") else { continue };
        let _ = header;
        let rest = format!("\n{}", &rest[..rest.find("\n    }\n").unwrap_or(rest.len())]);
        for chunk in rest.split("\n        fn ").skip(1) {
            let name = chunk.split_once('(').ok_or("a method without arguments")?.0.trim_start_matches("fn ").to_string();
            let call = chunk.split_once("let pair = ").ok_or("a method without a call")?.1;
            let (function, args) = call.split_once("(\n").ok_or("a call without arguments")?;
            let args = args.split_once("\n            ).unwrap_or_else").ok_or("a call that does not unwrap")?.0;
            let conversions = args
                .lines()
                .map(str::trim)
                .filter(|l| *l != "state.clone(),")
                .map(|l| l.trim_end_matches(',').to_string())
                .collect();
            ops.push(Op { name, function: function.trim().to_string(), conversions });
        }
    }
    if ops.is_empty() {
        return Err("no operations in the reference handler".into());
    }
    Ok((ops, start_fn, state_ty))
}
