//! Thin window shell. It links Tauri only when this crate is built.
//! Headless checks use `agg-poc` and do not open a window.
//! The tray-icon feature is intentionally absent.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let report = agg_core::assess_platform();
    if std::env::var("APP_AGG_OPEN_UI").ok().as_deref() != Some("1") {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".into())
        );
        eprintln!("headless exit. APP_AGG_OPEN_UI=1 opens the Tauri window on a machine with a WebView.");
        return;
    }
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("tauri event loop");
}
