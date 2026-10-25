//! Starting the core under test, and speaking to its admin API.
//!
//! `LAWTWIN_CORE` names the core: a command (`target/release/lawtwin-kafka`,
//! or any other core's program, given the core options), or an image
//! (`image:lawtwin/kafka:0.2.1`, run with podman, or `PODMAN`). Unset, the
//! Rust core runs in this process. Nothing else about the core is assumed:
//! the kit reaches it only over the Kafka protocol and the admin API.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

enum Running {
    InProcess(#[allow(dead_code)] tokio::runtime::Runtime),
    Program(Child),
    Container(String),
}

/// A core running for one test, on free ports, with a seed.
pub struct TestTwin {
    /// The Kafka listener.
    pub kafka: SocketAddr,
    /// The admin API.
    pub admin: SocketAddr,
    running: Running,
}

impl Drop for TestTwin {
    fn drop(&mut self) {
        match &mut self.running {
            Running::InProcess(_) => {}
            Running::Program(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
            Running::Container(id) => {
                let _ = Command::new(podman()).args(["rm", "-f", id]).stdout(Stdio::null()).stderr(Stdio::null()).status();
            }
        }
    }
}

fn podman() -> String {
    std::env::var("PODMAN").unwrap_or_else(|_| "podman".into())
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// The core under test, as `LAWTWIN_CORE` names it.
pub fn core_name() -> String {
    std::env::var("LAWTWIN_CORE").unwrap_or_else(|_| "the Rust core, in process".into())
}

impl TestTwin {
    /// The core under test, with a seed and a frozen clock.
    pub fn start(seed: u64) -> Self {
        Self::launch(seed, None)
    }

    /// The core under test, telling clients a fixed address instead of its
    /// own, so that its answers do not depend on the port it was given.
    pub fn start_advertising(seed: u64, advertised: &str) -> Self {
        Self::launch(seed, Some(advertised))
    }

    /// A given core (a command, or `image:<image>`; the Rust core in this
    /// process when none), with a seed and a frozen clock.
    pub fn start_core(core: Option<&str>, seed: u64) -> Self {
        Self::launch_core(core.map(str::to_string), seed, None)
    }

    fn launch(seed: u64, advertised: Option<&str>) -> Self {
        let core = std::env::var("LAWTWIN_CORE").ok().filter(|c| !c.trim().is_empty());
        Self::launch_core(core, seed, advertised)
    }

    fn launch_core(core: Option<String>, seed: u64, advertised: Option<&str>) -> Self {
        match core {
            None => Self::in_process(seed, advertised),
            Some(c) => match c.strip_prefix("image:") {
                Some(image) => Self::container(image, seed, advertised),
                None => Self::program(&c, seed, advertised),
            },
        }
    }

    fn in_process(seed: u64, advertised: Option<&str>) -> Self {
        let config = lawtwin_kafka::Config {
            seed,
            advertised: advertised.map(str::to_string),
            kafka_listen: "127.0.0.1:0".into(),
            admin_listen: "127.0.0.1:0".into(),
            ..Default::default()
        };
        let runtime = lawtwin_kafka::runtime().unwrap();
        let running = runtime.block_on(lawtwin_kafka::start(config)).unwrap();
        Self { kafka: running.kafka, admin: running.admin, running: Running::InProcess(runtime) }
    }

    fn program(command: &str, seed: u64, advertised: Option<&str>) -> Self {
        let (kafka, admin) = (free_port(), free_port());
        let mut parts = command.split_whitespace();
        let mut cmd = Command::new(parts.next().unwrap());
        cmd.args(parts)
            .args(["--host", "127.0.0.1", "--port", &kafka.to_string(), "--admin-port", &admin.to_string()])
            .args(["--seed", &seed.to_string(), "--clock", "frozen"])
            .args(advertised.map(|a| vec!["--advertise", a]).unwrap_or_default())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = cmd.spawn().unwrap_or_else(|e| panic!("cannot start the core {command}: {e}"));
        let twin = Self {
            kafka: ([127, 0, 0, 1], kafka).into(),
            admin: ([127, 0, 0, 1], admin).into(),
            running: Running::Program(child),
        };
        twin.wait_ready();
        twin
    }

    fn container(image: &str, seed: u64, advertised: Option<&str>) -> Self {
        let (kafka, admin) = (free_port(), free_port());
        let advertise = advertised.map(str::to_string).unwrap_or_else(|| format!("127.0.0.1:{kafka}"));
        let config = serde_json::json!({ "seed": seed, "clock": "frozen", "advertise": advertise }).to_string();
        let out = Command::new(podman())
            .args(["run", "-d", "--rm", "-p", &format!("127.0.0.1:{kafka}:9092"), "-p", &format!("127.0.0.1:{admin}:9644")])
            .args(["-e", &format!("TWIN_CONFIG={config}"), image])
            .output()
            .unwrap_or_else(|e| panic!("cannot run podman: {e}"));
        assert!(out.status.success(), "podman run {image}: {}", String::from_utf8_lossy(&out.stderr));
        let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let twin = Self {
            kafka: ([127, 0, 0, 1], kafka).into(),
            admin: ([127, 0, 0, 1], admin).into(),
            running: Running::Container(id),
        };
        twin.wait_ready();
        twin
    }

    fn wait_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok((200, _)) = self.try_call("GET", "/healthz", None) {
                if TcpStream::connect(self.kafka).is_ok() {
                    return;
                }
            }
            assert!(Instant::now() < deadline, "the core did not start: {}", core_name());
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The bootstrap address.
    pub fn bootstrap(&self) -> String {
        self.kafka.to_string()
    }

    /// The admin API's URL.
    pub fn admin_url(&self) -> String {
        format!("http://{}", self.admin)
    }

    fn raw(&self, method: &str, path: &str, body: Option<&str>) -> std::io::Result<(u16, String)> {
        let mut stream = TcpStream::connect(self.admin)?;
        let body = body.unwrap_or("");
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: twin\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )?;
        let mut raw = vec![];
        stream.read_to_end(&mut raw)?;
        let raw = String::from_utf8_lossy(&raw).to_string();
        let (head, payload) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
        let status = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let chunked = head.to_ascii_lowercase().contains("transfer-encoding: chunked");
        Ok((status, if chunked { dechunk(payload) } else { payload.to_string() }))
    }

    fn try_call(&self, method: &str, path: &str, body: Option<serde_json::Value>) -> std::io::Result<(u16, serde_json::Value)> {
        let body = body.filter(|b| !b.is_null()).map(|b| b.to_string());
        let (status, text) = self.raw(method, path, body.as_deref())?;
        Ok((status, serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text))))
    }

