#![cfg(unix)]

use agg_core::pin::native_archive_name;
use agg_core::supervisor::{SidecarLaunch, SpawnVerifiedError, StartRequest};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn scratch(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("agg-launch-{label}-{}-{nanos}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn tampered_core(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let marker = dir.join("spawned");
    let binary = dir.join(if cfg!(windows) {
        "cli-proxy-api.exe"
    } else {
        "cli-proxy-api"
    });
    let script = format!("#!/bin/sh\ntouch {}\n", marker.display());
    fs::write(&binary, script).unwrap();
    let mut perms = fs::metadata(&binary).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&binary, perms).unwrap();
    let archive = dir.join(native_archive_name().unwrap());
    let packed = dir.join("pack");
    fs::create_dir_all(&packed).unwrap();
    fs::copy(&binary, packed.join("cli-proxy-api")).unwrap();
    let status = Command::new("tar")
        .args(["-czf"])
        .arg(&archive)
        .arg("-C")
        .arg(&packed)
        .arg("cli-proxy-api")
        .status()
        .unwrap();
    assert!(status.success());
    (archive, binary, marker)
}

fn request(dir: &Path, binary: PathBuf) -> StartRequest {
    StartRequest {
        program: binary,
        work_dir: dir.join("work"),
        port: None,
        extra_env: Vec::new(),
        health_timeout: Duration::from_secs(2),
        gateway_key: "localgateway012345".into(),
        management_password: "mgmtsecret012345".into(),
    }
}

#[test]
fn every_launch_path_refuses_a_tampered_binary_before_spawn() {
    for path in [
        SidecarLaunch::DesktopGateway,
        SidecarLaunch::HeadlessDaemon,
        SidecarLaunch::PocStart,
        SidecarLaunch::CoreCheck,
    ] {
        let dir = scratch(&format!("{path:?}"));
        let (archive, binary, marker) = tampered_core(&dir);
        let err = path.spawn(&archive, request(&dir, binary)).unwrap_err();
        assert!(
            matches!(err, SpawnVerifiedError::Pin(_)),
            "{path:?} spawned or failed after pin: {err:?}"
        );
        assert!(err.child_absent(), "{path:?}");
        let message = err.message();
        assert!(
            message.contains("checksum")
                || message.contains("pinned")
                || message.contains("mismatch")
                || message.contains("binary does not match"),
            "{path:?}: {message}"
        );
        assert!(!marker.exists(), "{path:?} executed the tampered binary");
        let _ = fs::remove_dir_all(&dir);
    }
}
