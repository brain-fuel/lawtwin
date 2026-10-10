//! Record and replay (ref:DEC-fidelity): `lawtwin record http` is a proxy
//! in front of a real service that writes every exchange to a trace, each
//! read as a call of the contract and checked against the document;
//! `lawtwin replay http` sends a trace's requests to a twin and reports
//! every answer that differs, and checks the trace itself against the
//! contract: its laws over the recorded calls, and the model's answers to
//! them.

use super::{Args, Outcome, fail, write_report};
use lawtwin_wire::WireMap;
use lawtwin_wire::call::{Incoming, Outgoing};
use lawtwin_wire::history::op_entry;
use lawtwin_wire::http::{self, Endpoint};
use lawtwin_wire::laws;
use serde_json::{Value, json};
use std::io::{BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const HOP_BY_HOP: &[&str] = &["connection", "keep-alive", "transfer-encoding", "host", "content-length", "proxy-connection", "upgrade"];

fn body_json(body: &[u8]) -> Value {
    match std::str::from_utf8(body) {
        Ok(t) if !t.chars().any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t') => json!(t),
        _ => {
            use std::fmt::Write as _;
            let mut hex = String::new();
            for b in body {
                let _ = write!(hex, "{b:02x}");
            }
            json!({ "hex": hex })
        }
    }
}

fn body_bytes(v: &Value) -> Vec<u8> {
    match v {
        Value::String(s) => s.as_bytes().to_vec(),
        Value::Object(o) => o
            .get("hex")
            .and_then(Value::as_str)
            .map(|h| (0..h.len() / 2).filter_map(|i| u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).ok()).collect())
            .unwrap_or_default(),
        _ => vec![],
    }
}

fn kept(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers.iter().filter(|(k, _)| !HOP_BY_HOP.contains(&k.to_ascii_lowercase().as_str())).cloned().collect()
}

struct Recorder {
    map: WireMap,
    upstream: Endpoint,
    out: Mutex<Box<dyn Write + Send>>,
    seq: AtomicUsize,
}

impl Recorder {
    fn exchange(&self, req: &http::Received) -> (u16, String, Vec<(String, String)>, Vec<u8>) {
        let out = Outgoing { method: req.method.clone(), target: req.target.clone(), headers: kept(&req.headers), body: req.body.clone() };
        let (path, query) = req.target.split_once('?').unwrap_or((&req.target, ""));
        let call = self.map.read_request(&Incoming { method: &req.method, path, query, headers: &req.headers, body: &req.body });
        let reply = http::send(&self.upstream, &out, Duration::from_secs(60));
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let mut line = json!({
            "seq": seq,
            "request": { "method": req.method, "target": req.target, "headers": kept(&req.headers), "body": body_json(&req.body) },
        });
        let answer = match &reply {
            Ok(r) => {
                line["response"] = json!({ "status": r.status, "headers": kept(&r.headers), "body": body_json(&r.body) });
                (r.status, r.reason.clone(), kept(&r.headers), r.body.clone())
            }
            Err(e) => {
                line["response"] = json!({ "failure": e.to_string() });
                let body = serde_json::to_vec(&json!({ "message": format!("lawtwin record: the upstream gave no answer: {e}") })).unwrap_or_default();
                (502, "Bad Gateway".into(), vec![("Content-Type".into(), "application/json".into())], body)
            }
        };
        match (&call, &reply) {
            (Ok(call), Ok(r)) => {
                let op = self.map.op(&call.op).expect("a routed operation");
                let reading = self.map.read_response(op, r.status, &r.headers, &r.body);
                line["op"] = json!(call.op);
                line["args"] = Value::Object(call.args.iter().cloned().collect());
                line["answer"] = reading.answer;
                line["problems"] = json!(reading.problems);
            }
            (Err(refusal), _) => {
                line["refused"] = json!(format!("{refusal:?}"));
            }
            _ => {}
        }
        if let Ok(mut w) = self.out.lock() {
            let _ = writeln!(w, "{}", serde_json::to_string(&line).unwrap_or_default());
            let _ = w.flush();
        }
        answer
    }
}

