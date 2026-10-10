//! Binds the Kafka contract's generated `cluster` spec handler for the twin.
//!
//! LawSpec generates `Cluster`, the spec handler of `Broker`, `Admin` and
//! `GroupCoordinator`, into the contract's Rust test project. Its state is
//! private and a failing operation panics with the `KafkaError` it raises,
//! which suits a law but not a server. This script reads the generated
//! `impl Broker for Cluster` and writes, for each operation, a method of
//! `ClusterModel` that calls the same typed definition over a state the twin
//! owns, and gives the raised failure back as a value. Nothing is restated:
//! each method keeps the generated call and its argument conversions, so the
//! code the twin runs is the code the laws check.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let source = manifest.join("../../../contracts/kafka/test/rust/src/lawspec_abilities.rs");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed=build.rs");
    let text = std::fs::read_to_string(&source).unwrap_or_else(|e| panic!("read {}: {e}", source.display()));
    let methods = bind(&text, "Broker").unwrap_or_else(|e| panic!("binding the generated cluster handler: {e}"));
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("cluster_model.rs");
    std::fs::write(out, methods).unwrap();
}

/// The methods of `ClusterModel`, one per operation of the ability.
fn bind(text: &str, ability: &str) -> Result<String, String> {
    let module = text.find("pub mod lawtwin_kafka {").ok_or("no module lawtwin_kafka")?;
    let header = format!("impl super::lawtwin_kafka::{ability} for Cluster {{");
    let start = module + text[module..].find(&header).ok_or_else(|| format!("no {header}"))? + header.len();
    let end = start + text[start..].find("\n    }\n").ok_or("unterminated impl")?;
    let body = &text[start..end];
    let mut out = String::new();
    let mut count = 0;
    for chunk in body.split("\n        fn ").skip(1) {
        let (signature, rest) = chunk.split_once(" {\n").ok_or("a method without a body")?;
        let (name, rest_sig) = signature.split_once('(').ok_or("a method without arguments")?;
        let (params, ret) = rest_sig.rsplit_once(") -> ").ok_or("a method without a result type")?;
        let params = params.strip_prefix("&self").ok_or("a method without &self")?;
        let call = rest.split_once("let pair = ").ok_or("a method without a call")?.1;
        let (function, args) = call.split_once("(\n").ok_or("a call without arguments")?;
        let args = args.split_once("\n            ).unwrap_or_else").ok_or("a call that does not unwrap")?.0;
        let mut conversions = vec![];
        for line in args.lines().map(str::trim) {
            if line == "state.clone()," {
                continue;
            }
            conversions.push(line.trim_end_matches(',').to_string());
        }
        let fix = |s: &str| s.replace("crate::", "lawspec_example::");
        writeln!(out, "    /// `{name}`, by the generated `{}`.", function.rsplit("::").next().unwrap()).unwrap();
        writeln!(out, "    pub fn {name}(&mut self{}) -> Result<{}, Failure> {{", fix(params), fix(ret)).unwrap();
        writeln!(out, "        let state = self.state.clone();").unwrap();
        writeln!(out, "        let pair = run(move || {}(state, {}))?;", fix(function), fix(&conversions.join(", "))).unwrap();
        writeln!(out, "        let (result, next) = lawspec_example::lawspec_definitions::typed::pair_fields(pair);").unwrap();
        writeln!(out, "        self.state = next;").unwrap();
        writeln!(out, "        Ok(ls::fast::Public::to_native(&result))").unwrap();
        writeln!(out, "    }}\n").unwrap();
        count += 1;
    }
    if count == 0 {
        return Err("no operations".into());
    }
    // The start state: the definition the generated constructor calls.
    let marker = "pub fn new(ctx: &ls::Context) -> Self {";
    let ctor = module + text[module..].find(marker).ok_or("no generated constructor")?;
    let start_call = text[ctor..].split_once("Ok(crate::lawspec_definitions::typed::").ok_or("no start state")?.1;
    let start_fn = start_call.split_once('(').ok_or("no start state call")?.0;
    writeln!(out, "    /// The start state, by the generated `{start_fn}`.").unwrap();
    writeln!(out, "    pub(crate) fn start_state() -> ls::Result<lawspec_example::lawspec_definitions::typed::BrokerState> {{").unwrap();
    writeln!(out, "        lawspec_example::lawspec_definitions::typed::{start_fn}(())").unwrap();
    writeln!(out, "    }}").unwrap();
    Ok(format!("impl ClusterModel {{\n{out}}}\n"))
}
