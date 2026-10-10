//! `lawtwin`: the LawTwin command line.
//!
//! `lawtwin import openapi <document> --package <name> --out <dir>` turns an
//! OpenAPI 3.0 or 3.1 document into a LawSpec dependency-contract package.
//! `lawtwin up kafka` runs the Kafka twin, and `inspect`, `force`, `fault`,
//! `advance`, `history`, `replay`, `laws` and `seed` drive a running twin
//! through its control plane.

mod api;
mod control;
mod emit;
mod ir;
mod names;
mod natives;
mod openapi;
mod regex;
mod wiremap;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage:
  lawtwin import openapi <document> --package <name> --out <directory>
          [--version <x.y.z>] [--tag <tag>]... [--path <path>]...
  lawtwin --version
{CONTROL}

import openapi reads an OpenAPI 3.0 or 3.1 document, in JSON or YAML, and
writes a LawSpec package for it: its schemas as types, one ability per tag
with an operation for each path and method, a typed failure for each
documented failing status, a reference model for the resources it
recognises, and laws over them. --tag and --path keep only the operations
with one of the tags, or under one of the paths; both may be repeated.";

struct Options {
    document: PathBuf,
    package: String,
    out: PathBuf,
    version: String,
    tags: Vec<String>,
    paths: Vec<String>,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut document = None;
    let (mut package, mut out, mut version) = (None, None, "0.1.0".to_string());
    let (mut tags, mut paths) = (vec![], vec![]);
    let mut i = 0;
    while i < args.len() {
        let value = |i: usize| {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{} needs a value", args[i]))
        };
        match args[i].as_str() {
            "--package" => package = Some(value(i)?),
            "--out" => out = Some(PathBuf::from(value(i)?)),
            "--version" => version = value(i)?,
            "--tag" => tags.push(value(i)?),
            "--path" => paths.push(value(i)?),
            a if a.starts_with("--") => return Err(format!("unknown option {a}")),
            a => {
                if document.replace(PathBuf::from(a)).is_some() {
                    return Err("give one document".into());
                }
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    let package = package.ok_or("--package is required")?;
    if !package.split('.').all(|s| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && s.starts_with(|c: char| c.is_ascii_lowercase())
    }) {
        return Err(format!(
            "the package name {package} must be lower-case words separated by dots, such as lawtwin.petstore"
        ));
    }
    Ok(Options {
        document: document.ok_or("name the OpenAPI document")?,
        package,
        out: out.ok_or("--out is required")?,
        version,
        tags,
        paths,
    })
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Write a file the author owns from then on, unless it is already there.
fn write_once(path: &Path, text: &str) -> Result<bool, String> {
    if path.exists() {
        return Ok(false);
    }
    write(path, text)?;
    Ok(true)
}

fn import_openapi(o: &Options) -> Result<(), String> {
    let doc = openapi::Document::load(&o.document)?;
    let mut types = ir::Types::new(&doc);
    types.upper.reserve("Service");
    types.upper.reserve("State");
    types.upper.reserve("ApiError");
    types.upper.reserve("Reference");
    let include = |path: &str, _method: &str, tags: &[String]| -> bool {
        let tag_ok = o.tags.is_empty() || tags.iter().any(|t| o.tags.contains(t));
        let path_ok = o.paths.is_empty()
            || o.paths.iter().any(|p| {
                path == p || (path.starts_with(p.as_str()) && path[p.len()..].starts_with('/'))
            });
        tag_ok && path_ok
    };
    let api = api::Api::build(&doc, &mut types, &include)?;
    let source = o
        .document
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("the document")
        .to_string();
    let described = format!(
        "{} {} ({source}, OpenAPI {})",
        doc.title,
        doc.version,
        if doc.v31 { "3.1" } else { "3.0" }
    );
    let package = emit::Emitter::new(&o.package, &types, &api, &described).package();
    let src = o.out.join("src");
    write(
        &o.out.join("lawspec-package.json"),
        &format!(
            "{{\n  \"name\": \"{}\",\n  \"version\": \"{}\",\n  \"sources\": [\"src\"],\n  \"dependencies\": {{}}\n}}\n",
            o.package, o.version
        ),
    )?;
    write(&src.join("model.lawspec"), &package.model)?;
    write(&src.join("api.lawspec"), &package.facade)?;
    let hand = src.join("handwritten.lawspec");
    if !write_once(&hand, &package.handwritten)? {
        let existing =
            std::fs::read_to_string(&hand).map_err(|e| format!("{}: {e}", hand.display()))?;
        let missing: Vec<&String> = package
            .unmodelled
            .iter()
            .filter(|op| !existing.contains(&format!("definition {op} ")))
            .collect();
        if !missing.is_empty() {
            eprintln!(
                "lawtwin: {} is yours and was kept; add a definition for each of: {}",
                hand.display(),
                missing
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    write_once(&src.join("laws.lawspec"), &package.laws)?;
    let map = wiremap::wire_map(
        &o.package,
        &source,
        &doc.title,
        &doc.version,
        &types,
        &api,
        &package.unmodelled,
    );
    write(
        &o.out.join("lawtwin-openapi.json"),
        &(serde_json::to_string_pretty(&map).unwrap() + "\n"),
    )?;
    // The test project.
    let test = o.out.join("test");
    write_once(
        &test.join("lawspec.json"),
        &natives::lawspec_json(&o.package, &o.version),
    )?;
    write_once(
        &test.join("laws/suite.lawspec"),
        &natives::suite(&o.package),
    )?;
    write_once(&test.join("rust/Cargo.toml"), natives::CARGO_TOML)?;
    write_once(&test.join("rust/src/lib.rs"), natives::LIB_RS)?;
    write_once(&test.join("python/pyproject.toml"), natives::PYPROJECT)?;
    let unit_path = o.package.replace('.', "/");
    write(
        &test.join(format!("rust/src/{unit_path}.rs")),
        &natives::rust(&o.package, &api),
    )?;
    write(
        &test.join(format!("python/src/{unit_path}/__init__.py")),
        &natives::python(&o.package, &api),
    )?;
    let modelled = api.ops.len() - package.unmodelled.len();
    println!(
        "{}: {} type(s), {} operation(s) in {} abilit{}, {} resource(s), {} law(s); {} operation(s) modelled, {} left to handwritten.lawspec",
        o.package,
        types.defs.len(),
        api.ops.len(),
        api.abilities.len(),
        if api.abilities.len() == 1 { "y" } else { "ies" },
        api.resources.len(),
        package.law_count,
        modelled,
        package.unmodelled.len()
    );
    for note in types.notes.iter().chain(api.notes.iter()) {
        println!("note: {note}");
    }
    Ok(())
}

/// The usage text, with the control-plane commands.
fn usage() -> String {
    USAGE.replace("{CONTROL}", control::USAGE)
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // `--admin <url>` may come before the command; the command reads it after.
    if args.first().map(String::as_str) == Some("--admin") && args.len() >= 2 {
        let pair: Vec<String> = args.drain(..2).collect();
        args.extend(pair);
    }
    let result = match args.first().map(String::as_str) {
        Some("--version") => {
            println!("lawtwin {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("import") if args.get(1).map(String::as_str) == Some("openapi") => {
            parse(&args[2..]).and_then(|o| import_openapi(&o))
        }
        Some("help") | Some("--help") | Some("-h") | None => {
            println!("{}", usage());
            Ok(())
        }
        Some(command) => match control::run(command, &args[1..]) {
            Some(Ok(())) => Ok(()),
            Some(Err((e, code))) => {
                eprintln!("lawtwin: {e}");
                return ExitCode::from(code);
            }
            None => Err(format!("unknown command\n{}", usage())),
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lawtwin: {e}");
            ExitCode::FAILURE
        }
    }
}
