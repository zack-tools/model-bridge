//! Private PoC desktop shell. The gateway runs only on explicit user request.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use agg_core::{attribution, instance, pin, registry::SourceKind, oauth::{AuthState, FailureClass, Provider}, oauth_client::OAuthClient, supervisor::{ProcessHealth, StartRequest, Supervisor}};
use std::{path::PathBuf, sync::{Arc, Mutex}, time::Duration};
use tauri::{Manager, State};

#[derive(Default)]
struct Gateway {
    process: Option<Supervisor>,
    runtime: Option<PathBuf>,
    instance_lock: Option<std::fs::File>,
    run_guard: Option<PathBuf>,
    management_key: Option<String>,
    gateway_key: Option<String>,
    codex: Option<OAuthClient>,
    antigravity: Option<OAuthClient>,
    retain_on_exit: bool,
}
impl Drop for Gateway {
    fn drop(&mut self) {
        if !self.retain_on_exit {
            let _ = self.stop();
        }
    }
}
impl Gateway {
    fn stop(&mut self) -> Result<(), String> {
        if let Some(child) = self.process.as_mut() {
            child.stop_checked().map_err(|_| "無法確認閘道已停止；保留啟動保護".to_string())?;
        }
        self.process.take();
        self.codex.take();
        self.antigravity.take();
        self.management_key.take();
        self.gateway_key.take();
        // 依使用者需求：登入憑證存放在本機，關閉閘道不刪除憑證（保留 runtime/auth）
        if let Some(path) = self.run_guard.as_ref() {
            let _ = std::fs::remove_file(path);
            self.run_guard.take();
        }
        self.instance_lock.take();
        Ok(())
    }
}
type Shared = Arc<Mutex<Gateway>>;

fn ensure_local_keys(dir: &std::path::Path) -> Result<(String, String), String> {
    agg_core::instance::ensure_local_keys(dir)
}

#[tauri::command]
async fn start_gateway(app: tauri::AppHandle, state: State<'_, Shared>) -> Result<u16, String> {
    let lock_dir = app.path().app_data_dir().map_err(|_| "無法取得應用程式資料目錄".to_string())?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut gateway = state.lock().map_err(|_| "閘道狀態無法讀取".to_string())?;
        if gateway.process.is_some() { return Err("閘道已啟動".into()); }
        std::fs::create_dir_all(&lock_dir).map_err(|_| "無法建立鎖定目錄")?;
        let instance_lock = instance::acquire(&lock_dir.join("gateway.lock"))
            .map_err(|_| "另一個視窗已啟動閘道，或無法取得鎖定".to_string())?;
        let core_dir = std::env::var_os("APP_AGG_CORE_DIR").map(PathBuf::from)
            .ok_or("請先設定 APP_AGG_CORE_DIR，指向已下載的核心資料夾")?;
        let archive_name = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => "CLIProxyAPI_8.0.57_darwin_aarch64.tar.gz",
            ("macos", "x86_64") => "CLIProxyAPI_8.0.57_darwin_amd64.tar.gz",
            ("linux", "x86_64") => "CLIProxyAPI_8.0.57_linux_amd64.tar.gz",
            ("windows", "x86_64") => "CLIProxyAPI_8.0.57_windows_amd64.zip",
            ("windows", "aarch64") => "CLIProxyAPI_8.0.57_windows_aarch64.zip",
            _ => return Err("此平台的核心驗證尚未實作".into()),
        };
        let binary = core_dir.join(if cfg!(windows) { "cli-proxy-api.exe" } else { "cli-proxy-api" });
        pin::verify_native_binary(&core_dir.join(archive_name), &binary)
            .map_err(|_| "核心驗證失敗，請重新下載釘選版本".to_string())?;
        
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
        let result = Supervisor::start_detailed(StartRequest {
            program: binary, work_dir: dir.clone(), port: selected_port, extra_env: vec![],
            health_timeout: Duration::from_secs(45), gateway_key: gateway_key.clone(), management_password: management_key.clone(),
        });
        match result {
            Ok(process) => {
                let port = process.port;
                let child_pid = process.pid();
                gateway.runtime = Some(dir);
                gateway.process = Some(process);
                gateway.instance_lock = Some(instance_lock);
                gateway.run_guard = Some(guard_path);
                gateway.management_key = Some(management_key.clone());
                gateway.gateway_key = Some(gateway_key);

                // 自動從本地保存之憑證恢復 session
                if let Ok(_index) = attribution::single_active_index(port, &management_key, SourceKind::ChatGptCodex) {
                    let mut client = OAuthClient::new(port, management_key.clone(), Provider::ChatGptCodex);
                    client.session.state = AuthState::Authenticated;
                    client.session.live = true;
                    gateway.codex = Some(client);
                }
                if let Ok(_index) = attribution::single_active_index(port, &management_key, SourceKind::Antigravity) {
                    let mut client = OAuthClient::new(port, management_key.clone(), Provider::Antigravity);
                    client.session.state = AuthState::Authenticated;
                    client.session.live = true;
                    gateway.antigravity = Some(client);
                }
                if let Some(pid) = child_pid {
                    let _ = std::fs::write(lock_dir.join("gateway.pid"), pid.to_string());
                }
                gateway.retain_on_exit = true;

                // 自動同步本機登入憑證至 ChatGPT Desktop (auth.json)，保持桌面端帳號登入態
                let env_codex = std::env::var("CODEX_HOME").ok();
                let env_home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
                let os = std::env::consts::OS;
                if let Ok(info) = agg_core::codex::detect_codex_home(env_codex.as_deref(), env_home.as_deref(), os) {
                    if let Some(parent) = info.cli_config_path.parent() {
                        let _ = agg_core::codex::sync_chatgpt_desktop_auth(&auth_dir, parent);
                    }
                }
                Ok(port)
            }
            Err(_) => {
                let _ = std::fs::remove_file(&guard_path);
                Err("閘道啟動失敗；請先檢查核心日誌".into())
            }
        }
    }).await.map_err(|_| "啟動工作失敗".to_string())?
}

