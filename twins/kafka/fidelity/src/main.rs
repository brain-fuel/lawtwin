//! `lawtwin-kafka-fidelity`: the Kafka twin's fidelity tools, which
//! `lawtwin conform kafka`, `lawtwin record kafka`, `lawtwin replay --core`
//! and `lawtwin peer kafka` run.

use lawtwin_kafka::clock::ClockMode;
use lawtwin_kafka_fidelity::cli::{Args, USAGE};
use lawtwin_kafka_fidelity::{record, replay};
use std::path::PathBuf;
use std::process::ExitCode;

fn address(text: &str) -> String {
    if text.contains(':') { text.to_string() } else { format!("127.0.0.1:{text}") }
}

fn record_command(rest: &[String]) -> Result<(), String> {
    let a = Args::parse(rest, &["quiet"]);
    a.known(&["upstream", "listen", "admin-port", "advertise", "out", "clock", "quiet"])?;
    let upstream = a.get("upstream").ok_or("record needs --upstream <host:port>, the real broker")?.to_string();
    let config = record::RecorderConfig {
        upstream,
        listen: address(a.get("listen").unwrap_or("9092")),
        admin_listen: address(a.get("admin-port").unwrap_or("9644")),
        advertise: a.get("advertise").map(str::to_string),
        clock: match a.get("clock") {
            Some(c) => c.parse::<ClockMode>()?,
            None => ClockMode::Wall,
        },
        out: a.get("out").map(PathBuf::from),
        verbose: !a.has("quiet"),
    };
    let summary = record::run(config).map_err(|e| format!("{e:#}"))?;
    println!("{}", serde_json::to_string_pretty(&summary).unwrap_or_default());
    let clean = summary["divergences"].as_array().is_some_and(|d| d.is_empty())
        && summary["violations"].as_array().is_some_and(|v| v.is_empty());
    if clean { Ok(()) } else { Err("the recording diverged from the contract or broke a law".into()) }
}

fn replay_command(rest: &[String]) -> Result<(), String> {
    let a = Args::parse(rest, &[]);
    a.known(&["core", "report"])?;
    let trace = a.words.first().ok_or("replay needs a trace: its prefix, or its .frames.jsonl file")?;
    let core = a.get("core").ok_or("replay needs --core <command | image:<image>>")?;
    let report = replay::replay_trace(&replay::trace_prefix(trace), core).map_err(|e| format!("{e:#}"))?;
    let text = serde_json::to_string_pretty(&report).unwrap_or_default();
    match a.get("report") {
        Some(path) => std::fs::write(path, text + "\n").map_err(|e| format!("{path}: {e}"))?,
        None => println!("{text}"),
    }
    let n = report.divergences.len();
    if n == 0 && report.violations.is_empty() && report.stalled.is_empty() {
        eprintln!("lawtwin replay: {} requests replayed through {}; no divergence", report.requests, report.core);
        Ok(())
    } else {
        Err(format!(
            "replaying through {}: {n} divergence(s), {} broken law(s), {} stalled request(s)",
            report.core,
            report.violations.len(),
            report.stalled.len()
        ))
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("record") => record_command(&args[1..]),
        Some("replay") => replay_command(&args[1..]),
        Some("conform") => lawtwin_kafka_fidelity::conform::command(&args[1..]),
        Some("peer") => lawtwin_kafka_fidelity::peer::command(&args[1..]),
        Some("--version") => {
            println!("lawtwin-kafka-fidelity {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => {
            println!("{USAGE}");
            Ok(())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lawtwin: {e}");
            ExitCode::FAILURE
        }
    }
}
