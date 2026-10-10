//! Conformance against real Kafka: `lawtwin conform kafka --real <bootstrap>`.
//!
//! The core conformance kit's client scenarios run against the real broker
//! through the recorder, each in a namespace of its own, and the kit's
//! checks run over what the recorder decoded: the contract's laws over the
//! history and the state, and the reference model run over the history,
//! whose answers must be the broker's up to renaming. The same scenarios
//! run against a twin core beside them. The report gives each run, each
//! kind of divergence with the ledger decision that accounts for it, and
//! the conformance level the twin release reaches. ref:DEC-conformance-levels

use crate::compare::Divergence;
use crate::record::{self, RecorderConfig};
use lawtwin_kafka::clock::ClockMode;
use lawtwin_kafka::history::{Entry, Event};
use lawtwin_kafka_conformance::core::TestTwin;
use lawtwin_kafka_conformance::rdkafka_scenarios;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The clients the kit drives, with the versions it pins.
pub const CLIENTS: &[(&str, &str)] =
    &[("librdkafka", "2.12.1"), ("kafkajs", "2.2.4"), ("franz-go", "1.22.1"), ("java", "4.3.1")];

/// The scenarios, in order.
pub const SCENARIOS: &[&str] = rdkafka_scenarios::SCENARIOS;

/// A level of conformance a twin release reaches against the real thing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Level {
    /// Its number, from 0.
    pub number: u8,
    /// Its name.
    pub name: &'static str,
    /// What it takes.
    pub means: &'static str,
}

/// The levels, lowest first.
pub const LEVELS: &[Level] = &[
    Level { number: 0, name: "unchecked", means: "No conformance run against the real dependency." },
    Level {
        number: 1,
        name: "lawful",
        means: "The real dependency's histories satisfy every law the kit checks, in every scenario run: the contract describes the real thing.",
    },
    Level {
        number: 2,
        name: "scenarios",
        means: "Level 1, and every client scenario that passes against the twin passes against the real dependency, unchanged.",
    },
    Level {
        number: 3,
        name: "answers",
        means: "Level 2, and every answer of the real dependency is the reference model's, up to renaming, except divergences each accounted for by a ledger decision.",
    },
    Level { number: 4, name: "exact", means: "Level 3 with no divergence at all." },
];

/// A kind of divergence, and the ledger decision that accounts for it.
#[derive(Clone, Debug, Serialize)]
pub struct Kind {
    /// Its name.
    pub kind: String,
    /// The ledger decision that accounts for it, if one does.
    pub decision: Option<&'static str>,
    /// What differs.
    pub summary: String,
}

/// Names the kind of a divergence, and the decision that accounts for it.
pub fn classify(d: &Divergence) -> Kind {
    let op = d.op["op"].as_str().unwrap_or("");
    let recorded = &d.recorded;
    let model = &d.model;
    let error = |v: &Value| v.get("error").and_then(Value::as_str).map(str::to_string);
    let (kind, decision, summary): (&str, Option<&'static str>, String) = match (op, error(recorded), error(model)) {
        ("syncGroupRequest", None, None) => (
            "assignment by the client's assignor",
            Some("DEC-kafka-divergence-group-assignment"),
            "Kafka's classic protocol gives each member the partitions the leader's assignor chose; the contract's coordinator assigns round robin in member id order".into(),
        ),
        ("describeGroupRequest", None, None) => (
            "group view under the client's assignor",
            Some("DEC-kafka-divergence-group-assignment"),
            "the members' partitions in DescribeGroups are the leader's assignor's, not the contract coordinator's".into(),
        ),
        (_, Some(r), None) => ("refused by Kafka, accepted by the contract", None, format!("{op}: Kafka answered {r}")),
        (_, None, Some(m)) => ("accepted by Kafka, refused by the contract", None, format!("{op}: the contract answers {m}")),
        (_, Some(r), Some(m)) => ("a different refusal", None, format!("{op}: Kafka answered {r}, the contract {m}")),
        _ => ("a different answer", None, format!("{op}: Kafka answered {recorded}, the contract {model}")),
    };
    Kind { kind: kind.into(), decision, summary }
}

/// The ledger decision that accounts for a kind of note the recorder
/// wrote, if one does.
pub fn note_decision(kind: &str) -> Option<&'static str> {
    if kind.starts_with("retriable error") {
        Some("DEC-kafka-divergence-coordinator-timing")
    } else if kind == "rebalance in progress" {
        Some("DEC-kafka-divergence-rebalance-heartbeat")
    } else {
        None
    }
}

