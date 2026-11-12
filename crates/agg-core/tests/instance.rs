use agg_core::instance;
use std::{process::Command, time::{SystemTime, UNIX_EPOCH}};

#[cfg(windows)]
#[test]
fn adoption_rejects_unrelated_process_and_preserves_guard() {
    struct Unrelated;
    impl agg_core::supervisor::AdoptedProcess for Unrelated {
        fn pid(&self) -> u32 { std::process::id() }
        fn image(&self) -> Result<std::path::PathBuf, String> { std::env::current_exe().map_err(|e| e.to_string()) }
        fn alive(&self) -> Result<bool, String> { Ok(true) }
        fn stop(&self) -> Result<(), String> { panic!("unrelated process must not be stopped") }
        fn verify(&self, _: u16, _: &std::path::Path, _: &std::path::Path) -> Result<(), String> { panic!("unrelated binary must be rejected first") }
    }
    let dir = std::env::temp_dir().join(format!("agg-adopt-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    let runtime = dir.join("runtime");
    std::fs::create_dir_all(&runtime).unwrap();
    let guard = dir.join("gateway.running");
    instance::create_run_guard(&guard, &runtime).unwrap();
    assert!(agg_core::supervisor::Supervisor::adopt_desktop(&dir, Box::new(Unrelated), 50833, "testgatewaykey123", "testmanagement123").is_err());
    assert!(guard.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn local_keys_must_be_saved_and_invalid_files_are_preserved() {
    let dir = std::env::temp_dir().join(format!("agg-keys-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&dir).unwrap();
    let keys = instance::ensure_local_keys(&dir).unwrap();
    assert_eq!(instance::ensure_local_keys(&dir).unwrap(), keys);
    assert_eq!(instance::read_local_keys(&dir).unwrap(), keys);
    let path = dir.join("local_keys.json");
    std::fs::write(&path, "invalid").unwrap();
    assert!(instance::ensure_local_keys(&dir).is_err());
    assert!(instance::read_local_keys(&dir).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "invalid");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(instance::ensure_local_keys(&dir).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

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

#[test]
fn core_lookup_has_no_hardcoded_developer_path() {
    let src = include_str!("../src/instance.rs");
    assert!(!src.contains("/Users/Zack"));
    assert!(!src.contains("Documents/Codex"));
}

#[test]
fn isolation_accepts_only_paths_inside_the_handoff_root() {
    use agg_core::instance::{isolation_violation, IsolationPaths};
    let root = std::env::temp_dir().join(format!("agg-iso-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    let real = root.join("real");
    let handoff = root.join("handoff");
    for name in ["home", "codex-home", "app-data", "core"] {
        std::fs::create_dir_all(handoff.join(name)).unwrap();
    }
    std::fs::create_dir_all(real.join(".codex")).unwrap();
    std::fs::create_dir_all(real.join(".cli-proxy-api")).unwrap();
    let home = handoff.join("home");
    let codex_home = handoff.join("codex-home");
    let data_dir = handoff.join("app-data");
    let core_dir = handoff.join("core");
    let real_codex = real.join(".codex");
    let ok = IsolationPaths {
        root: &handoff,
        home: &home,
        codex_home: &codex_home,
        data_dir: &data_dir,
        core_dir: &core_dir,
        real_home: &real,
    };
    assert!(isolation_violation(ok).is_none());
    let outside = IsolationPaths { codex_home: &real_codex, ..ok };
    assert!(isolation_violation(outside).unwrap().contains("CODEX_HOME"));
    let real_home_used = IsolationPaths { home: &real, ..ok };
    assert!(isolation_violation(real_home_used).unwrap().contains("HOME"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn desktop_guard_accepts_only_the_expected_runtime() {
    let dir = std::env::temp_dir().join(format!("agg-desktop-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&dir).unwrap();
    let guard = dir.join("gateway.running");
    let runtime = dir.join("runtime");
    instance::create_run_guard(&guard, &runtime).unwrap();
    assert_eq!(instance::desktop_guard_runtime(&guard, &runtime).unwrap(), runtime);
    assert!(instance::desktop_guard_runtime(&guard, &dir.join("other")).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn keys_remain_stable_and_invalid_file_is_not_rotated() {
    let dir = std::env::temp_dir().join(format!("agg-keys-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    let first = instance::ensure_local_keys(&dir).unwrap();
    assert_eq!(instance::ensure_local_keys(&dir).unwrap(), first);
    let path = dir.join("local_keys.json");
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    std::fs::write(&path, "invalid").unwrap();
    assert!(instance::ensure_local_keys(&dir).is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "invalid");
    std::fs::remove_dir_all(dir).unwrap();
}
