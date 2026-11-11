//! Headless PoC entry. The Tauri window is a separate crate and is not linked here.

use agg_core::check::{run_pinned_core_check, sanitize_report};
use agg_core::pin::pinned_core;
use agg_core::platform::assess_platform;
use std::env;
use std::path::PathBuf;
use std::process::exit;

fn main() {
    if let Some(err) = agg_core::instance::required_isolation_error() {
        eprintln!("{err}");
        exit(1);
    }
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        None | Some("doctor") => {
            println!(
                "{}",
                serde_json::to_string_pretty(&assess_platform()).unwrap_or_else(|_| "{}".into())
            );
        }
        Some("pin") => {
            println!(
                "{}",
                serde_json::to_string_pretty(&pinned_core()).unwrap_or_else(|_| "{}".into())
            );
        }
        Some("core-check") => {
            let mut archive = None;
            let mut binary = None;
            let mut evidence = None;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--archive" => archive = args.next().map(PathBuf::from),
                    "--binary" => binary = args.next().map(PathBuf::from),
                    "--evidence" => evidence = args.next().map(PathBuf::from),
                    other => {
                        eprintln!("unknown argument {other}");
                        exit(2);
                    }
                }
            }
            let Some(archive) = archive else {
                eprintln!("--archive is required");
                exit(2);
            };
            let Some(binary) = binary else {
                eprintln!("--binary is required");
                exit(2);
            };
            match run_pinned_core_check(&archive, &binary) {
                Ok(report) => {
                    let body = sanitize_report(&report, &[]);
                    if let Some(path) = evidence {
                        if let Some(parent) = path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if let Err(err) = std::fs::write(&path, &body) {
                            eprintln!("evidence write: {err}");
                            exit(1);
                        }
                    }
                    println!("{body}");
                    if report.core_integration_pass != "pass" {
                        exit(1);
                    }
                }
                Err(err) => {
                    eprintln!("{err}");
                    exit(1);
                }
            }
        }
        Some("apply") => {
            let req = read_apply_request(&mut args);
            match agg_core::codex::apply_config(&req) {
                Ok(()) => println!("Successfully applied to {}", req.config_path.display()),
                Err(e) => {
                    eprintln!("apply failed: {e}");
                    exit(1);
                }
            }
        }
        Some("status") => {
            let app_data = agg_core::instance::app_data_dir().unwrap_or_else(|e| {
                eprintln!("{e}");
                exit(1);
            });
            let (gateway_key, management_key) = agg_core::instance::ensure_local_keys(&app_data).unwrap_or_else(|e| {
                eprintln!("無法讀取本機金鑰：{e}");
                exit(1);
            });
            let port = 50833u16;
            let bearer = format!("Bearer {management_key}");
            let redactor = agg_core::redact::Redactor::new([management_key.clone(), gateway_key.clone()]);
            match agg_core::http::exchange(port, "GET", "/v8/management/credentials", &[("Authorization", &bearer)], std::time::Duration::from_secs(2), &redactor) {
                Ok(resp) if resp.status == 200 => {
                    let creds: serde_json::Value = serde_json::from_str(&resp.body).unwrap_or_default();
                    let files = creds["files"].as_array().cloned().unwrap_or_default();
                    let providers: Vec<String> = files.iter().filter_map(|f| f["provider"].as_str().map(|s| s.to_string())).collect();
                    
                    let gw_bearer = format!("Bearer {gateway_key}");
                    let models_resp = agg_core::http::exchange(port, "GET", "/v1/models", &[("Authorization", &gw_bearer)], std::time::Duration::from_secs(2), &redactor);
                    let mut model_count = 0;
                    if let Ok(m) = models_resp {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&m.body) {
                            if let Some(arr) = v["data"].as_array() {
                                model_count = arr.iter().filter(|entry| {
                                    let id = entry["id"].as_str().unwrap_or("");
                                    !agg_core::registry::is_image_model(id)
                                }).count();
                            }
                        }
                    }

                    let env_codex = std::env::var("CODEX_HOME").ok();
                    let env_home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
                    let os = std::env::consts::OS;
                    let codex_status = agg_core::codex::detect_codex_home(env_codex.as_deref(), env_home.as_deref(), os)
                        .ok()
                        .and_then(|info| agg_core::codex::inspect_codex_config(&info.cli_config_path).ok());

                    let report = serde_json::json!({
                        "status": "healthy",
                        "port": port,
                        "base_url": format!("http://127.0.0.1:{port}/v1"),
                        "active_accounts": providers,
                        "account_count": files.len(),
                        "available_models_count": model_count,
                        "chatgpt_desktop_integrated": codex_status.as_ref().map(|s| s.managed_by_us).unwrap_or(false),
                        "chatgpt_desktop_current_model": codex_status.as_ref().and_then(|s| s.current_model.clone()),
                        "chatgpt_desktop_logged_in": serde_json::Value::Null,
                        "chatgpt_desktop_account_id": serde_json::Value::Null,
                        "codex_login_note": "gateway keeps its own OAuth login; the desktop auth file is not read",
                    });
                    println!("{}", serde_json::to_string_pretty(&report).unwrap());
                }
                _ => {
                    let report = serde_json::json!({
                        "status": "stopped",
                        "port": port,
                        "note": "閘道未在此連接埠回應；本地憑證存於 runtime/auth 目錄，下次啟動自動恢復",
                    });
                    println!("{}", serde_json::to_string_pretty(&report).unwrap());
                }
            }
        }
        Some("sync") => {
            let app_data = agg_core::instance::app_data_dir().unwrap_or_else(|e| {
                eprintln!("{e}");
                exit(1);
            });
            let (_gw, management_key) = agg_core::instance::ensure_local_keys(&app_data).unwrap_or_else(|e| {
                eprintln!("無法讀取本機金鑰：{e}");
                exit(1);
            });
            let port = 50833u16;
            let all_models = agg_core::attribution::discover_all_models(port, &management_key).unwrap_or_else(|e| {
                eprintln!("同步失敗，原有模型清單保留：{e}");
                exit(1);
            });
            let env_codex = std::env::var("CODEX_HOME").ok();
            let env_home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
            let os = std::env::consts::OS;
            let info = agg_core::codex::detect_codex_home(env_codex.as_deref(), env_home.as_deref(), os).expect("detect codex home");
            let cat_path = info.cli_config_path.with_file_name("model_catalog.json");
            match agg_core::codex::sync_model_catalog(&cat_path, &all_models) {
                Ok(summary) => {
                    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
                        "catalog_path": cat_path.display().to_string(),
                        "added_slugs": summary.added_slugs,
                        "existing_slugs": summary.existing_slugs,
                        "total_models": summary.total_models,
                        "backup_created": summary.backup_path.map(|p| p.display().to_string()),
                    })).unwrap());
                }
                Err(e) => {
                    eprintln!("同步模型目錄失敗：{e}");
                    exit(1);
                }
            }
        }
        Some("preview") => {
            let req = read_apply_request(&mut args);
            match agg_core::codex::preview_apply(&req) {
                Ok(preview) => println!("{}", preview.diff),
                Err(e) => {
                    eprintln!("preview failed: {e}");
                    exit(1);
                }
            }
        }
        Some("rollback") => {
            let mut config_path = None;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--config" => config_path = args.next().map(PathBuf::from),
                    other => {
                        eprintln!("unknown argument {other}");
                        exit(2);
                    }
                }
            }
            let config_path = config_path.unwrap_or_else(default_codex_config);
            match agg_core::codex::rollback_config(&config_path) {
                Ok(()) => println!("rolled back {}", config_path.display()),
                Err(e) => {
                    eprintln!("rollback failed: {e}");
                    exit(1);
                }
            }
        }
        Some("sync-auth") => {
            let mut confirm = false;
            for arg in args.by_ref() {
                match arg.as_str() {
                    "--confirm" => confirm = true,
                    other => {
                        eprintln!("unknown argument {other}");
                        exit(2);
                    }
                }
            }
            let app_data = agg_core::instance::app_data_dir().unwrap_or_else(|e| {
                eprintln!("{e}");
                exit(1);
            });
            let runtime_auth = app_data.join("runtime").join("auth");
            let codex_home = default_codex_config().parent().map(PathBuf::from).unwrap_or_else(|| {
                eprintln!("codex home directory is missing");
                exit(1);
            });
            if !confirm {
                match agg_core::codex::preview_chatgpt_desktop_auth(&runtime_auth, &codex_home) {
                    Ok(preview) => println!("{}", serde_json::to_string_pretty(&preview).unwrap()),
                    Err(e) => {
                        eprintln!("預覽失敗：{e}");
                        exit(1);
                    }
                }
            } else {
                match agg_core::codex::apply_chatgpt_desktop_auth(&runtime_auth, &codex_home, true) {
                    Ok(summary) => println!("{}", serde_json::to_string_pretty(&summary).unwrap()),
                    Err(e) => {
                        eprintln!("確認失敗：{e}");
                        exit(1);
                    }
                }
            }
        }
        Some("oauth-begin") => {
            let mut provider = None;
            let mut accept_file_tokens = false;
            let mut open = false;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--provider" => provider = args.next(),
                    "--accept-file-tokens" => accept_file_tokens = true,
                    "--open" => open = true,
                    other => {
                        eprintln!("unknown argument {other}");
                        exit(2);
                    }
                }
            }
            let Some(provider) = provider else {
                eprintln!("--provider is required");
                exit(2);
            };
            if provider != "codex" && provider != "antigravity" {
                eprintln!("unsupported provider {provider}");
                exit(2);
            }
            if let Err(err) = agg_core::platform::refuse_oauth_begin_without_file_token_acceptance(accept_file_tokens) {
                eprintln!("{err}");
                exit(1);
            }
            if let Err(err) = begin_provider_login(&provider, open) {
                eprintln!("{err}");
                exit(1);
            }
        }
        Some("start") => {
            let app_data = agg_core::instance::app_data_dir().unwrap_or_else(|e| {
                eprintln!("{e}");
                exit(1);
            });
            let (gateway_key, management_key) = agg_core::instance::ensure_local_keys(&app_data).unwrap_or_else(|e| {
                eprintln!("無法讀取本機金鑰：{e}");
                exit(1);
            });
            let port = 50833u16;

            let bearer = format!("Bearer {management_key}");
            let redactor = agg_core::redact::Redactor::new([management_key.clone(), gateway_key.clone()]);
            if let Ok(resp) = agg_core::http::exchange(port, "GET", "/v8/management/credentials", &[("Authorization", &bearer)], std::time::Duration::from_millis(800), &redactor) {
                if resp.status == 200 {
                    println!("閘道已在運作中：127.0.0.1:{port} (健康狀態良好)");
                    return;
                }
            }

            let binary = agg_core::instance::find_core_binary().unwrap_or_else(|e| {
                eprintln!("{e}");
                exit(1);
            });
            let archive = agg_core::instance::archive_beside(&binary).unwrap_or_else(|e| {
                eprintln!("{e}");
                exit(1);
            });
            let runtime_dir = app_data.join("runtime");
            let spawned = agg_core::supervisor::SidecarLaunch::PocStart.spawn(&archive, agg_core::supervisor::StartRequest {
                program: binary,
                work_dir: runtime_dir,
                port: Some(port),
                extra_env: vec![],
                health_timeout: std::time::Duration::from_secs(45),
                gateway_key: gateway_key.clone(),
                management_password: management_key.clone(),
            });
            let mut spawned = match spawned {
                Ok(spawned) => spawned,
                Err(err) => {
                    eprintln!("{}", err.message());
                    exit(1);
                }
            };
            let port = spawned.supervisor.port;
            if let Some(pid) = spawned.supervisor.pid() {
                let _ = std::fs::write(app_data.join("gateway.pid"), pid.to_string());
                println!("閘道啟動成功！PID: {pid}，監聽 127.0.0.1:{port}");
            } else {
                println!("閘道啟動成功！監聽 127.0.0.1:{port}");
            }
            let env_codex = std::env::var("CODEX_HOME").ok();
            let env_home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
            if let Ok(info) = agg_core::codex::detect_codex_home(env_codex.as_deref(), env_home.as_deref(), std::env::consts::OS) {
                let cat_path = info.cli_config_path.with_file_name("model_catalog.json");
                let req = agg_core::codex::ApplyRequest {
                    config_path: info.cli_config_path.clone(),
                    base_url: format!("http://127.0.0.1:{port}/v1"),
                    gateway_key: gateway_key.clone(),
                    model: None,
                    allow_foreign_override: true,
                    model_catalog_json: if cat_path.exists() { Some(cat_path) } else { None },
                };
                if agg_core::codex::apply_config(&req).is_ok() {
                    println!("已自動將聚合端點套用至 {}", info.cli_config_path.display());
                }
            }
            spawned.supervisor.detach();
        }
        Some("stop") => {
            let app_data = agg_core::instance::app_data_dir().unwrap_or_else(|e| {
                eprintln!("{e}");
                exit(1);
            });
            let pid_file = app_data.join("gateway.pid");
            let mut stopped = false;
            if let Ok(content) = std::fs::read_to_string(&pid_file) {
                if let Ok(pid) = content.trim().parse::<u32>() {
                    #[cfg(unix)]
                    {
                        let _ = std::process::Command::new("kill").arg(pid.to_string()).output();
                        stopped = true;
                    }
                    #[cfg(windows)]
                    {
                        let _ = std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]).output();
                        stopped = true;
                    }
                }
                let _ = std::fs::remove_file(&pid_file);
            }
            #[cfg(unix)]
            {
                if let Ok(output) = std::process::Command::new("lsof").args(["-tiTCP:50833", "-sTCP:LISTEN"]).output() {
                    let out = String::from_utf8_lossy(&output.stdout);
                    for p in out.lines() {
                        if !p.trim().is_empty() {
                            let _ = std::process::Command::new("kill").arg(p.trim()).output();
                            stopped = true;
                        }
                    }
                }
            }
            let guard = app_data.join("gateway.running");
            let _ = std::fs::remove_file(guard);
            if stopped {
                println!("閘道已停止。本機登入憑證已完整保留在 runtime/auth/，未作刪除。");
            } else {
                println!("閘道未在運作中。本機登入憑證完整保留在 runtime/auth/。");
            }
        }
        Some(other) => {
            eprintln!("unknown command {other}");
            eprintln!("usage: agg-poc [doctor|pin|core-check --archive FILE --binary FILE|apply --key KEY|preview --key KEY|rollback|status|sync|sync-auth [--confirm]|oauth-begin --provider codex|antigravity [--accept-file-tokens] [--open]|start|stop]");
            exit(2);
        }
    }
}