/// What a scenario did against one side.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Side {
    /// Whether the client's own checks passed.
    pub passed: bool,
    /// Why not, or what the client printed.
    pub output: String,
    /// Operations in the history.
    pub operations: usize,
    /// Laws that do not hold over the history or the state.
    pub violations: Vec<Value>,
    /// Answers that are not the reference model's.
    pub divergences: Vec<Value>,
    /// Notes the recorder wrote, by kind.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub notes: BTreeMap<String, u64>,
    /// How long it took, in milliseconds.
    pub millis: u128,
}

/// One scenario of one client against the real broker and the twin.
#[derive(Clone, Debug, Serialize)]
pub struct Run {
    /// The client.
    pub client: String,
    /// The scenario.
    pub scenario: String,
    /// Against the real broker, through the recorder.
    pub real: Side,
    /// Against a twin core.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub twin: Option<Side>,
}

/// How a conformance run is set up.
#[derive(Clone, Debug)]
pub struct ConformConfig {
    /// The real broker.
    pub real: String,
    /// What the real broker is, for the report, such as `apache/kafka:4.3.1`.
    pub real_name: String,
    /// The clients to run.
    pub clients: Vec<String>,
    /// The scenarios to run.
    pub scenarios: Vec<String>,
    /// Where the report and traces go.
    pub out: PathBuf,
    /// The twin core run beside: a command or `image:<image>`, or the Rust
    /// core in this process; none to skip.
    pub twin: Option<Option<String>>,
    /// The twin release the report is for.
    pub release: String,
    /// Where the client programs are.
    pub clients_dir: PathBuf,
    /// Keep each run's frames beside its history.
    pub frames: bool,
}

fn default_clients_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/clients")
}

fn have(tool: &str, arg: &str) -> bool {
    Command::new(tool).arg(arg).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// A client program ready to run a scenario.
enum Client {
    InProcess,
    Program { program: PathBuf, args: Vec<String>, dir: PathBuf },
}

fn prepare(client: &str, dir: &Path, scratch: &Path) -> Result<Client, String> {
    let d = dir.join(client);
    let run = |c: &mut Command, what: &str| -> Result<(), String> {
        let out = c.output().map_err(|e| format!("{what}: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("{what} failed: {}", String::from_utf8_lossy(&out.stderr)))
        }
    };
    match client {
        "librdkafka" => Ok(Client::InProcess),
        "kafkajs" => {
            if !have("node", "--version") {
                return Err("node is not installed".into());
            }
            if !d.join("node_modules/kafkajs").exists() {
                run(Command::new("npm").args(["ci", "--no-audit", "--no-fund"]).current_dir(&d), "npm ci")?;
            }
            Ok(Client::Program { program: "node".into(), args: vec!["scenarios.mjs".into()], dir: d })
        }
        "franz-go" => {
            if !have("go", "version") {
                return Err("go is not installed".into());
            }
            let bin = scratch.join("franzgo-scenarios");
            run(Command::new("go").arg("build").arg("-o").arg(&bin).arg(".").current_dir(&d), "go build")?;
            Ok(Client::Program { program: bin, args: vec![], dir: d })
        }
        "java" => {
            if !(have("mvn", "--version") && have("java", "-version")) {
                return Err("mvn or java is not installed".into());
            }
            run(Command::new("mvn").args(["-q", "-B", "package", "-DskipTests"]).current_dir(&d), "mvn package")?;
            let mut cp = vec![d.join("target/classes")];
            for jar in std::fs::read_dir(d.join("target/lib")).map_err(|e| e.to_string())? {
                cp.push(jar.map_err(|e| e.to_string())?.path());
            }
            let cp = std::env::join_paths(cp).map_err(|e| e.to_string())?.to_string_lossy().to_string();
            Ok(Client::Program {
                program: "java".into(),
                args: vec!["-cp".into(), cp, "dev.lawtwin.scenarios.Scenarios".into()],
                dir: d,
            })
        }
        other => Err(format!("no client {other}")),
    }
}

/// Runs a scenario with a client: whether it passed, and what it said.
fn drive(client: &Client, scenario: &str, bootstrap: &str, admin: &str, ns: &str) -> (bool, String) {
    match client {
        Client::InProcess => {
            let (scenario, bootstrap, admin, ns) = (scenario.to_string(), bootstrap.to_string(), admin.to_string(), ns.to_string());
            let handle = std::thread::spawn(move || rdkafka_scenarios::run(&scenario, &bootstrap, &admin, &ns));
            match handle.join() {
                Ok(Ok(v)) => (true, v.to_string()),
                Ok(Err(e)) => (false, e),
                Err(_) => (false, "the scenario panicked".into()),
            }
        }
        Client::Program { program, args, dir } => {
            let child = Command::new(program)
                .args(args)
                .args([scenario, bootstrap, admin])
                .env("LAWTWIN_NS", ns)
                .current_dir(dir)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn();
            let mut child = match child {
                Ok(c) => c,
                Err(e) => return (false, format!("cannot run {}: {e}", program.display())),
            };
            let deadline = Instant::now() + Duration::from_secs(180);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() > deadline => {
                        let _ = child.kill();
                        break;
                    }
                    _ => std::thread::sleep(Duration::from_millis(100)),
                }
            }
            match child.wait_with_output() {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    let text = if out.status.success() { stdout } else { tail(&format!("{stdout}\n{stderr}"), 1200) };
                    (out.status.success(), text)
                }
                Err(e) => (false, e.to_string()),
            }
        }
    }
}

