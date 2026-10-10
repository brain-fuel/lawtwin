//! Starting the core under test, and speaking to it.
//!
//! `LAWTWIN_CORE` names the core: a command (`target/release/lawtwin-http`,
//! or any other core's program, given the core options), or an image
//! (`image:lawtwin/http:0.1.0`, run with podman, or `PODMAN`). Unset, the
//! Rust core runs in this process. Nothing else about the core is assumed:
//! the kit reaches it only over HTTP and the admin API.

use lawtwin_wire::call::Outgoing;
use lawtwin_wire::http::{self, Endpoint, Failure, Reply};
use serde_json::Value;
use std::net::{SocketAddr, TcpListener};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

enum Running {
    InProcess(#[allow(dead_code)] lawtwin_http::Running),
    Program(Child),
    Container(String),
}

/// A core running for one test, on free ports, serving one contract.
pub struct TestTwin {
    /// The API.
    pub api: SocketAddr,
    /// The admin API.
    pub admin: SocketAddr,
    pub contract: String,
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

/// The contract's wire map, as the core serves it.
pub fn wire_map(contract: &str) -> lawtwin_wire::WireMap {
    let c = lawtwin_http::contracts::find(contract).unwrap_or_else(|| panic!("no compiled contract {contract}"));
    lawtwin_wire::WireMap::parse(c.wire_map).unwrap()
}

impl TestTwin {
    /// The core under test, serving a contract with a seed and a frozen clock.
    pub fn start(contract: &str, seed: u64) -> Self {
        Self::launch(contract, seed, None)
    }

    /// The core under test, telling clients a fixed address instead of its
    /// own, so its answers do not depend on the port it was given.
    pub fn start_advertising(contract: &str, seed: u64, advertised: &str) -> Self {
        Self::launch(contract, seed, Some(advertised))
    }

    fn launch(contract: &str, seed: u64, advertised: Option<&str>) -> Self {
        let core = std::env::var("LAWTWIN_CORE").ok().filter(|c| !c.trim().is_empty());
        match core {
            None => Self::in_process(contract, seed, advertised),
            Some(c) => match c.strip_prefix("image:") {
                Some(image) => Self::container(image, contract, seed, advertised),
                None => Self::program(&c, contract, seed, advertised),
            },
        }
    }

    fn in_process(contract: &str, seed: u64, advertised: Option<&str>) -> Self {
        let config = lawtwin_http::Config {
            contract: contract.into(),
            seed,
            advertised: advertised.map(str::to_string),
            listen: "127.0.0.1:0".into(),
            admin_listen: "127.0.0.1:0".into(),
            ..Default::default()
        };
        let running = lawtwin_http::start(config).unwrap();
        Self { api: running.api, admin: running.admin, contract: contract.into(), running: Running::InProcess(running) }
    }

    fn program(command: &str, contract: &str, seed: u64, advertised: Option<&str>) -> Self {
        let (api, admin) = (free_port(), free_port());
        let mut parts = command.split_whitespace();
        let mut cmd = Command::new(parts.next().unwrap());
        cmd.args(parts)
            .args(["--contract", contract, "--host", "127.0.0.1", "--port", &api.to_string(), "--admin-port", &admin.to_string()])
            .args(["--seed", &seed.to_string(), "--clock", "frozen"])
            .args(advertised.map(|a| vec!["--advertise", a]).unwrap_or_default())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = cmd.spawn().unwrap_or_else(|e| panic!("cannot start the core {command}: {e}"));
        let twin = Self {
            api: ([127, 0, 0, 1], api).into(),
            admin: ([127, 0, 0, 1], admin).into(),
            contract: contract.into(),
            running: Running::Program(child),
        };
        twin.wait_ready();
        twin
    }

