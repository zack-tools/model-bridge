//! Private PoC desktop shell. The gateway runs only on explicit user request.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use agg_core::{attribution, instance, pin, registry::SourceKind, oauth::{AuthState, FailureClass, Provider}, oauth_client::OAuthClient, supervisor::{ProcessHealth, StartRequest, Supervisor}};
use std::{path::PathBuf, sync::{Arc, Mutex}, time::Duration};
use tauri::{Manager, State};

mod account_renewal;

struct AccountFlow { client: OAuthClient, renewal: Option<account_renewal::Renewal> }

struct Gateway {
    process: Option<Arc<Mutex<Supervisor>>>,
    port: Option<u16>,
    epoch: u64,
    lifecycle_busy: bool,
    account_busy: [bool; 2],
    runtime: Option<PathBuf>,
    instance_lock: Option<std::fs::File>,
    run_guard: Option<PathBuf>,
    management_key: Option<String>,
    gateway_key: Option<String>,
    codex: Option<AccountFlow>,
    antigravity: Option<AccountFlow>,
    verified_codex: Option<String>,
    verified_antigravity: Option<String>,
    retain_on_exit: bool,
}
impl Default for Gateway {
    fn default() -> Self {
        Self { process: None, port: None, epoch: 0, lifecycle_busy: false, account_busy: [false; 2], runtime: None, instance_lock: None, run_guard: None,
            management_key: None, gateway_key: None, codex: None, antigravity: None,
            verified_codex: None, verified_antigravity: None,
            retain_on_exit: true }
    }
}
impl Drop for Gateway {
    fn drop(&mut self) {
        if !self.retain_on_exit {
            let _ = self.stop();
        } else if let Some(process) = self.process.as_mut() {
            if let Ok(mut process) = process.lock() { process.detach(); }
        }
    }
}
impl Gateway {
    fn stop(&mut self) -> Result<(), String> {
        if let Some(child) = self.process.as_mut() {
            child.lock().map_err(|_| "閘道程序狀態無法讀取")?.stop_checked().map_err(|_| "無法確認閘道已停止；保留啟動保護".to_string())?;
        }
        self.process.take();
        self.port = None;
        self.epoch = self.epoch.wrapping_add(1);
        self.account_busy = [false; 2];
        self.codex.take();
        self.antigravity.take();
        self.verified_codex.take();
        self.verified_antigravity.take();
        self.management_key.take();
        self.gateway_key.take();
        // 依使用者需求：登入憑證存放在本機，關閉閘道不刪除憑證（保留 runtime/auth）
        if let Some(path) = self.run_guard.as_ref() {
            match std::fs::remove_file(path) {
                Ok(()) => {},
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                Err(_) => return Err("閘道已停止，但啟動保護尚未清除；請重試停止".into()),
            }
            self.run_guard.take();
        }
        self.instance_lock.take();
        Ok(())
    }
}
type Shared = Arc<Mutex<Gateway>>;

#[derive(Clone)]
struct GatewaySnapshot { epoch: u64, port: u16, management_key: String, gateway_key: String, runtime: PathBuf }
impl GatewaySnapshot {
    fn read(state: &Shared) -> Result<Self, String> {
        let gw = state.lock().map_err(|_| "閘道狀態無法讀取")?;
        if gw.lifecycle_busy { return Err("閘道正在啟動或停止，請稍後重試".into()); }
        Ok(Self { epoch: gw.epoch, port: gw.port.ok_or("請先啟動閘道")?,
            management_key: gw.management_key.clone().ok_or("閘道管理金鑰不可用")?,
            gateway_key: gw.gateway_key.clone().ok_or("閘道存取金鑰不可用")?,
            runtime: gw.runtime.clone().ok_or("閘道目錄不可用")? })
    }
    fn check(&self, state: &Shared) -> Result<(), String> {
        let gw = state.lock().map_err(|_| "閘道狀態無法讀取")?;
        if gw.epoch != self.epoch || gw.port != Some(self.port) || gw.lifecycle_busy {
            return Err("閘道已變更，舊操作結果已捨棄；請重試".into());
        }
        Ok(())
    }
}

fn source_kind(provider: &str) -> Result<SourceKind, String> {
    match provider { "codex" => Ok(SourceKind::ChatGptCodex), "antigravity" => Ok(SourceKind::Antigravity), _ => Err("不支援的帳號來源".into()) }
}

fn catalog_path(config: &std::path::Path, custom: Option<String>) -> PathBuf {
    custom.filter(|s| !s.trim().is_empty()).map(PathBuf::from).unwrap_or_else(|| config.with_file_name("model_catalog.json"))
}


#[tauri::command]
fn auto_start_enabled() -> bool {
    std::env::var("APP_AGG_AUTO_START").ok().as_deref() == Some("1")
}

fn desktop_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os("APP_AGG_DATA_DIR") {
        if value.is_empty() {
            return Err("APP_AGG_DATA_DIR is empty".into());
        }
        return Ok(PathBuf::from(value));
    }
    app.path().app_data_dir().map_err(|_| "無法取得應用程式資料目錄".to_string())
}

fn codex_config_path(custom_path: Option<String>) -> Result<PathBuf, String> {
    if let Some(path) = custom_path.filter(|s| !s.trim().is_empty()) { return Ok(PathBuf::from(path)); }
    let codex_home = std::env::var("CODEX_HOME").ok();
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
    agg_core::codex::detect_codex_home(codex_home.as_deref(), home.as_deref(), std::env::consts::OS)
        .map(|info| info.cli_config_path).map_err(|e| e.to_string())
}

#[tauri::command]
async fn codex_suspend(custom_path: Option<String>) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = codex_config_path(custom_path)?;
        agg_core::codex::suspend_config(&path).map_err(|e| e.to_string())
    }).await.map_err(|_| "切換原生設定失敗".to_string())?
}

#[tauri::command]
async fn codex_resume(state: State<'_, Shared>, custom_path: Option<String>) -> Result<bool, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = GatewaySnapshot::read(&state)?;
        let path = codex_config_path(custom_path)?;
        snapshot.check(&state)?;
        agg_core::codex::resume_config(&path, snapshot.port, &snapshot.gateway_key).map_err(|e| e.to_string())
    }).await.map_err(|_| "恢復閘道設定失敗".to_string())?
}

fn ensure_local_keys(dir: &std::path::Path) -> Result<(String, String), String> {
    agg_core::instance::ensure_local_keys(dir)
}

#[tauri::command]
async fn start_gateway(app: tauri::AppHandle, state: State<'_, Shared>) -> Result<u16, String> {
    let lock_dir = desktop_data_dir(&app)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let epoch = {
            let mut gw = state.lock().map_err(|_| "閘道狀態無法讀取")?;
            if gw.process.is_some() || gw.lifecycle_busy { return Err("閘道已啟動或正在切換".into()); }
            gw.epoch = gw.epoch.wrapping_add(1);
            gw.lifecycle_busy = true;
            gw.epoch
        };
        let mut gateway = Gateway::default();
        gateway.retain_on_exit = false;
        let result = (|| -> Result<u16, String> {
        std::fs::create_dir_all(&lock_dir).map_err(|_| "無法建立鎖定目錄")?;
        let instance_lock = instance::acquire(&lock_dir.join("gateway.lock"))
            .map_err(|_| "另一個視窗已啟動閘道，或無法取得鎖定".to_string())?;
        let core_dir = std::env::var_os("APP_AGG_CORE_DIR").map(PathBuf::from).or_else(|| std::env::current_exe().ok().and_then(|p| { let mut d = p.parent()?.to_path_buf(); for _ in 0..6 { let c = d.join(".cache").join("cliproxyapi"); if c.join(if cfg!(windows) { "cli-proxy-api.exe" } else { "cli-proxy-api" }).exists() { return Some(c); } d = d.parent()?.to_path_buf(); } None }))
            .ok_or("請先設定 APP_AGG_CORE_DIR，指向已下載的核心資料夾")?;
        let binary = core_dir.join(if cfg!(windows) { "cli-proxy-api.exe" } else { "cli-proxy-api" });
        let archive = core_dir.join(pin::native_archive_name()?);
        
        let (gateway_key, management_key) = ensure_local_keys(&lock_dir)?;
        let dir = lock_dir.join("runtime");
        let auth_dir = dir.join("auth");
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)] { use std::os::unix::fs::DirBuilderExt; builder.mode(0o700); }
        let _ = builder.create(&dir);
        let _ = builder.create(&auth_dir);
        let guard_path = lock_dir.join("gateway.running");
        if instance::create_run_guard(&guard_path, &dir).is_err() {
            return Err("偵測到上次啟動紀錄或無法建立保護；請先檢查舊閘道".into());
        }
        let preferred_port = std::env::var("APP_AGG_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(50833);
        let selected_port = if agg_core::supervisor::ensure_port_free(preferred_port).is_ok() {
            Some(preferred_port)
        } else {
            None
        };
        let result = agg_core::supervisor::SidecarLaunch::DesktopGateway.spawn(&archive, StartRequest {
            program: binary, work_dir: dir.clone(), port: selected_port, extra_env: vec![],
            health_timeout: Duration::from_secs(45), gateway_key: gateway_key.clone(), management_password: management_key.clone(),
        });
        match result {
            Ok(spawned) => {
                let process = spawned.supervisor;
                let port = process.port;
                let child_pid = process.pid();
                gateway.runtime = Some(dir);
                gateway.process = Some(Arc::new(Mutex::new(process)));
                gateway.port = Some(port);
                gateway.instance_lock = Some(instance_lock);
                gateway.run_guard = Some(guard_path);
                gateway.management_key = Some(management_key.clone());
                gateway.gateway_key = Some(gateway_key);

                if let Some(pid) = child_pid {
                    let _ = std::fs::write(lock_dir.join("gateway.pid"), pid.to_string());
                }
                let codex_home = std::env::var_os("CODEX_HOME").map(PathBuf::from).or_else(|| {
                    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
                        .map(|home| PathBuf::from(home).join(".codex"))
                });
                if let Some(home) = codex_home {
                    let _ = agg_core::codex::after_gateway_start(&home);
                }
                Ok(port)
            }
            Err(failure) => {
                if failure.child_absent() { let _ = std::fs::remove_file(&guard_path); }
                Err("閘道啟動失敗；請先檢查核心日誌".into())
            }
        }
        })();
        let mut current = state.lock().map_err(|_| "閘道狀態無法讀取")?;
        current.lifecycle_busy = false;
        if current.epoch != epoch { return Err("啟動已取消".into()); }
        if result.is_ok() {
            gateway.epoch = epoch;
            gateway.retain_on_exit = current.retain_on_exit;
            std::mem::swap(&mut *current, &mut gateway);
        }
        result
    }).await.map_err(|_| "啟動工作失敗".to_string())?
}

