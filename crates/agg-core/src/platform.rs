//! Host observation and the secure-store decision.
//!
//! macOS Keychain, Windows Credential Manager, and Linux Secret Service are
//! design adapters. This crate never writes a plaintext stand-in for them.

use serde::Serialize;
use std::path::Path;
use std::process::Command;

pub const TERMS_NOTICE: &str = "請自行閱讀並遵守上游 CLIProxyAPI 與各服務商條款／公告。ChatGPT 訂閱不等於 OpenAI API 計費，也不表示任意 ChatGPT 模型可用。認證失敗或帳號／登入警告時停止該流程，不自動重試、不繞過。";

#[derive(Debug, Clone, Serialize)]
pub struct SecretAssessment {
    pub os_mechanism: String,
    pub availability: String,
    pub decision: String,
    pub upstream_sidecar_store: String,
    pub quiet_plaintext_fallback: bool,
    pub design_adapters: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlatformAssessment {
    pub os: String,
    pub arch: String,
    pub observed_distro: Option<String>,
    pub session_type: String,
    pub display_present: bool,
    pub wayland_display_set: bool,
    pub proposed_build_targets: Vec<String>,
    pub host_observation: String,
    pub webkitgtk_4_1: String,
    pub gtk3: String,
    pub ayatana_appindicator: String,
    pub tauri_shell: String,
    pub tray_verification: Vec<String>,
    pub secret_store: SecretAssessment,
    pub distro_baseline_proposal: String,
    pub minimum_os_note: String,
    pub desktop_client_note: String,
}

pub fn assess_platform() -> PlatformAssessment {
    let os = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    let observed_distro = read_os_release_pretty();
    let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unset".into());
    let display_present = std::env::var_os("DISPLAY").is_some();
    let wayland_display_set = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let webkitgtk_4_1 = pkg_exists("webkit2gtk-4.1");
    let gtk3 = pkg_exists("gtk+-3.0");
    let ayatana_appindicator = pkg_exists("ayatana-appindicator3-0.1");
    let host_observation = format!(
        "{os}/{arch} session={session_type} display={display_present} wayland={wayland_display_set} webkitgtk-4.1={webkitgtk_4_1}"
    );
    PlatformAssessment {
        os,
        arch,
        observed_distro,
        session_type,
        display_present,
        wayland_display_set,
        proposed_build_targets: vec![
            "windows-x64 (proposed, not tested here)".into(),
            "windows-arm64 (proposed if the pinned asset is used; not tested here)".into(),
            "macos-arm64 (proposed, not tested here)".into(),
            "macos-amd64 (proposed, not tested here)".into(),
            "linux-x86_64 (additional evaluation, not a release gate)".into(),
            "linux-aarch64 (additional evaluation asset exists; not run on this host)".into(),
        ],
        host_observation,
        webkitgtk_4_1,
        gtk3,
        ayatana_appindicator,
        tauri_shell: "source pinned to tauri 2.12.5 (Rust 1.90); not compiled in this environment".into(),
        tray_verification: vec![
            "macOS menu bar: not implemented and not tested".into(),
            "Windows system tray: not implemented and not tested".into(),
            "Linux tray icon was not enabled. Tauri's tray-icon feature is a separate crate feature and was not added.".into(),
            "Tauri tray docs (retrieved 2026-11-06) say Linux tray mouse events are unsupported even when an icon is shown and a context menu can still appear on right click.".into(),
            "Wayland versus X11 depends on a StatusNotifier watcher. This process does not claim either session works.".into(),
        ],
        secret_store: secret_assessment(),
        distro_baseline_proposal: "Proposed Linux evaluation baseline only: Ubuntu 24.04 LTS x86_64, because Tauri 2's Debian instructions use WebKitGTK 4.1 and this host observed Ubuntu 24.04. This is not a support promise for other distributions, libc variants, or architectures.".into(),
        minimum_os_note: "Product minimum OS versions are not established. Tauri's own prerequisite page (2026-08-20) names macOS Catalina 10.15 and Windows 7 plus WebView2; that is the toolkit note, not this product's tested support. CLIProxyAPI v8.0.57's Linux release text names a GLIBC 2.17 baseline for the default Linux archive.".into(),
        desktop_client_note: "ChatGPT Desktop / Codex GUI config path and reload behavior are unresolved on macOS, Windows, and Linux. OpenAI's config reference documents user config at ~/.codex/config.toml and uses $CODEX_HOME for profiles and logs. CLI success would not prove the desktop client.".into(),
    }
}

pub fn secret_assessment() -> SecretAssessment {
    SecretAssessment {
        os_mechanism: current_mechanism(),
        availability: current_availability(),
        decision: "refuse-no-quiet-plaintext-fallback".into(),
        upstream_sidecar_store: "CLIProxyAPI v8.0.57 local mode registers NewFileTokenStore(). The -h text has no keychain, Credential Manager, DPAPI, or Secret Service flag. Token material stays in JSON files under oauth.auth-dir. Encryption at rest is not documented. This PoC does not pretend that directory is a secure store.".into(),
        quiet_plaintext_fallback: false,
        design_adapters: vec![
            "macos: Keychain, design only, not implemented".into(),
            "windows: Credential Manager / DPAPI, design only, not implemented".into(),
            "linux: Secret Service (org.freedesktop.secrets), design only, not implemented".into(),
        ],
    }
}

pub fn store_secret(dir: &Path, _account: &str, _material: &str) -> Result<(), SecretAssessment> {
    let _ = dir;
    Err(secret_assessment())
}

fn current_mechanism() -> String {
    match std::env::consts::OS {
        "macos" => "macos-keychain".into(),
        "windows" => "windows-credential-manager-or-dpapi".into(),
        "linux" => "linux-secret-service".into(),
        other => format!("unsupported-{other}"),
    }
}

fn current_availability() -> String {
    match std::env::consts::OS {
        "linux" => linux_secret_service_availability(),
        "macos" => "not-probed-keychain-design-only".into(),
        "windows" => "not-probed-credential-manager-design-only".into(),
        _ => "unsupported".into(),
    }
}

fn linux_secret_service_availability() -> String {
    let session = std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some();
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").is_some();
    let secret_tool = Command::new("secret-tool")
        .arg("--version")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !session && !runtime {
        return "headless-absent-no-session-bus".into();
    }
    if !secret_tool {
        return "session-bus-or-runtime-present-secret-tool-absent-not-wired".into();
    }
    "secret-tool-present-not-wired".into()
}

fn pkg_exists(name: &str) -> String {
    match Command::new("pkg-config").arg("--exists").arg(name).status() {
        Ok(status) if status.success() => "present".into(),
        Ok(_) => "absent".into(),
        Err(_) => "pkg-config-unavailable".into(),
    }
}

fn read_os_release_pretty() -> Option<String> {
    let text = std::fs::read_to_string("/etc/os-release").ok()?;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_store_refuses_and_writes_nothing() {
        let dir = std::env::temp_dir().join(format!(
            "agg-secret-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let err = store_secret(&dir, "acct", "token-material-should-not-land").unwrap_err();
        assert_eq!(err.decision, "refuse-no-quiet-plaintext-fallback");
        assert!(!err.quiet_plaintext_fallback);
        assert!(std::fs::read_dir(&dir).unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(&dir);
        let encoded = serde_json::to_string(&assess_platform()).unwrap();
        assert!(!encoded.contains("token-material-should-not-land"));
    }
}