fn tail(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.len() <= n { s.to_string() } else { format!("...{}", &s[s.len() - n..]) }
}

fn calls(history: &[Entry]) -> usize {
    history.iter().filter(|e| matches!(e.event, Event::Call { .. })).count()
}

fn history_text(history: &[Entry]) -> String {
    let mut s = String::new();
    for e in history {
        s.push_str(&serde_json::to_string(e).unwrap_or_default());
        s.push('\n');
    }
    s
}

/// One scenario against the real broker, through a fresh recorder.
fn against_real(
    config: &ConformConfig,
    runtime: &tokio::runtime::Runtime,
    client: &Client,
    name: &str,
    scenario: &str,
    run_id: &str,
) -> Result<Side, String> {
    let ns = format!("conform-{run_id}-{name}-{scenario}-");
    let running = runtime
        .block_on(record::start(RecorderConfig { upstream: config.real.clone(), clock: ClockMode::Frozen, ..Default::default() }))
        .map_err(|e| format!("{e:#}"))?;
    let rec = running.recorder.clone();
    let started = Instant::now();
    let (passed, output) = drive(client, scenario, &running.kafka.to_string(), &format!("http://{}", running.admin), &ns);
    let millis = started.elapsed().as_millis();
    // Let the last requests (a consumer leaving) come back.
    std::thread::sleep(Duration::from_millis(500));
    let history = rec.history();
    let traces = config.out.join("traces").join(name);
    std::fs::create_dir_all(&traces).map_err(|e| e.to_string())?;
    std::fs::write(traces.join(format!("{scenario}.history.jsonl")), history_text(&history)).map_err(|e| e.to_string())?;
    if config.frames {
        std::fs::write(traces.join(format!("{scenario}.frames.jsonl")), crate::frame::frames_text(&rec.frames())).map_err(|e| e.to_string())?;
    }
    Ok(Side {
        passed,
        output,
        operations: calls(&history),
        violations: rec.violations().iter().map(|v| serde_json::to_value(v).unwrap_or_default()).collect(),
        divergences: rec.divergences().iter().map(|d| serde_json::to_value(d).unwrap_or_default()).collect(),
        notes: rec.notes(),
        millis,
    })
}

/// One scenario against a fresh twin core, with the kit's checks.
fn against_twin(core: Option<&str>, client: &Client, scenario: &str, seed: u64) -> Side {
    let twin = TestTwin::start_core(core, seed);
    let started = Instant::now();
    let (passed, output) = drive(client, scenario, &twin.bootstrap(), &twin.admin_url(), "");
    let millis = started.elapsed().as_millis();
    let text = twin.history_text();
    let entries = lawtwin_kafka::admin::parse_history(&text).unwrap_or_default();
    let (model, replayed) = lawtwin_kafka::history::replay(&entries);
    let mut violations: Vec<Value> =
        lawtwin_kafka::laws::check_history(&entries).iter().map(|v| serde_json::to_value(v).unwrap_or_default()).collect();
    violations.extend(lawtwin_kafka::laws::check_state(&model.snapshot()).iter().map(|v| serde_json::to_value(v).unwrap_or_default()));
    Side {
        passed,
        output,
        operations: calls(&entries),
        violations,
        divergences: replayed.iter().map(|d| serde_json::to_value(d).unwrap_or_default()).collect(),
        notes: BTreeMap::new(),
        millis,
    }
}