#[tauri::command]
async fn stop_gateway(app: tauri::AppHandle, state: State<'_, Shared>) -> Result<(), String> {
    let lock_dir = desktop_data_dir(&app)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        stop_shared_gateway(&state)?;
        let _ = std::fs::remove_file(lock_dir.join("gateway.pid"));
        Ok(())
    }).await.map_err(|_| "停止工作失敗".to_string())?
}

fn stop_shared_gateway(state: &Shared) -> Result<(), String> {
    let mut owned = {
        let mut gw = state.lock().map_err(|_| "閘道狀態無法讀取")?;
        if gw.lifecycle_busy {
            gw.epoch = gw.epoch.wrapping_add(1);
            return Err("已要求取消目前啟動；請等候切換完成後檢查狀態".into());
        }
        if gw.process.is_none() && gw.run_guard.is_none() { return Err("沒有此視窗可控制的閘道".into()); }
        let mut replacement = Gateway::default();
        replacement.epoch = gw.epoch.wrapping_add(1);
        replacement.lifecycle_busy = true;
        replacement.retain_on_exit = gw.retain_on_exit;
        std::mem::replace(&mut *gw, replacement)
    };
    let result = owned.stop();
    let mut gw = state.lock().map_err(|_| "閘道狀態無法讀取")?;
    if result.is_err() {
        owned.epoch = gw.epoch;
        owned.retain_on_exit = gw.retain_on_exit;
        std::mem::swap(&mut *gw, &mut owned);
    }
    gw.lifecycle_busy = false;
    result
}

#[tauri::command]
async fn set_retain_on_exit(state: State<'_, Shared>, retain: bool) -> Result<(), String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut gw = state.lock().map_err(|_| "閘道狀態無法讀取".to_string())?;
        gw.retain_on_exit = retain;
        Ok(())
    }).await.map_err(|_| "設定保留策略失敗".to_string())?
}

#[cfg(windows)]
fn read_autostart_gateway() -> Result<bool, String> {
    use std::os::windows::process::CommandExt;
    let output = std::process::Command::new("powershell.exe").creation_flags(0x08000000)
        .args(["-NoProfile", "-NonInteractive", "-Command", r#"$ErrorActionPreference='Stop'; try { $p='HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'; if ((Test-Path -LiteralPath $p) -and ((Get-Item -LiteralPath $p).GetValueNames() -contains 'ModelBridgeGateway')) { 'enabled' } else { 'disabled' } } catch { exit 2 }"#])
        .output().map_err(|_| "無法讀取開機自啟設定")?;
    if !output.status.success() { return Err("無法讀取開機自啟設定；原設定未確認".into()); }
    match String::from_utf8_lossy(&output.stdout).trim() {
        "enabled" => Ok(true), "disabled" => Ok(false), _ => Err("開機自啟設定回應無效".into())
    }
}

#[tauri::command]
async fn get_autostart_gateway() -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(|| {
        #[cfg(windows)] { read_autostart_gateway() }
        #[cfg(not(windows))] { Ok(false) }
    }).await.map_err(|_| "查詢開機自啟設定失敗".to_string())?
}

#[tauri::command]
async fn set_autostart_gateway(enabled: bool) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            if enabled {
                let exe = std::env::current_exe().map_err(|e| format!("無法取得執行檔路徑：{e}"))?;
                let exe_str = exe.to_str().ok_or("執行檔路徑無效")?;
                let cmd_val = format!("\"{}\" --daemon", exe_str);
                let output = std::process::Command::new("reg")
                    .creation_flags(CREATE_NO_WINDOW)
                    .args(["add", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run", "/v", "ModelBridgeGateway", "/t", "REG_SZ", "/d", &cmd_val, "/f"])
                    .output()
                    .map_err(|e| format!("寫入開機自啟設定失敗：{e}"))?;
                if !output.status.success() {
                    let err = String::from_utf8_lossy(&output.stderr);
                    return Err(format!("註冊自啟失敗：{err}"));
                }
            } else {
                if !read_autostart_gateway()? { return Ok(false); }
                let output = std::process::Command::new("reg")
                    .creation_flags(CREATE_NO_WINDOW)
                    .args(["delete", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run", "/v", "ModelBridgeGateway", "/f"])
                    .output().map_err(|_| "取消開機自啟失敗")?;
                if !output.status.success() { return Err("取消開機自啟失敗；原設定可能仍有效".into()); }
            }
            let observed = read_autostart_gateway()?;
            if observed != enabled { return Err("開機自啟設定未生效".into()); }
            Ok(observed)
        }
        #[cfg(not(windows))]
        Ok(false)
    }).await.map_err(|_| "設定開機自啟失敗".to_string())?
}

#[tauri::command]
async fn gateway_status(app: tauri::AppHandle, state: State<'_, Shared>) -> Result<serde_json::Value, String> {
    let dir = desktop_data_dir(&app)?;
    let guard_path = dir.join("gateway.running");
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (process, epoch) = {
            let gateway = state.lock().map_err(|_| "閘道狀態無法讀取")?;
            if gateway.lifecycle_busy { return Ok(serde_json::json!({"state":"transitioning"})); }
            if gateway.process.is_none() && gateway.run_guard.is_some() {
                return Ok(serde_json::json!({"state": "cleanup_pending"}));
            }
            (gateway.process.clone(), gateway.epoch)
        };
        if let Some(process) = process {
            let (health, port) = {
                let mut process = process.lock().map_err(|_| "閘道程序狀態無法讀取")?;
                (process.health().map_err(|_| "無法檢查閘道狀態")?, process.port)
            };
            let gw = state.lock().map_err(|_| "閘道狀態無法讀取")?;
            if gw.epoch != epoch || gw.lifecycle_busy { return Ok(serde_json::json!({"state":"transitioning"})); }
            drop(gw);
            return match health {
                ProcessHealth::Healthy => Ok(serde_json::json!({"state":"healthy","port":port})),
                ProcessHealth::Degraded => Ok(serde_json::json!({"state":"degraded","port":port})),
                ProcessHealth::Exited => { stop_shared_gateway(&state)?; Ok(serde_json::json!({"state":"exited"})) }
            };
        }
        if guard_path.exists() {
            #[cfg(windows)]
            if let Ok((mut process, instance_lock, gateway_key, management_key)) = adopt_previous_gateway(&dir) {
                let mut gateway = state.lock().map_err(|_| "閘道狀態無法讀取")?;
                if gateway.epoch != epoch || gateway.lifecycle_busy || gateway.process.is_some() {
                    process.detach();
                    return Ok(serde_json::json!({"state":"transitioning"}));
                }
                let port = process.port;
                gateway.process = Some(Arc::new(Mutex::new(process)));
                gateway.port = Some(port);
                gateway.epoch = gateway.epoch.wrapping_add(1);
                gateway.instance_lock = Some(instance_lock);
                gateway.run_guard = Some(guard_path.clone());
                gateway.runtime = Some(dir.join("runtime"));
                gateway.gateway_key = Some(gateway_key);
                gateway.management_key = Some(management_key);
                return Ok(serde_json::json!({"state": "healthy", "port": port, "adopted": true}));
            }
            let pid_file = dir.join("gateway.pid");
            let running_pid = std::fs::read_to_string(&pid_file)
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok());

            let is_alive = if let Some(pid) = running_pid {
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    const CREATE_NO_WINDOW: u32 = 0x08000000;
                    let check_cmd = format!("Get-Process -Id {pid} -ErrorAction SilentlyContinue | Where-Object {{ $_.ProcessName -like '*cli-proxy-api*' }} | Select-Object -ExpandProperty Id");
                    std::process::Command::new("powershell")
                        .creation_flags(CREATE_NO_WINDOW)
                        .args(["-NoProfile", "-Command", &check_cmd])
                        .output()
                        .map(|out| !String::from_utf8_lossy(&out.stdout).trim().is_empty())
                        .unwrap_or(false)
                }
                #[cfg(not(windows))]
                {
                    std::process::Command::new("kill").args(["-0", &pid.to_string()]).status().map(|s| s.success()).unwrap_or(false)
                }
            } else {
                false
            };

            if is_alive {
                let config_file = dir.join("runtime").join("config.yaml");
                let port = std::fs::read_to_string(&config_file).ok().and_then(|content| {
                    for line in content.lines() {
                        let l = line.trim();
                        if l.starts_with("port:") {
                            return l.split(':').nth(1)?.trim().parse::<u16>().ok();
                        }
                    }
                    None
                }).unwrap_or(50833);

                // 若該埠號確實正在被監聽，表示核心健康運行中！
                if agg_core::supervisor::ensure_port_free(port).is_err() {
                    return Ok(serde_json::json!({"state": "detached", "port": port}));
                }
            }
            return Ok(serde_json::json!({"state": "needs_recovery"}));
        }

        Ok(serde_json::json!({"state": "stopped"}))
    }).await.map_err(|_| "狀態檢查失敗".to_string())?
}

