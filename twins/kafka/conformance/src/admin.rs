//! A client of LawTwin's control plane at a URL: a twin core's admin API,
//! or the recorder's in front of a real broker.

use std::io::{Read, Write};
use std::net::TcpStream;

/// The control plane at `http://host:port`.
#[derive(Clone, Debug)]
pub struct Admin {
    authority: String,
}

impl Admin {
    /// The control plane at a URL such as `http://127.0.0.1:9644`.
    pub fn new(url: &str) -> Self {
        let authority = url.trim_start_matches("http://").trim_end_matches('/').to_string();
        Self { authority }
    }

    /// One request: the status and the body as text.
    pub fn raw(&self, method: &str, path: &str, body: Option<&str>) -> std::io::Result<(u16, String)> {
        let mut stream = TcpStream::connect(&self.authority)?;
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

    /// One request: the status and the JSON body.
    pub fn call(&self, method: &str, path: &str, body: Option<serde_json::Value>) -> (u16, serde_json::Value) {
        let body = body.filter(|b| !b.is_null()).map(|b| b.to_string());
        let (status, text) = self.raw(method, path, body.as_deref()).unwrap_or_else(|e| panic!("{method} {path}: {e}"));
        (status, serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text)))
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
}

/// The body of a chunked HTTP response.
pub fn dechunk(mut s: &str) -> String {
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
