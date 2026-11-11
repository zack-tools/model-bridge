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

/// Data directory for this process. `APP_AGG_DATA_DIR` wins so a handoff can
/// keep runtime files inside the working copy.
pub fn app_data_dir() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os("APP_AGG_DATA_DIR") {
        if value.is_empty() {
            return Err("APP_AGG_DATA_DIR is empty".into());
        }
        return Ok(PathBuf::from(value));
    }
    default_app_data_dir()
}

pub fn core_binary_name() -> &'static str {
    if cfg!(windows) { "cli-proxy-api.exe" } else { "cli-proxy-api" }
}

pub fn archive_beside(binary: &Path) -> Result<PathBuf, String> {
    let dir = binary.parent().ok_or("core binary has no parent directory")?;
    Ok(dir.join(crate::pin::native_archive_name()?))
}

/// Paths the macOS handoff must keep inside the working copy.
#[derive(Clone, Copy)]
pub struct IsolationPaths<'a> {
    pub root: &'a Path,
    pub home: &'a Path,
    pub codex_home: &'a Path,
    pub data_dir: &'a Path,
    pub core_dir: &'a Path,
    pub real_home: &'a Path,
}

pub fn isolation_violation(paths: IsolationPaths<'_>) -> Option<String> {
    if paths.real_home.as_os_str().is_empty() || paths.root.as_os_str().is_empty() {
        return Some("isolation is not in effect: real home or handoff root is empty".into());
    }
    let forbidden = [
        paths.real_home.join(".codex"),
        paths.real_home.join(".cli-proxy-api"),
        paths.real_home.join("Library/Application Support/io.github.zacktools.modelbridge"),
    ];
    let labeled = [
        ("HOME", paths.home),
        ("CODEX_HOME", paths.codex_home),
        ("APP_AGG_DATA_DIR", paths.data_dir),
        ("APP_AGG_CORE_DIR", paths.core_dir),
    ];
    for (name, path) in labeled {
        if path.as_os_str().is_empty() {
            return Some(format!("isolation is not in effect: {name} is empty"));
        }
        if path.components().any(|component| matches!(component, std::path::Component::ParentDir)) && !path.exists() {
            return Some(format!("isolation is not in effect: {name} contains '..'"));
        }
        if !is_within(paths.root, path) {
            return Some(format!("isolation is not in effect: {name} is outside the handoff root"));
        }
        for bad in &forbidden {
            if path == bad || path.starts_with(bad) {
                return Some(format!("isolation is not in effect: {name} points at a real user path"));
            }
        }
    }
    if paths.home == paths.real_home {
        return Some("isolation is not in effect: HOME is the real home".into());
    }
    let real_codex = paths.real_home.join(".codex");
    if paths.codex_home == real_codex {
        return Some("isolation is not in effect: CODEX_HOME is the real ~/.codex".into());
    }
    None
}

/// When `APP_AGG_REQUIRE_ISOLATION=1`, refuse to continue unless every
/// handoff path is inside `APP_AGG_HANDOFF_ROOT` and away from the real home.
pub fn required_isolation_error() -> Option<String> {
    if std::env::var("APP_AGG_REQUIRE_ISOLATION").ok().as_deref() != Some("1") {
        return None;
    }
    let read = |key: &str| -> Result<PathBuf, String> {
        std::env::var(key).map(PathBuf::from).map_err(|_| format!("isolation is not in effect: {key} is unset"))
    };
    let root = match read("APP_AGG_HANDOFF_ROOT") { Ok(path) => path, Err(err) => return Some(err) };
    let home = match read("HOME") { Ok(path) => path, Err(err) => return Some(err) };
    let codex_home = match read("CODEX_HOME") { Ok(path) => path, Err(err) => return Some(err) };
    let data_dir = match read("APP_AGG_DATA_DIR") { Ok(path) => path, Err(err) => return Some(err) };
    let core_dir = match read("APP_AGG_CORE_DIR") { Ok(path) => path, Err(err) => return Some(err) };
    let real_home = match read("APP_AGG_REAL_HOME") { Ok(path) => path, Err(err) => return Some(err) };
    isolation_violation(IsolationPaths { root: &root, home: &home, codex_home: &codex_home, data_dir: &data_dir, core_dir: &core_dir, real_home: &real_home })
}

fn is_within(root: &Path, path: &Path) -> bool {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    path.starts_with(&root)
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
pub fn read_local_keys(dir: &Path) -> Result<(String, String), String> {
    let content = std::fs::read_to_string(dir.join("local_keys.json"))
        .map_err(|_| "無法讀取既有本機金鑰".to_string())?;
    parse_local_keys(&content)
}

fn parse_local_keys(content: &str) -> Result<(String, String), String> {
    let val: serde_json::Value = serde_json::from_str(content)
        .map_err(|_| "本機金鑰檔格式錯誤；保留原檔".to_string())?;
    let valid = |key: &str| key.len() >= 16 && key.bytes().all(|b| b.is_ascii_alphanumeric());
    let gw = val["gateway_key"].as_str().filter(|s| valid(s)).ok_or("閘道金鑰無效；保留原檔")?;
    let mgmt = val["management_key"].as_str().filter(|s| valid(s)).ok_or("管理金鑰無效；保留原檔")?;
    Ok((gw.to_string(), mgmt.to_string()))
}

pub fn ensure_local_keys(dir: &Path) -> Result<(String, String), String> {
    let key_file = dir.join("local_keys.json");
    match std::fs::read_to_string(&key_file) {
        Ok(content) => return parse_local_keys(&content),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(format!("無法讀取本機金鑰；保留原檔：{err}")),
    }
    let mut random = [0u8; 64];
    getrandom::getrandom(&mut random).map_err(|_| "無法產生本機密鑰")?;
    let gateway_key = format!("agglocal{}", crate::pin::sha256_bytes(&random[..32]));
    let management_key = format!("mgmt{}", crate::pin::sha256_bytes(&random[32..]));
    let val = serde_json::json!({
        "gateway_key": gateway_key,
        "management_key": management_key,
    });
    std::fs::create_dir_all(dir).map_err(|e| format!("無法建立金鑰目錄：{e}"))?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&key_file).map_err(|e| format!("無法建立本機金鑰檔：{e}"))?;
    let content = serde_json::to_vec_pretty(&val).map_err(|e| e.to_string())?;
    file.write_all(&content).and_then(|_| file.sync_all())
        .map_err(|e| format!("無法保存本機金鑰；請檢查金鑰檔：{e}"))?;
    Ok((gateway_key, management_key))
}

/// 尋找已釘選之 sidecar 核心二進位檔案。
/// 只看 `APP_AGG_CORE_DIR`，其次是應用資料目錄下的 `core`。
pub fn find_core_binary() -> Result<PathBuf, String> {
    let name = core_binary_name();
    if let Some(core_dir) = std::env::var_os("APP_AGG_CORE_DIR").map(PathBuf::from) {
        let binary = core_dir.join(name);
        if binary.is_file() {
            return Ok(binary);
        }
    }
    if let Ok(dir) = app_data_dir() {
        let binary = dir.join("core").join(name);
        if binary.is_file() {
            return Ok(binary);
        }
    }
    Err("未找到已釘選之 cli-proxy-api 執行檔；請設定 APP_AGG_CORE_DIR".into())
}
