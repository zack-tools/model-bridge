use agg_core::check::{run_pinned_core_check, sanitize_report};
use std::path::PathBuf;

#[test]
#[ignore = "pinned CLIProxyAPI linux/amd64 process; not a unit or native-UI result"]
fn pinned_linux_core() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let archive = std::env::var("APP_AGG_CORE_ARCHIVE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            root.join(".cache/cliproxyapi/CLIProxyAPI_8.0.57_linux_amd64.tar.gz")
        });
    let binary = std::env::var("APP_AGG_CORE_BIN").map(PathBuf::from).unwrap_or_else(|_| {
        root.join(".cache/cliproxyapi/cli-proxy-api")
    });
    let report = run_pinned_core_check(&archive, &binary).unwrap_or_else(|err| {
        panic!("core check failed before a report: {err}")
    });
    let body = sanitize_report(&report, &[]);
    let evidence = PathBuf::from("/opt/cursor/artifacts/phase1-core-check.json");
    let _ = std::fs::create_dir_all("/opt/cursor/artifacts");
    std::fs::write(&evidence, &body).unwrap();
    assert!(
        !body.contains("agglocal") && !body.contains("access_token"),
        "evidence leaked credential material"
    );
    assert_eq!(report.oauth_live_pass, "unverified");
    assert_eq!(report.native_ui_pass, "unverified");
    assert_eq!(report.user_present_marks, 0);
    assert!(!report.wildcard_listener);
    assert_eq!(
        report.core_integration_pass, "pass",
        "integration report:\n{body}"
    );
}
