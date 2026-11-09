#![cfg(unix)]

use std::fs;
use std::os::unix::fs::FileTypeExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn scratch(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("agg-auth-{label}-{}-{nanos}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn wait_finish(child: &mut std::process::Child, what: &str) -> (std::process::ExitStatus, String) {
    let mut stderr = child.stderr.take().unwrap();
    let started = Instant::now();
    let status = loop {
        if started.elapsed() > Duration::from_secs(3) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{what} blocked; it opened the auth fifo");
        }
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    let mut err = String::new();
    use std::io::Read;
    stderr.read_to_string(&mut err).unwrap();
    (status, err)
}

#[test]
fn agg_poc_apply_does_not_open_auth_json() {
    let dir = scratch("apply");
    let config = dir.join("config.toml");
    fs::write(&config, "personality = \"keep\"\n").unwrap();
    let auth = dir.join("auth.json");
    assert!(Command::new("mkfifo")
        .arg(&auth)
        .status()
        .unwrap()
        .success());
    let data = dir.join("data");
    fs::create_dir_all(&data).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_agg-poc"))
        .args([
            "apply",
            "--config",
            config.to_str().unwrap(),
            "--base-url",
            "http://127.0.0.1:18431/v1",
            "--key",
            "localgateway012345",
        ])
        .env("HOME", &dir)
        .env("CODEX_HOME", &dir)
        .env("APP_AGG_DATA_DIR", &data)
        .env_remove("APP_AGG_REQUIRE_ISOLATION")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (status, err) = wait_finish(&mut child, "agg-poc apply");
    assert!(status.success(), "{err}");
    assert!(fs::symlink_metadata(&auth).unwrap().file_type().is_fifo());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn agg_poc_start_refuses_tampered_core_without_opening_auth_json() {
    let dir = scratch("start");
    let auth = dir.join("auth.json");
    assert!(Command::new("mkfifo")
        .arg(&auth)
        .status()
        .unwrap()
        .success());
    let core = dir.join("core");
    let data = dir.join("data");
    fs::create_dir_all(&core).unwrap();
    fs::create_dir_all(&data).unwrap();
    let marker = dir.join("spawned");
    let binary = core.join("cli-proxy-api");
    fs::write(&binary, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
    let mut perms = fs::metadata(&binary).unwrap().permissions();
    use std::os::unix::fs::PermissionsExt;
    perms.set_mode(0o755);
    fs::set_permissions(&binary, perms).unwrap();
    let archive_name = agg_core::pin::native_archive_name().unwrap();
    fs::write(core.join(archive_name), b"not-a-pinned-archive").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_agg-poc"))
        .arg("start")
        .env("HOME", &dir)
        .env("CODEX_HOME", &dir)
        .env("APP_AGG_DATA_DIR", &data)
        .env("APP_AGG_CORE_DIR", &core)
        .env_remove("APP_AGG_REQUIRE_ISOLATION")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (status, _err) = wait_finish(&mut child, "agg-poc start");
    assert!(!status.success());
    assert!(fs::symlink_metadata(&auth).unwrap().file_type().is_fifo());
    assert!(!marker.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn oauth_begin_exits_without_file_token_acceptance() {
    for provider in ["codex", "antigravity"] {
        let output = Command::new(env!("CARGO_BIN_EXE_agg-poc"))
            .args(["oauth-begin", "--provider", provider, "--open"])
            .env_remove("APP_AGG_REQUIRE_ISOLATION")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{provider}");
        let err = String::from_utf8_lossy(&output.stderr);
        assert!(err.contains("--accept-file-tokens"), "{err}");
        assert!(err.contains("NewFileTokenStore"), "{err}");
        assert!(err.contains("No login was started"), "{err}");
    }
}

#[test]
fn accepting_file_tokens_without_open_does_not_spawn_the_core() {
    let dir = scratch("oauth");
    let core = dir.join("core");
    fs::create_dir_all(&core).unwrap();
    let marker = dir.join("spawned");
    let binary = core.join("cli-proxy-api");
    fs::write(&binary, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_agg-poc"))
        .args(["oauth-begin", "--provider", "codex", "--accept-file-tokens"])
        .env("APP_AGG_CORE_DIR", &core)
        .env("APP_AGG_DATA_DIR", dir.join("data"))
        .env("HOME", &dir)
        .env("CODEX_HOME", dir.join("codex"))
        .env_remove("APP_AGG_REQUIRE_ISOLATION")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("未連線"));
    assert!(!marker.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn desktop_shell_does_not_name_the_codex_auth_file() {
    let src = include_str!("../../../apps/desktop/src-tauri/src/main.rs");
    assert!(!src.contains("auth.json"));
    assert!(!src.contains("sync_chatgpt_desktop_auth"));
    assert!(src.contains("SidecarLaunch::DesktopGateway"));
    assert!(src.contains("SidecarLaunch::HeadlessDaemon"));
    assert!(src.contains("after_gateway_start"));
    assert!(src.contains("after_daemon_start"));
    assert!(src.contains("preview_chatgpt_desktop_auth"));
    assert!(src.contains("apply_chatgpt_desktop_auth"));
}

#[test]
fn handoff_script_refuses_a_real_codex_home() {
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/macos-arm64-handoff.sh");
    let root = scratch("handoff");
    let real = root.join("real-home");
    let handoff = root.join(".handoff");
    for name in ["home", "codex-home", "app-data", "core"] {
        fs::create_dir_all(handoff.join(name)).unwrap();
    }
    fs::create_dir_all(real.join(".codex")).unwrap();
    let ok = Command::new("bash")
        .arg("-c")
        .arg(format!(
            "source '{}'; REAL_HOME='{}'; HANDOFF_ROOT='{}'; HOME='{}'; CODEX_HOME='{}'; APP_AGG_DATA_DIR='{}'; APP_AGG_CORE_DIR='{}'; assert_isolation",
            script.display(),
            real.display(),
            handoff.display(),
            handoff.join("home").display(),
            handoff.join("codex-home").display(),
            handoff.join("app-data").display(),
            handoff.join("core").display(),
        ))
        .status()
        .unwrap();
    assert!(ok.success());
    let bad = Command::new("bash")
        .arg("-c")
        .arg(format!(
            "source '{}'; REAL_HOME='{}'; HANDOFF_ROOT='{}'; HOME='{}'; CODEX_HOME='{}'; APP_AGG_DATA_DIR='{}'; APP_AGG_CORE_DIR='{}'; assert_isolation",
            script.display(),
            real.display(),
            handoff.display(),
            handoff.join("home").display(),
            real.join(".codex").display(),
            handoff.join("app-data").display(),
            handoff.join("core").display(),
        ))
        .status()
        .unwrap();
    assert!(!bad.success());
    let _ = fs::remove_dir_all(&root);
}
