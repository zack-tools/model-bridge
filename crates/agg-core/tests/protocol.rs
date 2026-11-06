use agg_core::http::exchange;
use agg_core::oauth::{AccountSession, AuthState, FailureClass, Provider, StartOutcome};
use agg_core::redact::Redactor;
use agg_core::registry::{CatalogModel, ModelRegistry};
use agg_core::supervisor::{inspect_listeners, StartRequest, Supervisor};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

fn boot(mode: &str) -> (Supervisor, PathBuf) {
    let mut nonce = [0u8; 4];
    getrandom::getrandom(&mut nonce).unwrap();
    let dir = std::env::temp_dir().join(format!("agg-proto-{}-{}", std::process::id(), nonce[0]));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let supervisor = Supervisor::start(StartRequest {
        program: PathBuf::from(env!("CARGO_BIN_EXE_agg-mock-core")),
        work_dir: dir.clone(),
        port: None,
        extra_env: vec![("APP_AGG_MOCK_MODE".into(), mode.into())],
        health_timeout: Duration::from_secs(3),
        gateway_key: "agglocal0123456789abcdef".into(),
        management_password: "mgmt0123456789abcdef".into(),
    })
    .unwrap_or_else(|err| panic!("mock start: {err}"));
    (supervisor, dir)
}

#[test]
fn mock_core_matches_the_documented_contract() {
    let (supervisor, dir) = boot("healthy");
    let redactor = Redactor::new(["agglocal0123456789abcdef", "mgmt0123456789abcdef"]);
    let port = supervisor.port;
    let health = exchange(port, "GET", "/healthz", &[], Duration::from_secs(2), &redactor).unwrap();
    assert_eq!(health.status, 200);
    let listeners = inspect_listeners(port).unwrap();
    assert!(listeners.loopback_v4);
    assert!(!listeners.wildcard_v4);

    let missing = exchange(port, "GET", "/v1/models", &[], Duration::from_secs(2), &redactor).unwrap();
    assert_eq!(missing.status, 401);
    let models = exchange(
        port,
        "GET",
        "/v1/models",
        &[("Authorization", "Bearer agglocal0123456789abcdef")],
        Duration::from_secs(2),
        &redactor,
    )
    .unwrap();
    assert_eq!(models.status, 200);
    let alt = exchange(
        port,
        "GET",
        "/v1/models",
        &[("X-Api-Key", "agglocal0123456789abcdef")],
        Duration::from_secs(2),
        &redactor,
    )
    .unwrap();
    assert_eq!(alt.status, 200);

    let value: Value = serde_json::from_str(&models.body).unwrap();
    let catalog: Vec<CatalogModel> = value["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| CatalogModel {
            id: item["id"].as_str().unwrap().to_string(),
            owned_by: item["owned_by"].as_str().map(str::to_string),
        })
        .collect();
    let registry = ModelRegistry::new();
    let obs = registry.observe_gateway_catalog(&catalog);
    assert!(obs.rejected_claude >= 1);
    assert_eq!(obs.selectable_added, 0);
    assert!(registry.selectable().is_empty());

    let unauth = exchange(
        port,
        "GET",
        "/v8/management/config",
        &[],
        Duration::from_secs(2),
        &redactor,
    )
    .unwrap();
    assert_ne!(unauth.status, 200);
    let config = exchange(
        port,
        "GET",
        "/v8/management/config",
        &[("Authorization", "Bearer mgmt0123456789abcdef")],
        Duration::from_secs(2),
        &redactor,
    )
    .unwrap();
    assert!(config.body.contains("127.0.0.1"));
    assert!(config.body.contains("\"allow-remote\":false"));

    let started = exchange(
        port,
        "GET",
        "/v8/management/oauth/auth-url?provider=codex",
        &[("Authorization", "Bearer mgmt0123456789abcdef")],
        Duration::from_secs(2),
        &redactor,
    )
    .unwrap();
    assert_eq!(started.status, 200);
    let body: Value = serde_json::from_str(&started.body).unwrap();
    assert!(body["url"].as_str().unwrap().starts_with("https://auth.example.invalid/"));
    let state = body["state"].as_str().unwrap();
    let mut session = AccountSession::new(Provider::ChatGptCodex);
    session.acknowledge_terms();
    session
        .begin(StartOutcome::Started {
            upstream_state: state.into(),
            url_host: "auth.example.invalid".into(),
        })
        .unwrap();
    assert_eq!(session.state, AuthState::Authorizing);
    let cancel = exchange(
        port,
        "DELETE",
        &format!("/v8/management/oauth/session?state={state}"),
        &[("Authorization", "Bearer mgmt0123456789abcdef")],
        Duration::from_secs(2),
        &redactor,
    )
    .unwrap();
    assert!(cancel.body.contains("\"cancelled\":true"));
    session.cancel();
    assert!(!session.live);
    assert_eq!(session.user_present_marks, 0);

    let agy = exchange(
        port,
        "GET",
        "/v8/management/oauth/auth-url?provider=antigravity",
        &[("Authorization", "Bearer mgmt0123456789abcdef")],
        Duration::from_secs(2),
        &redactor,
    )
    .unwrap();
    assert_eq!(agy.status, 200);
    let unknown = exchange(
        port,
        "GET",
        "/v8/management/oauth/auth-url?provider=claude",
        &[("Authorization", "Bearer mgmt0123456789abcdef")],
        Duration::from_secs(2),
        &redactor,
    )
    .unwrap();
    assert_eq!(unknown.status, 404);
    drop(supervisor);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn account_warning_stops_without_a_second_attempt() {
    let (supervisor, dir) = boot("warn");
    let redactor = Redactor::new(Vec::<String>::new());
    let resp = exchange(
        supervisor.port,
        "GET",
        "/v8/management/oauth/auth-url?provider=codex",
        &[("Authorization", "Bearer mgmt0123456789abcdef")],
        Duration::from_secs(2),
        &redactor,
    )
    .unwrap();
    assert_eq!(resp.status, 403);
    let mut session = AccountSession::new(Provider::ChatGptCodex);
    session.acknowledge_terms();
    let err = session
        .begin(StartOutcome::Failed(FailureClass::AccountWarning))
        .unwrap_err();
    assert_eq!(err, FailureClass::AccountWarning);
    assert!(session
        .begin(StartOutcome::Started {
            upstream_state: "no".into(),
            url_host: "auth.example.invalid".into()
        })
        .is_err());
    drop(supervisor);
    let _ = std::fs::remove_dir_all(&dir);
}
