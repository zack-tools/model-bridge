//! One-shot check against the pinned CLIProxyAPI binary.
//!
//! OAuth is started only through the documented management route and is
//! cancelled. A provider warning stops that provider. No catalog row is
//! treated as a live verified model.

use crate::http::exchange;
use crate::oauth::{AccountSession, AuthState, FailureClass, PollOutcome, Provider, StartOutcome};
use crate::pin::{self, sha256_file};
use crate::redact::Redactor;
use crate::registry::{is_claude, CatalogModel, ModelRegistry, ModelStatus};
use crate::supervisor::{inspect_listeners, Supervisor, StartRequest};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct OAuthProbe {
    pub provider: String,
    pub outcome: String,
    pub http_status: Option<u16>,
    pub url_host: Option<String>,
    pub auth_state: String,
    pub live: bool,
    pub user_present_marks: u32,
    pub credential_token_material: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CoreCheckReport {
    pub core_version_pin: String,
    pub core_git_sha: String,
    pub archive_sha256_ok: bool,
    pub observed_binary_sha256: String,
    pub health_status: Option<u16>,
    pub loopback_v4: bool,
    pub wildcard_listener: bool,
    pub non_loopback_connect_failed: Option<bool>,
    pub management_without_key_status: Option<u16>,
    pub host: Option<String>,
    pub allow_remote: Option<bool>,
    pub models_auth: String,
    pub catalog_rejected_claude: usize,
    pub selectable_count: usize,
    pub selectable_live_verified: usize,
    pub user_present_marks: u32,
    pub core_integration_pass: String,
    pub protocol_pinned_core: String,
    pub oauth_live_pass: String,
    pub native_ui_pass: String,
    pub oauth: Vec<OAuthProbe>,
    pub version_line: Option<String>,
    pub notes: Vec<String>,
}

pub fn run_pinned_core_check(archive: &Path, binary: &Path) -> Result<CoreCheckReport, String> {
    let archive_sha = pin::verify_archive(archive, pin::LINUX_AMD64_ARCHIVE)?;
    let _ = archive_sha;
    let observed_binary_sha256 = sha256_file(binary)?;
    let home_marker = Path::new(&std::env::var("HOME").unwrap_or_else(|_| "/home/ubuntu".into()))
        .join(".cli-proxy-api");
    let home_existed = home_marker.exists();

    let gateway_key = random_token("agglocal")?;
    let management_password = random_token("mgmt")?;
    let redactor = Redactor::new([gateway_key.clone(), management_password.clone()]);
    let work = std::env::temp_dir().join(format!(
        "agg-core-check-{}",
        &pin::sha256_bytes(gateway_key.as_bytes())[..12]
    ));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;

    let mut report = CoreCheckReport {
        core_version_pin: pin::CORE_VERSION.into(),
        core_git_sha: pin::CORE_GIT_SHA.into(),
        archive_sha256_ok: true,
        observed_binary_sha256,
        health_status: None,
        loopback_v4: false,
        wildcard_listener: false,
        non_loopback_connect_failed: None,
        management_without_key_status: None,
        host: None,
        allow_remote: None,
        models_auth: "not-run".into(),
        catalog_rejected_claude: 0,
        selectable_count: 0,
        selectable_live_verified: 0,
        user_present_marks: 0,
        core_integration_pass: "fail".into(),
        protocol_pinned_core: "not-run".into(),
        oauth_live_pass: "unverified".into(),
        native_ui_pass: "unverified".into(),
        oauth: Vec::new(),
        version_line: None,
        notes: Vec::new(),
    };

    let supervisor = Supervisor::start(StartRequest {
        program: binary.to_path_buf(),
        work_dir: work.clone(),
        port: None,
        extra_env: Vec::new(),
        health_timeout: Duration::from_secs(45),
        gateway_key: gateway_key.clone(),
        management_password: management_password.clone(),
    });
    let mut supervisor = match supervisor {
        Ok(s) => s,
        Err(err) => {
            report.notes.push(redactor.redact(&err));
            let _ = fs::remove_dir_all(&work);
            return Ok(report);
        }
    };
    report.version_line = extract_version(&supervisor.redacted_logs());

    let health = exchange(
        supervisor.port,
        "GET",
        "/healthz",
        &[],
        Duration::from_secs(3),
        &redactor,
    );
    if let Ok(resp) = &health {
        report.health_status = Some(resp.status);
    } else if let Err(err) = &health {
        report.notes.push(redactor.redact(err));
    }

    match inspect_listeners(supervisor.port) {
        Ok(listeners) => {
            report.loopback_v4 = listeners.loopback_v4;
            report.wildcard_listener = listeners.wildcard_v4 || listeners.wildcard_v6;
        }
        Err(err) => report.notes.push(err),
    }
    report.non_loopback_connect_failed = non_loopback_blocked(supervisor.port);

    let unauth = exchange(
        supervisor.port,
        "GET",
        "/v8/management/config",
        &[],
        Duration::from_secs(3),
        &redactor,
    );
    if let Ok(resp) = unauth {
        report.management_without_key_status = Some(resp.status);
    }

    let config = exchange(
        supervisor.port,
        "GET",
        "/v8/management/config",
        &[("Authorization", &format!("Bearer {management_password}"))],
        Duration::from_secs(5),
        &redactor,
    );
    if let Ok(resp) = config {
        if resp.status == 200 {
            if let Ok(value) = serde_json::from_str::<Value>(&resp.body) {
                report.host = value
                    .pointer("/server/host")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or_else(|| find_string(&value, "host"));
                report.allow_remote = value
                    .pointer("/management/allow-remote")
                    .and_then(|v| v.as_bool())
                    .or_else(|| find_bool(&value, "allow-remote"));
            } else {
                report.notes.push("management config was not json".into());
            }
        } else {
            report.notes.push(format!("management config status {}", resp.status));
        }
    }

    report.models_auth = probe_models(
        supervisor.port,
        &gateway_key,
        &redactor,
        &mut report,
    );

    for provider in [Provider::ChatGptCodex, Provider::Antigravity] {
        report.oauth.push(probe_oauth(
            supervisor.port,
            &management_password,
            &supervisor.auth_dir,
            provider,
            &redactor,
        ));
    }
    report.user_present_marks = report.oauth.iter().map(|p| p.user_present_marks).sum();

    let logs = supervisor.redacted_logs();
    if logs.contains("unsafe example API key") {
        report.notes.push("upstream rejected the local gateway key as an example key".into());
    }
    if home_existed != home_marker.exists() && !home_existed {
        report.notes.push(
            "sidecar created ~/.cli-proxy-api outside the temp HOME; isolation failed".into(),
        );
    }
    supervisor.stop();
    let _ = fs::remove_dir_all(&work);

    let integration_ok = report.health_status == Some(200)
        && report.loopback_v4
        && !report.wildcard_listener
        && report.non_loopback_connect_failed != Some(false)
        && report.management_without_key_status.is_some_and(|s| s != 200)
        && report.host.as_deref() == Some("127.0.0.1")
        && report.allow_remote == Some(false)
        && report.notes.iter().all(|n| !n.contains("isolation failed"));
    report.core_integration_pass = if integration_ok { "pass" } else { "fail" }.into();

    let unsafe_oauth = report.oauth.iter().any(|probe| {
        probe.live
            || probe.user_present_marks != 0
            || probe.credential_token_material
            || probe.outcome == "refused-unwitnessed-success"
    });
    let oauth_ok = report.oauth.len() == 2
        && report.oauth.iter().all(|probe| {
            matches!(
                probe.outcome.as_str(),
                "started-and-cancelled" | "warning-stopped" | "upstream-error"
            )
        });
    let models_ok = report.models_auth != "failed"
        && report.models_auth != "not-run"
        && report.selectable_count == 0
        && report.selectable_live_verified == 0;
    report.protocol_pinned_core = if unsafe_oauth {
        "fail"
    } else if integration_ok && models_ok && oauth_ok {
        "pass"
    } else if integration_ok {
        "partial"
    } else {
        "fail"
    }
    .into();
    Ok(report)
}

fn probe_models(
    port: u16,
    gateway_key: &str,
    redactor: &Redactor,
    report: &mut CoreCheckReport,
) -> String {
    let bearer = format!("Bearer {gateway_key}");
    let attempts = [
        ("bearer", vec![("Authorization", bearer.as_str())]),
        ("x-api-key", vec![("X-Api-Key", gateway_key)]),
    ];
    for (name, headers) in attempts {
        let resp = match exchange(
            port,
            "GET",
            "/v1/models",
            &headers,
            Duration::from_secs(8),
            redactor,
        ) {
            Ok(resp) => resp,
            Err(err) => {
                report.notes.push(redactor.redact(&err));
                continue;
            }
        };
        if resp.status != 200 {
            continue;
        }
        let registry = ModelRegistry::new();
        match parse_catalog(&resp.body) {
            Ok(models) => {
                let obs = registry.observe_gateway_catalog(&models);
                report.catalog_rejected_claude = obs.rejected_claude;
                report.selectable_count = registry.selectable().len();
                report.selectable_live_verified = registry
                    .selectable()
                    .iter()
                    .filter(|entry| {
                        matches!(entry.status, ModelStatus::Verified { live: true, .. })
                    })
                    .count();
            }
            Err(err) => report.notes.push(err),
        }
        return name.into();
    }
    "failed".into()
}

fn probe_oauth(
    port: u16,
    password: &str,
    auth_dir: &Path,
    provider: Provider,
    redactor: &Redactor,
) -> OAuthProbe {
    let mut session = AccountSession::new(provider);
    session.acknowledge_terms();
    let path = format!(
        "/v8/management/oauth/auth-url?provider={}",
        provider.upstream_name()
    );
    let bearer = format!("Bearer {password}");
    let resp = exchange(
        port,
        "GET",
        &path,
        &[("Authorization", bearer.as_str())],
        Duration::from_secs(20),
        redactor,
    );
    let mut probe = OAuthProbe {
        provider: provider.upstream_name().into(),
        outcome: "transport-error".into(),
        http_status: None,
        url_host: None,
        auth_state: "loggedOut".into(),
        live: false,
        user_present_marks: 0,
        credential_token_material: dir_has_token_material(auth_dir),
    };
    let resp = match resp {
        Ok(resp) => resp,
        Err(err) => {
            let _ = session.begin(StartOutcome::Failed(FailureClass::Transport));
            probe.auth_state = format!("{:?}", session.state);
            probe.outcome = format!("transport-error: {}", redact_note(&err));
            return probe;
        }
    };
    probe.http_status = Some(resp.status);
    if resp.status >= 400 || looks_like_warning(&resp.body) {
        let class = if looks_like_warning(&resp.body) {
            FailureClass::AccountWarning
        } else {
            FailureClass::UpstreamRejected
        };
        let _ = session.begin(StartOutcome::Failed(class));
        probe.outcome = if class == FailureClass::AccountWarning {
            "warning-stopped"
        } else {
            "upstream-error"
        }
        .into();
        probe.auth_state = state_name(session.state);
        probe.live = session.live;
        probe.credential_token_material = dir_has_token_material(auth_dir);
        return probe;
    }
    let host = json_url_host(&resp.body);
    let state = json_string(&resp.body, "state").unwrap_or_default();
    if state.is_empty() || host.is_none() {
        let _ = session.begin(StartOutcome::Failed(FailureClass::UpstreamRejected));
        probe.outcome = "upstream-error".into();
        probe.auth_state = state_name(session.state);
        return probe;
    }
    let _ = session.begin(StartOutcome::Started {
        upstream_state: state.clone(),
        url_host: host.clone().unwrap_or_default(),
    });
    probe.url_host = host;
    let status_path = format!(
        "/v8/management/oauth/status?state={}",
        urlencoding_min(&state)
    );
    if let Ok(status) = exchange(
        port,
        "GET",
        &status_path,
        &[("Authorization", bearer.as_str())],
        Duration::from_secs(10),
        redactor,
    ) {
        let kind = json_string(&status.body, "status").unwrap_or_default();
        if kind == "ok" {
            session.observe_poll(PollOutcome::Success);
        } else if kind == "error" || looks_like_warning(&status.body) {
            session.observe_poll(PollOutcome::Failed(FailureClass::AccountWarning));
        } else {
            session.observe_poll(PollOutcome::Wait);
        }
    }
    let cancel_path = format!(
        "/v8/management/oauth/session?state={}",
        urlencoding_min(&state)
    );
    let _ = exchange(
        port,
        "DELETE",
        &cancel_path,
        &[("Authorization", bearer.as_str())],
        Duration::from_secs(10),
        redactor,
    );
    if session.state != AuthState::Error {
        session.cancel();
    }
    probe.auth_state = state_name(session.state);
    probe.live = session.live;
    probe.user_present_marks = session.user_present_marks;
    probe.credential_token_material = dir_has_token_material(auth_dir);
    probe.outcome = if session.failure == Some(FailureClass::RefusedUnwitnessedSuccess) {
        "refused-unwitnessed-success"
    } else if session.failure == Some(FailureClass::AccountWarning) {
        "warning-stopped"
    } else {
        "started-and-cancelled"
    }
    .into();
    probe
}

fn parse_catalog(body: &str) -> Result<Vec<CatalogModel>, String> {
    let value: Value = serde_json::from_str(body).map_err(|e| format!("models json: {e}"))?;
    let data = value.get("data").and_then(|v| v.as_array()).ok_or("models missing data")?;
    let mut out = Vec::new();
    for item in data {
        let id = item
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            continue;
        }
        let owned_by = item
            .get("owned_by")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        if is_claude(&id, owned_by.as_deref()) {
            out.push(CatalogModel { id, owned_by });
        } else {
            out.push(CatalogModel { id, owned_by });
        }
    }
    Ok(out)
}