#[cfg(windows)]
fn adopt_previous_gateway(dir: &std::path::Path) -> Result<(Supervisor, std::fs::File, String, String), String> {
    let lock = instance::acquire(&dir.join("gateway.lock"))?;
    let pid = std::fs::read_to_string(dir.join("gateway.pid")).map_err(|_| "無法讀取背景程序紀錄")?
        .trim().parse::<u32>().map_err(|_| "背景程序紀錄無效")?;
    let config = std::fs::read_to_string(dir.join("runtime").join("config.yaml")).map_err(|_| "無法讀取背景閘道設定")?;
    let ports: Vec<u16> = config.lines().filter_map(|line| line.strip_prefix("  port: ")?.trim().parse().ok()).collect();
    if ports.len() != 1 || ports[0] == 0 { return Err("背景閘道埠號不明".into()); }
    let (gateway_key, management_key) = instance::read_local_keys(dir)?;
    let handle = windows_adoption::Process::open(pid)?;
    let process = Supervisor::adopt_desktop(dir, Box::new(handle), ports[0], &gateway_key, &management_key)?;
    Ok((process, lock, gateway_key, management_key))
}

fn account_slot<'a>(gateway: &'a mut Gateway, provider: &str) -> Result<&'a mut Option<AccountFlow>, String> {
    match provider { "codex" => Ok(&mut gateway.codex), "antigravity" => Ok(&mut gateway.antigravity), _ => Err("不支援的帳號來源".into()) }
}

fn verified_slot<'a>(gateway: &'a mut Gateway, provider: &str) -> Result<&'a mut Option<String>, String> {
    match provider { "codex" => Ok(&mut gateway.verified_codex), "antigravity" => Ok(&mut gateway.verified_antigravity), _ => Err("不支援的帳號來源".into()) }
}

struct AccountWork {
    state: Shared, provider: String, snapshot: GatewaySnapshot, flow: Option<AccountFlow>, verified: Option<String>, index: usize,
}
impl AccountWork {
    fn take(state: &Shared, provider: &str) -> Result<Self, String> {
        source_kind(provider)?;
        let snapshot = GatewaySnapshot::read(state)?;
        let index = if provider == "codex" { 0 } else { 1 };
        let mut gw = state.lock().map_err(|_| "閘道狀態無法讀取")?;
        if gw.epoch != snapshot.epoch || gw.lifecycle_busy { return Err("閘道已變更，請重試".into()); }
        if gw.account_busy[index] { return Err("此帳號有操作進行中，請稍後重試".into()); }
        gw.account_busy[index] = true;
        let flow = account_slot(&mut gw, provider)?.take();
        let verified = verified_slot(&mut gw, provider)?.clone();
        Ok(Self { state: state.clone(), provider: provider.into(), snapshot, flow, verified, index })
    }
}
impl Drop for AccountWork {
    fn drop(&mut self) {
        if let Ok(mut gw) = self.state.lock() {
            if gw.epoch == self.snapshot.epoch && !gw.lifecycle_busy {
                if let Ok(slot) = account_slot(&mut gw, &self.provider) { *slot = self.flow.take(); }
                if let Ok(slot) = verified_slot(&mut gw, &self.provider) { *slot = self.verified.take(); }
                gw.account_busy[self.index] = false;
            }
        }
    }
}

fn account_result(status: &str, message: &str, has_credential: bool, can_cancel: bool) -> serde_json::Value {
    let active = matches!(status, "authorizing" | "awaitingConfirmation");
    serde_json::json!({"state": status, "message": message, "hasCredential":has_credential,
        "canStart": !active && !has_credential, "canRenew": !active && has_credential,
        "canConfirm":status=="awaitingConfirmation", "canCancel":can_cancel})
}

fn core_binary() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("APP_AGG_CORE_DIR") { return Ok(PathBuf::from(path).join(instance::core_binary_name())); }
    let mut dir = std::env::current_exe().map_err(|_| "無法取得程式路徑")?.parent().ok_or("無法取得程式目錄")?.to_path_buf();
    for _ in 0..6 {
        let path = dir.join(".cache").join("cliproxyapi").join(instance::core_binary_name());
        if path.is_file() { return Ok(path); }
        match dir.parent() { Some(parent) => dir=parent.to_path_buf(), None => break }
    }
    Err("請設定 APP_AGG_CORE_DIR 指向已釘選核心".into())
}

#[cfg(target_os = "windows")]
fn open_system_browser(url: &str) -> bool {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(hwnd: *mut std::ffi::c_void, operation: *const u16, file: *const u16,
            parameters: *const u16, directory: *const u16, show: i32) -> isize;
    }
    let operation: Vec<u16> = std::ffi::OsStr::new("open").encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = std::ffi::OsStr::new(url).encode_wide().chain(Some(0)).collect();
    // ShellExecuteW opens the validated URL directly, without a command shell.
    unsafe { ShellExecuteW(std::ptr::null_mut(), operation.as_ptr(), target.as_ptr(),
        std::ptr::null(), std::ptr::null(), 1) > 32 }
}

#[cfg(target_os = "macos")]
fn open_system_browser(url: &str) -> bool {
    std::process::Command::new("/usr/bin/open").arg(url)
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn open_system_browser(_url: &str) -> bool { false }

#[tauri::command]
async fn oauth_start(state: State<'_, Shared>, provider: String, terms: bool, renew: Option<bool>) -> Result<serde_json::Value, String> {
    if !terms { return Err("請先確認登入提示".into()); }
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut work = AccountWork::take(&state, &provider)?;
        if work.flow.as_ref().is_some_and(|f| matches!(f.client.session.state, AuthState::Authorizing | AuthState::AwaitingConfirmation)) {
            return Err("此帳號仍在授權中；請先完成或取消".into());
        }
        let snapshot = work.snapshot.clone();
        let source = source_kind(&provider)?;
        let existing = attribution::has_provider_credential(snapshot.port, &snapshot.management_key, source)?;
        let renewing = renew.unwrap_or(false);
        if existing != renewing {
            return Err(if existing { "此類型已有憑證，請使用重新授權；暫不開放新增同類帳號" } else { "此類型沒有既有憑證，請使用新增帳號" }.into());
        }
        let kind = if provider=="codex" { Provider::ChatGptCodex } else { Provider::Antigravity };
        work.flow.take();
        work.verified = None;
        let renewal = if renewing { Some(account_renewal::Renewal::start(&snapshot.runtime, &core_binary()?, kind)?) } else { None };
        let (port,key) = renewal.as_ref().map(|r| (r.port(),r.management_key().to_owned())).unwrap_or((snapshot.port,snapshot.management_key.clone()));
        let mut client = OAuthClient::new(port,key,kind);
        client.session.acknowledge_terms();
        snapshot.check(&state)?;
        let url = client.begin().map_err(|failure| match failure {
            FailureClass::PortConflict => format!("登入回呼連接埠 {} 被佔用，請關閉佔用程式後重試",client.callback_port()),
            _ => "授權啟動失敗；既有憑證未變更，可重新開始".into(),
        })?;
        work.flow = Some(AccountFlow { client, renewal });
        snapshot.check(&state)?;
        if !open_system_browser(&url) {
            let flow = work.flow.as_mut().unwrap();
            if flow.renewal.is_some() || flow.client.cancel().is_ok() { work.flow.take(); }
            return Err("無法開啟系統瀏覽器；請檢查狀態或取消後重試".into());
        }
        Ok(account_result("authorizing", if renewing { "已開啟重新授權；請登入原帳號。完成確認前保留舊憑證" } else { "已要求系統開啟瀏覽器；完成後按檢查狀態" },existing,true))
    }).await.map_err(|_| "授權工作失敗".to_string())?
}