#[tauri::command]
async fn stop_gateway(app: tauri::AppHandle, state: State<'_, Shared>) -> Result<(), String> {
    let lock_dir = app.path().app_data_dir().map_err(|_| "無法取得應用程式資料目錄".to_string())?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut gw = state.lock().map_err(|_| "閘道狀態無法讀取".to_string())?;
        gw.retain_on_exit = false;
        gw.stop()?;
        let _ = std::fs::remove_file(lock_dir.join("gateway.pid"));
        Ok(())
    }).await.map_err(|_| "停止工作失敗".to_string())?
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

#[tauri::command]
async fn gateway_status(app: tauri::AppHandle, state: State<'_, Shared>) -> Result<serde_json::Value, String> {
    let guard_path = app.path().app_data_dir().map_err(|_| "無法取得應用程式資料目錄".to_string())?.join("gateway.running");
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut gateway = state.lock().map_err(|_| "閘道狀態無法讀取".to_string())?;
        if gateway.process.is_none() && gateway.run_guard.is_some() {
            return Ok(serde_json::json!({"state": "cleanup_pending"}));
        }
        let Some(process) = gateway.process.as_mut() else {
            let state = match std::fs::symlink_metadata(&guard_path) {
                Ok(_) => "needs_recovery",
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => "stopped",
                Err(_) => return Err("無法檢查上次啟動紀錄".into()),
            };
            return Ok(serde_json::json!({"state": state}));
        };
        let port = process.port;
        match process.health().map_err(|_| "無法檢查閘道狀態".to_string())? {
            ProcessHealth::Healthy => Ok(serde_json::json!({"state": "healthy", "port": port})),
            ProcessHealth::Degraded => Ok(serde_json::json!({"state": "degraded", "port": port})),
            ProcessHealth::Exited => {
                gateway.stop()?;
                Ok(serde_json::json!({"state": "exited"}))
            }
        }
    }).await.map_err(|_| "狀態檢查失敗".to_string())?
}

