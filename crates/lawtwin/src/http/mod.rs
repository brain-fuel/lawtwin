//! The HTTP twin's commands: `up http` starts a core, `peer http call` is
//! the simulated client of an application's API, `conform http` runs the
//! contract's laws against a real service and the twin, and `record http`
//! and `replay http` record real traffic through a proxy and replay it
//! through the twin. None of them knows a core's language: they speak HTTP
//! and the admin API (docs/reference/http-twin.md).

mod record;
mod session;

use lawtwin_wire::WireMap;
use lawtwin_wire::http::Endpoint;
use lawtwin_wire::map::find_contract;
use serde_json::{Value, json};
use session::{Options, Session};
use std::time::Duration;

pub const USAGE: &str = "  lawtwin up http --contract <name> [--port <n>] [--admin-port <n>] [--host <addr>]
          [--advertise <host:port>] [--seed <n>] [--clock frozen|wall]
          [--core <command> | --container | --image <image>]
  lawtwin peer http call --contract <name> --target <url> [--seed <n>] [--rounds <n>]
          [--invalid] [--retries] [--duplicates] [--timeouts] [--all-operations]
          [--timeout <duration>] [--report <file>]
  lawtwin conform http --contract <name> --real <url> [--twin <url> --twin-admin <url>]
          [--seed <n>] [--rounds <n>] [--report <file>]
  lawtwin record http --contract <name> --upstream <url> --listen <host:port>
          [--out <trace.jsonl>] [--count <n>]
  lawtwin replay http <trace.jsonl> --contract <name> [--target <url>] [--admin <url>]
          [--no-reset] [--report <file>]";

/// A command's outcome: success, or failure with a message and exit code.
pub type Outcome = Result<(), (String, u8)>;

fn fail(e: impl Into<String>) -> (String, u8) {
    (e.into(), 1)
}

/// The value of an option, and the rest of the arguments.
pub struct Args {
    items: Vec<String>,
}

impl Args {
    pub fn new(items: &[String]) -> Args {
        Args { items: items.to_vec() }
    }

    /// Takes `--name <value>`.
    pub fn value(&mut self, name: &str) -> Result<Option<String>, String> {
        match self.items.iter().position(|a| a == name) {
            Some(i) if i + 1 < self.items.len() => {
                let v = self.items.remove(i + 1);
                self.items.remove(i);
                Ok(Some(v))
            }
            Some(_) => Err(format!("{name} needs a value")),
            None => Ok(None),
        }
    }

    /// Takes `--name`.
    pub fn flag(&mut self, name: &str) -> bool {
        match self.items.iter().position(|a| a == name) {
            Some(i) => {
                self.items.remove(i);
                true
            }
            None => false,
        }
    }

    /// What is left, which must be nothing but positional arguments.
    pub fn rest(self) -> Result<Vec<String>, String> {
        if let Some(a) = self.items.iter().find(|a| a.starts_with("--")) {
            return Err(format!("unknown option {a}"));
        }
        Ok(self.items)
    }
}

/// Reads a duration such as `250ms`, `5s` or `2m`.
pub fn duration(s: &str) -> Result<Duration, String> {
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (n, unit) = s.split_at(split);
    let n: u64 = n.parse().map_err(|_| format!("{s} is not a duration such as 500ms or 5s"))?;
    Ok(match unit {
        "ms" => Duration::from_millis(n),
        "s" | "" => Duration::from_secs(n),
        "m" => Duration::from_secs(n * 60),
        _ => return Err(format!("{s} is not a duration such as 500ms or 5s")),
    })
}

/// The wire map of a contract named on the command line.
pub fn contract(args: &mut Args) -> Result<WireMap, String> {
    let name = args.value("--contract")?.ok_or("--contract is required: a contract's name (petstore), package or directory")?;
    WireMap::load(&find_contract(&name)?)
}

fn write_report(path: Option<String>, report: &Value) -> Result<(), String> {
    if let Some(p) = path {
        std::fs::write(&p, serde_json::to_string_pretty(report).unwrap() + "\n").map_err(|e| format!("{p}: {e}"))?;
    }
    Ok(())
}

/// Runs an HTTP command; `None` when it is not one.
pub fn run(command: &str, rest: &[String]) -> Option<Outcome> {
    let sub = rest.first().map(String::as_str);
    let out = match (command, sub) {
        ("up", Some("http")) => up(&rest[1..]).map_err(fail),
        ("peer", Some("http")) if rest.get(1).map(String::as_str) == Some("call") => peer(&rest[2..]),
        ("peer", _) => Err(fail("peer takes: http call, kafka produce or kafka consume")),
        ("conform", Some("http")) => conform(&rest[1..]),
        ("record", Some("http")) => record::record(&rest[1..]).map_err(fail),
        ("replay", Some("http")) => record::replay(&rest[1..]),
        _ => return None,
    };
    Some(out)
}

