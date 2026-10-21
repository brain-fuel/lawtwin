//! The test project of an imported package: its LawSpec configuration, and
//! native handlers on Rust and Python that answer through the reference
//! model, so the package's laws run before any twin or real service does.

use crate::api::Api;
use crate::ir::{Scalar, Ty};
use std::fmt::Write as _;

fn rust_ty(ty: &Ty) -> String {
    match ty {
        Ty::Scalar(s, _) => match s {
            Scalar::Text | Scalar::Json => "std::string::String".into(),
            Scalar::Int32 => "i32".into(),
            Scalar::Int64 => "i64".into(),
            Scalar::Float64 => "f64".into(),
            Scalar::Bool => "bool".into(),
            Scalar::Bytes => "std::vec::Vec<u8>".into(),
            Scalar::Unit => "()".into(),
        },
        Ty::List(t, _) => format!("std::vec::Vec<{}>", rust_ty(t)),
        Ty::Maybe(t) => format!("std::option::Option<{}>", rust_ty(t)),
        Ty::Map(t) => format!(
            "std::collections::BTreeMap<std::string::String, {}>",
            rust_ty(t)
        ),
        Ty::Named(n) => format!("crate::lawspec_data::{n}"),
    }
}

/// The Rust module path of a unit's generated abilities.
fn rust_module(unit: &str) -> String {
    unit.replace('.', "_")
}

/// The Rust native handlers of the package's abilities.
pub fn rust(unit: &str, api: &Api) -> String {
    let m = rust_module(unit);
    let mut out = format!(
        "// Written by `lawtwin import openapi`, and written again on every import.\n//! The native handlers of {unit}, for running its laws: each answers\n//! through the reference model, held by the case's Service. A connection\n//! to the HTTP twin or to the real service takes its place in conformance\n//! runs.\n#![allow(unused_variables, unused_imports, non_snake_case)]\nuse crate::lawspec_abilities::{m} as abilities;\nuse crate::lawspec_runtime as ls;\nuse std::sync::Arc;\n\n/// What a Service holds: the reference model.\npub struct ReferenceService {{\n    model: Arc<abilities::Reference>,\n}}\n\nfn reference(service: &crate::lawspec_data::Service) -> &ReferenceService {{\n    service.native::<ReferenceService>().unwrap_or_else(|error| panic!(\"{{error}}\"))\n}}\n\n// LawSpec: (Unit -> {unit}::type::Service)\npub fn connectService(value0: ()) -> crate::lawspec_data::Service {{\n    ls::Handle::new(ReferenceService {{ model: Arc::new(abilities::Reference::new(&ls::Context::default())) }})\n}}\n\n// LawSpec: ({unit}::type::Service -> Unit)\npub fn disconnectService(value0: crate::lawspec_data::Service) -> () {{}}\n"
    );
    for ability in &api.abilities {
        let _ = write!(
            out,
            "\n/// {ability}, answered by the reference model.\npub struct {ability}Handler {{\n    pub resource: crate::lawspec_data::Service,\n}}\n\nimpl {ability}Handler {{\n    pub fn new(resource: crate::lawspec_data::Service) -> Self {{\n        Self {{ resource }}\n    }}\n}}\n\nimpl abilities::{ability} for {ability}Handler {{\n"
        );
        let mut first = true;
        for op in api.ops.iter().filter(|o| &o.ability == ability) {
            if !first {
                out.push('\n');
            }
            first = false;
            let params: String = op
                .params
                .iter()
                .enumerate()
                .map(|(i, p)| format!(", value{i}: {}", rust_ty(&p.ty)))
                .collect();
            let args: Vec<String> = (0..op.params.len()).map(|i| format!("value{i}")).collect();
            let call_args = if args.is_empty() {
                String::new()
            } else {
                format!(", {}", args.join(", "))
            };
            let _ = write!(
                out,
                "    fn {}(&self{params}) -> {} {{\n        <abilities::Reference as abilities::{ability}>::{}(&reference(&self.resource).model{call_args})\n    }}\n",
                op.name,
                rust_ty(&op.response),
                op.name
            );
        }
        out.push_str("}\n");
    }
    out
}

