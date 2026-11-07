//! Loopback-only HTTP/1.1 client. Refuses any non-loopback address.

use crate::redact::Redactor;
use std::io::{BufRead, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

#[derive(Clone)]
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
    if method.is_empty() || !method.bytes().all(|b| b.is_ascii_uppercase()) { return Err("invalid HTTP method".into()); }
    if !path_and_query.starts_with('/') || path_and_query.bytes().any(|b| b <= b' ' || b == 127) {
        return Err("refusing a non-path request".into());
    }
    for (name, value) in headers {
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)) {
            return Err("invalid header name".into());
        }
        if value.contains('\n') || value.contains('\r') {
            return Err("refusing a header value with a line break".into());
        }
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
                return Err("read timed out before connection closed".into());
            }
            Err(e) => return Err(redactor.redact(&format!("read: {e}"))),
        }
    }
    parse_response(&buf).map_err(|e| redactor.redact(&e))
}

fn parse_response(bytes: &[u8]) -> Result<HttpResponse, String> {
    let split = bytes.windows(4).position(|w| w == b"\r\n\r\n").ok_or("response had no header terminator")?;
    let head = std::str::from_utf8(&bytes[..split]).map_err(|_| "invalid response headers")?;
    let status_line = head.lines().next().ok_or("missing status line")?;
    let mut parts = status_line.split_whitespace();
    let http = parts.next().ok_or("missing HTTP version")?;
    if !matches!(http, "HTTP/1.1" | "HTTP/1.0") { return Err("invalid HTTP version".into()); }
    let status: u16 = parts.next().ok_or("missing status code")?.parse().map_err(|_| "bad status code")?;
    let headers_lower = head.to_ascii_lowercase();
    let mut transfer = None;
    for line in headers_lower.lines() {
        if let Some(v) = line.strip_prefix("transfer-encoding:") {
            if transfer.is_some() { return Err("duplicate transfer encoding".into()); }
            transfer = Some(v.trim());
        }
    }
    if transfer.is_some() && content_length(&headers_lower)?.is_some() {
        return Err("ambiguous response framing".into());
    }
    let raw_body = &bytes[split + 4..];
    let body_bytes = if let Some(encoding) = transfer {
        if encoding != "chunked" {
            return Err("transfer encoding is not supported by this client".into());
        }
        let mut cursor = std::io::Cursor::new(raw_body);
        let mut decoded = Vec::new();
        decode_chunked(&mut cursor, 8 * 1024 * 1024, |chunk| {
            decoded.extend_from_slice(chunk);
            Ok(())
        })?;
        decoded
    } else if let Some(len) = content_length(&headers_lower)? {
        if raw_body.len() != len { return Err("response length mismatch".into()); }
        raw_body[..len].to_vec()
    } else {
        raw_body.to_vec()
    };
    let body = std::str::from_utf8(&body_bytes).map_err(|_| "invalid response body UTF-8")?.to_string();
    Ok(HttpResponse { status, body })
}

fn content_length(headers_lower: &str) -> Result<Option<usize>, String> {
    let mut length = None;
    for line in headers_lower.lines() {
        if let Some(v) = line.strip_prefix("content-length:") {
            if length.is_some() { return Err("duplicate content length".into()); }
            length = Some(v.trim().parse().map_err(|_| "invalid content length")?);
        }
    }
    Ok(length)
}

