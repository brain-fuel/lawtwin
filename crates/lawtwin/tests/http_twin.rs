//! The HTTP commands, end to end: the OpenAPI client twin (`peer http
//! call`) against the HTTP twin and against a tiny Petstore written by hand
//! (examples/http/petstore-app), with and without its bugs; `conform http`;
//! and a trace recorded through `record http` in front of the Petstore,
//! replayed through the twin with no divergence, and through a mutated twin,
//! which is caught.

use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

fn lawtwin(args: &[&str]) -> Output {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    Command::new(env!("CARGO_BIN_EXE_lawtwin")).current_dir(root).args(args).output().unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn wait_for(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "nothing listens on {port}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A child process killed when the test ends.
struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn twin(contract: &str, mutant: Option<&str>) -> lawtwin_http::Running {
    lawtwin_http::start(lawtwin_http::Config {
        contract: contract.into(),
        listen: "127.0.0.1:0".into(),
        admin_listen: "127.0.0.1:0".into(),
        mutant: mutant.map(str::to_string),
        ..Default::default()
    })
    .unwrap()
}

/// The hand-written Petstore, with a bug if asked; `None` without python3.
fn petstore_app(bug: Option<&str>) -> Option<(Proc, u16)> {
    Command::new("python3").arg("--version").output().ok()?;
    let port = free_port();
    let app = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/http/petstore-app/app.py");
    let mut cmd = Command::new("python3");
    cmd.arg(app).args(["--port", &port.to_string()]).stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(b) = bug {
        cmd.args(["--bug", b]);
    }
    let child = Proc(cmd.spawn().ok()?);
    wait_for(port);
    Some((child, port))
}

const FAULTY: &[&str] = &["--invalid", "--retries", "--duplicates", "--timeouts"];

#[test]
fn the_client_twin_finds_the_twin_lawful() {
    for contract in ["petstore", "github"] {
        let t = twin(contract, None);
        let target = format!("http://{}", t.api);
        let mut args = vec!["peer", "http", "call", "--contract", contract, "--target", &target, "--seed", "4"];
        args.extend(FAULTY);
        let o = lawtwin(&args);
        assert!(o.status.success(), "{contract}: {}", text(&o));
        assert!(text(&o).contains("0 violation(s), 0 finding(s), 0 problem(s)"), "{}", text(&o));
    }
}

#[test]
fn the_client_twin_catches_a_mutated_twin() {
    let t = twin("petstore", Some("skip-delete"));
    let o = lawtwin(&["peer", "http", "call", "--contract", "petstore", "--target", &format!("http://{}", t.api), "--rounds", "1"]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("violation: `a deleted pet is not found`"), "{}", text(&o));
}

#[test]
fn the_client_twin_checks_an_application_and_finds_its_bugs() {
    let Some((_app, port)) = petstore_app(None) else { return eprintln!("python3 is not installed; skipped") };
    let target = format!("http://127.0.0.1:{port}");
    let mut args = vec!["peer", "http", "call", "--contract", "petstore", "--target", &target, "--seed", "6"];
    args.extend(FAULTY);
    let o = lawtwin(&args);
    assert!(o.status.success(), "{}", text(&o));
    for (bug, expected) in [
        ("forget-delete", "violation: `a deleted pet is not found`"),
        ("lose-field", "pet reads back"),
        ("crash-invalid", "failed with 500 on a request that breaks the document"),
    ] {
        let (_app, port) = petstore_app(Some(bug)).unwrap();
        let target = format!("http://127.0.0.1:{port}");
        let o = lawtwin(&["peer", "http", "call", "--contract", "petstore", "--target", &target, "--rounds", "1", "--invalid"]);
        assert_eq!(o.status.code(), Some(1), "{bug}: {}", text(&o));
        assert!(text(&o).contains(expected), "{bug}: {}", text(&o));
    }
}

#[test]
fn conformance_against_the_petstore_finds_the_twin_conformant() {
    let Some((_app, port)) = petstore_app(None) else { return eprintln!("python3 is not installed; skipped") };
    let t = twin("petstore", None);
    let report = std::env::temp_dir().join(format!("lawtwin-conform-{port}.json"));
    let o = lawtwin(&[
        "conform", "http", "--contract", "petstore", "--real", &format!("http://127.0.0.1:{port}"),
        "--twin", &format!("http://{}", t.api), "--twin-admin", &format!("http://{}", t.admin),
        "--seed", "8", "--rounds", "2", "--retries", "--report", report.to_str().unwrap(),
    ]);
    assert!(o.status.success(), "{}", text(&o));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    assert_eq!(v["level"], "conformant", "{}", text(&o));
}

#[test]
fn a_recorded_trace_replays_through_the_twin_and_catches_a_mutant() {
    let Some((_app, port)) = petstore_app(None) else { return eprintln!("python3 is not installed; skipped") };
    let proxy = free_port();
    let trace = std::env::temp_dir().join(format!("lawtwin-trace-{proxy}.jsonl"));
    let recorder = Proc(
        Command::new(env!("CARGO_BIN_EXE_lawtwin"))
            .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
            .args(["record", "http", "--contract", "petstore", "--upstream", &format!("http://127.0.0.1:{port}")])
            .args(["--listen", &format!("127.0.0.1:{proxy}"), "--out", trace.to_str().unwrap()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_for(proxy);
    let o = lawtwin(&["peer", "http", "call", "--contract", "petstore", "--target", &format!("http://127.0.0.1:{proxy}"), "--seed", "12", "--retries"]);
    assert!(o.status.success(), "{}", text(&o));
    // Requests that break the document, to operations the Petstore serves.
    for (method, target, body) in [("POST", "/pet", "{not json"), ("POST", "/pet", r#"{"photoUrls":[]}"#), ("GET", "/pet/abc", ""), ("POST", "/store/order", r#"{"shipDate":"soon"}"#)] {
        let out = lawtwin_wire::call::Outgoing { method: method.into(), target: target.into(), headers: vec![("Content-Type".into(), "application/json".into())], body: body.as_bytes().to_vec() };
        let ep = lawtwin_wire::http::Endpoint::parse(&format!("http://127.0.0.1:{proxy}")).unwrap();
        assert!(lawtwin_wire::http::send(&ep, &out, Duration::from_secs(5)).unwrap().status >= 400);
    }
    drop(recorder);
    let lines = std::fs::read_to_string(&trace).unwrap().lines().count();
    assert!(lines > 40, "the trace has {lines} exchanges");
    let t = twin("petstore", None);
    let o = lawtwin(&["replay", "http", trace.to_str().unwrap(), "--contract", "petstore", "--target", &format!("http://{}", t.api), "--admin", &format!("http://{}", t.admin)]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains(": 0 divergence(s)"), "{}", text(&o));
    for mutant in ["drop-field", "skip-delete"] {
        let m = twin("petstore", Some(mutant));
        let o = lawtwin(&["replay", "http", trace.to_str().unwrap(), "--contract", "petstore", "--target", &format!("http://{}", m.api), "--admin", &format!("http://{}", m.admin)]);
        assert_eq!(o.status.code(), Some(1), "{mutant}: {}", text(&o));
        assert!(text(&o).contains("divergence at"), "{mutant}: {}", text(&o));
    }
}