    /// One request to the admin API: the status and the JSON body.
    pub fn call(&self, method: &str, path: &str, body: Option<serde_json::Value>) -> (u16, serde_json::Value) {
        self.try_call(method, path, body).unwrap_or_else(|e| panic!("{method} {path}: {e}"))
    }

    /// A POST that must succeed.
    pub fn post(&self, path: &str, body: serde_json::Value) -> serde_json::Value {
        let (status, v) = self.call("POST", path, Some(body));
        assert!((200..300).contains(&status), "POST {path}: {status} {v}");
        v
    }

    /// A GET that must succeed.
    pub fn get(&self, path: &str) -> serde_json::Value {
        let (status, v) = self.call("GET", path, None);
        assert_eq!(status, 200, "GET {path}: {v}");
        v
    }

    /// The history as JSON lines, exactly as the admin API gives it.
    pub fn history_text(&self) -> String {
        let (status, text) = self.raw("GET", "/v1/history?format=jsonl", None).unwrap();
        assert_eq!(status, 200);
        text
    }

    /// The core's history, replayed by the kit through the reference model,
    /// gives the same answers, and the kit's own law checks hold over it and
    /// over the replayed state. The core's own `/v1/laws` must agree.
    pub fn assert_lawful(&self) {
        let text = self.history_text();
        let entries = lawtwin_kafka::admin::parse_history(&text).unwrap_or_else(|e| panic!("the core's history does not parse: {e}"));
        assert!(entries.iter().any(|e| matches!(e.event, lawtwin_kafka::history::Event::Call { .. })), "an empty history");
        let (model, divergences) = lawtwin_kafka::history::replay(&entries);
        assert!(divergences.is_empty(), "the core's answers differ from the reference model's: {divergences:#?}");
        let broken = lawtwin_kafka::laws::check_history(&entries);
        assert!(broken.is_empty(), "laws broken over the core's history: {broken:#?}");
        let broken = lawtwin_kafka::laws::check_state(&model.snapshot());
        assert!(broken.is_empty(), "laws broken over the core's state: {broken:#?}");
        let laws = self.get("/v1/laws");
        assert_eq!(laws["holds"], true, "the core's own law check: {laws:#}");
    }
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
