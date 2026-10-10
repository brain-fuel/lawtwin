//! Starting a twin for a test, and speaking to its admin API.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};

/// A twin running on its own runtime, on free ports.
pub struct TestTwin {
    pub kafka: SocketAddr,
    pub admin: SocketAddr,
    pub twin: std::sync::Arc<lawtwin_kafka::Twin>,
    _runtime: tokio::runtime::Runtime,
}

impl TestTwin {
    pub fn start(seed: u64) -> Self {
        Self::with(lawtwin_kafka::Config { seed, ..Default::default() })
    }

    pub fn with(mut config: lawtwin_kafka::Config) -> Self {
        config.kafka_listen = "127.0.0.1:0".into();
        config.admin_listen = "127.0.0.1:0".into();
        let runtime = lawtwin_kafka::runtime().unwrap();
        let running = runtime.block_on(lawtwin_kafka::start(config)).unwrap();
        Self { kafka: running.kafka, admin: running.admin, twin: running.twin, _runtime: runtime }
    }

    pub fn bootstrap(&self) -> String {
        self.kafka.to_string()
    }

    pub fn admin_url(&self) -> String {
        format!("http://{}", self.admin)
    }

    /// One request to the admin API: the status and the JSON body.
    pub fn call(&self, method: &str, path: &str, body: Option<serde_json::Value>) -> (u16, serde_json::Value) {
        let mut stream = TcpStream::connect(self.admin).unwrap();
        let body = body.filter(|b| !b.is_null()).map(|b| b.to_string()).unwrap_or_default();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: twin\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).unwrap();
        let (head, payload) = raw.split_once("\r\n\r\n").unwrap();
        let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, serde_json::from_str(payload).unwrap_or(serde_json::Value::String(payload.to_string())))
    }

    pub fn post(&self, path: &str, body: serde_json::Value) -> serde_json::Value {
        let (status, v) = self.call("POST", path, Some(body));
        assert!((200..300).contains(&status), "POST {path}: {status} {v}");
        v
    }

    pub fn get(&self, path: &str) -> serde_json::Value {
        let (status, v) = self.call("GET", path, None);
        assert_eq!(status, 200, "GET {path}: {v}");
        v
    }

    /// The history as JSON lines, exactly as the admin API gives it.
    pub fn history_text(&self) -> String {
        let mut stream = TcpStream::connect(self.admin).unwrap();
        write!(stream, "GET /v1/history?format=jsonl HTTP/1.1\r\nHost: twin\r\nConnection: close\r\n\r\n").unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).unwrap();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap();
        if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
            let mut out = String::new();
            let mut s = body;
            while let Some((size, rest)) = s.split_once("\r\n") {
                let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
                if n == 0 {
                    break;
                }
                out.push_str(&rest[..n]);
                s = &rest[n + 2..];
            }
            out
        } else {
            body.to_string()
        }
    }

    /// Every law holds over the twin's state and history, and replaying the
    /// history through a fresh model gives the same answers.
    pub fn assert_lawful(&self) {
        let laws = self.get("/v1/laws");
        assert_eq!(laws["holds"], true, "laws: {laws:#}");
        let replay = self.post("/v1/history/replay", serde_json::Value::Null);
        assert_eq!(replay["divergences"], serde_json::json!([]), "replay: {replay:#}");
        assert!(replay["operations"].as_u64().unwrap() > 0);
    }
}
