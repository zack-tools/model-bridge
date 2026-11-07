use agg_core::http::exchange;
use agg_core::oauth::{AccountSession, AuthState, FailureClass, Provider, StartOutcome};
use agg_core::oauth_client::OAuthClient;
use agg_core::redact::Redactor;
use agg_core::registry::{CatalogModel, ModelRegistry};
#[cfg(target_os = "linux")]
use agg_core::supervisor::inspect_listeners;
use agg_core::supervisor::{StartRequest, Supervisor};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

static OAUTH_TEST_LOCK: Mutex<()> = Mutex::new(());

fn boot(mode: &str) -> (Supervisor, PathBuf) {
    let mut nonce = [0u8; 4];
    getrandom::getrandom(&mut nonce).unwrap();
    let dir = std::env::temp_dir().join(format!("agg-proto-{}-{}", std::process::id(), agg_core::pin::sha256_bytes(&nonce)));
    std::fs::create_dir(&dir).unwrap();
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
    #[cfg(target_os = "linux")]
    {
        let listeners = inspect_listeners(port).unwrap();
        assert!(listeners.loopback_v4);
        assert!(!listeners.wildcard_v4);
    }

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
    assert_eq!(config.status, 200);

    let list = exchange(
        port,
        "GET",
        "/v8/management/credentials",
        &[("Authorization", "Bearer mgmt0123456789abcdef")],
        Duration::from_secs(2),
        &redactor,
    )
    .unwrap();
    assert_eq!(list.status, 200);
    let value: Value = serde_json::from_str(&list.body).unwrap();
    let files = value["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["auth_index"].as_str().unwrap(), "abc123456789");

    drop(supervisor);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn account_warning_stops_without_a_second_attempt() {
    let (supervisor, dir) = boot("warn");
    let redactor = Redactor::new(["mgmt0123456789abcdef"]);
    let port = supervisor.port;
    let resp = exchange(
        port,
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

#[test]
fn oauth_client_requires_terms_starts_polls_and_cancels_both_providers() {
    let _guard = OAUTH_TEST_LOCK.lock().unwrap();
    let (supervisor, dir) = boot("oauth-client");
    for provider in [Provider::ChatGptCodex, Provider::Antigravity] {
        let mut client = OAuthClient::new(supervisor.port, "mgmt0123456789abcdef".into(), provider).with_callback_port(0);
        assert_eq!(client.begin().unwrap_err(), FailureClass::TermsRequired);
        client.session.acknowledge_terms();
        let url = client.begin().unwrap();
        assert!(url.starts_with("https://"));
        assert_eq!(client.begin().unwrap_err(), FailureClass::FlowStopped);
        assert_eq!(client.poll().unwrap(), AuthState::Authorizing);
        let serialized = serde_json::to_string(&client.session).unwrap();
        assert!(!serialized.contains("mock-codex"));
        assert!(!serialized.contains("mock-antigravity"));
        assert!(!serialized.contains("mgmt0123456789abcdef"));
        client.cancel().unwrap();
        assert_eq!(client.session.state, AuthState::LoggedOut);
        assert!(!client.session.live);
    }
    drop(supervisor);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[ignore = "requires verified native sidecar archive and executable"]
fn oauth_client_with_pinned_core_starts_and_cancels_without_login() {
    let _guard = OAUTH_TEST_LOCK.lock().unwrap();
    let archive = PathBuf::from(std::env::var("APP_AGG_CORE_ARCHIVE").expect("archive required"));
    let binary = PathBuf::from(std::env::var("APP_AGG_CORE_BIN").expect("binary required"));
    agg_core::pin::verify_native_binary(&archive, &binary).unwrap();
    let dir = std::env::temp_dir().join(format!("agg-oauth-check-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let supervisor = Supervisor::start(StartRequest {
        program: binary, work_dir: dir.clone(), port: None, extra_env: vec![],
        health_timeout: Duration::from_secs(45), gateway_key: "agglocal0123456789abcdef".into(),
        management_password: "mgmt0123456789abcdef".into(),
    }).unwrap();
    for provider in [Provider::ChatGptCodex, Provider::Antigravity] {
        let mut client = OAuthClient::new(supervisor.port, "mgmt0123456789abcdef".into(), provider).with_callback_port(0);
        client.session.acknowledge_terms();
        let _url = client.begin().expect("begin must return an allowed authorization URL");
        assert_eq!(client.poll().unwrap(), AuthState::Authorizing);
        client.cancel().expect("cancel must be confirmed by upstream");
        assert!(!client.session.live);
    }
    drop(supervisor);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn oauth_client_rejects_unexpected_host_and_stops_on_error() {
    let _guard = OAUTH_TEST_LOCK.lock().unwrap();
    let (supervisor, dir) = boot("healthy");
    let mut client = OAuthClient::new(supervisor.port, "mgmt0123456789abcdef".into(), Provider::ChatGptCodex).with_callback_port(0);
    client.session.acknowledge_terms();
    assert_eq!(client.begin().unwrap_err(), FailureClass::UpstreamRejected);
    assert_eq!(client.session.state, AuthState::Error);
    assert_eq!(client.begin().unwrap_err(), FailureClass::FlowStopped);
    drop(supervisor);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn oauth_upstream_success_requires_user_confirmation() {
    let _guard = OAUTH_TEST_LOCK.lock().unwrap();
    let (supervisor, dir) = boot("oauth-success");
    let mut client = OAuthClient::new(supervisor.port, "mgmt0123456789abcdef".into(), Provider::ChatGptCodex).with_callback_port(0);
    client.session.acknowledge_terms();
    let _url = client.begin().unwrap();
    assert_eq!(client.session.mark_user_present_authenticated(), Err(FailureClass::FlowStopped));
    assert_eq!(client.poll_user_present().unwrap(), AuthState::AwaitingConfirmation);
    assert!(!client.session.live);
    assert_eq!(client.session.user_present_marks, 0);
    drop(supervisor);
    let _ = std::fs::remove_dir_all(dir);
}

fn send_raw_http(port: u16, request: &str) -> String {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut resp = Vec::new();
    let _ = stream.read_to_end(&mut resp);
    String::from_utf8_lossy(&resp).into_owned()
}

#[test]
fn callback_does_not_report_success_when_core_rejects() {
    use agg_core::loopback_callback::LoopbackCallbackServer;
    use std::io::{Read, Write};
    let core = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let core_port = core.local_addr().unwrap().port();
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = core.accept().unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request);
        socket.write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    let (listener, port) = LoopbackCallbackServer::bind(0).unwrap();
    let _server = LoopbackCallbackServer::start(listener, Provider::ChatGptCodex, "test-state".into(), core_port);
    let response = send_raw_http(port, "GET /auth/callback?code=secret&state=test-state HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert!(response.starts_with("HTTP/1.1 502"));
    assert!(!response.contains("授權已安全接收"));
    assert!(!response.contains("secret"));
    worker.join().unwrap();
}

#[test]
fn callback_rejects_ambiguous_parameters_without_consuming_listener() {
    use agg_core::loopback_callback::LoopbackCallbackServer;
    use std::io::{Read, Write};
    let core = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let core_port = core.local_addr().unwrap().port();
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = core.accept().unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request);
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    let (listener, port) = LoopbackCallbackServer::bind(0).unwrap();
    let _server = LoopbackCallbackServer::start(listener, Provider::ChatGptCodex, "test-state".into(), core_port);
    for query in ["state=test-state", "state=test-state&state=test-state&code=x", "state=test-state&code=x&error=denied", "state=test-state&code="] {
        let request = format!("GET /auth/callback?{query} HTTP/1.1\r\nHost: localhost\r\n\r\n");
        assert!(send_raw_http(port, &request).starts_with("HTTP/1.1 400"));
    }
    let response = send_raw_http(port, "GET /auth/callback?state=test-state&code=valid HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert!(response.starts_with("HTTP/1.1 200"));
    worker.join().unwrap();
}

#[test]
fn oauth_loopback_callback_forwards_to_core_and_returns_clean_html() {
    let _guard = OAUTH_TEST_LOCK.lock().unwrap();
    let (supervisor, dir) = boot("oauth-client");
    let mut client = OAuthClient::new(supervisor.port, "mgmt0123456789abcdef".into(), Provider::ChatGptCodex).with_callback_port(0);
    client.session.acknowledge_terms();
    let _url = client.begin().unwrap();
    assert_ne!(client.callback_port(), 0);

    let resp = send_raw_http(
        client.callback_port(),
        "GET /auth/callback?code=mock-code-123&state=mock-codex HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(resp.starts_with("HTTP/1.1 200 OK"));
    assert!(resp.contains("授權已安全接收"));

    std::thread::sleep(Duration::from_millis(100));

    assert_eq!(client.poll_user_present().unwrap(), AuthState::AwaitingConfirmation);
    client.session.mark_user_present_authenticated().unwrap();
    assert_eq!(client.session.state, AuthState::Authenticated);
    assert!(client.session.live);

    drop(client);
    drop(supervisor);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn oauth_loopback_callback_for_antigravity() {
    let _guard = OAUTH_TEST_LOCK.lock().unwrap();
    let (supervisor, dir) = boot("oauth-client");
    let mut client = OAuthClient::new(supervisor.port, "mgmt0123456789abcdef".into(), Provider::Antigravity).with_callback_port(0);
    client.session.acknowledge_terms();
    let _url = client.begin().unwrap();
    assert_ne!(client.callback_port(), 0);

    let resp = send_raw_http(
        client.callback_port(),
        "GET /oauth-callback?code=mock-ag-code&state=mock-antigravity HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(resp.starts_with("HTTP/1.1 200 OK"));
    assert!(resp.contains("授權已安全接收"));

    std::thread::sleep(Duration::from_millis(100));

    assert_eq!(client.poll_user_present().unwrap(), AuthState::AwaitingConfirmation);
    client.session.mark_user_present_authenticated().unwrap();
    assert_eq!(client.session.state, AuthState::Authenticated);
    assert!(client.session.live);

    drop(client);
    drop(supervisor);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn oauth_loopback_callback_rejects_mismatched_state() {
    let _guard = OAUTH_TEST_LOCK.lock().unwrap();
    let (supervisor, dir) = boot("oauth-client");
    let mut client = OAuthClient::new(supervisor.port, "mgmt0123456789abcdef".into(), Provider::ChatGptCodex).with_callback_port(0);
    client.session.acknowledge_terms();
    let _url = client.begin().unwrap();

    let resp = send_raw_http(
        client.callback_port(),
        "GET /auth/callback?code=mock-code&state=WRONG-STATE HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(resp.starts_with("HTTP/1.1 400 Bad Request"));
    assert!(resp.contains("Invalid or mismatched OAuth state parameter"));

    client.cancel().unwrap();
    drop(supervisor);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn oauth_loopback_callback_handles_browser_error() {
    let _guard = OAUTH_TEST_LOCK.lock().unwrap();
    let (supervisor, dir) = boot("oauth-client");
    let mut client = OAuthClient::new(supervisor.port, "mgmt0123456789abcdef".into(), Provider::ChatGptCodex).with_callback_port(0);
    client.session.acknowledge_terms();
    let _url = client.begin().unwrap();

    let resp = send_raw_http(
        client.callback_port(),
        "GET /auth/callback?error=access_denied&state=mock-codex HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(resp.starts_with("HTTP/1.1 200 OK"));
    assert!(resp.contains("授權已取消或遭拒絕"));

    drop(client);
    drop(supervisor);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn oauth_loopback_detects_port_conflict() {
    let _guard = OAUTH_TEST_LOCK.lock().unwrap();
    let (supervisor, dir) = boot("oauth-client");
    let occupant = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let occupied_port = occupant.local_addr().unwrap().port();

    let mut client = OAuthClient::new(supervisor.port, "mgmt0123456789abcdef".into(), Provider::ChatGptCodex).with_callback_port(occupied_port);
    client.session.acknowledge_terms();
    let err = client.begin().unwrap_err();
    assert_eq!(err, FailureClass::PortConflict);
    assert_eq!(client.session.state, AuthState::Error);
    assert_eq!(client.session.failure, Some(FailureClass::PortConflict));

    drop(occupant);
    drop(supervisor);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn oauth_loopback_cancel_and_drop_cleanly_releases_port() {
    let _guard = OAUTH_TEST_LOCK.lock().unwrap();
    let (supervisor, dir) = boot("oauth-client");

    // Test cancel frees port
    {
        let mut client = OAuthClient::new(supervisor.port, "mgmt0123456789abcdef".into(), Provider::ChatGptCodex).with_callback_port(0);
        client.session.acknowledge_terms();
        let _url = client.begin().unwrap();
        let port = client.callback_port();
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_err());
        client.cancel().unwrap();
        let listener = std::net::TcpListener::bind(("127.0.0.1", port)).expect("port must be free after cancel");
        drop(listener);
    }

    // Test drop frees port
    {
        let mut client = OAuthClient::new(supervisor.port, "mgmt0123456789abcdef".into(), Provider::ChatGptCodex).with_callback_port(0);
        client.session.acknowledge_terms();
        let _url = client.begin().unwrap();
        let port = client.callback_port();
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_err());
        drop(client);
        let listener = std::net::TcpListener::bind(("127.0.0.1", port)).expect("port must be free after drop");
        drop(listener);
    }

    drop(supervisor);
    let _ = std::fs::remove_dir_all(dir);
}
