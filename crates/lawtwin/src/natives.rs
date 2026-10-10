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

pub fn lawspec_json(package: &str, version: &str) -> String {
    format!(
        "{{\n  \"version\": 1,\n  \"sources\": [\n    \"laws\"\n  ],\n  \"dependencies\": {{\n    \"{package}\": \"{version}\"\n  }},\n  \"packages\": [\n    \"..\"\n  ],\n  \"targets\": [\n    {{\n      \"language\": \"rust\",\n      \"root\": \"rust\"\n    }},\n    {{\n      \"language\": \"python\",\n      \"root\": \"python\",\n      \"python\": \".venv/bin/python\"\n    }}\n  ]\n}}\n"
    )
}

pub fn suite(unit: &str) -> String {
    let last = unit.rsplit('.').next().unwrap_or(unit);
    format!(
        "unit contracttest.{last}\n\nimport {unit} as api\n\n-- The project that runs the {unit} contract's laws. Its native handlers\n-- answer through the reference model; see the native handlers in rust/src\n-- and python/src.\ndefinition suiteNamesThePackage (u :: Unit) :: Unit is unitValue end\n"
    )
}

pub const CARGO_TOML: &str = "[package]\nname = \"lawspec-example\"\nversion = \"0.1.0\"\nedition = \"2024\"\nrust-version = \"1.85\"\npublish = false\n\n[dependencies]\nnum-bigint = \"=0.4.8\"\nnum-rational = \"=0.4.2\"\nnum-complex = \"=0.4.6\"\nnum-traits = \"=0.2.19\"\n\n[dev-dependencies]\nproptest = \"=1.11.0\"\n";

pub const LIB_RS: &str = "// Application library. LawSpec maintains the included module declarations.\ninclude!(\"lawspec_modules.rs\");\n";

pub const PYPROJECT: &str = "[build-system]\nrequires = [\"setuptools==80.9.0\"]\nbuild-backend = \"setuptools.build_meta\"\n\n[project]\nname = \"lawspec-example\"\nversion = \"0.1.0\"\nrequires-python = \">=3.13\"\ndependencies = []\n\n[project.optional-dependencies]\ntest = [\"pytest==8.4.2\", \"hypothesis==6.135.26\"]\n\n[tool.setuptools.packages.find]\nwhere = [\"src\"]\n\n[tool.pytest.ini_options]\npythonpath = [\"src\"]\ntestpaths = [\"tests\"]\n";