#[tauri::command]
async fn oauth_action(state: State<'_, Shared>, provider: String, action: String) -> Result<serde_json::Value, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut work = AccountWork::take(&state, &provider)?;
        let snapshot = work.snapshot.clone();
        let source = source_kind(&provider)?;
        let existing = attribution::has_provider_credential(snapshot.port, &snapshot.management_key, source)?;
        snapshot.check(&state)?;
        if action=="cancel" {
            let flow = work.flow.as_mut().ok_or("目前沒有需要取消的授權流程")?;
            if flow.renewal.is_none() { flow.client.cancel().map_err(|_| "取消未獲上游確認，請稍後重試")?; }
            work.flow.take();
            work.verified = None;
            return Ok(account_result(if existing {"present"} else {"absent"},"授權流程已清除；既有憑證保留",existing,false));
        }
        if action=="confirm" {
            let flow = work.flow.as_mut().ok_or("目前沒有待確認授權")?;
            if flow.client.session.state != AuthState::AwaitingConfirmation { return Err("尚未收到上游成功回報".into()); }
            if let Some(renewal) = flow.renewal.as_mut() { renewal.commit()?; }
            flow.client.session.mark_user_present_authenticated().map_err(|_| "授權狀態已變更")?;
            flow.renewal.take();
            snapshot.check(&state)?;
            return Ok(account_result("authenticated","已確認授權完成並保存憑證；模型可用性仍需查詢或測試",true,false));
        }
        if action!="poll" { return Err("不支援的操作".into()); }
        if let Some(flow) = work.flow.as_mut() {
            let status = flow.client.poll_user_present().unwrap_or(AuthState::Error);
            snapshot.check(&state)?;
            if status != AuthState::Authenticated {
                let (name,message)=match status {
                    AuthState::Authorizing => ("authorizing","等待瀏覽器授權"),
                    AuthState::AwaitingConfirmation => ("awaitingConfirmation","上游回報完成；請確認是你本人剛完成登入"),
                    AuthState::Expired => ("expired","授權已逾時；可清除流程後重新授權"),
                    _ => ("error","授權失敗或遭拒絕；可清除流程後重新開始"),
                };
                return Ok(account_result(name,message,existing,true));
            }
        }
        if let Some(previous) = work.verified.as_ref() {
            if attribution::single_active_index(snapshot.port,&snapshot.management_key,source).ok().as_ref()==Some(previous) {
                snapshot.check(&state)?;
                return Ok(account_result("verified","本次模型測試通過，追蹤標頭符合該憑證；不保證後續持續有效",existing,false));
            }
            work.verified=None;
        }
        snapshot.check(&state)?;
        Ok(if existing { account_result("present","已偵測到本機憑證；有效性尚未確認，可查詢模型或重新授權",true,false) }
           else { account_result("absent","尚無本機憑證；請選擇帳號類型開始授權",false,false) })
    }).await.map_err(|_| "授權工作失敗".to_string())?
}

#[tauri::command]
async fn discovered_models(state: State<'_, Shared>, provider: String) -> Result<Vec<String>, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = GatewaySnapshot::read(&state)?;
        let port = snapshot.port;
        let mgmt_key = snapshot.management_key.clone();
        let source = match provider.as_str() {
            "codex" => SourceKind::ChatGptCodex,
            "antigravity" => SourceKind::Antigravity,
            _ => return Err("不支援的帳號來源".into()),
        };
        let index = attribution::single_active_index(port, &mgmt_key, source).map_err(|e| format!("無法確定唯一可用帳號：{e}"))?;
        let (observed_source, models) = attribution::discover_models(port, &mgmt_key, &index).map_err(|e| format!("無法取得帳號模型清單：{e}"))?;
        if source != observed_source { return Err("帳號來源已變更；請檢查本機憑證後重試".into()); }
        snapshot.check(&state)?;
        Ok(models)
    }).await.map_err(|_| "模型查詢失敗".to_string())?
}

#[tauri::command]
async fn verify_model_roundtrip(state: State<'_, Shared>, provider: String, model: String, quota_confirmed: bool) -> Result<String, String> {
    if !quota_confirmed {
        return Err("未取得模型用量同意；真實模型回合已取消，未消耗額度".into());
    }
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut work = AccountWork::take(&state, &provider)?;
        let snapshot = work.snapshot.clone();
        let port = snapshot.port;
        let mgmt_key = snapshot.management_key.clone();
        let gw_key = snapshot.gateway_key.clone();
        let source = match provider.as_str() {
            "codex" => SourceKind::ChatGptCodex,
            "antigravity" => SourceKind::Antigravity,
            _ => return Err("不支援的帳號來源".into()),
        };
        work.verified = None;
        let index = attribution::single_active_index(port, &mgmt_key, source)
            .map_err(|e| format!("無法確定唯一可用帳號：{e}"))?;
        let (observed_source, models) = attribution::discover_models(port, &mgmt_key, &index)
            .map_err(|e| format!("無法取得帳號模型清單：{e}"))?;
        if source != observed_source {
            return Err("帳號來源已變更；請檢查本機憑證後重試".into());
        }
        if !models.iter().any(|m| m == &model) {
            return Err(format!("所選模型 {model} 不在該帳號之可用清單中"));
        }

        let request = serde_json::json!({
            "model": model,
            "stream": true,
            "input": [
                {"role": "user", "content": "Please verify this session by calling tool check."}
            ],
            "tools": [
                {
                    "type": "function",
                    "name": "check",
                    "description": "Session verification probe tool",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "status": {"type": "string", "description": "Verification probe status"}
                        }
                    }
                }
            ]
        });

        snapshot.check(&state)?;
        let result = agg_core::response_evidence::inspect_real_model_tool_roundtrip(
            port,
            &gw_key,
            &request,
            &model,
            Duration::from_secs(45),
            "check",
            &index,
            || snapshot.check(&state),
        )?;
        if attribution::single_active_index(port, &mgmt_key, source)? != index {
            return Err("驗證期間憑證已變更，無法確認結果".into());
        }
        snapshot.check(&state)?;
        work.verified = Some(index);
        Ok(result)
    }).await.map_err(|_| "驗證工作失敗".to_string())?
}

#[tauri::command]
async fn inspect_previous_gateway(app: tauri::AppHandle) -> Result<String, String> {
    let dir = desktop_data_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let guard = dir.join("gateway.running");
        if !guard.exists() { return Ok("沒有上次啟動保護紀錄。".to_string()); }
        let runtime = instance::desktop_guard_runtime(&guard, &dir.join("runtime")).map_err(|_| "保護紀錄缺少可辨識的目錄；需人工檢查，保護仍保留。".to_string())?;
        #[cfg(target_os = "macos")]
        {
            let output = std::process::Command::new("/bin/ps").args(["-axo", "command="])
                .output().map_err(|_| "無法檢查程序；保護仍保留".to_string())?;
            if !output.status.success() { return Err("程序檢查失敗；保護仍保留".into()); }
            let config = runtime.join("config.yaml");
            let config = config.to_str().ok_or("路徑無法顯示")?;
            let found = String::from_utf8_lossy(&output.stdout).lines()
                .any(|line| line.contains("cli-proxy-api") && line.contains(config));
            let observation = if found { "發現命令列包含此目錄的核心程序。" } else { "未在程序命令列中找到此目錄；這不能證明舊程序已停止。" };
            return Ok(format!("{observation} 執行目錄：{}。保護仍保留，請勿直接刪除。", runtime.display()));
        }
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            let config = runtime.join("config.yaml");
            let config = config.to_str().ok_or("路徑無法顯示")?;
            let output = std::process::Command::new("powershell")
                .creation_flags(CREATE_NO_WINDOW)
                .args(["-NoProfile", "-Command", "Get-CimInstance Win32_Process | Where-Object { $_.Name -like '*cli-proxy-api*' } | ForEach-Object { $_.CommandLine }"])
                .output().map_err(|_| "無法檢查程序；保護仍保留".to_string())?;
            let found = if output.status.success() {
                String::from_utf8_lossy(&output.stdout).lines()
                    .any(|line| line.contains("cli-proxy-api") && line.contains(config))
            } else {
                false
            };
            let observation = if found { "發現命令列包含此目錄的核心程序。" } else { "未在程序命令列中找到此目錄；這不能證明舊程序已停止。" };
            return Ok(format!("{observation} 執行目錄：{}。保護仍保留，請確認後清除。", runtime.display()));
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        Ok(format!("執行目錄：{}。此平台的程序檢查尚未實作；保護仍保留。", runtime.display()))
    }).await.map_err(|_| "檢查工作失敗".to_string())?
}

