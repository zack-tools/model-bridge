use agg_core::redact::Redactor;
use agg_core::supervisor::{ProcessHealth, StartRequest, Supervisor};
use std::path::PathBuf;
use std::time::Duration;

fn work_dir(label: &str) -> PathBuf {
    let mut nonce = [0u8; 8];
    getrandom::getrandom(&mut nonce).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "agg-life-{label}-{}",
        agg_core::pin::sha256_bytes(&nonce)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn start(dir: &std::path::Path, mode: &str, timeout: Duration) -> Result<Supervisor, String> {
    Supervisor::start(StartRequest {
        program: PathBuf::from(env!("CARGO_BIN_EXE_agg-mock-core")),
        work_dir: dir.to_path_buf(),
        port: None,
        extra_env: vec![("APP_AGG_MOCK_MODE".into(), mode.into())],
        health_timeout: timeout,
        gateway_key: "agglocal0123456789abcdef".into(),
        management_password: "mgmt0123456789abcdef".into(),
    })
}

#[test]
fn crash_is_reported_without_the_leaked_token() {
    let dir = work_dir("crash");
    let err = start(&dir, "crash", Duration::from_secs(2)).unwrap_err();
    assert!(!err.contains("leakthisvalue"), "{err}");
    assert!(err.contains("<redacted>") || err.contains("exited"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn healthy_logs_are_redacted_and_second_start_is_blocked() {
    let dir = work_dir("lock");
    let mut first = start(&dir, "leak", Duration::from_secs(3)).unwrap();
    let logs = first.redacted_logs();
    assert!(!logs.contains("leakthisvalue"), "{logs}");
    assert!(!logs.contains("leakrefreshvalue"), "{logs}");
    let second = start(&dir, "healthy", Duration::from_millis(800));
    assert!(second.is_err(), "second instance should not start");
    let message = second.err().unwrap_or_default();
    assert!(
        message.contains("single instance") || message.contains("port conflict"),
        "{message}"
    );
    first.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nohealth_times_out_and_does_not_stay_up() {
    let dir = work_dir("nohealth");
    let err = start(&dir, "nohealth", Duration::from_millis(900)).unwrap_err();
    let redacted = Redactor::new(Vec::<String>::new()).redact(&err);
    assert!(redacted.contains("health timed out") || redacted.contains("exited"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failed_start_reports_confirmed_child_absence() {
    let dir1 = work_dir("failure-state-1");
    let request1 = |program: PathBuf, mode: &str| StartRequest {
        program, work_dir: dir1.clone(), port: None,
        extra_env: vec![("APP_AGG_MOCK_MODE".into(), mode.into())],
        health_timeout: Duration::from_millis(800),
        gateway_key: "agglocal0123456789abcdef".into(),
        management_password: "mgmt0123456789abcdef".into(),
    };
    let missing = Supervisor::start_detailed(request1(dir1.join("missing"), "healthy")).unwrap_err();
    assert!(missing.child_absent);
    let _ = std::fs::remove_dir_all(&dir1);

    let dir2 = work_dir("failure-state-2");
    let request2 = |program: PathBuf, mode: &str| StartRequest {
        program, work_dir: dir2.clone(), port: None,
        extra_env: vec![("APP_AGG_MOCK_MODE".into(), mode.into())],
        health_timeout: Duration::from_millis(800),
        gateway_key: "agglocal0123456789abcdef".into(),
        management_password: "mgmt0123456789abcdef".into(),
    };
    let timeout = Supervisor::start_detailed(request2(PathBuf::from(env!("CARGO_BIN_EXE_agg-mock-core")), "nohealth")).unwrap_err();
    assert!(timeout.child_absent, "mock child termination must be confirmed");
    assert!(timeout.message.contains("health timed out"), "expected timeout message, got: {}", timeout.message);
    let _ = std::fs::remove_dir_all(&dir2);
}

#[test]
fn detects_exit_after_initial_health() {
    let dir = work_dir("late-exit");
    let mut supervisor = start(&dir, "exit-after-health", Duration::from_secs(3)).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        if supervisor.health().unwrap() == ProcessHealth::Exited { break; }
        assert!(std::time::Instant::now() < deadline, "child did not exit");
        std::thread::sleep(Duration::from_millis(20));
    }
    supervisor.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn detached_supervisor_leaves_child_running() {
    let dir = work_dir("detach");
    let mut supervisor = start(&dir, "healthy", Duration::from_secs(3)).unwrap();
    let pid = supervisor.pid().unwrap().to_string();
    supervisor.detach();
    drop(supervisor);
    let alive = std::process::Command::new("/bin/kill").args(["-0", &pid]).status().unwrap().success();
    let killed = std::process::Command::new("/bin/kill").args(["-KILL", &pid]).status().unwrap().success();
    assert!(alive && killed, "detached child must remain alive until explicitly stopped");
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn detached_gateway_can_be_reattached_and_stopped() {
    let dir = work_dir("reattach");
    let mut original = start(&dir, "healthy", Duration::from_secs(3)).unwrap();
    let pid = original.pid().unwrap();
    let port = original.port;
    original.detach();
    drop(original);
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_agg-mock-core"));
    let mut attached = Supervisor::attach(&binary, &dir, pid, "agglocal0123456789abcdef".into(), "mgmt0123456789abcdef".into()).unwrap();
    assert_eq!(attached.port, port);
    assert_eq!(attached.health().unwrap(), ProcessHealth::Healthy);
    attached.stop_checked().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
#[ignore = "requires the pinned native core archive and executable"]
fn native_core_can_be_reattached_after_it_hashes_management_key() {
    let archive = PathBuf::from(std::env::var("APP_AGG_CORE_ARCHIVE").expect("archive required"));
    let binary = PathBuf::from(std::env::var("APP_AGG_CORE_BIN").expect("binary required"));
    agg_core::pin::verify_native_binary(&archive, &binary).unwrap();
    let dir = work_dir("native-reattach");
    let gateway_key = "agglocal0123456789abcdef";
    let management_key = "mgmt0123456789abcdef";
    let mut original = Supervisor::start(StartRequest {
        program: binary.clone(), work_dir: dir.clone(), port: None, extra_env: vec![],
        health_timeout: Duration::from_secs(45), gateway_key: gateway_key.into(), management_password: management_key.into(),
    }).unwrap();
    let pid = original.pid().unwrap();
    let port = original.port;
    let config = std::fs::read_to_string(dir.join("config.yaml")).unwrap();
    assert_ne!(agg_core::corecfg::parse_managed_config(&config).unwrap().management_key, management_key);
    original.detach();
    drop(original);
    let mut attached = Supervisor::attach(&binary, &dir, pid, gateway_key.into(), management_key.into()).unwrap();
    assert_eq!(attached.port, port);
    assert_eq!(attached.health().unwrap(), ProcessHealth::Healthy);
    attached.stop_checked().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