fn account_slot<'a>(gateway: &'a mut Gateway, provider: &str) -> Result<&'a mut Option<OAuthClient>, String> {
    match provider { "codex" => Ok(&mut gateway.codex), "antigravity" => Ok(&mut gateway.antigravity), _ => Err("不支援的帳號來源".into()) }
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
async fn oauth_start(state: State<'_, Shared>, provider: String, terms: bool) -> Result<String, String> {
    if !terms { return Err("請先確認登入提示".into()); }
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut gateway = state.lock().map_err(|_| "閘道狀態無法讀取".to_string())?;
        let port = gateway.process.as_ref().ok_or("請先啟動閘道")?.port;
        let key = gateway.management_key.clone().ok_or("閘道金鑰不可用")?;
        let kind = match provider.as_str() { "codex" => Provider::ChatGptCodex, "antigravity" => Provider::Antigravity, _ => return Err("不支援的帳號來源".into()) };
        let slot = account_slot(&mut gateway, &provider)?;
        if slot.is_some() { return Err("此帳號流程已存在；請先取消或停止閘道".into()); }
        let mut client = OAuthClient::new(port, key, kind);
        client.session.acknowledge_terms();
        let url = match client.begin() {
            Ok(url) => url,
            Err(FailureClass::PortConflict) => {
                let p = client.callback_port();
                return Err(format!("本機連接埠 {p} 已被其他程式佔用，請先關閉佔用該埠的程式後重試。"));
            }
            Err(_) => { *slot = Some(client); return Err("授權啟動失敗，流程已停止；請檢查上游帳號提示".into()); }
        };
        let opened = open_system_browser(&url);
        if !opened {
            let cancelled = client.cancel().is_ok();
            if !cancelled { *slot = Some(client); }
            return Err(if cancelled { "無法開啟系統瀏覽器，授權已取消" } else { "無法開啟系統瀏覽器，授權取消未確認；請停止閘道" }.into());
        }
        *slot = Some(client);
        Ok("已要求系統開啟瀏覽器；完成後按檢查狀態".into())
    }).await.map_err(|_| "授權工作失敗".to_string())?
}

#[tauri::command]
async fn oauth_action(state: State<'_, Shared>, provider: String, action: String) -> Result<String, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut gateway = state.lock().map_err(|_| "閘道狀態無法讀取".to_string())?;
        let port = gateway.process.as_ref().ok_or("請先啟動閘道")?.port;
        let mgmt_key = gateway.management_key.clone().ok_or("閘道金鑰不可用")?;
        let slot = account_slot(&mut gateway, &provider)?;
        let client = match slot.as_mut() {
            Some(c) => c,
            None => {
                let (source, prov) = match provider.as_str() {
                    "codex" => (SourceKind::ChatGptCodex, Provider::ChatGptCodex),
                    "antigravity" => (SourceKind::Antigravity, Provider::Antigravity),
                    _ => return Err("不支援的帳號來源".into()),
                };
                if let Ok(_index) = attribution::single_active_index(port, &mgmt_key, source) {
                    let mut new_client = OAuthClient::new(port, mgmt_key, prov);
                    new_client.session.state = AuthState::Authenticated;
                    new_client.session.live = true;
                    *slot = Some(new_client);
                    return Ok("已從本機保存憑證確認登入 (已驗證)；停止閘道保留憑證".into());
                } else {
                    return Err("此帳號尚未啟動登入".into());
                }
            }
        };
        match action.as_str() {
            "poll" => match client.poll_user_present() {
                Ok(AuthState::Authorizing) => Ok("等待瀏覽器授權".into()),
                Ok(AuthState::AwaitingConfirmation) => Ok("上游回報完成；請確認是你本人剛完成登入".into()),
                Ok(AuthState::Authenticated) => Ok("已確認登入；憑證保存在本機，停止閘道不會刪除".into()),
                Ok(AuthState::Expired) => Ok("授權已逾時；請重新開始".into()),
                _ => Err("授權失敗或遭拒絕；流程已停止".into()),
            },
            "confirm" => {
                client.session.mark_user_present_authenticated().map_err(|_| "尚未收到上游成功回報".to_string())?;
                Ok("本機已確認登入；憑證保存在本機，停止閘道不會刪除".into())
            },
            "cancel" => {
                client.cancel().map_err(|_| "取消未獲上游確認；請停止閘道".to_string())?;
                *slot = None;
                Ok("授權已取消".into())
            },
            _ => Err("不支援的操作".into()),
        }
    }).await.map_err(|_| "授權工作失敗".to_string())?
}

