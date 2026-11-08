//! Loopback-only OAuth callback listener.
//! Strictly binds 127.0.0.1, validates state, rejects non-loopback clients,
//! forwards callback to upstream core management on 127.0.0.1,
//! returns safe localized HTML to the browser, and avoids exposing or logging
//! auth codes, tokens, or sensitive query strings.

use crate::{
    http::exchange,
    oauth::{FailureClass, Provider},
    redact::Redactor,
};
use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const CODEX_CALLBACK_PORT: u16 = 1455;
pub const ANTIGRAVITY_CALLBACK_PORT: u16 = 51121;
pub const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);

pub struct LoopbackCallbackServer {
    stop_signal: Arc<AtomicBool>,
    thread_handle: Option<JoinHandle<()>>,
    pub port: u16,
}

impl LoopbackCallbackServer {
    pub fn default_port_for_provider(provider: Provider) -> u16 {
        match provider {
            Provider::ChatGptCodex => CODEX_CALLBACK_PORT,
            Provider::Antigravity => ANTIGRAVITY_CALLBACK_PORT,
        }
    }

    /// Attempts to bind to 127.0.0.1 on the specified port.
    /// If port is 0, OS selects an ephemeral port (useful for isolated tests).
    pub fn bind(port: u16) -> Result<(TcpListener, u16), FailureClass> {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
        let listener = TcpListener::bind(addr).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                FailureClass::PortConflict
            } else {
                FailureClass::Transport
            }
        })?;
        let bound_port = listener.local_addr().map_err(|_| FailureClass::Transport)?.port();
        listener.set_nonblocking(true).map_err(|_| FailureClass::Transport)?;
        Ok((listener, bound_port))
    }

    /// Spawns the background loopback callback forwarder worker.
    pub fn start(
        listener: TcpListener,
        provider: Provider,
        expected_state: String,
        core_port: u16,
    ) -> Self {
        let bound_port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        let stop_signal = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop_signal);

        let thread_handle = thread::spawn(move || {
            run_server_loop(listener, provider, expected_state, core_port, stop_clone);
        });

        Self {
            stop_signal,
            thread_handle: Some(thread_handle),
            port: bound_port,
        }
    }

    pub fn stop(&mut self) {
        self.stop_signal.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for LoopbackCallbackServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_server_loop(
    listener: TcpListener,
    provider: Provider,
    expected_state: String,
    core_port: u16,
    stop_signal: Arc<AtomicBool>,
) {
    let start_time = Instant::now();

    while !stop_signal.load(Ordering::Relaxed) && start_time.elapsed() < CALLBACK_TIMEOUT {
        match listener.accept() {
            Ok((mut stream, peer_addr)) => {
                // Security check 1: Client must be loopback!
                if !peer_addr.ip().is_loopback() {
                    drop(stream);
                    continue;
                }

                // Accepted sockets can inherit nonblocking mode on some platforms.
                if stream.set_nonblocking(false).is_err() { continue; }
                let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));

                let mut buf = [0u8; 8192];
                let n = match stream.read(&mut buf) {
                    Ok(n) if n > 0 => n,
                    _ => continue,
                };

                let request = String::from_utf8_lossy(&buf[..n]);
                let first_line = match request.lines().next() {
                    Some(line) => line,
                    None => continue,
                };

                // Parse: GET /path?query HTTP/...
                let parts: Vec<&str> = first_line.split_whitespace().collect();
                if parts.len() < 2 || parts[0] != "GET" {
                    let _ = stream.write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                }

                let target = parts[1];
                let mut split = target.splitn(2, '?');
                let path = split.next().unwrap_or("");
                let query = split.next().unwrap_or("");

                // Return 404 for favicon or non-callback paths without query
                if path == "/favicon.ico" {
                    let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                }

                let received_state = parse_query_param(query, "state");
                let received_error = parse_query_param(query, "error").or_else(|| parse_query_param(query, "error_description"));

                // Security check 2: State parameter verification
                if received_state.as_deref() != Some(&expected_state) {
                    let err_body = "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\n\r\nInvalid or mismatched OAuth state parameter.";
                    let _ = stream.write_all(err_body.as_bytes());
                    let _ = stream.flush();
                    continue;
                }

                // Reject ambiguous OAuth parameters before consuming this listener.
                let count = |name: &str| query.split('&').filter(|pair| {
                    pair.split_once('=').map_or(*pair, |(key, _)| key) == name
                }).count();
                let code = parse_query_param(query, "code");
                let error = parse_query_param(query, "error");
                if count("state") != 1 || count("code") > 1 || count("error") > 1
                    || code.as_deref().is_some_and(str::is_empty)
                    || error.as_deref().is_some_and(str::is_empty)
                    || code.is_some() == error.is_some()
                {
                    let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n");
                    continue;
                }

                // State matches! Forward to upstream core
                let core_path = match provider {
                    Provider::ChatGptCodex => "/codex/callback",
                    Provider::Antigravity => "/antigravity/callback",
                };
                let path_and_query = format!("{core_path}?{query}");
                let redactor = Redactor::new([expected_state.clone()]);
                let forwarded = exchange(core_port, "GET", &path_and_query, &[], Duration::from_secs(10), &redactor)
                    .is_ok_and(|response| (200..300).contains(&response.status));
                if !forwarded {
                    let html = "<!doctype html><meta charset=\"utf-8\"><script>try{history.replaceState({},'', '/')}catch(e){}</script><p>授權回呼轉送失敗，請返回應用程式停止此流程並重新登入。</p>";
                    let response = format!("HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}", html.len(), html);
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                    break;
                }

                // Send browser response
                if received_error.is_some() {
                    let cancel_html = r#"<!DOCTYPE html>
<html lang="zh-Hant">
<head>
  <meta charset="utf-8">
  <title>授權已取消 - APP-AGG</title>
  <script>try{history.replaceState({}, '', '/');}catch(e){}</script>
  <style>
    body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; background: #0f172a; color: #f8fafc; }
    .card { background: #1e293b; padding: 2.5rem; border-radius: 12px; box-shadow: 0 10px 25px rgba(0,0,0,0.3); text-align: center; max-width: 420px; }
    h2 { color: #f43f5e; margin-top: 0; font-size: 1.4rem; }
    p { color: #94a3b8; font-size: 0.95rem; line-height: 1.6; margin: 1rem 0; }
  </style>
</head>
<body>
  <div class="card">
    <h2>授權已取消或遭拒絕</h2>
    <p>您已取消或拒絕授權流程。您可以關閉此分頁並返回 APP-AGG 桌面客戶端。</p>
  </div>
</body>
</html>"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
                        cancel_html.len(),
                        cancel_html
                    );
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.flush();
                } else {
                    let success_html = r#"<!DOCTYPE html>
<html lang="zh-Hant">
<head>
  <meta charset="utf-8">
  <title>授權完成 - APP-AGG</title>
  <script>try{history.replaceState({}, '', '/');}catch(e){}</script>
  <style>
    body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; background: #0f172a; color: #f8fafc; }
    .card { background: #1e293b; padding: 2.5rem; border-radius: 12px; box-shadow: 0 10px 25px rgba(0,0,0,0.3); text-align: center; max-width: 420px; }
    h2 { color: #38bdf8; margin-top: 0; font-size: 1.4rem; }
    p { color: #94a3b8; font-size: 0.95rem; line-height: 1.6; margin: 1rem 0; }
  </style>
</head>
<body>
  <div class="card">
    <h2>✓ 授權已安全接收</h2>
    <p>OAuth 驗證資訊已安全送回本機應用程式。您可以關閉此分頁並返回 APP-AGG 桌面客戶端完成後續確認。</p>
  </div>
</body>
</html>"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
                        success_html.len(),
                        success_html
                    );
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.flush();
                }

                break;
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(30));
            }
            Err(_) => break,
        }
    }
}

fn parse_query_param(query: &str, param_name: &str) -> Option<String> {
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        let k = kv.next()?.trim();
        if k == param_name {
            let v = kv.next().unwrap_or("");
            return Some(percent_decode_str(v));
        }
    }
    None
}

fn percent_decode_str(input: &str) -> String {
    let mut out = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(val) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                out.push(val);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