/// Decode a chunked HTTP body incrementally, requiring the terminating chunk and
/// complete trailers. The consumer must discard partial output if this fails.
pub fn decode_chunked<R, F>(reader: &mut R, max_body: usize, mut consume: F) -> Result<(), String>
where
    R: std::io::BufRead,
    F: FnMut(&[u8]) -> Result<(), String>,
{
    fn line<R: std::io::BufRead>(reader: &mut R) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        reader.take(8193).read_until(b'\n', &mut bytes).map_err(|_| "chunk line read failed")?;
        if bytes.len() > 8192 || !bytes.ends_with(b"\r\n") { return Err("incomplete or oversized chunk line".into()); }
        bytes.truncate(bytes.len() - 2);
        Ok(bytes)
    }
    let mut total = 0usize;
    loop {
        let header = line(reader)?;
        let mut fields = header.splitn(2, |b| *b == b';');
        let digits = fields.next().unwrap_or_default();
        if digits.is_empty() || digits.len() > 16 || !digits.iter().all(u8::is_ascii_hexdigit) {
            return Err("invalid chunk size".into());
        }
        if fields.next().is_some_and(|ext| ext.iter().any(|b| *b < 32 || *b == 127)) {
            return Err("invalid chunk extension".into());
        }
        let size = usize::from_str_radix(std::str::from_utf8(digits).map_err(|_| "invalid chunk size")?, 16)
            .map_err(|_| "chunk size overflow")?;
        if size == 0 {
            let mut trailer_bytes = 0usize;
            loop {
                let trailer = line(reader)?;
                trailer_bytes = trailer_bytes.checked_add(trailer.len() + 2).ok_or("trailer overflow")?;
                if trailer_bytes > 8192 { return Err("trailers exceed limit".into()); }
                if trailer.is_empty() { return Ok(()); }
                let colon = trailer.iter().position(|b| *b == b':').ok_or("invalid trailer")?;
                let name = &trailer[..colon];
                if name.is_empty() || !name.iter().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(b))
                    || name.eq_ignore_ascii_case(b"content-length") || name.eq_ignore_ascii_case(b"transfer-encoding")
                    || trailer[colon + 1..].iter().any(|b| (*b < 32 && *b != b'\t') || *b == 127) {
                    return Err("invalid trailer".into());
                }
            }
        }
        total = total.checked_add(size).ok_or("body size overflow")?;
        if total > max_body { return Err("body exceeds limit".into()); }
        let mut remaining = size;
        let mut buf = [0u8; 8192];
        while remaining > 0 {
            let n = reader.read(&mut buf[..remaining.min(8192)]).map_err(|_| "chunk read failed")?;
            if n == 0 { return Err("truncated chunk".into()); }
            consume(&buf[..n])?;
            remaining -= n;
        }
        let mut ending = [0u8; 2];
        reader.read_exact(&mut ending).map_err(|_| "truncated chunk delimiter")?;
        if ending != *b"\r\n" { return Err("invalid chunk delimiter".into()); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn chunked_requires_complete_framing() {
        fn decode(input: &[u8], max: usize) -> Result<Vec<u8>, String> {
            let mut result = Vec::new();
            let mut reader = std::io::BufReader::with_capacity(2, input);
            decode_chunked(&mut reader, max, |bytes| { result.extend_from_slice(bytes); Ok(()) })?;
            Ok(result)
        }
        assert_eq!(decode(b"3\r\nabc\r\n2;ext=x\r\nde\r\n0\r\nX-Test: ok\r\n\r\n", 5).unwrap(), b"abcde");
        for bad in [b"5\r\nabc".as_slice(), b"3\r\nabc\r\n".as_slice(), b"3\r\nabc\n0\r\n\r\n".as_slice(), b"0\r\nX-Test: ok\r\n".as_slice(), b"0\r\nContent-Length: 4\r\n\r\n".as_slice()] {
            assert!(decode(bad, 100).is_err());
        }
        assert!(decode(b"3\r\nabc\r\n0\r\n\r\n", 2).is_err());
    }

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

    #[test]
    fn content_length_is_in_bytes_and_truncation_is_rejected() {
        let body = "{\"message\":\"你好\"}";
        let valid = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}", body.len(), body);
        assert_eq!(parse_response(valid.as_bytes()).unwrap().body, body);
        let truncated = &valid.as_bytes()[..valid.len() - 1];
        assert!(parse_response(truncated).is_err());
        let extra = format!("{valid}x");
        assert!(parse_response(extra.as_bytes()).is_err());
        assert_eq!(parse_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n").unwrap().body, "");
        assert_eq!(parse_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n").unwrap().body, "hello");
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\n").is_err());
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 0\r\n\r\n").is_err());
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: wrong\r\n\r\n").is_err());
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n").is_err());
    }

    #[test]
    fn rejects_header_injection_before_connecting() {
        let redactor = Redactor::new(Vec::<String>::new());
        assert!(matches!(exchange(1, "GET", "/", &[("X-Test\r\nInjected", "value")], Duration::from_millis(50), &redactor), Err(e) if e == "invalid header name"));
    }

    #[test]
    fn partial_response_timeout_is_an_error() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf);
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n{").unwrap();
            thread::sleep(Duration::from_millis(250));
        });
        let redactor = Redactor::new(Vec::<String>::new());
        assert!(exchange(port, "GET", "/healthz", &[], Duration::from_millis(60), &redactor).is_err());
        handle.join().unwrap();
    }
}
