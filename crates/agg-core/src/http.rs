//! Loopback-only HTTP/1.1 client. Refuses any non-loopback address.

use crate::redact::Redactor;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

pub fn exchange(
    port: u16,
    method: &str,
    path_and_query: &str,
    headers: &[(&str, &str)],
    timeout: Duration,
    redactor: &Redactor,
) -> Result<HttpResponse, String> {
    if !path_and_query.starts_with('/') {
        return Err("refusing a non-path request".into());
    }
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| redactor.redact(&format!("connect 127.0.0.1:{port}: {e}")))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;

    let mut req = format!(
        "{method} {path_and_query} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nAccept: application/json\r\n"
    );
    for (name, value) in headers {
        if value.contains('\n') || value.contains('\r') {
            return Err("refusing a header value with a line break".into());
        }
        req.push_str(name);
        req.push_str(": ");
        req.push_str(value);
        req.push_str("\r\n");
    }
    req.push_str("\r\n");
    stream
        .write_all(req.as_bytes())
        .map_err(|e| redactor.redact(&format!("write: {e}")))?;

    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if buf.len() > 8 * 1024 * 1024 {
            return Err("response exceeded 8 MiB".into());
        }
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                if !buf.is_empty() {
                    break;
                }
                return Err("read timed out".into());
            }
            Err(e) => return Err(redactor.redact(&format!("read: {e}"))),
        }
    }
    parse_response(&buf).map_err(|e| redactor.redact(&e))
}

fn parse_response(bytes: &[u8]) -> Result<HttpResponse, String> {
    let text = String::from_utf8_lossy(bytes);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| "response had no header terminator".to_string())?;
    let status_line = head.lines().next().unwrap_or("");
    let mut parts = status_line.split_whitespace();
    let _http = parts.next().ok_or("missing status line")?;
    let status: u16 = parts
        .next()
        .ok_or("missing status code")?
        .parse()
        .map_err(|_| format!("bad status line: {status_line}"))?;
    let headers_lower = head.to_ascii_lowercase();
    let body = if let Some(len) = content_length(&headers_lower) {
        body.chars().take(len).collect::<String>()
    } else {
        body.to_string()
    };
    Ok(HttpResponse { status, body })
}

fn content_length(headers_lower: &str) -> Option<usize> {
    for line in headers_lower.lines() {
        if let Some(v) = line.strip_prefix("content-length:") {
            return v.trim().parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn client_reads_loopback_json() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf);
            let body = "{\"status\":\"ok\"}";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(resp.as_bytes()).unwrap();
        });
        let redactor = Redactor::new(Vec::<String>::new());
        let resp = exchange(port, "GET", "/healthz", &[], Duration::from_secs(2), &redactor)
            .unwrap();
        assert_eq!(resp.status, 200);
        assert!(resp.body.contains("ok"));
    }
}