// Caller holds gateway.lock throughout inspection and removal.
fn remove_stale_guard(dir: &std::path::Path) -> Result<(), String> {
    let guard = dir.join("gateway.running");
    if !guard.exists() { return Ok(()); }
    let runtime = instance::desktop_guard_runtime(&guard, &dir.join("runtime"))?;
    let _runtime_lock = instance::acquire(&runtime.join("instance.lock"))?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let output = std::process::Command::new("powershell.exe").creation_flags(0x08000000)
            .env("MB_CONFIG", runtime.join("config.yaml"))
            .args(["-NoProfile", "-NonInteractive", "-Command", r#"$ErrorActionPreference='Stop'; try { $all=@(Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object { $_.Name -like '*cli-proxy-api*' }); foreach ($p in $all) { if (-not $p.CommandLine -or $p.CommandLine.IndexOf($env:MB_CONFIG,[StringComparison]::OrdinalIgnoreCase) -ge 0) { exit 3 } }; 'absent' } catch { exit 2 }"#])
            .output().map_err(|_| "無法檢查舊核心；保護紀錄仍保留")?;
        if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim() != "absent" {
            return Err("舊核心可能仍在執行或無法檢查；保護紀錄仍保留".into());
        }
    }
    #[cfg(target_os="macos")]
    {
        let output = std::process::Command::new("/bin/ps").args(["-axo", "command="]).output().map_err(|_| "無法檢查舊核心")?;
        if !output.status.success() || String::from_utf8_lossy(&output.stdout).lines().any(|line| line.contains("cli-proxy-api") && line.contains(&runtime.join("config.yaml").to_string_lossy().to_string())) {
            return Err("舊核心可能仍在執行；保護紀錄仍保留".into());
        }
    }
    #[cfg(not(any(windows,target_os="macos")))]
    return Err("此平台尚無法自動確認舊核心已停止".into());
    #[cfg(any(windows,target_os="macos"))]
    {
        match std::fs::read_to_string(runtime.join("config.yaml")) {
            Ok(config) => {
                let ports: Vec<u16> = config.lines().filter_map(|l| l.trim().strip_prefix("port:")?.trim().parse().ok()).collect();
                if ports.len()!=1 || ports[0]==0 { return Err("舊核心埠號不明；保護紀錄仍保留".into()); }
                agg_core::supervisor::ensure_port_free(ports[0]).map_err(|_| "舊監聽埠仍被使用；保護紀錄仍保留")?;
            },
            Err(e) if e.kind()==std::io::ErrorKind::NotFound => {},
            Err(_) => return Err("無法讀取舊核心設定；保護紀錄仍保留".into()),
        }
        std::fs::remove_file(&guard).map_err(|_| "無法清除啟動保護")?;
        let _ = std::fs::remove_file(dir.join("gateway.pid"));
        Ok(())
    }
}

#[tauri::command]
async fn clear_run_guard(app: tauri::AppHandle) -> Result<String, String> {
    let dir = desktop_data_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let guard = dir.join("gateway.running");
        if !guard.exists() { return Ok("沒有啟動保護紀錄需要清除".into()); }
        let _lock = instance::acquire(&dir.join("gateway.lock"))?;
        remove_stale_guard(&dir)?;
        Ok("已確認沒有對應核心程序並清除保護紀錄；登入憑證已保留".into())
    }).await.map_err(|_| "清除工作失敗".to_string())?
}

#[tauri::command]
async fn codex_detect() -> Result<agg_core::codex::Detection, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let env_codex = std::env::var("CODEX_HOME").ok();
        let env_home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
        let os = std::env::consts::OS;
        agg_core::codex::detect_codex_home(env_codex.as_deref(), env_home.as_deref(), os)
            .map_err(|e| e.to_string())
    }).await.map_err(|_| "偵測工作失敗".to_string())?
}

#[tauri::command]
async fn codex_status(custom_path: Option<String>) -> Result<agg_core::codex::CodexConfigStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = if let Some(p) = custom_path.filter(|s| !s.trim().is_empty()) {
            PathBuf::from(p)
        } else {
            let env_codex = std::env::var("CODEX_HOME").ok();
            let env_home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
            let os = std::env::consts::OS;
            agg_core::codex::detect_codex_home(env_codex.as_deref(), env_home.as_deref(), os)
                .map_err(|e| e.to_string())?
                .cli_config_path
        };
        agg_core::codex::inspect_codex_config(&path).map_err(|e| e.to_string())
    }).await.map_err(|_| "檢查工作失敗".to_string())?
}

#[tauri::command]
async fn codex_preview(state: State<'_, Shared>, custom_path: Option<String>, model: Option<String>, allow_override: Option<bool>, custom_catalog_path: Option<String>) -> Result<agg_core::codex::ApplyPreview, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = GatewaySnapshot::read(&state)?;
        let port = snapshot.port;
        let gw_key = snapshot.gateway_key.clone();
        let path = if let Some(p) = custom_path.filter(|s| !s.trim().is_empty()) {
            PathBuf::from(p)
        } else {
            let env_codex = std::env::var("CODEX_HOME").ok();
            let env_home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
            let os = std::env::consts::OS;
            agg_core::codex::detect_codex_home(env_codex.as_deref(), env_home.as_deref(), os)
                .map_err(|e| e.to_string())?
                .cli_config_path
        };
        let cat_path = catalog_path(&path, custom_catalog_path);
        let model_catalog_json = if cat_path.exists() { Some(cat_path) } else { None };
        let req = agg_core::codex::ApplyRequest {
            config_path: path,
            base_url: format!("http://127.0.0.1:{port}/v1"),
            gateway_key: gw_key,
            model: model.filter(|s| !s.trim().is_empty()),
            allow_foreign_override: allow_override.unwrap_or(false),
            model_catalog_json,
        };
        snapshot.check(&state)?;
        agg_core::codex::preview_apply(&req).map_err(|e| e.to_string())
    }).await.map_err(|_| "預覽工作失敗".to_string())?
}

#[tauri::command]
async fn codex_apply(state: State<'_, Shared>, custom_path: Option<String>, model: Option<String>, allow_override: Option<bool>, custom_catalog_path: Option<String>) -> Result<String, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = GatewaySnapshot::read(&state)?;
        let port = snapshot.port;
        let gw_key = snapshot.gateway_key.clone();
        let path = if let Some(p) = custom_path.filter(|s| !s.trim().is_empty()) {
            PathBuf::from(p)
        } else {
            let env_codex = std::env::var("CODEX_HOME").ok();
            let env_home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
            let os = std::env::consts::OS;
            agg_core::codex::detect_codex_home(env_codex.as_deref(), env_home.as_deref(), os)
                .map_err(|e| e.to_string())?
                .cli_config_path
        };
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| format!("無法建立設定檔目錄：{e}"))?;
            }
        }
        let cat_path = catalog_path(&path, custom_catalog_path);
        let model_catalog_json = if cat_path.exists() { Some(cat_path) } else { None };
        let req = agg_core::codex::ApplyRequest {
            config_path: path.clone(),
            base_url: format!("http://127.0.0.1:{port}/v1"),
            gateway_key: gw_key,
            model: model.filter(|s| !s.trim().is_empty()),
            allow_foreign_override: allow_override.unwrap_or(false),
            model_catalog_json,
        };
        snapshot.check(&state)?;
        agg_core::codex::apply_config(&req).map_err(|e| e.to_string())?;
        Ok(format!("已成功套用至 {} (備份與 ownership 檔已寫入)", path.display()))
    }).await.map_err(|_| "套用工作失敗".to_string())?
}