#[tauri::command]
async fn discovered_models(state: State<'_, Shared>, provider: String) -> Result<Vec<String>, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut gateway = state.lock().map_err(|_| "閘道狀態無法讀取".to_string())?;
        let port = gateway.process.as_ref().ok_or("請先啟動閘道")?.port;
        let key = gateway.management_key.clone().ok_or("閘道金鑰不可用")?;
        let source = match provider.as_str() {
            "codex" => SourceKind::ChatGptCodex,
            "antigravity" => SourceKind::Antigravity,
            _ => return Err("不支援的帳號來源".into()),
        };
        let client = account_slot(&mut gateway, &provider)?.as_ref().ok_or("請先完成本人登入確認")?;
        if client.session.state != AuthState::Authenticated || !client.session.live {
            return Err("請先完成本人登入確認".into());
        }
        let index = attribution::single_active_index(port, &key, source).map_err(|e| format!("無法確定唯一可用帳號：{e}"))?;
        let (observed_source, models) = attribution::discover_models(port, &key, &index).map_err(|e| format!("無法取得帳號模型清單：{e}"))?;
        if source != observed_source { return Err("帳號來源已變更；請重新登入".into()); }
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
        let mut gateway = state.lock().map_err(|_| "閘道狀態無法讀取".to_string())?;
        let port = gateway.process.as_ref().ok_or("請先啟動閘道")?.port;
        let mgmt_key = gateway.management_key.clone().ok_or("閘道管理金鑰不可用")?;
        let gw_key = gateway.gateway_key.clone().ok_or("閘道存取金鑰不可用")?;
        let source = match provider.as_str() {
            "codex" => SourceKind::ChatGptCodex,
            "antigravity" => SourceKind::Antigravity,
            _ => return Err("不支援的帳號來源".into()),
        };
        let client = account_slot(&mut gateway, &provider)?.as_ref().ok_or("請先完成本人登入確認")?;
        if client.session.state != AuthState::Authenticated || !client.session.live {
            return Err("請先完成本人登入確認".into());
        }
        let index = attribution::single_active_index(port, &mgmt_key, source)
            .map_err(|e| format!("無法確定唯一可用帳號：{e}"))?;
        let (observed_source, models) = attribution::discover_models(port, &mgmt_key, &index)
            .map_err(|e| format!("無法取得帳號模型清單：{e}"))?;
        if source != observed_source {
            return Err("帳號來源已變更；請重新登入".into());
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

        agg_core::response_evidence::inspect_real_model_tool_roundtrip(
            port,
            &gw_key,
            &request,
            &model,
            Duration::from_secs(45),
            "check",
            &index,
        )
    }).await.map_err(|_| "驗證工作失敗".to_string())?
}

#[tauri::command]
async fn inspect_previous_gateway(app: tauri::AppHandle) -> Result<String, String> {
    let dir = app.path().app_data_dir().map_err(|_| "無法取得應用程式資料目錄".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let guard = dir.join("gateway.running");
        if !guard.exists() { return Ok("沒有上次啟動保護紀錄。".to_string()); }
        let runtime = instance::guard_runtime(&guard).map_err(|_| "保護紀錄缺少可辨識的目錄；需人工檢查，保護仍保留。".to_string())?;
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
        #[cfg(not(target_os = "macos"))]
        Ok(format!("執行目錄：{}。此平台的程序檢查尚未實作；保護仍保留。", runtime.display()))
    }).await.map_err(|_| "檢查工作失敗".to_string())?
}