fn default_codex_config() -> PathBuf {
    let env_codex = std::env::var("CODEX_HOME").ok();
    let env_home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
    match agg_core::codex::detect_codex_home(env_codex.as_deref(), env_home.as_deref(), std::env::consts::OS) {
        Ok(info) => info.cli_config_path,
        Err(err) => {
            eprintln!("{err}");
            exit(1);
        }
    }
}

fn read_apply_request(args: &mut impl Iterator<Item = String>) -> agg_core::codex::ApplyRequest {
    let mut config_path = None;
    let mut base_url = None;
    let mut gateway_key = None;
    let mut model = None;
    let mut catalog_path = None;
    let mut allow_override = true;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config_path = args.next().map(PathBuf::from),
            "--base-url" => base_url = args.next(),
            "--key" => gateway_key = args.next(),
            "--model" => model = args.next(),
            "--catalog" => catalog_path = args.next().map(PathBuf::from),
            "--allow-override" => allow_override = true,
            other => {
                eprintln!("unknown argument {other}");
                exit(2);
            }
        }
    }
    let Some(gateway_key) = gateway_key else {
        eprintln!("--key is required");
        exit(2);
    };
    agg_core::codex::ApplyRequest {
        config_path: config_path.unwrap_or_else(default_codex_config),
        base_url: base_url.unwrap_or_else(|| "http://127.0.0.1:50833/v1".into()),
        gateway_key,
        model,
        allow_foreign_override: allow_override,
        model_catalog_json: catalog_path,
    }
}

