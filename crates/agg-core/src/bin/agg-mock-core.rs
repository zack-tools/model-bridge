//! Fixture sidecar that speaks the documented routes used by the contract tests.
//! It is not CLIProxyAPI and it never contacts a provider.

use agg_core::corecfg::parse_managed_config;
use std::collections::HashSet;
use std::env;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::process::exit;
use std::sync::Mutex;
use std::time::Duration;

fn main() {
    let mut config_path = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-config" | "--config" => config_path = args.next(),
            "-local-model" | "--local-model" | "-h" | "--help" => {}
            _ => {}
        }
    }
    let mode = env::var("APP_AGG_MOCK_MODE").unwrap_or_else(|_| "healthy".into());
    if mode == "crash" {
        eprintln!("access_token=leakthisvalue");
        exit(1);
    }
    if mode == "leak" {
        eprintln!("access_token=leakthisvalue refresh_token=leakrefreshvalue");
    }
    let config_path = config_path.unwrap_or_else(|| {
        eprintln!("missing -config");
        exit(2);
    });
    let text = std::fs::read_to_string(&config_path).unwrap_or_else(|err| {
        eprintln!("read config: {err}");
        exit(2);
    });
    let config = parse_managed_config(&text).unwrap_or_else(|err| {
        eprintln!("parse config: {err}");
        exit(2);
    });
    if config.host != "127.0.0.1" || config.allow_remote || config.discovery_enabled {
        eprintln!("mock refusing a non-loopback config");
        exit(3);
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, config.port)).unwrap_or_else(|err| {
        eprintln!("bind: {err}");
        exit(4);
    });
    let sessions = Mutex::new(HashSet::<String>::new());
    let mut health_responses = 0;
    for conn in listener.incoming() {
        let Ok(mut conn) = conn else { continue };
        let _ = conn.set_read_timeout(Some(Duration::from_secs(2)));
        if let Some((method, path, headers)) = read_request(&mut conn) {
            let body = route(
                &mode,
                &config.management_key,
                &config.gateway_key,
                &method,
                &path,
                &headers,
                &sessions,
            );
            let _ = conn.write_all(body.as_bytes());
            if mode == "exit-after-health" && path == "/healthz" {
                health_responses += 1;
                if health_responses == 1 { break; }
            }
        }
    }
}

fn route(
    mode: &str,
    password: &str,
    gateway_key: &str,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    sessions: &Mutex<HashSet<String>>,
) -> String {
    let target = path.split('?').next().unwrap_or(path);
    if target == "/healthz" {
        if mode == "nohealth" {
            return respond(500, "{\"status\":\"bad\"}");
        }
        return respond(200, "{\"status\":\"ok\"}");
    }
    if target == "/v1/models" {
        if !gateway_ok(headers, gateway_key) {
            return respond(401, "{\"error\":\"unauthorized\"}");
        }
        return respond(
            200,
            "{\"object\":\"list\",\"data\":[{\"id\":\"claude-opus\",\"object\":\"model\",\"owned_by\":\"anthropic\"},{\"id\":\"gpt-example\",\"object\":\"model\",\"owned_by\":\"openai\"},{\"id\":\"gemini-example\",\"object\":\"model\",\"owned_by\":\"google\"}]}",
        );
    }
    if target.starts_with("/v8/management") && !management_ok(headers, password) {
        return respond(401, "{\"error\":\"unauthorized\"}");
    }
    if target == "/v8/management/config" && method == "GET" {
        return respond(
            200,
            "{\"server\":{\"host\":\"127.0.0.1\"},\"management\":{\"allow-remote\":false}}",
        );
    }
    if target == "/v8/management/oauth/auth-url" && method == "GET" {
        let provider = query(path, "provider").unwrap_or_default();
        if provider != "codex" && provider != "antigravity" {
            return respond(404, "{\"error\":\"provider_not_found\"}");
        }
        if mode == "warn" {
            return respond(403, "{\"error\":\"account_warning\"}");
        }
        let state = format!("mock-{provider}");
        sessions.lock().expect("sessions").insert(state.clone());
        let host = if (mode == "oauth-client" || mode == "oauth-success") && provider == "codex" {
            "auth.openai.com"
        } else if mode == "oauth-client" || mode == "oauth-success" {
            "accounts.google.com"
        } else if provider == "codex" {
            "auth.example.invalid"
        } else {
            "accounts.example.invalid"
        };
        return respond(
            200,
            &format!(
                "{{\"status\":\"ok\",\"url\":\"https://{host}/oauth?state={state}&code=not-a-real-code\",\"state\":\"{state}\"}}"
            ),
        );
    }
    if target == "/v8/management/oauth/status" && method == "GET" {
        let state = query(path, "state").unwrap_or_default();
        let guard = sessions.lock().expect("sessions");
        if guard.contains(&format!("{state}:ok")) || mode == "oauth-success" {
            return respond(200, "{\"status\":\"ok\"}");
        }
        if guard.contains(&state) {
            return respond(200, "{\"status\":\"wait\"}");
        }
        return respond(404, "{\"error\":\"unknown_state\"}");
    }
    if (target == "/codex/callback" || target == "/antigravity/callback") && method == "GET" {
        let state = query(path, "state").unwrap_or_default();
        let mut guard = sessions.lock().expect("sessions");
        if guard.contains(&state) {
            if query(path, "error").is_some() || query(path, "error_description").is_some() {
                guard.remove(&state);
                return respond(200, "{\"status\":\"error_received\"}");
            }
            guard.insert(format!("{state}:ok"));
            return respond(200, "{\"status\":\"ok\"}");
        }
        return respond(400, "{\"error\":\"invalid_state\"}");
    }
    if target == "/v8/management/oauth/session" && method == "DELETE" {
        let state = query(path, "state").unwrap_or_default();
        let mut guard = sessions.lock().expect("sessions");
        let removed1 = guard.remove(&state);
        let removed2 = guard.remove(&format!("{state}:ok"));
        let removed = removed1 || removed2;
        return respond(200, &format!("{{\"status\":\"ok\",\"cancelled\":{removed}}}"));
    }
    if target == "/v8/management/credentials" && method == "GET" {
        return respond(
            200,
            "{\"files\":[{\"name\":\"fixture.json\",\"provider\":\"codex\",\"auth_index\":\"abc123456789\",\"status\":\"ready\",\"disabled\":false,\"unavailable\":false,\"source\":\"file\"}]}",
        );
    }
    respond(404, "{\"error\":\"not_found\"}")
}

fn gateway_ok(headers: &[(String, String)], key: &str) -> bool {
    header(headers, "authorization") == Some(format!("Bearer {key}"))
        || header(headers, "x-api-key") == Some(key.to_string())
}

fn management_ok(headers: &[(String, String)], password: &str) -> bool {
    header(headers, "authorization") == Some(format!("Bearer {password}"))
        || header(headers, "x-management-key") == Some(password.to_string())
}

fn header(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

fn query(path: &str, key: &str) -> Option<String> {
    let query = path.split_once('?')?.1;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == key {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn read_request(conn: &mut TcpStream) -> Option<(String, String, Vec<(String, String)>)> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16384 {
        match conn.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let (head, _) = text.split_once("\r\n\r\n")?;
    let mut lines = head.lines();
    let request = lines.next()?;
    let mut parts = request.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    Some((method, path, headers))
}

fn respond(status: u16, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