#[tauri::command]
async fn clear_run_guard(app: tauri::AppHandle) -> Result<String, String> {
    let dir = app.path().app_data_dir().map_err(|_| "無法取得應用程式資料目錄".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let guard = dir.join("gateway.running");
        if !guard.exists() { return Ok("沒有啟動保護紀錄需要清除".into()); }
        let runtime = instance::guard_runtime(&guard)?;
        #[cfg(target_os = "macos")]
        {
            let output = std::process::Command::new("/bin/ps").args(["-axo", "command="])
                .output().map_err(|_| "無法檢查程序".to_string())?;
            if !output.status.success() { return Err("程序檢查失敗".into()); }
            let config = runtime.join("config.yaml");
            let config_str = config.to_str().ok_or("路徑無法顯示")?;
            let found = String::from_utf8_lossy(&output.stdout).lines()
                .any(|line| line.contains("cli-proxy-api") && line.contains(config_str));
            if found {
                return Err("偵測到舊核心程序仍在執行中，請先終止舊程序後再清除保護".into());
            }
        }
        let _ = std::fs::remove_dir_all(&runtime);
        std::fs::remove_file(&guard).map_err(|e| format!("無法移除啟動保護檔：{e}"))?;
        Ok("已確認舊程序停止，成功清理上次暫存與保護紀錄".into())
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
async fn codex_preview(state: State<'_, Shared>, custom_path: Option<String>, model: Option<String>, allow_override: Option<bool>) -> Result<agg_core::codex::ApplyPreview, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let gateway = state.lock().map_err(|_| "閘道狀態無法讀取".to_string())?;
        let port = gateway.process.as_ref().ok_or("請先啟動閘道，以獲取本機監聽埠")?.port;
        let gw_key = gateway.gateway_key.clone().ok_or("閘道存取金鑰不可用")?;
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
        let cat_path = path.with_file_name("model_catalog.json");
        let model_catalog_json = if cat_path.exists() { Some(cat_path) } else { None };
        let req = agg_core::codex::ApplyRequest {
            config_path: path,
            base_url: format!("http://127.0.0.1:{port}/v1"),
            gateway_key: gw_key,
            model: model.filter(|s| !s.trim().is_empty()),
            allow_foreign_override: allow_override.unwrap_or(false),
            model_catalog_json,
        };
        agg_core::codex::preview_apply(&req).map_err(|e| e.to_string())
    }).await.map_err(|_| "預覽工作失敗".to_string())?
}

#[tauri::command]
async fn codex_apply(state: State<'_, Shared>, custom_path: Option<String>, model: Option<String>, allow_override: Option<bool>) -> Result<String, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let gateway = state.lock().map_err(|_| "閘道狀態無法讀取".to_string())?;
        let port = gateway.process.as_ref().ok_or("請先啟動閘道，以獲取本機監聽埠")?.port;
        let gw_key = gateway.gateway_key.clone().ok_or("閘道存取金鑰不可用")?;
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
        let cat_path = path.with_file_name("model_catalog.json");
        let model_catalog_json = if cat_path.exists() { Some(cat_path) } else { None };
        let req = agg_core::codex::ApplyRequest {
            config_path: path.clone(),
            base_url: format!("http://127.0.0.1:{port}/v1"),
            gateway_key: gw_key,
            model: model.filter(|s| !s.trim().is_empty()),
            allow_foreign_override: allow_override.unwrap_or(false),
            model_catalog_json,
        };
        agg_core::codex::apply_config(&req).map_err(|e| e.to_string())?;

        // 套用時自動同步本地憑證至 auth.json，使 ChatGPT Desktop 保持登入
        let sync_msg = if let (Ok(app_data), Some(parent)) = (instance::default_app_data_dir(), path.parent()) {
            let auth_dir = app_data.join("runtime").join("auth");
            match agg_core::codex::sync_chatgpt_desktop_auth(&auth_dir, parent) {
                Ok(s) => format!("，ChatGPT Desktop 登入狀態：{}", s.message),
                Err(_) => String::new(),
            }
        } else {
            String::new()
        };

        Ok(format!("已成功套用至 {} (備份與 ownership 檔已寫入){sync_msg}", path.display()))
    }).await.map_err(|_| "套用工作失敗".to_string())?
}

