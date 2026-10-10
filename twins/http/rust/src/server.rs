//! HTTP/1.1 on the API port: requests read off each connection in turn,
//! answered as the twin plans, with keep-alive. The server is written
//! directly on the socket so a fault can hold a connection, reset it, or
//! send a body a few bytes at a time.

use crate::twin::{Action, Twin, reason};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The largest request the twin reads.
const MAX_REQUEST: usize = 16 << 20;

/// A request off the wire.
struct Request {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    close: bool,
}

enum Read {
    Request(Request),
    Closed,
    Bad(String),
}

/// Serves the API until the process ends.
pub async fn serve(listener: TcpListener, twin: Arc<Twin>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
        stream.set_nodelay(true).ok();
        let twin = twin.clone();
        tokio::spawn(async move {
            let _ = connection(stream, twin).await;
        });
    }
}

async fn connection(mut stream: TcpStream, twin: Arc<Twin>) -> std::io::Result<()> {
    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    loop {
        let req = match read_request(&mut stream, &mut buf).await? {
            Read::Request(r) => r,
            Read::Closed => return Ok(()),
            Read::Bad(why) => {
                let body = serde_json::to_vec(&serde_json::json!({ "message": why })).unwrap_or_default();
                let head = format!("HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                stream.write_all(head.as_bytes()).await?;
                stream.write_all(&body).await?;
                return Ok(());
            }
        };
        let t = twin.clone();
        let close = req.close;
        let plan = tokio::task::spawn_blocking(move || t.handle(&req.method, &req.target, &req.headers, &req.body))
            .await
            .map_err(std::io::Error::other)?;
        if plan.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(plan.delay_ms)).await;
        }
        match plan.action {
            Action::Hang(ms) => {
                // Read and drop whatever the client sends while it waits.
                let mut sink = [0u8; 1024];
                let _ = tokio::time::timeout(Duration::from_millis(ms), async {
                    loop {
                        match stream.read(&mut sink).await {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {}
                        }
                    }
                })
                .await;
                return Ok(());
            }
            Action::Reset => {
                // Lingering for no time closes with a reset, at once.
                #[allow(deprecated)]
                stream.set_linger(Some(Duration::ZERO)).ok();
                return Ok(());
            }
            Action::Respond { status, headers, body, slow } => {
                let mut head = format!("HTTP/1.1 {status} {}\r\n", reason(status));
                for (k, v) in &headers {
                    head.push_str(&format!("{k}: {v}\r\n"));
                }
                head.push_str(&format!("Content-Length: {}\r\n", body.len()));
                if close {
                    head.push_str("Connection: close\r\n");
                }
                head.push_str("\r\n");
                stream.write_all(head.as_bytes()).await?;
                match slow {
                    Some((chunk, delay)) if !body.is_empty() => {
                        for part in body.chunks(chunk) {
                            stream.write_all(part).await?;
                            stream.flush().await?;
                            tokio::time::sleep(Duration::from_millis(delay)).await;
                        }
                    }
                    _ => stream.write_all(&body).await?,
                }
                stream.flush().await?;
            }
        }
        if close {
            return Ok(());
        }
    }
}

/// Reads one request: its head, then its body by length or in chunks.
async fn read_request(stream: &mut TcpStream, buf: &mut Vec<u8>) -> std::io::Result<Read> {
    loop {
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(buf) {
            Ok(httparse::Status::Complete(head_len)) => {
                let method = req.method.unwrap_or("GET").to_string();
                let target = req.path.unwrap_or("/").to_string();
                let version = req.version.unwrap_or(1);
                let hs: Vec<(String, String)> =
                    req.headers.iter().map(|h| (h.name.to_string(), String::from_utf8_lossy(h.value).trim().to_string())).collect();
                let find = |n: &str| hs.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.to_ascii_lowercase());
                let close = match find("connection") {
                    Some(c) if c.contains("close") => true,
                    Some(c) if c.contains("keep-alive") => false,
                    _ => version == 0,
                };
                buf.drain(..head_len);
                let body = if find("transfer-encoding").is_some_and(|t| t.contains("chunked")) {
                    match read_chunked(stream, buf).await? {
                        Some(b) => b,
                        None => return Ok(Read::Bad("a chunked body was cut off".into())),
                    }
                } else {
                    let len: usize = match find("content-length").map(|l| l.parse()) {
                        None => 0,
                        Some(Ok(n)) => n,
                        Some(Err(_)) => return Ok(Read::Bad("Content-Length is not a number".into())),
                    };
                    if len > MAX_REQUEST {
                        return Ok(Read::Bad("the request is too large".into()));
                    }
                    while buf.len() < len {
                        if !fill(stream, buf).await? {
                            return Ok(Read::Closed);
                        }
                    }
                    buf.drain(..len).collect()
                };
                return Ok(Read::Request(Request { method, target, headers: hs, body, close }));
            }
            Ok(httparse::Status::Partial) => {
                if buf.len() > MAX_REQUEST {
                    return Ok(Read::Bad("the request head is too large".into()));
                }
                if !fill(stream, buf).await? {
                    return Ok(Read::Closed);
                }
            }
            Err(e) => return Ok(Read::Bad(format!("not an HTTP/1.1 request: {e}"))),
        }
    }
}

async fn fill(stream: &mut TcpStream, buf: &mut Vec<u8>) -> std::io::Result<bool> {
    let mut chunk = [0u8; 8192];
    match stream.read(&mut chunk).await {
        Ok(0) => Ok(false),
        Ok(n) => {
            buf.extend_from_slice(&chunk[..n]);
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => Ok(false),
        Err(e) => Err(e),
    }
}

async fn read_chunked(stream: &mut TcpStream, buf: &mut Vec<u8>) -> std::io::Result<Option<Vec<u8>>> {
    let mut body = vec![];
    loop {
        let line_end = loop {
            if let Some(i) = buf.windows(2).position(|w| w == b"\r\n") {
                break i;
            }
            if !fill(stream, buf).await? {
                return Ok(None);
            }
        };
        let size_text = String::from_utf8_lossy(&buf[..line_end]).to_string();
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16).unwrap_or(0);
        buf.drain(..line_end + 2);
        if size == 0 {
            // Trailers, up to an empty line.
            loop {
                if let Some(i) = buf.windows(2).position(|w| w == b"\r\n") {
                    let empty = i == 0;
                    buf.drain(..i + 2);
                    if empty {
                        return Ok(Some(body));
                    }
                } else if !fill(stream, buf).await? {
                    return Ok(Some(body));
                }
            }
        }
        while buf.len() < size + 2 {
            if !fill(stream, buf).await? {
                return Ok(None);
            }
        }
        body.extend(buf.drain(..size));
        buf.drain(..2);
        if body.len() > MAX_REQUEST {
            return Ok(None);
        }
    }
}