    fn container(image: &str, contract: &str, seed: u64, advertised: Option<&str>) -> Self {
        let (api, admin) = (free_port(), free_port());
        let advertise = advertised.map(str::to_string).unwrap_or_else(|| format!("127.0.0.1:{api}"));
        let config = serde_json::json!({ "contract": contract, "seed": seed, "clock": "frozen", "advertise": advertise }).to_string();
        let out = Command::new(podman())
            .args(["run", "-d", "--rm", "-p", &format!("127.0.0.1:{api}:8080"), "-p", &format!("127.0.0.1:{admin}:9644")])
            .args(["-e", &format!("TWIN_CONFIG={config}"), image])
            .output()
            .unwrap_or_else(|e| panic!("cannot run podman: {e}"));
        assert!(out.status.success(), "podman run {image}: {}", String::from_utf8_lossy(&out.stderr));
        let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let twin = Self {
            api: ([127, 0, 0, 1], api).into(),
            admin: ([127, 0, 0, 1], admin).into(),
            contract: contract.into(),
            running: Running::Container(id),
        };
        twin.wait_ready();
        twin
    }

    fn wait_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok((200, _)) = self.try_admin("GET", "/healthz", "") {
                if std::net::TcpStream::connect(self.api).is_ok() {
                    return;
                }
            }
            assert!(Instant::now() < deadline, "the core did not start: {}", core_name());
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The API's URL.
    pub fn url(&self) -> String {
        format!("http://{}", self.api)
    }

    /// The API's endpoint.
    pub fn endpoint(&self) -> Endpoint {
        Endpoint::parse(&self.url()).unwrap()
    }

    /// A request to the API.
    pub fn send(&self, out: &Outgoing) -> Result<Reply, Failure> {
        http::send(&self.endpoint(), out, Duration::from_secs(10))
    }

    fn try_admin(&self, method: &str, path: &str, body: &str) -> Result<(u16, Value), Failure> {
        let ep = Endpoint::parse(&format!("http://{}", self.admin)).unwrap();
        let out = Outgoing {
            method: method.into(),
            target: path.into(),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.as_bytes().to_vec(),
        };
        let r = http::send(&ep, &out, Duration::from_secs(30))?;
        let v = serde_json::from_slice(&r.body).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&r.body).into_owned()));
        Ok((r.status, v))
    }

    /// A call of the admin API, and its JSON answer.
    pub fn admin(&self, method: &str, path: &str, body: &str) -> (u16, Value) {
        self.try_admin(method, path, body).unwrap_or_else(|e| panic!("{method} {path}: {e}"))
    }

    /// The history, as JSON lines.
    pub fn history_text(&self) -> String {
        match self.admin("GET", "/v1/history?format=jsonl", "") {
            (200, Value::String(s)) => s,
            other => panic!("the history: {other:?}"),
        }
    }

    /// The history's entries.
    pub fn history(&self) -> Vec<Value> {
        lawtwin_wire::history::parse(&self.history_text()).unwrap()
    }

    /// Replays the core's history through the reference model and checks
    /// the contract's laws over it and over the replayed state; and checks
    /// that the core's own law check agrees.
    pub fn assert_lawful(&self) {
        let contract = lawtwin_http::contracts::find(&self.contract).unwrap();
        let map = lawtwin_wire::WireMap::parse(contract.wire_map).unwrap();
        let entries = self.history();
        let replayed = lawtwin_http::history::replay(&map, contract.start, &entries).unwrap();
        assert!(replayed.divergences.is_empty(), "{}: the history diverges from the reference model: {:#?}", core_name(), replayed.divergences);
        let hist = lawtwin_wire::laws::check_history(&map, &entries);
        assert!(hist.is_empty(), "{}: a law does not hold over the history: {hist:#?}", core_name());
        let state = lawtwin_wire::laws::check_state(&map, &replayed.model.state());
        assert!(state.is_empty(), "{}: a law does not hold over the state: {state:#?}", core_name());
        let (status, laws) = self.admin("GET", "/v1/laws", "");
        assert_eq!(status, 200);
        assert_eq!(laws["holds"], true, "{}: the core's own law check: {laws:#}", core_name());
    }
}