/// The report of a conformance run.
pub fn report(config: &ConformConfig, runs: &[Run], skipped: &BTreeMap<String, String>, real_version: &str) -> Value {
    let mut kinds: BTreeMap<String, (Kind, usize, Vec<Value>)> = BTreeMap::new();
    for r in runs {
        for d in &r.real.divergences {
            let Ok(d) = serde_json::from_value::<Divergence>(d.clone()) else { continue };
            let k = classify(&d);
            let e = kinds.entry(k.kind.clone()).or_insert((k, 0, vec![]));
            e.1 += 1;
            if e.2.len() < 3 {
                e.2.push(json!({ "client": r.client, "scenario": r.scenario, "divergence": d }));
            }
        }
    }
    let mut notes: BTreeMap<String, u64> = BTreeMap::new();
    for r in runs {
        for (k, n) in &r.real.notes {
            *notes.entry(k.clone()).or_default() += n;
        }
    }
    let real_lawful = runs.iter().all(|r| r.real.violations.is_empty());
    let scenarios_hold = runs.iter().all(|r| r.real.passed || r.twin.as_ref().is_some_and(|t| !t.passed));
    let accounted = kinds.values().all(|(k, _, _)| k.decision.is_some()) && notes.keys().all(|k| note_decision(k).is_some());
    let exact = kinds.is_empty();
    let number = if runs.is_empty() {
        0
    } else if !real_lawful {
        0
    } else if !scenarios_hold {
        1
    } else if !accounted {
        2
    } else if !exact {
        3
    } else {
        4
    };
    let level = LEVELS[number as usize];
    let twin_ok = runs.iter().all(|r| r.twin.as_ref().is_none_or(|t| t.passed && t.violations.is_empty() && t.divergences.is_empty()));
    json!({
        "twin": "kafka",
        "release": config.release,
        "contract": { "package": "lawtwin.kafka", "version": "0.1.0" },
        "real": { "dependency": config.real_name, "version": real_version, "bootstrap": config.real },
        "twinCore": match &config.twin { None => Value::Null, Some(None) => json!("the Rust core"), Some(Some(c)) => json!(c) },
        "level": level,
        "levels": LEVELS,
        "summary": {
            "runs": runs.len(),
            "realPassed": runs.iter().filter(|r| r.real.passed).count(),
            "twinPassed": runs.iter().filter(|r| r.twin.as_ref().is_some_and(|t| t.passed)).count(),
            "realLawful": real_lawful,
            "twinLawful": twin_ok,
            "operations": runs.iter().map(|r| r.real.operations).sum::<usize>(),
            "divergences": kinds.values().map(|(_, n, _)| n).sum::<usize>(),
        },
        "clients": CLIENTS.iter().filter(|(c, _)| config.clients.iter().any(|x| x == c)).map(|(c, v)| json!({ "client": c, "version": v, "skipped": skipped.get(*c) })).collect::<Vec<_>>(),
        "wireNotes": notes.iter().map(|(k, n)| json!({ "note": k, "count": n, "decision": note_decision(k) })).collect::<Vec<_>>(),
        "divergenceKinds": kinds.values().map(|(k, n, examples)| json!({ "kind": k.kind, "decision": k.decision, "summary": k.summary, "count": n, "examples": examples })).collect::<Vec<_>>(),
        "runs": runs,
    })
}

fn mark(side: Option<&Side>) -> String {
    match side {
        None => "not run".into(),
        Some(s) if !s.passed => "**fails**".into(),
        Some(s) if !s.violations.is_empty() => format!("passes; **{} broken law(s)**", s.violations.len()),
        Some(s) if !s.divergences.is_empty() => format!("passes; {} divergence(s)", s.divergences.len()),
        Some(_) => "passes".into(),
    }
}

