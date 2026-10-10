//! The control-plane commands of `lawtwin`: `up` starts a twin, and the
//! others speak to a running twin's admin API (`--admin`, or
//! `LAWTWIN_ADMIN`, by default `http://127.0.0.1:9644`).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// The usage of the control-plane commands.
pub const USAGE: &str = "  lawtwin up kafka [--port <n>] [--admin-port <n>] [--host <addr>] [--advertise <host:port>]
          [--seed <n>] [--clock frozen|wall] [--partitions <n>] [--no-auto-create]
          [--rebalance-wait <duration>] [--container [--image <image>]]
  lawtwin inspect [--records]
  lawtwin force <state.json | ->
  lawtwin fault <name> [<param>=<value>]...
  lawtwin fault clear [<name>]
  lawtwin faults
  lawtwin advance <duration>
  lawtwin history [--since <seq>] [--limit <n>] [--array]
  lawtwin replay [<history file> | -]
  lawtwin laws
  lawtwin seed [<n>]
  lawtwin reset

The commands after up speak to a running twin's admin API, at --admin <url>
or LAWTWIN_ADMIN (default http://127.0.0.1:9644).";

/// What an admin request answered.
struct Reply {
    status: u16,
    body: String,
}

fn admin_url(args: &mut Vec<String>) -> String {
    if let Some(i) = args.iter().position(|a| a == "--admin") {
        if i + 1 < args.len() {
            let url = args.remove(i + 1);
            args.remove(i);
            return url;
        }
    }
    std::env::var("LAWTWIN_ADMIN").unwrap_or_else(|_| "http://127.0.0.1:9644".into())
}

/// One HTTP/1.1 request to the admin API.
fn request(base: &str, method: &str, path: &str, body: Option<&str>) -> Result<Reply, String> {
    let rest = base.strip_prefix("http://").ok_or_else(|| format!("the admin URL {base} must start with http://"))?;
    let authority = rest.trim_end_matches('/');
    let mut stream = TcpStream::connect(authority).map_err(|e| format!("cannot reach the twin's admin API at {base}: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(300))).ok();
    let body = body.unwrap_or("");
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(body.as_bytes())).map_err(|e| e.to_string())?;
    let mut raw = vec![];
    stream.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&raw).to_string();
    let (head, payload) = text.split_once("\r\n\r\n").ok_or("the twin sent no HTTP response")?;
    let status = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or("the twin sent no status")?;
    let chunked = head.lines().any(|l| l.to_ascii_lowercase().starts_with("transfer-encoding:") && l.contains("chunked"));
    let body = if chunked { dechunk(payload) } else { payload.to_string() };
    Ok(Reply { status, body })
}

fn dechunk(mut s: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = s.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 || rest.len() < n {
            break;
        }
        out.push_str(&rest[..n]);
        s = rest[n..].trim_start_matches("\r\n");
    }
    out
}

fn pretty(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .map(|v| serde_json::to_string_pretty(&v).unwrap_or_default())
        .unwrap_or_else(|_| body.to_string())
}

fn ok(reply: Reply) -> Result<(), String> {
    if (200..300).contains(&reply.status) {
        println!("{}", pretty(&reply.body));
        Ok(())
    } else {
        let v: serde_json::Value = serde_json::from_str(&reply.body).unwrap_or_default();
        Err(v["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("the twin answered {}: {}", reply.status, reply.body)))
    }
}

fn read_input(path: &str) -> Result<String, String> {
    if path == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).map_err(|e| e.to_string())?;
        Ok(s)
    } else {
        std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
    }
}

/// A command's outcome: success, or failure with a message and exit code.
pub type Outcome = Result<(), (String, u8)>;

fn fail(e: String) -> (String, u8) {
    (e, 1)
}