fn dir_has_token_material(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let Ok(text) = fs::read_to_string(entry.path()) else {
            continue;
        };
        let lower = text.to_ascii_lowercase();
        if lower.contains("access_token") || lower.contains("refresh_token") {
            return true;
        }
    }
    false
}

fn looks_like_warning(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    ["account_warning", "suspended", "banned", "policy", "login_warning"]
        .iter()
        .any(|w| lower.contains(w))
}

fn json_string(body: &str, key: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    value.get(key)?.as_str().map(str::to_string)
}

fn json_url_host(body: &str) -> Option<String> {
    let url = json_string(body, "url")?;
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?.trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn urlencoding_min(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn state_name(state: AuthState) -> String {
    format!("{state:?}")
}

fn redact_note(text: &str) -> String {
    let clean = crate::redact::redact_text(text);
    if clean.len() > 180 {
        format!("{}…", &clean[..180])
    } else {
        clean
    }
}

fn find_string(value: &Value, key: &str) -> Option<String> {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(s)) = map.get(key) {
                return Some(s.clone());
            }
            for child in map.values() {
                if let Some(found) = find_string(child, key) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(|item| find_string(item, key)),
        _ => None,
    }
}

fn find_bool(value: &Value, key: &str) -> Option<bool> {
    match value {
        Value::Object(map) => {
            if let Some(Value::Bool(v)) = map.get(key) {
                return Some(*v);
            }
            for child in map.values() {
                if let Some(found) = find_bool(child, key) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(|item| find_bool(item, key)),
        _ => None,
    }
}

fn extract_version(logs: &str) -> Option<String> {
    for line in logs.lines() {
        if line.contains("CLIProxyAPI Version:") {
            return Some(line.trim().to_string());
        }
    }
    None
}

fn non_loopback_blocked(port: u16) -> Option<bool> {
    let output = std::process::Command::new("ip")
        .args(["-4", "-o", "addr", "show", "scope", "global"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let ip = text.split_whitespace().find(|p| p.contains('.'))?;
    let ip = ip.split('/').next()?.to_string();
    let addr = format!("{ip}:{port}");
    let socket: std::net::SocketAddr = addr.parse().ok()?;
    let blocked = std::net::TcpStream::connect_timeout(&socket, Duration::from_millis(400)).is_err();
    Some(blocked)
}

fn random_token(prefix: &str) -> Result<String, String> {
    let mut buf = [0u8; 24];
    getrandom::getrandom(&mut buf).map_err(|e| e.to_string())?;
    Ok(format!("{prefix}{}", pin::sha256_bytes(&buf)))
}

pub fn sanitize_report(report: &CoreCheckReport, secrets: &[&str]) -> String {
    let text = serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".into());
    Redactor::new(secrets.iter().copied()).redact(&text)
}
