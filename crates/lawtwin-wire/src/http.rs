//! A small HTTP/1.1 client over `std::net`, one connection per request.
//! Peers use it to call an application the way a careless client would as
//! well as a careful one: with a deadline, dropping a request half-way, or
//! sending it twice. The recording proxy uses its reader and writer.

use crate::call::Outgoing;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

/// Where requests go: `http://host:port/base`.
#[derive(Clone, Debug)]
pub struct Endpoint {
    /// `host:port`.
    pub authority: String,
    /// A path every request's target is put under, without a trailing `/`.
    pub base: String,
}

impl Endpoint {
    /// Reads `http://host[:port][/base]`.
    pub fn parse(url: &str) -> Result<Endpoint, String> {
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| format!("{url}: give an http:// URL (TLS is not spoken)"))?;
        let (authority, base) = match rest.find('/') {
            Some(i) => (&rest[..i], rest[i..].trim_end_matches('/')),
            None => (rest, ""),
        };
        let authority = if authority.contains(':') { authority.to_string() } else { format!("{authority}:80") };
        Ok(Endpoint { authority, base: base.to_string() })
    }

    /// The URL again.
    pub fn url(&self) -> String {
        format!("http://{}{}", self.authority, self.base)
    }
}

/// A response.
#[derive(Clone, Debug)]
pub struct Reply {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// How long it took, from connecting to the last byte.
    pub elapsed: Duration,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        crate::call::header(&self.headers, name)
    }
}

/// Why no response came.
#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// The connection could not be made.
    Connect(String),
    /// No response within the deadline.
    Timeout,
    /// The connection was closed or reset before a whole response came.
    Closed(String),
    /// What came is not HTTP.
    Malformed(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Connect(e) => write!(f, "cannot connect: {e}"),
            Failure::Timeout => write!(f, "no response in time"),
            Failure::Closed(e) => write!(f, "the connection closed without a whole response ({e})"),
            Failure::Malformed(e) => write!(f, "not an HTTP response: {e}"),
        }
    }
}

fn connect(ep: &Endpoint, timeout: Duration) -> Result<TcpStream, Failure> {
    let addr = ep
        .authority
        .to_socket_addrs()
        .map_err(|e| Failure::Connect(format!("{}: {e}", ep.authority)))?
        .next()
        .ok_or_else(|| Failure::Connect(format!("{} has no address", ep.authority)))?;
    let s = TcpStream::connect_timeout(&addr, timeout.min(Duration::from_secs(10))).map_err(|e| Failure::Connect(e.to_string()))?;
    s.set_nodelay(true).ok();
    s.set_read_timeout(Some(timeout)).ok();
    s.set_write_timeout(Some(timeout)).ok();
    Ok(s)
}