/// Runs a control-plane command; `None` when the command is not one.
pub fn run(command: &str, rest: &[String]) -> Option<Outcome> {
    let mut args = rest.to_vec();
    let admin = admin_url(&mut args);
    let a = |i: usize| args.get(i).map(String::as_str);
    let result: Outcome = match command {
        "up" => up(&args).map_err(fail),
        "inspect" => {
            let records = args.iter().any(|x| x == "--records");
            request(&admin, "GET", if records { "/v1/state?records=true" } else { "/v1/state" }, None).and_then(ok).map_err(fail)
        }
        "force" => {
            let Some(path) = a(0) else { return Some(Err(fail("force needs a state file, or - for standard input".into()))) };
            match read_input(path).and_then(|body| request(&admin, "POST", "/v1/state", Some(&body))) {
                Err(e) => Err(fail(e)),
                Ok(r) if r.status == 422 => {
                    let v: serde_json::Value = serde_json::from_str(&r.body).unwrap_or_default();
                    Err((
                        format!(
                            "refused: the law `{}` forbids this state: {}",
                            v["law"].as_str().unwrap_or("?"),
                            v["reason"].as_str().unwrap_or("?")
                        ),
                        2,
                    ))
                }
                Ok(r) => ok(r).map_err(fail),
            }
        }
        "fault" => match a(0) {
            None => Err(fail("fault needs a fault name; `lawtwin faults` lists them".into())),
            Some("clear") => {
                let path = match a(1) {
                    Some(name) => format!("/v1/faults/{name}"),
                    None => "/v1/faults".into(),
                };
                request(&admin, "DELETE", &path, None).and_then(ok).map_err(fail)
            }
            Some(name) => {
                let mut params = serde_json::Map::new();
                for p in &args[1..] {
                    let Some((k, v)) = p.split_once('=') else {
                        return Some(Err(fail(format!("{p} is not a parameter; write name=value"))));
                    };
                    let value = v.parse::<i64>().map(serde_json::Value::from).unwrap_or_else(|_| v.into());
                    params.insert(k.to_string(), value);
                }
                let body = serde_json::Value::Object(params).to_string();
                request(&admin, "POST", &format!("/v1/faults/{name}"), Some(&body)).and_then(ok).map_err(fail)
            }
        },
        "faults" => request(&admin, "GET", "/v1/faults", None).and_then(ok).map_err(fail),
        "advance" => match a(0) {
            None => Err(fail("advance needs a duration, such as 5s".into())),
            Some(d) => {
                let body = serde_json::json!({ "by": d }).to_string();
                request(&admin, "POST", "/v1/clock/advance", Some(&body)).and_then(ok).map_err(fail)
            }
        },
        "history" => {
            let mut query = vec![];
            let mut array = false;
            let mut i = 0;
            while i < args.len() {
                match args[i].as_str() {
                    "--since" | "--limit" if i + 1 < args.len() => {
                        query.push(format!("{}={}", &args[i][2..], args[i + 1]));
                        i += 1;
                    }
                    "--array" => array = true,
                    other => return Some(Err(fail(format!("history does not take {other}")))),
                }
                i += 1;
            }
            if !array {
                query.push("format=jsonl".into());
            }
            let path = format!("/v1/history?{}", query.join("&"));
            match request(&admin, "GET", &path, None) {
                Ok(r) if array => ok(r).map_err(fail),
                Ok(r) if r.status == 200 => {
                    print!("{}", r.body);
                    Ok(())
                }
                Ok(r) => Err(fail(format!("the twin answered {}", r.status))),
                Err(e) => Err(fail(e)),
            }
        }
        "replay" => {
            let body = match a(0) {
                Some(path) => match read_input(path) {
                    Ok(b) => b,
                    Err(e) => return Some(Err(fail(e))),
                },
                None => String::new(),
            };
            match request(&admin, "POST", "/v1/history/replay", Some(&body)) {
                Ok(r) if r.status == 200 => {
                    let v: serde_json::Value = serde_json::from_str(&r.body).unwrap_or_default();
                    println!("{}", pretty(&r.body));
                    let diverged = v["divergences"].as_array().is_some_and(|d| !d.is_empty());
                    let broken = v["violations"]["state"].as_array().is_some_and(|d| !d.is_empty())
                        || v["violations"]["history"].as_array().is_some_and(|d| !d.is_empty());
                    if diverged || broken { Err(("the replay diverged or broke a law".into(), 1)) } else { Ok(()) }
                }
                Ok(r) => ok(r).map_err(fail),
                Err(e) => Err(fail(e)),
            }
        }
        "laws" => match request(&admin, "GET", "/v1/laws", None) {
            Ok(r) if r.status == 200 => {
                println!("{}", pretty(&r.body));
                let v: serde_json::Value = serde_json::from_str(&r.body).unwrap_or_default();
                if v["holds"].as_bool() == Some(true) { Ok(()) } else { Err(("a law does not hold".into(), 1)) }
            }
            Ok(r) => ok(r).map_err(fail),
            Err(e) => Err(fail(e)),
        },
        "seed" => match a(0) {
            None => request(&admin, "GET", "/v1/seed", None).and_then(ok).map_err(fail),
            Some(n) => match n.parse::<u64>() {
                Ok(n) => {
                    let body = serde_json::json!({ "seed": n }).to_string();
                    request(&admin, "POST", "/v1/seed", Some(&body)).and_then(ok).map_err(fail)
                }
                Err(_) => Err(fail(format!("{n} is not a seed; give a whole number"))),
            },
        },
        "reset" => request(&admin, "DELETE", "/v1/reset", None).and_then(ok).map_err(fail),
        _ => return None,
    };
    Some(result)
}

