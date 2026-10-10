//! kafkajs, franz-go and the Java client against the twin. Each client runs
//! the same scenarios (producing and consuming, a group rebalance, the
//! idempotent producer with a lost acknowledgement, transactions read
//! committed with a fenced zombie, every declared fault, and forced states)
//! against a fresh twin; the scenario checks what the client sees, and the
//! test then checks the contract's laws over the twin's history and replays
//! it through a fresh model.
//!
//! A client whose toolchain (node, go, mvn and java) is missing is skipped,
//! unless LAWTWIN_REQUIRE_CLIENTS is set, as `make twin-test` sets it.

mod common;

use common::TestTwin;
use std::path::{Path, PathBuf};
use std::process::Command;

const SCENARIOS: &[&str] = &["produce-consume", "group-rebalance", "idempotent", "transactions", "faults", "force"];

fn dir(client: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/clients").join(client)
}

fn have(tool: &str, arg: &str) -> bool {
    Command::new(tool).arg(arg).output().is_ok_and(|o| o.status.success())
}

fn skip(client: &str, why: &str) -> bool {
    if std::env::var_os("LAWTWIN_REQUIRE_CLIENTS").is_some() {
        panic!("{client}: {why}");
    }
    eprintln!("skipping {client}: {why}");
    true
}

fn run(prepare: &mut Command, what: &str) {
    let out = prepare.output().unwrap_or_else(|e| panic!("{what}: {e}"));
    assert!(out.status.success(), "{what} failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

/// Runs every scenario of a client, each against a fresh twin.
fn scenarios(client: &str, command: impl Fn(&str, &TestTwin) -> Command) {
    for (i, scenario) in SCENARIOS.iter().enumerate() {
        let twin = TestTwin::start(100 + i as u64);
        let out = command(scenario, &twin).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{client} {scenario} failed:\n{stdout}{}\ntwin history tail:\n{}",
            String::from_utf8_lossy(&out.stderr),
            twin.history_text().lines().rev().take(15).collect::<Vec<_>>().join("\n")
        );
        println!("{}", stdout.trim());
        twin.assert_lawful();
    }
}

#[test]
fn kafkajs() {
    if !have("node", "--version") && skip("kafkajs", "node is not installed") {
        return;
    }
    let d = dir("kafkajs");
    if !d.join("node_modules/kafkajs").exists() {
        run(Command::new("npm").args(["ci", "--no-audit", "--no-fund"]).current_dir(&d), "npm ci");
    }
    scenarios("kafkajs", |scenario, twin| {
        let mut c = Command::new("node");
        c.arg("scenarios.mjs").arg(scenario).arg(twin.bootstrap()).arg(twin.admin_url()).current_dir(&d);
        c
    });
}

#[test]
fn franz_go() {
    if !have("go", "version") && skip("franz-go", "go is not installed") {
        return;
    }
    let d = dir("franz-go");
    let bin = Path::new(env!("CARGO_TARGET_TMPDIR")).join("franzgo-scenarios");
    run(Command::new("go").arg("build").arg("-o").arg(&bin).arg(".").current_dir(&d), "go build");
    scenarios("franz-go", |scenario, twin| {
        let mut c = Command::new(&bin);
        c.arg(scenario).arg(twin.bootstrap()).arg(twin.admin_url());
        c
    });
}

#[test]
fn java() {
    if !(have("mvn", "--version") && have("java", "-version")) && skip("java", "mvn or java is not installed") {
        return;
    }
    let d = dir("java");
    run(Command::new("mvn").args(["-q", "-B", "package", "-DskipTests"]).current_dir(&d), "mvn package");
    let mut cp = vec![d.join("target/classes")];
    for jar in std::fs::read_dir(d.join("target/lib")).unwrap() {
        cp.push(jar.unwrap().path());
    }
    let cp = std::env::join_paths(cp).unwrap();
    scenarios("java", |scenario, twin| {
        let mut c = Command::new("java");
        c.arg("-cp").arg(&cp).arg("dev.lawtwin.scenarios.Scenarios").arg(scenario).arg(twin.bootstrap()).arg(twin.admin_url());
        c
    });
}