#[tauri::command]
async fn sync_model_catalog(state: State<'_, Shared>, custom_catalog_path: Option<String>) -> Result<agg_core::codex::CatalogSyncSummary, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let gateway = state.lock().map_err(|_| "閘道狀態無法讀取".to_string())?;
        let port = gateway.process.as_ref().ok_or("請先啟動閘道，以獲取可用模型清單")?.port;
        let mgmt_key = gateway.management_key.clone().ok_or("閘道管理金鑰不可用")?;

        let mut all_models = Vec::new();
        if let Ok(index) = attribution::single_active_index(port, &mgmt_key, SourceKind::ChatGptCodex) {
            if let Ok((_, models)) = attribution::discover_models(port, &mgmt_key, &index) {
                for m in models {
                    if !all_models.contains(&m) { all_models.push(m); }
                }
            }
        }
        if let Ok(index) = attribution::single_active_index(port, &mgmt_key, SourceKind::Antigravity) {
            if let Ok((_, models)) = attribution::discover_models(port, &mgmt_key, &index) {
                for m in models {
                    if !all_models.contains(&m) { all_models.push(m); }
                }
            }
        }
        if all_models.is_empty() {
            return Err("目前尚未發現可用之模型，請先完成登入並查詢模型".into());
        }

        let cat_path = if let Some(p) = custom_catalog_path.filter(|s| !s.trim().is_empty()) {
            PathBuf::from(p)
        } else {
            let env_codex = std::env::var("CODEX_HOME").ok();
            let env_home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
            let os = std::env::consts::OS;
            let det = agg_core::codex::detect_codex_home(env_codex.as_deref(), env_home.as_deref(), os)
                .map_err(|e| e.to_string())?;
            det.cli_config_path.with_file_name("model_catalog.json")
        };

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

#[tauri::command]
async fn sync_chatgpt_auth() -> Result<agg_core::codex::AuthSyncSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let app_data = instance::default_app_data_dir().map_err(|e| e.to_string())?;
        let auth_dir = app_data.join("runtime").join("auth");
        let env_codex = std::env::var("CODEX_HOME").ok();
        let env_home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
        let os = std::env::consts::OS;
        let info = agg_core::codex::detect_codex_home(env_codex.as_deref(), env_home.as_deref(), os)
            .map_err(|e| e.to_string())?;
        let parent = info.cli_config_path.parent().ok_or("無法定位 codex home 目錄")?;
        agg_core::codex::sync_chatgpt_desktop_auth(&auth_dir, parent).map_err(|e| e.to_string())
    }).await.map_err(|_| "同步工作失敗".to_string())?
}

#[tauri::command]
async fn log_message(app: tauri::AppHandle, level: String, message: String, details: String) -> Result<(), String> {
    let dir = app.path().app_data_dir().map_err(|_| "無法取得應用程式資料目錄".to_string())?;
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

fn main() {
    if std::env::var("APP_AGG_OPEN_UI").ok().as_deref() != Some("1") {
        println!("{}", serde_json::to_string_pretty(&agg_core::assess_platform()).unwrap_or_default());
        return;
    }
    let app = tauri::Builder::default().manage(Arc::new(Mutex::new(Gateway::default())) as Shared)
        .invoke_handler(tauri::generate_handler![
            start_gateway, stop_gateway, set_retain_on_exit, gateway_status, inspect_previous_gateway, clear_run_guard,
            oauth_start, oauth_action, discovered_models, verify_model_roundtrip, log_message,
            codex_detect, codex_status, codex_preview, codex_apply, codex_rollback, sync_model_catalog, sync_chatgpt_auth
        ])
        .build(tauri::generate_context!()).expect("tauri setup");
    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            let gateway = app.state::<Shared>().inner().clone();
            let should_stop = gateway.lock().map(|gw| !gw.retain_on_exit).unwrap_or(true);
            if should_stop {
                let (sender, receiver) = std::sync::mpsc::sync_channel(1);
                std::thread::spawn(move || {
                    let _ = sender.send(gateway.lock().map_err(|_| ()).and_then(|mut value| value.stop().map_err(|_| ())));
                });
                // If cleanup cannot finish, retain the persistent guard for recovery.
                let _ = receiver.recv_timeout(Duration::from_secs(5));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