fn up(args: &[String]) -> Result<(), String> {
    let twin = args.first().map(String::as_str).ok_or("up needs a twin: kafka")?;
    if twin != "kafka" {
        return Err(format!("{twin} is not a twin; this release has kafka"));
    }
    let mut config = lawtwin_kafka::Config::default();
    let (mut host, mut port, mut admin_port) = ("127.0.0.1".to_string(), 9092u16, 9644u16);
    let (mut container, mut image) = (false, "lawtwin/kafka:0.1.0".to_string());
    let mut i = 1;
    while i < args.len() {
        let value = |i: usize| args.get(i + 1).cloned().ok_or_else(|| format!("{} needs a value", args[i]));
        match args[i].as_str() {
            "--port" => port = value(i)?.parse().map_err(|_| "--port is a port number")?,
            "--admin-port" => admin_port = value(i)?.parse().map_err(|_| "--admin-port is a port number")?,
            "--host" => host = value(i)?,
            "--advertise" => config.advertised = Some(value(i)?),
            "--seed" => config.seed = value(i)?.parse().map_err(|_| "--seed is a whole number")?,
            "--clock" => config.clock = value(i)?.parse()?,
            "--partitions" => config.default_partitions = value(i)?.parse().map_err(|_| "--partitions is a whole number")?,
            "--rebalance-wait" => {
                config.rebalance_wait_ms = lawtwin_kafka::clock::parse_duration(&value(i)?)? as u64;
            }
            "--image" => image = value(i)?,
            "--no-auto-create" => {
                config.auto_create_topics = false;
                i += 1;
                continue;
            }
            "--container" => {
                container = true;
                i += 1;
                continue;
            }
            other => return Err(format!("up does not take {other}")),
        }
        i += 2;
    }
    if container {
        let mut cmd = std::process::Command::new(std::env::var("PODMAN").unwrap_or_else(|_| "podman".into()));
        cmd.args(["run", "--rm", "-d", "-p", &format!("{port}:9092"), "-p", &format!("{admin_port}:9644"), &image]);
        cmd.args(["--seed", &config.seed.to_string(), "--advertise", &config.advertised.clone().unwrap_or(format!("{host}:{port}"))]);
        let status = cmd.status().map_err(|e| format!("cannot run podman: {e}"))?;
        return if status.success() { Ok(()) } else { Err("podman run failed".into()) };
    }
    config.kafka_listen = format!("{host}:{port}");
    config.admin_listen = format!("{host}:{admin_port}");
    lawtwin_kafka::run(config).map_err(|e| e.to_string())
}