fn session_options(args: &mut Args) -> Result<Options, String> {
    let mut o = Options::default();
    if let Some(s) = args.value("--seed")? {
        o.seed = s.parse().map_err(|_| "--seed is a whole number")?;
    }
    if let Some(n) = args.value("--rounds")? {
        o.rounds = n.parse().map_err(|_| "--rounds is a whole number")?;
    }
    if let Some(t) = args.value("--timeout")? {
        o.timeout = duration(&t)?;
    }
    o.invalid = args.flag("--invalid");
    o.retries = args.flag("--retries");
    o.duplicates = args.flag("--duplicates");
    o.timeouts = args.flag("--timeouts");
    o.all_operations = args.flag("--all-operations");
    Ok(o)
}

fn print_summary(what: &str, report: &Value) {
    let n = |k: &str| report[k].as_array().map(Vec::len).unwrap_or(0);
    println!(
        "{what}: {} against {}: {} requests, {} laws checked over {} calls; {} violation(s), {} finding(s), {} problem(s)",
        report["contract"].as_str().unwrap_or("?"),
        report["target"].as_str().unwrap_or("?"),
        report["requests"],
        n("laws"),
        report["calls"],
        n("violations"),
        n("findings"),
        n("problems"),
    );
    for v in report["violations"].as_array().into_iter().flatten() {
        println!("  violation: `{}`: {}", v["law"].as_str().unwrap_or("?"), v["detail"].as_str().unwrap_or(""));
    }
    for f in report["findings"].as_array().into_iter().flatten() {
        println!("  finding: {}", f["finding"].as_str().unwrap_or(""));
    }
    for p in report["problems"].as_array().into_iter().flatten().take(20) {
        println!("  problem: {}", p["problem"].as_str().unwrap_or(""));
    }
}

/// `lawtwin peer http call`: the simulated client of an application's API.
fn peer(rest: &[String]) -> Outcome {
    let mut args = Args::new(rest);
    let map = contract(&mut args).map_err(fail)?;
    let target = args.value("--target").map_err(fail)?.ok_or_else(|| fail("--target is required: the application's URL"))?;
    let report_path = args.value("--report").map_err(fail)?;
    let opts = session_options(&mut args).map_err(fail)?;
    args.rest().map_err(fail)?;
    let ep = Endpoint::parse(&target).map_err(fail)?;
    let mut s = Session::new(&map, ep, opts);
    s.run();
    let report = s.report(&target);
    print_summary("lawtwin peer http call", &report);
    write_report(report_path, &report).map_err(fail)?;
    if report["holds"] == json!(true) && report["problems"].as_array().is_some_and(Vec::is_empty) {
        Ok(())
    } else {
        Err(("the application broke the contract".into(), 1))
    }
}

fn admin_request(admin: &str, method: &str, path: &str, body: &str) -> Result<(u16, Value), String> {
    let ep = Endpoint::parse(admin)?;
    let out = lawtwin_wire::call::Outgoing {
        method: method.into(),
        target: path.into(),
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: body.as_bytes().to_vec(),
    };
    let r = lawtwin_wire::http::send(&ep, &out, Duration::from_secs(60)).map_err(|e| format!("the twin's admin API at {admin}: {e}"))?;
    Ok((r.status, serde_json::from_slice(&r.body).unwrap_or(Value::Null)))
}