/// The report as Markdown.
pub fn markdown(report: &Value, runs: &[Run]) -> String {
    let mut s = String::new();
    let release = report["release"].as_str().unwrap_or("?");
    s.push_str(&format!("# Kafka twin {release}: conformance against {}\n\n", report["real"]["dependency"].as_str().unwrap_or("?")));
    s.push_str("Written by `lawtwin conform kafka`; do not edit. The JSON beside it has every run in full.\n\n");
    let level = &report["level"];
    s.push_str(&format!(
        "**Level {} ({})**: {}\n\n",
        level["number"],
        level["name"].as_str().unwrap_or(""),
        level["means"].as_str().unwrap_or("")
    ));
    let sum = &report["summary"];
    s.push_str(&format!(
        "{} runs: {} pass against the real broker and {} against the twin; {} operations recorded from the real broker; {} divergence(s); the real broker's histories {} every law the kit checks.\n\n",
        sum["runs"], sum["realPassed"], sum["twinPassed"], sum["operations"], sum["divergences"],
        if sum["realLawful"] == true { "satisfy" } else { "do not satisfy" }
    ));
    s.push_str("| Client | Scenario | Real broker, through the recorder | Twin |\n| --- | --- | --- | --- |\n");
    for r in runs {
        s.push_str(&format!("| {} | {} | {} | {} |\n", r.client, r.scenario, mark(Some(&r.real)), mark(r.twin.as_ref())));
    }
    s.push_str("\n## Divergences\n\n");
    let kinds = report["divergenceKinds"].as_array().cloned().unwrap_or_default();
    if kinds.is_empty() {
        s.push_str("None: every answer of the real broker is the reference model's, up to renaming.\n");
    } else {
        s.push_str("| Kind | Count | Accounted for by | What differs |\n| --- | --- | --- | --- |\n");
        for k in &kinds {
            s.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                k["kind"].as_str().unwrap_or(""),
                k["count"],
                k["decision"].as_str().map(|d| format!("ref:{d}")).unwrap_or_else(|| "**no decision**".into()),
                k["summary"].as_str().unwrap_or("").replace('|', "\\|")
            ));
        }
    }
    s.push_str("\n## What the wire showed beside the answers\n\n");
    let notes = report["wireNotes"].as_array().cloned().unwrap_or_default();
    if notes.is_empty() {
        s.push_str("Nothing: every exchange decoded to operations of the contract.\n");
    } else {
        s.push_str("Exchanges that are not operations of the contract, written to the history as notes.\n\n| Note | Count | Accounted for by |\n| --- | --- | --- |\n");
        for n in &notes {
            s.push_str(&format!(
                "| {} | {} | {} |\n",
                n["note"].as_str().unwrap_or(""),
                n["count"],
                n["decision"].as_str().map(|d| format!("ref:{d}")).unwrap_or_else(|| "**no decision**".into())
            ));
        }
    }
    let failures: Vec<&Run> = runs.iter().filter(|r| !r.real.passed || !r.real.violations.is_empty()).collect();
    if !failures.is_empty() {
        s.push_str("\n## Failures against the real broker\n\n");
        for r in failures {
            s.push_str(&format!("### {} {}\n\n```text\n{}\n```\n\n", r.client, r.scenario, r.real.output.replace("```", "'''")));
            for v in &r.real.violations {
                s.push_str(&format!("- broken law: {v}\n"));
            }
        }
    }
    s.push_str("\n## Levels\n\n");
    for l in LEVELS {
        s.push_str(&format!("{}. **{}**: {}\n", l.number, l.name, l.means));
    }
    s
}

/// Runs the conformance kit against the real broker, and writes the report.
pub fn conform(config: &ConformConfig) -> Result<Value, String> {
    std::fs::create_dir_all(&config.out).map_err(|e| format!("{}: {e}", config.out.display()))?;
    let scratch = std::env::temp_dir().join(format!("lawtwin-conform-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;
    let runtime = lawtwin_kafka::runtime().map_err(|e| e.to_string())?;
    let real_version = runtime.block_on(probe_version(&config.real)).unwrap_or_else(|e| format!("unknown ({e})"));
    let run_id = format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
    let mut runs = vec![];
    let mut skipped = BTreeMap::new();
    for (ci, name) in config.clients.iter().enumerate() {
        let client = match prepare(name, &config.clients_dir, &scratch) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("lawtwin conform: skipping {name}: {e}");
                skipped.insert(name.clone(), e);
                continue;
            }
        };
        for (si, scenario) in config.scenarios.iter().enumerate() {
            eprintln!("lawtwin conform: {name} {scenario} against {}", config.real);
            let real = against_real(config, &runtime, &client, name, scenario, &run_id)?;
            let twin = config.twin.as_ref().map(|core| {
                eprintln!("lawtwin conform: {name} {scenario} against the twin");
                against_twin(core.as_deref(), &client, scenario, 1000 + (ci * 100 + si) as u64)
            });
            eprintln!(
                "lawtwin conform: {name} {scenario}: real {}, twin {}",
                mark(Some(&real)).replace("**", ""),
                mark(twin.as_ref()).replace("**", "")
            );
            runs.push(Run { client: name.clone(), scenario: scenario.clone(), real, twin });
        }
    }
    let report = report(config, &runs, &skipped, &real_version);
    std::fs::write(config.out.join("report.json"), serde_json::to_string_pretty(&report).unwrap_or_default() + "\n").map_err(|e| e.to_string())?;
    std::fs::write(config.out.join("report.md"), markdown(&report, &runs)).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(report)
}