fn serve(stream: TcpStream, rec: Arc<Recorder>) {
    let Ok(mut w) = stream.try_clone() else { return };
    let mut r = BufReader::new(stream);
    while let Ok(Some(req)) = http::read_request(&mut r) {
        let (status, reason, headers, body) = rec.exchange(&req);
        if http::write_response(&mut w, status, &reason, &headers, &body, req.close).is_err() || req.close {
            return;
        }
    }
}

/// `lawtwin record http`: a recording proxy in front of a real service.
pub fn record(rest: &[String]) -> Result<(), String> {
    let mut args = Args::new(rest);
    let map = super::contract(&mut args)?;
    let upstream = Endpoint::parse(&args.value("--upstream")?.ok_or("--upstream is required: the real service's URL")?)?;
    let listen = args.value("--listen")?.unwrap_or_else(|| "127.0.0.1:8081".into());
    let path = args.value("--out")?.unwrap_or_else(|| "trace.jsonl".into());
    let count: Option<usize> = args.value("--count")?.map(|c| c.parse().map_err(|_| "--count is a whole number")).transpose()?;
    args.rest()?;
    let file: Box<dyn Write + Send> = if path == "-" {
        Box::new(std::io::stdout())
    } else {
        Box::new(std::fs::File::create(&path).map_err(|e| format!("{path}: {e}"))?)
    };
    let listener = TcpListener::bind(&listen).map_err(|e| format!("{listen}: {e}"))?;
    eprintln!("lawtwin record http: {} at {} through http://{listen}, writing {path}", map.package, upstream.url());
    let rec = Arc::new(Recorder { map, upstream, out: Mutex::new(file), seq: AtomicUsize::new(0) });
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    loop {
        if count.is_some_and(|c| rec.seq.load(Ordering::SeqCst) >= c) {
            // Let the last answer reach its client.
            std::thread::sleep(Duration::from_millis(100));
            return Ok(());
        }
        match listener.accept() {
            Ok((s, _)) => {
                s.set_nonblocking(false).ok();
                let rec = rec.clone();
                std::thread::spawn(move || serve(s, rec));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn same_body(a: &[u8], b: &[u8]) -> bool {
    match (serde_json::from_slice::<Value>(a), serde_json::from_slice::<Value>(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// `lawtwin replay http`: a trace's requests sent to a twin, each answer
/// compared with the recorded one; and the trace checked against the contract.
pub fn replay(rest: &[String]) -> Outcome {
    let mut args = Args::new(rest);
    let map = super::contract(&mut args).map_err(fail)?;
    let target = args.value("--target").map_err(fail)?.unwrap_or_else(|| "http://127.0.0.1:8080".into());
    let admin = args
        .value("--admin")
        .map_err(fail)?
        .or_else(|| std::env::var("LAWTWIN_ADMIN").ok())
        .unwrap_or_else(|| "http://127.0.0.1:9644".into());
    let no_reset = args.flag("--no-reset");
    let report_path = args.value("--report").map_err(fail)?;
    let files = args.rest().map_err(fail)?;
    let [file] = files.as_slice() else { return Err(fail("replay http needs one trace file")) };
    let text = std::fs::read_to_string(file).map_err(|e| fail(format!("{file}: {e}")))?;
    let trace: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| serde_json::from_str(l).map_err(|e| fail(format!("{file}: line {}: {e}", i + 1))))
        .collect::<Result<_, _>>()?;
    if !no_reset {
        let (status, _) = super::admin_request(&admin, "DELETE", "/v1/reset", "").map_err(fail)?;
        if status != 200 {
            return Err(fail(format!("the twin at {admin} did not reset ({status})")));
        }
    }
    let ep = Endpoint::parse(&target).map_err(fail)?;
    let mut divergences = vec![];
    let mut sent = 0;
    for x in &trace {
        let Some(status) = x["response"]["status"].as_u64() else { continue };
        let req = &x["request"];
        let headers: Vec<(String, String)> = req["headers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|h| Some((h[0].as_str()?.to_string(), h[1].as_str()?.to_string())))
            .collect();
        let out = Outgoing {
            method: req["method"].as_str().unwrap_or("GET").into(),
            target: req["target"].as_str().unwrap_or("/").into(),
            headers,
            body: body_bytes(&req["body"]),
        };
        sent += 1;
        let recorded_body = body_bytes(&x["response"]["body"]);
        match http::send(&ep, &out, Duration::from_secs(30)) {
            Ok(r) if r.status as u64 == status && same_body(&r.body, &recorded_body) => {}
            Ok(r) => divergences.push(json!({
                "seq": x["seq"], "op": x["op"], "request": format!("{} {}", out.method, out.target),
                "recorded": { "status": status, "body": x["response"]["body"] },
                "replayed": { "status": r.status, "body": body_json(&r.body) },
            })),
            Err(e) => divergences.push(json!({
                "seq": x["seq"], "op": x["op"], "request": format!("{} {}", out.method, out.target),
                "recorded": { "status": status, "body": x["response"]["body"] },
                "replayed": { "failure": e.to_string() },
            })),
        }
    }
    // The trace as a history: the calls it holds, with the answers the real service gave.
    let entries: Vec<Value> = trace
        .iter()
        .filter(|x| x.get("op").is_some() && x.get("answer").is_some())
        .enumerate()
        .map(|(i, x)| {
            let args: Vec<(String, Value)> = x["args"].as_object().map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
            op_entry(i as u64 + 1, 0, "wire", x["op"].as_str().unwrap_or(""), &args, x["answer"].clone())
        })
        .collect();
    let violations = laws::check_history(&map, &entries);
    let problems: Vec<Value> = trace.iter().flat_map(|x| x["problems"].as_array().cloned().unwrap_or_default()).collect();
    let body = entries.iter().map(|e| serde_json::to_string(e).unwrap_or_default()).collect::<Vec<_>>().join("\n");
    let model = super::admin_request(&admin, "POST", "/v1/history/replay", &body).map(|(_, v)| v).unwrap_or(Value::Null);
    let model_divergences = model["divergences"].as_array().cloned().unwrap_or_default();
    let report = json!({
        "contract": map.package,
        "trace": file,
        "target": target,
        "exchanges": trace.len(),
        "replayed": sent,
        "divergences": divergences,
        "trace check": { "calls": entries.len(), "violations": violations, "problems": problems, "model divergences": model_divergences },
    });
    println!(
        "lawtwin replay http: {} exchange(s) of {file} replayed through {target}: {} divergence(s); the trace holds {} call(s), with {} law violation(s), {} problem(s) and {} answer(s) the model gives differently",
        sent,
        divergences.len(),
        entries.len(),
        violations.len(),
        problems.len(),
        model_divergences.len()
    );
    for d in divergences.iter().take(20) {
        println!("  divergence at {}: {} recorded {} replayed {}", d["seq"], d["request"].as_str().unwrap_or(""), d["recorded"], d["replayed"]);
    }
    for v in violations.iter().take(20) {
        println!("  violation: `{}`: {}", v["law"].as_str().unwrap_or(""), v["detail"].as_str().unwrap_or(""));
    }
    for d in model_divergences.iter().take(20) {
        println!("  the model answers {} at {} with {}, the trace has {}", d["op"], d["seq"], d["replayed"], d["recorded"]);
    }
    write_report(report_path, &report).map_err(fail)?;
    if divergences.is_empty() && violations.is_empty() && model_divergences.is_empty() {
        Ok(())
    } else {
        Err(("the twin parts from the trace".into(), 1))
    }
}