/// `lawtwin conform http`: the contract's laws against a real service, and
/// against the twin with the same traffic; each answer the twin gives
/// differently is a divergence.
fn conform(rest: &[String]) -> Outcome {
    let mut args = Args::new(rest);
    let map = contract(&mut args).map_err(fail)?;
    let real = args.value("--real").map_err(fail)?.ok_or_else(|| fail("--real is required: the real service's URL"))?;
    let twin = args.value("--twin").map_err(fail)?;
    let twin_admin = args.value("--twin-admin").map_err(fail)?;
    let report_path = args.value("--report").map_err(fail)?;
    let opts = session_options(&mut args).map_err(fail)?;
    args.rest().map_err(fail)?;
    let mut real_s = Session::new(&map, Endpoint::parse(&real).map_err(fail)?, opts.clone());
    real_s.run();
    let real_report = real_s.report(&real);
    print_summary("real", &real_report);
    let mut report = json!({ "contract": map.package, "real": real_report });
    let mut divergences = vec![];
    if let Some(t) = &twin {
        if let Some(a) = &twin_admin {
            admin_request(a, "DELETE", "/v1/reset", "").map_err(fail)?;
        }
        let mut twin_s = Session::new(&map, Endpoint::parse(t).map_err(fail)?, opts);
        twin_s.run();
        let twin_report = twin_s.report(t);
        print_summary("twin", &twin_report);
        for (a, b) in real_s.exchanges.iter().zip(&twin_s.exchanges) {
            let same = a.get("target") == b.get("target") && a.get("status") == b.get("status") && a.get("answer") == b.get("answer");
            if !same {
                divergences.push(json!({ "op": a["op"], "real": a, "twin": b }));
            }
        }
        if real_s.exchanges.len() != twin_s.exchanges.len() {
            divergences.push(json!({ "detail": format!("the real service saw {} requests and the twin {}", real_s.exchanges.len(), twin_s.exchanges.len()) }));
        }
        report["twin"] = twin_report;
    }
    let real_lawful = report["real"]["violations"].as_array().is_some_and(Vec::is_empty);
    let real_clean = report["real"]["findings"].as_array().is_some_and(Vec::is_empty);
    let level = match (real_lawful, real_clean, twin.is_some(), divergences.is_empty()) {
        (false, ..) => "divergent: the real service breaks a law of the contract",
        (true, true, true, true) => "conformant: the laws hold on the real service and the twin answers every request as it does",
        (true, _, true, _) => "lawful: the laws hold on the real service; it does what the document forbids, or the twin answers some requests differently",
        (true, ..) => "lawful: the laws hold on the real service",
    };
    for d in divergences.iter().take(20) {
        println!(
            "  divergence: {} {}: real {} {}, twin {} {}",
            d["real"]["method"].as_str().unwrap_or(""),
            d["real"]["target"].as_str().unwrap_or(d["detail"].as_str().unwrap_or("")),
            d["real"]["status"],
            d["real"]["answer"],
            d["twin"]["status"],
            d["twin"]["answer"]
        );
    }
    println!("lawtwin conform http: {} divergence(s); {level}", divergences.len());
    report["divergences"] = Value::Array(divergences);
    report["level"] = json!(level.split(':').next().unwrap_or(level));
    write_report(report_path, &report).map_err(fail)?;
    if real_lawful { Ok(()) } else { Err(("the real service breaks a law of the contract".into(), 1)) }
}

/// `lawtwin up http`: runs one of the HTTP twin's cores, as a program or an image.
fn up(args: &[String]) -> Result<(), String> {
    let mut a = Args::new(args);
    let core = a.value("--core")?;
    let image = a.value("--image")?;
    let container = a.flag("--container") || image.is_some();
    let image = image.unwrap_or_else(|| format!("lawtwin/http:{}", env!("CARGO_PKG_VERSION")));
    let port = a.value("--port")?.unwrap_or_else(|| "8080".into());
    let admin_port = a.value("--admin-port")?.unwrap_or_else(|| "9644".into());
    let mut pass = vec![];
    for opt in ["--contract", "--host", "--advertise", "--seed", "--clock"] {
        if let Some(v) = a.value(opt)? {
            pass.push(opt.to_string());
            pass.push(v);
        }
    }
    a.rest()?;
    if container {
        let podman = std::env::var("PODMAN").unwrap_or_else(|_| "podman".into());
        let mut cmd = std::process::Command::new(podman);
        cmd.args(["run", "--rm", "-p", &format!("{port}:8080"), "-p", &format!("{admin_port}:9644"), &image]);
        let mut i = 0;
        while i < pass.len() {
            if pass[i] != "--host" {
                cmd.args([&pass[i], &pass[i + 1]]);
            }
            i += 2;
        }
        if !pass.iter().any(|p| p == "--advertise") {
            cmd.args(["--advertise", &format!("localhost:{port}")]);
        }
        let status = cmd.status().map_err(|e| format!("cannot run podman: {e}"))?;
        return if status.success() { Ok(()) } else { Err("the twin's container stopped with an error".into()) };
    }
    let command = core.or_else(|| std::env::var("LAWTWIN_HTTP_CORE").ok()).unwrap_or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join("lawtwin-http")))
            .filter(|p| p.exists())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "lawtwin-http".into())
    });
    let mut parts = command.split_whitespace();
    let program = parts.next().ok_or("the core command is empty")?;
    let status = std::process::Command::new(program)
        .args(parts)
        .args(["--port", &port, "--admin-port", &admin_port])
        .args(&pass)
        .status()
        .map_err(|e| format!("cannot run the core {program}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("the core {program} stopped with an error")) }
}