/// The Python native handlers of the package's abilities.
pub fn python(unit: &str, api: &Api) -> String {
    let mut out = format!(
        "# Written by `lawtwin import openapi`, and written again on every import.\n\"\"\"The native handlers of {unit}, for running its laws: each answers\nthrough the reference model, held by the case's Service. A connection to\nthe HTTP twin or to the real service takes its place in conformance runs.\n\"\"\"\nimport lawspec_abilities.{unit} as abilities\n\n\nclass ReferenceService:\n    \"\"\"What a Service holds: the reference model.\"\"\"\n\n    def __init__(self) -> None:\n        self.model = abilities.Reference({{}})\n\n\ndef connectService(value0):\n    return ReferenceService()\n\n\ndef disconnectService(value0) -> None:\n    return None\n\n\nclass _Reference:\n    \"\"\"A native handler that answers every operation through the model.\"\"\"\n\n    def __init__(self, resource) -> None:\n        self.resource = resource\n\n    def __getattr__(self, name):\n        return getattr(self.resource.model, name)\n"
    );
    for ability in &api.abilities {
        let _ = write!(
            out,
            "\n\nclass {ability}Handler(_Reference):\n    \"\"\"{ability}, answered by the reference model.\"\"\"\n"
        );
    }
    out
}

/// The test project's lawspec.json: the package, on Rust and Python.
pub fn lawspec_json(package: &str, version: &str) -> String {
    format!(
        "{{\n  \"version\": 2,\n  \"languageProfile\": \"1.0-rc.2\",\n  \"verification\": {{\"layers\": [\"proof\", \"exhaustive\", \"property\", \"examples\", \"runtime\", \"model\"]}},\n  \"sources\": [\n    \"laws\"\n  ],\n  \"dependencies\": {{\n    \"{package}\": \"{version}\"\n  }},\n  \"packages\": [\n    \"..\"\n  ],\n  \"targets\": [\n    {{\n      \"language\": \"rust\",\n      \"root\": \"rust\"\n    }},\n    {{\n      \"language\": \"python\",\n      \"root\": \"python\",\n      \"python\": \".venv/bin/python\"\n    }}\n  ]\n}}\n"
    )
}

/// The test project's own unit, which imports the package.
pub fn suite(unit: &str) -> String {
    let last = unit.rsplit('.').next().unwrap_or(unit);
    format!(
        "unit contracttest.{last}\n\nimport {unit} as api\n\n-- The project that runs the {unit} contract's laws. Its native handlers\n-- answer through the reference model; see the native handlers in rust/src\n-- and python/src.\nfunction suiteNamesThePackage (u :: Unit) -> Unit is unitValue end function\nend unit\n"
    )
}

/// The Rust test project's manifest.
const CARGO_TOML: &str = "[package]\nname = \"lawspec-example\"\nversion = \"0.1.0\"\nedition = \"2024\"\nrust-version = \"1.85\"\npublish = false\n\n[dependencies]\nnum-bigint = \"=0.4.8\"\nnum-rational = \"=0.4.2\"\nnum-complex = \"=0.4.6\"\nnum-traits = \"=0.2.19\"\n\n[dev-dependencies]\nproptest = \"=1.11.0\"\n";

/// The Rust test project's library root.
pub const LIB_RS: &str = "// Application library. LawSpec maintains the included module declarations.\ninclude!(\"lawspec_modules.rs\");\n";

/// The Python test project's manifest.
pub const PYPROJECT: &str = "[build-system]\nrequires = [\"setuptools==80.9.0\"]\nbuild-backend = \"setuptools.build_meta\"\n\n[project]\nname = \"lawspec-example\"\nversion = \"0.1.0\"\nrequires-python = \">=3.13\"\ndependencies = []\n\n[project.optional-dependencies]\ntest = [\"pytest==8.4.2\", \"hypothesis==6.135.26\"]\n\n[tool.setuptools.packages.find]\nwhere = [\"src\"]\n\n[tool.pytest.ini_options]\npythonpath = [\"src\"]\ntestpaths = [\"tests\"]\n";

