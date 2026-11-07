use agg_core::instance;
use std::{process::Command, time::{SystemTime, UNIX_EPOCH}};

#[test]
fn instance_lock_across_processes() {
    if let Ok(path) = std::env::var("APP_AGG_TEST_LOCK") {
        assert!(instance::acquire(path.as_ref()).is_err(), "second process acquired a held lock");
        return;
    }
    let path = std::env::temp_dir().join(format!("agg-instance-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    let lock = instance::acquire(&path).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "instance_lock_across_processes"])
        .env("APP_AGG_TEST_LOCK", &path).status().unwrap();
    assert!(status.success());
    drop(lock);
    assert!(instance::acquire(&path).is_ok(), "released lock must be reusable");
    let _ = std::fs::remove_file(path);
}

#[test]
fn run_guard_survives_owner_exit_and_blocks_replacement() {
    let path = std::env::temp_dir().join(format!("agg-guard-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    let runtime = std::env::temp_dir().join(format!("app-agg-poc-{}", "a".repeat(64)));
    instance::create_run_guard(&path, &runtime).unwrap();
    assert_eq!(instance::guard_runtime(&path).unwrap(), runtime);
    assert!(instance::create_run_guard(&path, &runtime).is_err());
    std::fs::remove_file(&path).unwrap();
    instance::create_run_guard(&path, &runtime).unwrap();
    std::fs::remove_file(path).unwrap();
}

#[test]
fn legacy_and_malformed_guards_stay_untrusted() {
    let path = std::env::temp_dir().join(format!("agg-legacy-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::write(&path, b"").unwrap();
    assert!(instance::guard_runtime(&path).is_err());
    std::fs::write(&path, br#"{"version":1,"runtime":"/tmp/untrusted"}"#).unwrap();
    assert!(instance::guard_runtime(&path).is_err());
    assert!(path.exists());
    std::fs::remove_file(path).unwrap();
}