fn begin_provider_login(provider: &str, open: bool) -> Result<(), String> {
    use agg_core::oauth::{AuthState, Provider};
    use agg_core::oauth_client::OAuthClient;
    if provider != "codex" && provider != "antigravity" {
        return Err("unsupported provider".into());
    }
    if !open {
        println!("已接受檔案權杖儲存。加上 --open 才會開始登入。未印出授權網址，也未連線。");
        return Ok(());
    }
    let app_data = agg_core::instance::app_data_dir()?;
    let (gateway_key, management_key) = agg_core::instance::ensure_local_keys(&app_data)?;
    let port = std::env::var("APP_AGG_PORT").ok().and_then(|value| value.parse().ok()).unwrap_or(50833u16);
    let bearer = format!("Bearer {management_key}");
    let redactor = agg_core::redact::Redactor::new([management_key.clone(), gateway_key.clone()]);
    let healthy = agg_core::http::exchange(port, "GET", "/v8/management/credentials", &[("Authorization", &bearer)], std::time::Duration::from_millis(800), &redactor)
        .map(|resp| resp.status == 200)
        .unwrap_or(false);
    if !healthy {
        let binary = agg_core::instance::find_core_binary()?;
        let archive = agg_core::instance::archive_beside(&binary)?;
        let mut spawned = agg_core::supervisor::SidecarLaunch::PocStart.spawn(&archive, agg_core::supervisor::StartRequest {
            program: binary,
            work_dir: app_data.join("runtime"),
            port: Some(port),
            extra_env: vec![],
            health_timeout: std::time::Duration::from_secs(45),
            gateway_key,
            management_password: management_key.clone(),
        }).map_err(|err| err.message().to_string())?;
        if let Some(pid) = spawned.supervisor.pid() {
            let _ = std::fs::write(app_data.join("gateway.pid"), pid.to_string());
        }
        spawned.supervisor.detach();
    }
    let kind = match provider {
        "codex" => Provider::ChatGptCodex,
        "antigravity" => Provider::Antigravity,
        _ => return Err("unsupported provider".into()),
    };
    let mut client = OAuthClient::new(port, management_key, kind);
    client.session.acknowledge_terms();
    let url = client.begin().map_err(|_| "授權啟動失敗，流程已停止".to_string())?;
    if !open {
        let _ = client.cancel();
        println!("已接受檔案權杖儲存，但未開啟瀏覽器。加上 --open 才會開始登入。未印出授權網址。");
        return Ok(());
    }
    if !open_browser(&url) {
        let _ = client.cancel();
        return Err("無法開啟系統瀏覽器，授權已取消".into());
    }
    println!("已開啟系統瀏覽器。未印出授權網址。權杖只會寫入閘道自己的 auth 目錄。");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    loop {
        if std::time::Instant::now() > deadline {
            let _ = client.cancel();
            return Err("授權等待逾時".into());
        }
        match client.poll_user_present() {
            Ok(AuthState::Authorizing) => std::thread::sleep(std::time::Duration::from_secs(1)),
            Ok(AuthState::AwaitingConfirmation) => {
                println!("上游回報完成。請在視窗按確認。桌面 Codex 的登入檔沒有被寫入。");
                return Ok(());
            }
            Ok(AuthState::Expired) => return Err("授權已逾時".into()),
            Ok(_) => return Err("授權失敗或遭拒絕".into()),
            Err(_) => return Err("授權失敗或遭拒絕".into()),
        }
    }
}

fn open_browser(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("/usr/bin/open");
    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("cmd");
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    {
        command.args(["/C", "start", "", url]);
    }
    #[cfg(not(target_os = "windows"))]
    {
        command.arg(url);
    }
    command.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