#[tauri::command]
async fn sync_model_catalog(state: State<'_, Shared>, custom_catalog_path: Option<String>, custom_path: Option<String>) -> Result<agg_core::codex::CatalogSyncSummary, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = GatewaySnapshot::read(&state)?;
        let port = snapshot.port;
        let mgmt_key = snapshot.management_key.clone();
        let all_models = attribution::discover_all_models(port, &mgmt_key)
            .map_err(|e| format!("模型清單未變更：{e}"))?;

        let cat_path = catalog_path(&codex_config_path(custom_path)?, custom_catalog_path);
        snapshot.check(&state)?;

        agg_core::codex::sync_model_catalog(&cat_path, &all_models).map_err(|e| e.to_string())
    }).await.map_err(|_| "同步模型清單工作失敗".to_string())?
}

#[tauri::command]
async fn codex_rollback(custom_path: Option<String>) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = if let Some(p) = custom_path.filter(|s| !s.trim().is_empty()) {
            PathBuf::from(p)
        } else {
            let env_codex = std::env::var("CODEX_HOME").ok();
            let env_home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
            let os = std::env::consts::OS;
            agg_core::codex::detect_codex_home(env_codex.as_deref(), env_home.as_deref(), os)
                .map_err(|e| e.to_string())?
                .cli_config_path
        };
        agg_core::codex::rollback_config(&path).map_err(|e| e.to_string())?;
        Ok(format!("已成功還原 {} 至套用前狀態", path.display()))
    }).await.map_err(|_| "還原工作失敗".to_string())?
}

fn codex_home_dir() -> Result<PathBuf, String> {
    let env_codex = std::env::var("CODEX_HOME").ok();
    let env_home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
    let info = agg_core::codex::detect_codex_home(env_codex.as_deref(), env_home.as_deref(), std::env::consts::OS)
        .map_err(|e| e.to_string())?;
    info.cli_config_path.parent().map(PathBuf::from).ok_or_else(|| "無法定位 codex home 目錄".into())
}

#[tauri::command]
async fn preview_chatgpt_auth(app: tauri::AppHandle) -> Result<agg_core::codex::AuthSyncPreview, String> {
    let app_data = desktop_data_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let auth_dir = app_data.join("runtime").join("auth");
        let parent = codex_home_dir()?;
        agg_core::codex::preview_chatgpt_desktop_auth(&auth_dir, &parent).map_err(|e| e.to_string())
    }).await.map_err(|_| "預覽工作失敗".to_string())?
}

#[tauri::command]
async fn confirm_chatgpt_auth(app: tauri::AppHandle) -> Result<agg_core::codex::AuthSyncSummary, String> {
    let app_data = desktop_data_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let auth_dir = app_data.join("runtime").join("auth");
        let parent = codex_home_dir()?;
        agg_core::codex::apply_chatgpt_desktop_auth(&auth_dir, &parent, true).map_err(|e| e.to_string())
    }).await.map_err(|_| "確認工作失敗".to_string())?
}


#[tauri::command]
async fn vscode_status(custom_path: Option<String>) -> Result<agg_core::vscode::VSCodeStatusInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let os = std::env::consts::OS;
        let path = agg_core::vscode::detect_vscode_models_path(custom_path.as_deref().map(std::path::Path::new), os)
            .ok_or_else(|| "無法自動偵測 VS Code 設定檔路徑".to_string())?;
        Ok(agg_core::vscode::inspect_vscode_status(&path))
    }).await.map_err(|_| "檢查 VS Code 設定失敗".to_string())?
}

#[tauri::command]
async fn vscode_preview(state: State<'_, Shared>, _custom_path: Option<String>) -> Result<String, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = GatewaySnapshot::read(&state)?;
        let port = snapshot.port;
        let gw_key = snapshot.gateway_key.clone();
        let base_url = format!("http://127.0.0.1:{port}/v1");
        snapshot.check(&state)?;
        Ok(agg_core::vscode::preview_vscode_config(&base_url, &gw_key))
    }).await.map_err(|_| "產生 VS Code 預覽失敗".to_string())?
}

#[tauri::command]
async fn vscode_deploy(state: State<'_, Shared>, custom_path: Option<String>) -> Result<agg_core::vscode::VSCodeDeployResult, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = GatewaySnapshot::read(&state)?;
        let port = snapshot.port;
        let gw_key = snapshot.gateway_key.clone();
        let base_url = format!("http://127.0.0.1:{port}/v1");
        let os = std::env::consts::OS;
        let path = agg_core::vscode::detect_vscode_models_path(custom_path.as_deref().map(std::path::Path::new), os)
            .ok_or_else(|| "無法自動偵測 VS Code 設定檔路徑".to_string())?;
        snapshot.check(&state)?;
        agg_core::vscode::apply_vscode_config(&path, &base_url, &gw_key)
    }).await.map_err(|_| "部署至 VS Code 失敗".to_string())?
}

#[tauri::command]
async fn log_message(app: tauri::AppHandle, level: String, message: String, details: String) -> Result<(), String> {
    let dir = desktop_data_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let log_file = dir.join("desktop.log");
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        let line = if details.is_empty() {
            format!("[{now}] [{level}] {message}\n")
        } else {
            format!("[{now}] [{level}] {message} - {details}\n")
        };
        eprint!("{line}");
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log_file) {
            let _ = f.write_all(line.as_bytes());
        }
        Ok(())
    }).await.map_err(|_| "日誌寫入失敗".to_string())?
}

fn daemon_data_dir() -> PathBuf {
    if let Some(value) = std::env::var_os("APP_AGG_DATA_DIR") {
        if !value.is_empty() {
            return PathBuf::from(value);
        }
    }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| format!("{home}\\AppData\\Roaming"));
    PathBuf::from(appdata).join("io.github.zacktools.modelbridge")
}