/// Writes a request, with `Host`, `Content-Length` and `Connection: close`.
pub fn write_request(w: &mut impl Write, host: &str, base: &str, req: &Outgoing) -> std::io::Result<()> {
    let mut head = format!("{} {}{} HTTP/1.1\r\nHost: {host}\r\n", req.method, base, req.target);
    let mut has_length = false;
    for (k, v) in &req.headers {
        if k.eq_ignore_ascii_case("host") || k.eq_ignore_ascii_case("connection") {
            continue;
        }
        has_length |= k.eq_ignore_ascii_case("content-length");
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if !has_length && (!req.body.is_empty() || matches!(req.method.as_str(), "POST" | "PUT" | "PATCH")) {
        head.push_str(&format!("Content-Length: {}\r\n", req.body.len()));
    }
    head.push_str("Connection: close\r\n\r\n");
    w.write_all(head.as_bytes())?;
    w.write_all(&req.body)?;
    w.flush()
}

fn io_failure(e: std::io::Error) -> Failure {
    match e.kind() {
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => Failure::Timeout,
        _ => Failure::Closed(e.to_string()),
    }
}

/// Reads a response head: the status, reason and headers.
pub fn read_head(r: &mut impl BufRead) -> Result<(u16, String, Vec<(String, String)>), Failure> {
    let mut line = String::new();
    let n = r.read_line(&mut line).map_err(io_failure)?;
    if n == 0 {
        return Err(Failure::Closed("nothing was answered".into()));
    }
    let mut parts = line.trim_end().splitn(3, ' ');
    let version = parts.next().unwrap_or("");
    if !version.starts_with("HTTP/1.") {
        return Err(Failure::Malformed(format!("the status line {:?}", line.trim_end())));
    }
    let status: u16 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(|| Failure::Malformed("no status".into()))?;
    let reason = parts.next().unwrap_or("").to_string();
    let mut headers = vec![];
    loop {
        line.clear();
        let n = r.read_line(&mut line).map_err(io_failure)?;
        if n == 0 {
            return Err(Failure::Closed("the headers were cut off".into()));
        }
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    Ok((status, reason, headers))
}

/// Reads a body after its head: by `Content-Length`, chunked, or to the end.
pub fn read_body(r: &mut impl BufRead, headers: &[(String, String)], status: u16, method: &str) -> Result<Vec<u8>, Failure> {
    if method == "HEAD" || status == 204 || status == 304 || (100..200).contains(&status) {
        return Ok(vec![]);
    }
    let chunked = crate::call::header(headers, "transfer-encoding").is_some_and(|t| t.to_ascii_lowercase().contains("chunked"));
    if chunked {
        let mut body = vec![];
        loop {
            let mut size = String::new();
            r.read_line(&mut size).map_err(io_failure)?;
            let n = usize::from_str_radix(size.trim().split(';').next().unwrap_or("0"), 16)
                .map_err(|_| Failure::Malformed(format!("a chunk size {:?}", size.trim())))?;
            if n == 0 {
                let mut trailer = String::new();
                loop {
                    trailer.clear();
                    if r.read_line(&mut trailer).map_err(io_failure)? == 0 || trailer.trim().is_empty() {
                        break;
                    }
                }
                return Ok(body);
            }
            let mut chunk = vec![0; n];
            r.read_exact(&mut chunk).map_err(io_failure)?;
            body.extend(chunk);
            let mut crlf = String::new();
            r.read_line(&mut crlf).map_err(io_failure)?;
        }
    }
    if let Some(len) = crate::call::header(headers, "content-length") {
        let n: usize = len.parse().map_err(|_| Failure::Malformed(format!("Content-Length {len}")))?;
        let mut body = vec![0; n];
        r.read_exact(&mut body).map_err(|e| match e.kind() {
            std::io::ErrorKind::UnexpectedEof => Failure::Closed("the body was cut off".into()),
            _ => io_failure(e),
        })?;
        return Ok(body);
    }
    let mut body = vec![];
    r.read_to_end(&mut body).map_err(io_failure)?;
    Ok(body)
}

/// Sends a request and reads the whole response, within the deadline.
pub fn send(ep: &Endpoint, req: &Outgoing, timeout: Duration) -> Result<Reply, Failure> {
    let start = Instant::now();
    let mut s = connect(ep, timeout)?;
    write_request(&mut s, &ep.authority, &ep.base, req).map_err(io_failure)?;
    let mut r = BufReader::new(s);
    let (status, reason, headers) = read_head(&mut r)?;
    let body = read_body(&mut r, &headers, status, &req.method)?;
    Ok(Reply { status, reason, headers, body, elapsed: start.elapsed() })
}

/// Sends a request, then hangs up after `after` without reading the answer,
/// as a client whose own timeout fires does.
pub fn send_and_hang_up(ep: &Endpoint, req: &Outgoing, after: Duration) -> Result<(), Failure> {
    let mut s = connect(ep, Duration::from_secs(10))?;
    write_request(&mut s, &ep.authority, &ep.base, req).map_err(io_failure)?;
    std::thread::sleep(after);
    let _ = s.shutdown(std::net::Shutdown::Both);
    Ok(())
}

/// A request read off a connection, as a server or proxy receives it.
#[derive(Clone, Debug)]
pub struct Received {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Whether the client asked for the connection to close after it.
    pub close: bool,
}

/// Reads one request; `None` when the client closed the connection first.
pub fn read_request(r: &mut impl BufRead) -> Result<Option<Received>, Failure> {
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line).map_err(io_failure)? == 0 {
            return Ok(None);
        }
        if !line.trim().is_empty() {
            break;
        }
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().ok_or_else(|| Failure::Malformed(format!("the request line {:?}", line.trim_end())))?.to_string();
    let version = parts.next().unwrap_or("HTTP/1.1").to_string();
    let mut headers = vec![];
    loop {
        line.clear();
        if r.read_line(&mut line).map_err(io_failure)? == 0 {
            return Err(Failure::Closed("the headers were cut off".into()));
        }
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let close = match crate::call::header(&headers, "connection").map(str::to_ascii_lowercase) {
        Some(c) if c.contains("close") => true,
        Some(c) if c.contains("keep-alive") => false,
        _ => version == "HTTP/1.0",
    };
    let has_body = crate::call::header(&headers, "content-length").is_some()
        || crate::call::header(&headers, "transfer-encoding").is_some();
    let body = if has_body { read_body(r, &headers, 200, "POST")? } else { vec![] };
    Ok(Some(Received { method, target, headers, body, close }))
}

/// Writes a response with its `Content-Length`; hop-by-hop headers are dropped.
pub fn write_response(w: &mut impl Write, status: u16, reason: &str, headers: &[(String, String)], body: &[u8], close: bool) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
    for (k, v) in headers {
        let lower = k.to_ascii_lowercase();
        if matches!(lower.as_str(), "content-length" | "transfer-encoding" | "connection" | "keep-alive") {
            continue;
        }
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    if close {
        head.push_str("Connection: close\r\n");
    }
    head.push_str("\r\n");
    w.write_all(head.as_bytes())?;
    w.write_all(body)?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chunked and sized bodies read the same.
    #[test]
    fn reads_bodies() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        let mut r = BufReader::new(&raw[..]);
        let (status, _, headers) = read_head(&mut r).unwrap();
        assert_eq!(read_body(&mut r, &headers, status, "GET").unwrap(), b"abcde");
        let raw = b"HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\n\r\n{}";
        let mut r = BufReader::new(&raw[..]);
        let (status, reason, headers) = read_head(&mut r).unwrap();
        assert_eq!((status, reason.as_str()), (404, "Not Found"));
        assert_eq!(read_body(&mut r, &headers, status, "GET").unwrap(), b"{}");
        assert_eq!(Endpoint::parse("http://localhost:8080/api/").unwrap().base, "/api");
    }
}