/// The broker's software version, from a Metadata or ApiVersions exchange:
/// the highest Produce version it offers stands in when nothing names it.
async fn probe_version(bootstrap: &str) -> anyhow::Result<String> {
    use kafka_protocol::messages::*;
    use kafka_protocol::protocol::{Decodable, Encodable, StrBytes};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(bootstrap).await?;
    let header = RequestHeader::default()
        .with_request_api_key(ApiKey::ApiVersions as i16)
        .with_request_api_version(3)
        .with_correlation_id(1)
        .with_client_id(Some(StrBytes::from_static_str("lawtwin-conform")));
    let mut buf = bytes::BytesMut::new();
    header.encode(&mut buf, ApiKey::ApiVersions.request_header_version(3))?;
    ApiVersionsRequest::default()
        .with_client_software_name(StrBytes::from_static_str("lawtwin"))
        .with_client_software_version(StrBytes::from_static_str("0.1.0"))
        .encode(&mut buf, 3)?;
    s.write_all(&(buf.len() as i32).to_be_bytes()).await?;
    s.write_all(&buf).await?;
    let mut len = [0u8; 4];
    s.read_exact(&mut len).await?;
    let mut body = vec![0u8; i32::from_be_bytes(len) as usize];
    s.read_exact(&mut body).await?;
    let mut b = bytes::Bytes::from(body);
    let _ = ResponseHeader::decode(&mut b, 0)?;
    let r = ApiVersionsResponse::decode(&mut b, 3)?;
    let produce = r.api_keys.iter().find(|k| k.api_key == 0).map(|k| k.max_version).unwrap_or(-1);
    let features: Vec<String> = r.finalized_features.iter().map(|f| format!("{}={}", f.name, f.max_version_level)).collect();
    Ok(format!("Produce up to v{produce}; finalized features {}", features.join(", ")))
}

/// `lawtwin conform kafka`.
pub fn command(rest: &[String]) -> Result<(), String> {
    let a = crate::cli::Args::parse(rest, &["no-twin", "frames"]);
    a.known(&["real", "real-name", "client", "scenario", "out", "twin", "no-twin", "release", "clients", "frames"])?;
    let real = a.get("real").ok_or("conform needs --real <host:port>, the real broker")?.to_string();
    let clients: Vec<String> = match a.all("client") {
        v if v.is_empty() => CLIENTS.iter().map(|(c, _)| c.to_string()).collect(),
        v => v,
    };
    let scenarios: Vec<String> = match a.all("scenario") {
        v if v.is_empty() => SCENARIOS.iter().map(|s| s.to_string()).collect(),
        v => v,
    };
    let release = a.get("release").unwrap_or(env!("CARGO_PKG_VERSION")).to_string();
    let config = ConformConfig {
        real_name: a.get("real-name").unwrap_or("a Kafka broker").to_string(),
        real,
        clients,
        scenarios,
        out: PathBuf::from(a.get("out").map(str::to_string).unwrap_or_else(|| format!("conformance-kafka-{release}"))),
        twin: if a.has("no-twin") { None } else { Some(a.get("twin").map(str::to_string)) },
        release,
        clients_dir: a.get("clients").map(PathBuf::from).unwrap_or_else(default_clients_dir),
        frames: a.has("frames"),
    };
    let report = conform(&config)?;
    let level = &report["level"];
    eprintln!(
        "lawtwin conform: level {} ({}); report in {}",
        level["number"],
        level["name"].as_str().unwrap_or(""),
        config.out.join("report.md").display()
    );
    let sum = &report["summary"];
    if sum["realLawful"] == true && sum["twinLawful"] == true {
        Ok(())
    } else {
        Err("a law does not hold over a history, or the twin failed its own checks".into())
    }
}