fn run_headless_daemon() {
    let lock_dir = daemon_data_dir();
    let runtime_dir = lock_dir.join("runtime");
    let guard_path = lock_dir.join("gateway.running");
    if std::fs::create_dir_all(&lock_dir).is_err() { return; }
    let _instance_lock = match instance::acquire(&lock_dir.join("gateway.lock")) {
        Ok(l) => l,
        Err(_) => return,
    };
    if guard_path.exists() && remove_stale_guard(&lock_dir).is_err() { return; }

    let core_dir = std::env::var_os("APP_AGG_CORE_DIR").map(PathBuf::from).or_else(|| {
        let mut d = std::env::current_exe().ok()?.parent()?.to_path_buf();
        for _ in 0..6 {
            let c = d.join(".cache").join("cliproxyapi");
            if c.join(if cfg!(windows) { "cli-proxy-api.exe" } else { "cli-proxy-api" }).exists() {
                return Some(c);
            }
            d = d.parent()?.to_path_buf();
        }
        None
    });
    let Some(core_dir) = core_dir else { return; };
    let binary = core_dir.join(if cfg!(windows) { "cli-proxy-api.exe" } else { "cli-proxy-api" });
    let archive = match pin::native_archive_name() {
        Ok(name) => core_dir.join(name),
        Err(err) => { eprintln!("{err}"); return; }
    };

    let Ok((gateway_key, management_key)) = ensure_local_keys(&lock_dir) else { return; };
    let _ = std::fs::create_dir_all(&runtime_dir);
    let auth_dir = runtime_dir.join("auth");
    let _ = std::fs::create_dir_all(&auth_dir);

    if instance::create_run_guard(&guard_path, &runtime_dir).is_err() {
        return;
    }

    let preferred_port = std::env::var("APP_AGG_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(50833);
    let selected_port = if agg_core::supervisor::ensure_port_free(preferred_port).is_ok() {
        Some(preferred_port)
    } else {
        None
    };

    let result = agg_core::supervisor::SidecarLaunch::HeadlessDaemon.spawn(&archive, StartRequest {
        program: binary,
        work_dir: runtime_dir.clone(),
        port: selected_port,
        extra_env: vec![],
        health_timeout: Duration::from_secs(45),
        gateway_key: gateway_key.clone(),
        management_password: management_key.clone(),
    });

    match result {
        Ok(mut spawned) => {
            let port = spawned.supervisor.port;
            if let Some(pid) = spawned.supervisor.pid() {
                let _ = std::fs::write(lock_dir.join("gateway.pid"), pid.to_string());
            }
            let env_codex = std::env::var("CODEX_HOME").ok();
            let env_home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
            if let Ok(info) = agg_core::codex::detect_codex_home(env_codex.as_deref(), env_home.as_deref(), std::env::consts::OS) {
                let _ = agg_core::codex::after_daemon_start(&info.cli_config_path, port, &gateway_key);
            }
            spawned.supervisor.detach();
        }
        Err(err) => {
            eprintln!("{}", err.message());
            if err.child_absent() {
                let _ = std::fs::remove_file(&guard_path);
            }
        }
    }
}

fn main() {
    if let Some(err) = instance::required_isolation_error() {
        eprintln!("{err}");
        std::process::exit(1);
    }
    if std::env::args().any(|arg| arg == "--probe") {
        println!("{}", serde_json::to_string_pretty(&agg_core::assess_platform()).unwrap_or_default());
        return;
    }
    if std::env::args().any(|arg| arg == "--daemon" || arg == "--headless") {
        run_headless_daemon();
        return;
    }
    let app = tauri::Builder::default().manage(Arc::new(Mutex::new(Gateway::default())) as Shared)
        .invoke_handler(tauri::generate_handler![
            auto_start_enabled, start_gateway, stop_gateway, codex_suspend, codex_resume, set_retain_on_exit, gateway_status, inspect_previous_gateway, clear_run_guard,
            get_autostart_gateway, set_autostart_gateway,
            oauth_start, oauth_action, discovered_models, verify_model_roundtrip, log_message,
            codex_detect, codex_status, codex_preview, codex_apply, codex_rollback, sync_model_catalog, preview_chatgpt_auth, confirm_chatgpt_auth, vscode_status, vscode_preview, vscode_deploy
        ])
        .build(tauri::generate_context!()).expect("tauri setup");
    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            let gateway = app.state::<Shared>().inner().clone();
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            std::thread::spawn(move || {
                let should_stop = gateway.lock().map(|gw| !gw.retain_on_exit).unwrap_or(true);
                if should_stop { let _ = stop_shared_gateway(&gateway); }
                let _ = sender.send(());
            });
            let _ = receiver.recv_timeout(Duration::from_secs(5));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_gateway() -> Shared {
        let mut gw = Gateway::default();
        gw.port = Some(1234);
        gw.management_key = Some("fixture-management".into());
        gw.gateway_key = Some("fixture-gateway".into());
        gw.runtime = Some(std::env::temp_dir());
        Arc::new(Mutex::new(gw))
    }

    #[test]
    fn account_io_does_not_hold_global_lock_and_stale_result_is_discarded() {
        let state = fixture_gateway();
        let mut work = AccountWork::take(&state,"codex").unwrap();
        assert!(state.try_lock().is_ok());
        assert!(AccountWork::take(&state,"codex").is_err());
        assert!(AccountWork::take(&state,"antigravity").is_ok());
        {
            let mut gw = state.lock().unwrap();
            gw.epoch += 1;
            gw.account_busy = [false;2];
        }
        assert!(work.snapshot.check(&state).is_err());
        work.verified = Some("old-result".into());
        drop(work);
        assert!(state.lock().unwrap().verified_codex.is_none());
        assert!(AccountWork::take(&state,"codex").is_ok());
    }

    #[test]
    fn account_actions_distinguish_existing_credentials_and_pending_confirmation() {
        let present=account_result("present","fixture",true,false);
        assert_eq!(present["canStart"],false);
        assert_eq!(present["canRenew"],true);
        assert_eq!(present["canConfirm"],false);
        let waiting=account_result("awaitingConfirmation","fixture",true,true);
        assert_eq!(waiting["canRenew"],false);
        assert_eq!(waiting["canConfirm"],true);
        assert_eq!(waiting["canCancel"],true);
    }

    #[test]
    fn custom_catalog_path_is_shared_by_preview_apply_and_sync() {
        let path=std::env::temp_dir().join("config.toml");
        let selected=std::env::temp_dir().join("other").join("models.json");
        assert_eq!(catalog_path(&path,Some(selected.to_string_lossy().into_owned())),selected);
        assert_eq!(catalog_path(&path,Some(" ".into())),path.with_file_name("model_catalog.json"));
    }

    #[test]
    fn cancelled_probe_does_not_open_a_connection() {
        let result=agg_core::response_evidence::inspect_real_model_tool_roundtrip(
            1,"fixture",&serde_json::json!({}),"fixture",Duration::from_secs(1),"check","0123456789abcdef",
            || Err("cancelled".into()));
        assert_eq!(result,Err("cancelled".into()));
    }

    #[cfg(windows)]
    #[test]
    fn stale_guard_requires_free_runtime_lock_and_port_before_removal() {
        let mut nonce=[0u8;8]; getrandom::getrandom(&mut nonce).unwrap();
        let dir=std::env::temp_dir().join(format!("mb-stale-{}",pin::sha256_bytes(&nonce)));
        let runtime=dir.join("runtime"); std::fs::create_dir_all(&runtime).unwrap();
        let _lock=instance::acquire(&dir.join("gateway.lock")).unwrap();
        let guard=dir.join("gateway.running"); instance::create_run_guard(&guard,&runtime).unwrap();
        let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        std::fs::write(runtime.join("config.yaml"),format!("  port: {}\n",listener.local_addr().unwrap().port())).unwrap();
        let held=instance::acquire(&runtime.join("instance.lock")).unwrap();
        assert!(remove_stale_guard(&dir).is_err()); assert!(guard.exists()); drop(held);
        assert!(remove_stale_guard(&dir).is_err()); assert!(guard.exists()); drop(listener);
        remove_stale_guard(&dir).unwrap(); assert!(!guard.exists());
        drop(_lock); std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn model_verification_is_per_provider_and_cleared_on_stop() {
        let mut gateway = Gateway::default();
        *verified_slot(&mut gateway, "codex").unwrap() = Some("0123456789abcdef".into());
        assert!(verified_slot(&mut gateway, "antigravity").unwrap().is_none());
        assert!(verified_slot(&mut gateway, "unknown").is_err());
        gateway.stop().unwrap();
        assert!(verified_slot(&mut gateway, "codex").unwrap().is_none());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires MB_TEST_CORE_DIR with pinned native core"]
    fn reconnects_and_stops_an_isolated_gateway() {
        let core_dir = PathBuf::from(std::env::var_os("MB_TEST_CORE_DIR").expect("MB_TEST_CORE_DIR"));
        let mut nonce = [0u8; 8];
        getrandom::getrandom(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!("agg-adopt-test-{}", pin::sha256_bytes(&nonce)));
        let runtime = root.join("runtime");
        std::fs::create_dir_all(&runtime).unwrap();
        let (gateway_key, management_key) = instance::ensure_local_keys(&root).unwrap();
        instance::create_run_guard(&root.join("gateway.running"), &runtime).unwrap();
        let mut spawned = agg_core::supervisor::SidecarLaunch::DesktopGateway.spawn(&core_dir.join(pin::native_archive_name().unwrap()), StartRequest {
            program: core_dir.join("cli-proxy-api.exe"), work_dir: runtime, port: None, extra_env: vec![],
            health_timeout: Duration::from_secs(45), gateway_key, management_password: management_key,
        }).unwrap();
        let pid = spawned.supervisor.pid().unwrap();
        std::fs::write(root.join("gateway.pid"), pid.to_string()).unwrap();
        struct Cleanup(windows_adoption::Process);
        impl Drop for Cleanup { fn drop(&mut self) { let _ = self.0.stop(); } }
        let cleanup = Cleanup(windows_adoption::Process::open(pid).unwrap());
        spawned.supervisor.detach();
        drop(spawned);
        let keys_path = root.join("local_keys.json");
        let saved_keys = std::fs::read(&keys_path).unwrap();
        std::fs::write(&keys_path, r#"{"gateway_key":"incorrectgateway123","management_key":"incorrectmanager123"}"#).unwrap();
        assert!(adopt_previous_gateway(&root).is_err());
        assert!(cleanup.0.alive().unwrap());
        assert!(root.join("gateway.running").exists());
        std::fs::write(&keys_path, saved_keys).unwrap();
        let (mut adopted, lock, _, _) = adopt_previous_gateway(&root).unwrap();
        assert_eq!(adopted.pid(), Some(pid));
        assert_eq!(adopted.health().unwrap(), ProcessHealth::Healthy);
        adopted.stop_checked().unwrap();
        drop(lock);
        drop(cleanup);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn suspend_reports_catalog_failure() {
        tauri::async_runtime::block_on(async {
            let mut nonce = [0u8; 8];
            getrandom::getrandom(&mut nonce).unwrap();
            let dir = std::env::temp_dir().join(format!("agg-catalog-error-{}", pin::sha256_bytes(&nonce)));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("config.toml");
            std::fs::write(&path, "").unwrap();
            std::fs::write(dir.join("model_catalog.json"), "invalid json").unwrap();
            assert!(codex_suspend(Some(path.to_string_lossy().into_owned())).await.is_err());
            std::fs::remove_dir_all(dir).unwrap();
        });
    }

    #[test]
    fn persistent_runtime_is_preserved_on_stop() {
        let mut nonce = [0u8; 16];
        getrandom::getrandom(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!("agg-stop-test-{}", pin::sha256_bytes(&nonce)));
        std::fs::create_dir(&root).unwrap();
        let runtime = root.join("runtime");
        let auth = runtime.join("auth");
        std::fs::create_dir_all(&auth).unwrap();
        std::fs::write(auth.join("token.json"), b"credentials-must-persist").unwrap();
        let guard = root.join("gateway.running");
        std::fs::write(&guard, b"guard").unwrap();
        let mut gateway = Gateway::default();
        gateway.runtime = Some(runtime.clone());
        gateway.run_guard = Some(guard.clone());
        
        // 依使用者指示：停止閘道時保留憑證
        gateway.stop().unwrap();
        
        // 啟動保護移除，但本地憑證檔案完整保留！
        assert!(!guard.exists());
        assert!(auth.join("token.json").exists());
        assert_eq!(std::fs::read(auth.join("token.json")).unwrap(), b"credentials-must-persist");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn codex_desktop_status_and_rollback() {
        tauri::async_runtime::block_on(async {
            let mut nonce = [0u8; 8];
            getrandom::getrandom(&mut nonce).unwrap();
            let dir = std::env::temp_dir().join(format!("agg-desk-codex-{}", pin::sha256_bytes(&nonce)));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("config.toml");
            std::fs::write(&path, "personality = \"tester\"\n").unwrap();

            // 1. Status of unmanaged
            let st = codex_status(Some(path.to_str().unwrap().to_string())).await.unwrap();
            assert!(st.exists);
            assert!(!st.managed_by_us);

            // 2. Direct apply via agg_core to simulate applied state
            let req = agg_core::codex::ApplyRequest {
                config_path: path.clone(),
                base_url: "http://127.0.0.1:12345/v1".into(),
                gateway_key: "gwkey12345678".into(),
                model: Some("gpt-6-astra".into()),
                allow_foreign_override: false,
                model_catalog_json: None,
            };
            agg_core::codex::apply_config(&req).unwrap();

            // 3. Status of managed
            let st2 = codex_status(Some(path.to_str().unwrap().to_string())).await.unwrap();
            assert!(st2.managed_by_us);
            assert!(!st2.external_edit_detected);
            assert_eq!(st2.current_model.as_deref(), Some("gpt-6-astra"));

            // 4. Rollback command
            let msg = codex_rollback(Some(path.to_str().unwrap().to_string())).await.unwrap();
            assert!(msg.contains("已成功還原"));

            // 5. Status after rollback
            let st3 = codex_status(Some(path.to_str().unwrap().to_string())).await.unwrap();
            assert!(!st3.managed_by_us);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "personality = \"tester\"\n");

            let _ = std::fs::remove_dir_all(&dir);
        });
    }
    #[test]
    fn codex_desktop_suspend_restores_native_catalog() {
        tauri::async_runtime::block_on(async {
            let mut nonce = [0u8; 8];
            getrandom::getrandom(&mut nonce).unwrap();
            let dir = std::env::temp_dir().join(format!("agg-desk-suspend-{}", pin::sha256_bytes(&nonce)));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("config.toml");
            std::fs::write(&path, "personality = \"tester\"\n").unwrap();

            let cat_path = dir.join("model_catalog.json");
            let initial = serde_json::json!({
                "models": [
                    { "slug": "gpt-6-astra", "display_name": "GPT-6-Astra", "description": "Native" },
                    { "slug": "ccs-gemini/gemini-3.7-flash", "display_name": "CC Switch", "comp_hash": "cc-switch" },
                    { "slug": "gemini-3.8-flash-high", "display_name": "Aggregated", "description": "model-bridge aggregated model: gemini-3.8-flash-high" }
                ]
            });
            std::fs::write(&cat_path, serde_json::to_string(&initial).unwrap()).unwrap();

            let req = agg_core::codex::ApplyRequest {
                config_path: path.clone(),
                base_url: "http://127.0.0.1:12345/v1".into(),
                gateway_key: "gwkey12345678".into(),
                model: Some("gemini-3.8-flash-high".into()),
                allow_foreign_override: false,
                model_catalog_json: Some(cat_path.clone()),
            };
            agg_core::codex::apply_config(&req).unwrap();

            // Run codex_suspend
            let suspended = codex_suspend(Some(path.to_str().unwrap().to_string())).await.unwrap();
            assert!(suspended);

            // Verify config.toml removed model_provider
            let cfg_str = std::fs::read_to_string(&path).unwrap();
            assert!(!cfg_str.contains("model_provider ="));
            assert!(!cfg_str.contains("model_catalog_json ="));
            assert!(!cfg_str.contains("model ="));

            // Preserve models owned by other tools.
            let cat_text = std::fs::read_to_string(&cat_path).unwrap();
            let cat_json: serde_json::Value = serde_json::from_str(&cat_text).unwrap();
            let models = cat_json["models"].as_array().unwrap();
            assert_eq!(models.len(), 2);
            assert_eq!(models[0]["slug"], "gpt-6-astra");
            assert_eq!(models[1]["slug"], "ccs-gemini/gemini-3.7-flash");

            let _ = std::fs::remove_dir_all(&dir);
        });
    }

}

#[cfg(windows)]
mod windows_adoption {
    use std::{ffi::c_void, os::windows::{io::{AsRawHandle, FromRawHandle, OwnedHandle}, process::CommandExt}, path::{Path, PathBuf}, process::Command};

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        fn TerminateProcess(handle: *mut c_void, code: u32) -> i32;
        fn QueryFullProcessImageNameW(handle: *mut c_void, flags: u32, path: *mut u16, size: *mut u32) -> i32;
    }

    pub struct Process { handle: OwnedHandle, pub pid: u32 }
    impl agg_core::supervisor::AdoptedProcess for Process {
        fn pid(&self) -> u32 { self.pid }
        fn image(&self) -> Result<PathBuf, String> { Process::image(self) }
        fn alive(&self) -> Result<bool, String> { Process::alive(self) }
        fn stop(&self) -> Result<(), String> { Process::stop(self) }
        fn verify(&self, port: u16, binary: &Path, config: &Path) -> Result<(), String> {
            verify_process(self.pid, port, binary, config)
        }
    }
    impl Process {
        pub fn open(pid: u32) -> Result<Self, String> {
            let handle = unsafe { OpenProcess(0x0010_1001, 0, pid) };
            if handle.is_null() { return Err("無法取得背景程序控制權".into()); }
            Ok(Self { handle: unsafe { OwnedHandle::from_raw_handle(handle) }, pid })
        }
        pub fn alive(&self) -> Result<bool, String> {
            match unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 0) } {
                0 => Ok(false), 258 => Ok(true), _ => Err("無法確認背景程序狀態".into())
            }
        }
        pub fn image(&self) -> Result<PathBuf, String> {
            let mut path = vec![0u16; 32768];
            let mut len = path.len() as u32;
            if unsafe { QueryFullProcessImageNameW(self.handle.as_raw_handle(), 0, path.as_mut_ptr(), &mut len) } == 0 {
                return Err("無法讀取背景程序路徑".into());
            }
            Ok(PathBuf::from(String::from_utf16(&path[..len as usize]).map_err(|_| "程序路徑無效")?))
        }
        pub fn stop(&self) -> Result<(), String> {
            if !self.alive()? { return Ok(()); }
            if unsafe { TerminateProcess(self.handle.as_raw_handle(), 0) } == 0 && self.alive()? {
                return Err("無法停止背景閘道".into());
            }
            if unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 5000) } != 0 { return Err("無法確認背景閘道已停止".into()); }
            Ok(())
        }
    }

    pub fn verify_process(pid: u32, port: u16, binary: &Path, config: &Path) -> Result<(), String> {
        let script = r#"$p = Get-CimInstance Win32_Process -Filter ('ProcessId = ' + $env:MB_PID) -ErrorAction Stop; $escaped = [regex]::Escape($env:MB_CONFIG); $matchesConfig = $p.CommandLine -match ('(?:^|\s)-config\s+(?:"' + $escaped + '"|' + $escaped + ')(?:\s|$)'); $listener = Get-NetTCPConnection -LocalPort ([int]$env:MB_PORT) -State Listen -ErrorAction Stop | Where-Object { $_.OwningProcess -eq [int]$env:MB_PID -and $_.LocalAddress -eq '127.0.0.1' }; if ($p.ExecutablePath -and ([System.IO.Path]::GetFullPath($p.ExecutablePath) -ieq [System.IO.Path]::GetFullPath($env:MB_BINARY)) -and $matchesConfig -and @($listener).Count -eq 1) { 'verified' }"#;
        let result = Command::new("powershell.exe").creation_flags(0x08000000)
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .env("MB_PID", pid.to_string()).env("MB_PORT", port.to_string())
            .env("MB_BINARY", binary).env("MB_CONFIG", config).output()
            .map_err(|_| "無法檢查背景程序".to_string())?;
        if !result.status.success() || String::from_utf8_lossy(&result.stdout).trim() != "verified" {
            return Err("背景程序、設定或監聽埠不符；保護紀錄仍保留".into());
        }
        Ok(())
    }
}
