//! Per-user lock for the desktop gateway. The lock file must not be deleted.
use fs2::FileExt;
use std::{fs::{File, OpenOptions}, io::Write, path::{Path, PathBuf}};

pub fn acquire(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|e| format!("open instance lock: {e}"))?;
    file.try_lock_exclusive().map_err(|e| format!("instance lock unavailable: {e}"))?;
    Ok(file)
}

/// Persist a guard before spawning. A leftover guard must not be removed automatically:
/// an orphaned gateway may still be running after its parent exits.
pub fn create_run_guard(path: &Path, runtime: &Path) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| format!("create run guard: {e}"))?;
    let record = serde_json::json!({"version": 1, "runtime": runtime});
    serde_json::to_writer(&mut file, &record).map_err(|e| format!("write run guard: {e}"))?;
    file.flush().map_err(|e| format!("flush run guard: {e}"))?;
    file.sync_all().map_err(|e| format!("sync run guard: {e}"))
}

/// Read only a well-formed runtime reference; old empty guards remain blocked.
pub fn guard_runtime(path: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read run guard: {e}"))?;
    if bytes.len() > 4096 { return Err("run guard is too large".into()); }
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| "legacy or invalid run guard")?;
    if value["version"] != 1 { return Err("unsupported run guard version".into()); }
    let runtime = PathBuf::from(value["runtime"].as_str().ok_or("missing runtime")?);
    let name = runtime.file_name().and_then(|v| v.to_str()).ok_or("invalid runtime name")?;
    let suffix = name.strip_prefix("app-agg-poc-").ok_or("invalid runtime name")?;
    if !runtime.is_absolute() || suffix.len() != 64 || !suffix.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid runtime path".into());
    }
    Ok(runtime)
}

/// Read a desktop guard only if it points to this application's runtime.
pub fn desktop_guard_runtime(path: &Path, expected: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read run guard: {e}"))?;
    if bytes.len() > 4096 { return Err("run guard is too large".into()); }
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| "invalid run guard")?;
    if value["version"] != 1 { return Err("unsupported run guard version".into()); }
    let runtime = PathBuf::from(value["runtime"].as_str().ok_or("missing runtime")?);
    if !runtime.is_absolute() || runtime != expected { return Err("unexpected runtime path".into()); }
    Ok(runtime)
}

/// 預設應用程式資料目錄（跨平台相容）
pub fn default_app_data_dir() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "無法取得使用者家目錄 (HOME / USERPROFILE 未設定)".to_string())?;
    let path = match std::env::consts::OS {
        "macos" => PathBuf::from(home).join("Library/Application Support/local.appagg001.poc"),
        "windows" => {
            if let Ok(appdata) = std::env::var("APPDATA") {
                PathBuf::from(appdata).join("local.appagg001.poc")
            } else {
                PathBuf::from(home).join("AppData/Roaming/local.appagg001.poc")
            }
        }
        _ => PathBuf::from(home).join(".local/share/local.appagg001.poc"),
    };
    Ok(path)
}

/// 確保本機存取金鑰與管理金鑰存在（0600 權限保存）
pub fn ensure_local_keys(dir: &Path) -> Result<(String, String), String> {
    let key_file = dir.join("local_keys.json");
    if key_file.exists() {
        if let Ok(content) = std::fs::read_to_string(&key_file) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                if let (Some(gw), Some(mgmt)) = (val["gateway_key"].as_str(), val["management_key"].as_str()) {
                    return Ok((gw.to_string(), mgmt.to_string()));
                }
            }
        }
    }
    let mut random = [0u8; 64];
    getrandom::getrandom(&mut random).map_err(|_| "無法產生本機密鑰")?;
    let gateway_key = format!("agglocal{}", crate::pin::sha256_bytes(&random[..32]));
    let management_key = format!("mgmt{}", crate::pin::sha256_bytes(&random[32..]));
    let val = serde_json::json!({
        "gateway_key": gateway_key,
        "management_key": management_key,
    });
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(&key_file, serde_json::to_string_pretty(&val).unwrap_or_default());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600));
    }
    Ok((gateway_key, management_key))
}

/// 尋找已釘選之 sidecar 核心二進位檔案
pub fn find_core_binary() -> Result<PathBuf, String> {
    if let Some(core_dir) = std::env::var_os("APP_AGG_CORE_DIR").map(PathBuf::from) {
        let binary = core_dir.join(if cfg!(windows) { "cli-proxy-api.exe" } else { "cli-proxy-api" });
        if binary.exists() {
            return Ok(binary);
        }
    }
    let candidates = [
        PathBuf::from("/Users/Zack/Documents/Codex/2026-11-06/model-bridge/.cache/cliproxyapi"),
        default_app_data_dir().unwrap_or_default().join("core"),
    ];
    for dir in &candidates {
        let binary = dir.join(if cfg!(windows) { "cli-proxy-api.exe" } else { "cli-proxy-api" });
        if binary.exists() {
            return Ok(binary);
        }
    }
    Err("未找到已釘選之 cli-proxy-api 執行檔；請設定 APP_AGG_CORE_DIR".into())
}