/// The HTTP core's adapter, generated from the OpenAPI IR. It consumes only
/// public native types and handler methods; no generated Rust is parsed.
pub fn embedding(unit: &str, version: &str, types: &crate::ir::Types<'_>, api: &Api) -> String {
    use crate::ir::Def;
    let alias = format!("{}_model", rust_module(unit));
    let mut out = format!(
        r#"// Written by `lawtwin import openapi`. Do not edit.
// ref:DEC-embed-generated-model ref:DEC-http-contract-registry
use {alias}::lawspec_runtime as ls;
use {alias}::lawspec_data as data;
use {alias}::lawspec_abilities::{module}::Reference;
use crate::model::{{Model as _, Outcome}};
use crate::V;

pub const PACKAGE: &str = {unit:?};
pub const VERSION: &str = {version:?};
pub const WIRE_MAP: &str = include_str!("lawtwin-openapi.json");
pub const TWIN_FILE: &str = include_str!("lawtwin.json");

fn to_value(v: V) -> ls::Value {{
    match v {{
        V::Int(i) => ls::IntoValue::into_value(i),
        V::Float(f) => ls::Value::Float64(f),
        V::Bool(b) => ls::Value::Bool(b),
        V::Text(s) => ls::Value::Text(s),
        V::Bytes(b) => ls::Value::Bytes(b),
        V::Unit => ls::Value::Unit,
        V::Maybe(o) => ls::Value::Maybe(o.map(|v| Box::new(to_value(*v)))),
        V::List(xs) => ls::Value::List(xs.into_iter().map(to_value).collect()),
        V::Map(xs) => ls::Value::List(xs.into_iter().map(|(k,v)| ls::Value::Data(
            "lawspec.collections::type::Entry::Entry".into(), vec![to_value(k),to_value(v)]
        )).collect()),
        V::Data {{ty,ctor,fields}} => ls::Value::Data(
            format!("{{PACKAGE}}.model::type::{{ty}}::{{ctor}}"), fields.into_iter().map(to_value).collect()
        ),
    }}
}}

fn run<T>(body: impl FnOnce() -> Result<T, Outcome>) -> Result<T, Outcome> {{
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(body))
        .unwrap_or_else(|payload| Err(Outcome::Defect(crate::model::panic_text(&payload))))
}}

pub struct Model {{ handler: Reference }}
impl Clone for Model {{
    fn clone(&self) -> Self {{
        let handler = Reference::start();
        handler.set_state(self.handler.state());
        Self {{ handler }}
    }}
}}
pub fn start() -> Box<dyn crate::model::Model> {{ Box::new(Model {{handler: Reference::start()}}) }}

impl crate::model::Model for Model {{
    fn call(&mut self, op: &str, args: Vec<V>) -> Result<V, Outcome> {{
        let mut args = args.into_iter();
        run(|| match op {{
"#,
        module = rust_module(unit)
    );
    for op in &api.ops {
        writeln!(out, "            {:?} => {{", op.name).unwrap();
        writeln!(out, "                if args.len() != {} {{ return Err(Outcome::Defect(\"wrong argument count\".into())); }}", op.params.len()).unwrap();
        for (i, p) in op.params.iter().enumerate() {
            writeln!(out,"                let value{i}: {} = ls::FromValue::from_value(to_value(args.next().unwrap())).map_err(Outcome::Defect)?;", rust_ty(&p.ty).replace("crate::lawspec_data::", "data::")).unwrap();
        }
        let args = (0..op.params.len())
            .map(|i| format!("value{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(out, "                let value = self.handler.try_{}({args}).map_err(|e| match ls::IntoValue::into_value(e) {{ ls::Value::Data(tag, _) => Outcome::Failure(tag.rsplit(\"::\").next().unwrap().into()), other => Outcome::Defect(format!(\"unexpected failure {{other:?}}\")) }})?;",op.name).unwrap();
        writeln!(
            out,
            "                Ok({})\n            }}",
            encode(&op.response, "value")
        )
        .unwrap();
    }
    out.push_str(r#"            other => Err(Outcome::Defect(format!("the contract has no operation {other}"))),
        })
    }
    fn state(&self) -> V { encode_State(self.handler.state()) }
    fn boxed(&self) -> Box<dyn crate::model::Model> { Box::new(self.clone()) }
}
"#);
    for def in &types.defs {
        writeln!(
            out,
            "#[allow(non_snake_case)]\nfn encode_{}(value: data::{}) -> V {{",
            def.name(),
            def.name()
        )
        .unwrap();
        match def {
            Def::Record { name, fields, .. } => {
                let vals = fields
                    .iter()
                    .map(|f| encode(&f.ty, &format!("value.{}", f.name)))
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(out,"    V::Data {{ty: {name:?}.into(), ctor: {name:?}.into(), fields: vec![{vals}]}}").unwrap();
            }
            Def::Enum { name, ctors, .. } => {
                if let [(ctor, _)] = ctors.as_slice() {
                    writeln!(out, "    let _ = value;\n    V::Data {{ty: {name:?}.into(), ctor: {ctor:?}.into(), fields: vec![]}}").unwrap();
                    out.push_str("}\n");
                    continue;
                }
                out.push_str("    match value {\n");
                for (ctor, _) in ctors {
                    writeln!(out,"        data::{name}::{ctor} => V::Data {{ty: {name:?}.into(), ctor: {ctor:?}.into(), fields: vec![]}},").unwrap();
                }
                out.push_str("    }\n");
            }
            Def::Union { name, alts, .. } => {
                if let [(ctor, ty)] = alts.as_slice() {
                    writeln!(out,"    V::Data {{ty: {name:?}.into(), ctor: {ctor:?}.into(), fields: vec![{}]}}", encode(ty,"value.value")).unwrap();
                    out.push_str("}\n");
                    continue;
                }
                out.push_str("    match value {\n");
                for (ctor, ty) in alts {
                    writeln!(out,"        data::{name}::{ctor} {{value}} => V::Data {{ty: {name:?}.into(), ctor: {ctor:?}.into(), fields: vec![{}]}},",encode(ty,"value")).unwrap();
                }
                out.push_str("    }\n");
            }
        }
        out.push_str("}\n");
    }
    // State contains resource maps and counters, in the same order as the
    // model and wire map emitted from this IR.
    out.push_str("#[allow(non_snake_case)]\nfn encode_State(value: data::State) -> V {\n    V::Data {ty: \"State\".into(), ctor: \"State\".into(), fields: vec![\n");
    for r in &api.resources {
        let key = if let Some(kr) = &r.key_record {
            format!("encode_{kr}(k)")
        } else {
            encode(&r.key_ty, "k")
        };
        writeln!(
            out,
            "        V::Map(value.{}.into_iter().map(|(k,v)| ({key}, encode_{}(v))).collect()),",
            r.map_field, r.record
        )
        .unwrap();
        if let Some(c) = &r.counter_field {
            writeln!(out, "        {},", encode(&r.key_ty, &format!("value.{c}"))).unwrap();
        }
    }
    out.push_str("    ]}\n}\n");
    for r in &api.resources {
        if let Some(kr) = &r.key_record {
            writeln!(out,"#[allow(non_snake_case)]\nfn encode_{kr}(value: data::{kr}) -> V {{ V::Data {{ty: {kr:?}.into(), ctor: {kr:?}.into(), fields: vec![").unwrap();
            for (n, t) in &r.parents {
                writeln!(out, "    {},", encode(t, &format!("value.{n}"))).unwrap();
            }
            writeln!(
                out,
                "    {}]}} }}",
                encode(&r.key_ty, &format!("value.{}", r.key_param))
            )
            .unwrap();
        }
    }
    out
}

/// Encode public native values without relying on the compiler's internal
/// dynamic record representation or typed definition names.
fn encode(ty: &Ty, value: &str) -> String {
    match ty {
        Ty::Scalar(s, _) => match s {
            Scalar::Text | Scalar::Json => format!("V::Text({value})"),
            Scalar::Int32 => format!("V::Int(i64::from({value}))"),
            Scalar::Int64 => format!("V::Int({value})"),
            Scalar::Float64 => format!("V::Float({value})"),
            Scalar::Bool => format!("V::Bool({value})"),
            Scalar::Bytes => format!("V::Bytes({value})"),
            Scalar::Unit => format!("{{ let _ = {value}; V::Unit }}"),
        },
        Ty::Maybe(t) => format!("V::Maybe({value}.map(|v| Box::new({})))", encode(t, "v")),
        Ty::List(t, _) => format!(
            "V::List({value}.into_iter().map(|v| {}).collect())",
            encode(t, "v")
        ),
        Ty::Map(t) => format!(
            "V::Map({value}.into_iter().map(|(k,v)| (V::Text(k), {})).collect())",
            encode(t, "v")
        ),
        Ty::Named(n) => format!("encode_{n}({value})"),
    }
}

/// Distinct package identities allow multiple generated models in one core.
pub fn cargo_toml(unit: &str) -> String {
    CARGO_TOML.replace(
        "name = \"lawspec-example\"",
        &format!("name = \"{}-model\"", unit.replace('.', "-")),
    ) + "\n[lib]\nname = \"lawspec_example\"\n\n[workspace]\n"
}
