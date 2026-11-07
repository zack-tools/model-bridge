//! Headless PoC entry. The Tauri window is a separate crate and is not linked here.

use agg_core::check::{run_pinned_core_check, sanitize_report};
use agg_core::pin::pinned_core;
use agg_core::platform::assess_platform;
use std::env;
use std::path::PathBuf;
use std::process::exit;

fn main() {
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
            let config_path = config_path.unwrap_or_else(|| {
                let env_codex = std::env::var("CODEX_HOME").ok();
                let env_home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
                let os = std::env::consts::OS;
                agg_core::codex::detect_codex_home(env_codex.as_deref(), env_home.as_deref(), os)
                    .expect("detect codex home")
                    .cli_config_path
            });
            let base_url = base_url.unwrap_or_else(|| "http://127.0.0.1:50833/v1".into());
            let gateway_key = gateway_key.expect("--key is required");
            let req = agg_core::codex::ApplyRequest {
                config_path: config_path.clone(),
                base_url,
                gateway_key,
                model,
                allow_foreign_override: allow_override,
                model_catalog_json: catalog_path,
            };
            match agg_core::codex::apply_config(&req) {
                Ok(()) => {
                    println!("Successfully applied to {}", config_path.display());
                    if let (Ok(app_data), Some(parent)) = (agg_core::instance::default_app_data_dir(), config_path.parent()) {
                        let auth_dir = app_data.join("runtime").join("auth");
                        if let Ok(summary) = agg_core::codex::sync_chatgpt_desktop_auth(&auth_dir, parent) {
                            println!("ChatGPT Desktop 登入憑證：{}", summary.message);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("apply failed: {e}");
                    exit(1);
                }
            }
        }
        Some("status") => {
            let app_data = agg_core::instance::default_app_data_dir().unwrap_or_else(|e| {
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

                    let (chatgpt_desktop_logged_in, desktop_account) = if let Some(parent) = codex_status.as_ref().map(|s| &s.config_path).and_then(|p| p.parent()) {
                        let auth_file = parent.join("auth.json");
                        if auth_file.exists() {
                            std::fs::read_to_string(&auth_file).ok().and_then(|c| {
                                serde_json::from_str::<serde_json::Value>(&c).ok().and_then(|v| {
                                    let has_token = v.get("tokens").and_then(|t| t.get("access_token")).and_then(|s| s.as_str()).map(|s| !s.is_empty()).unwrap_or(false);
                                    let acc_id = v.get("tokens").and_then(|t| t.get("account_id")).and_then(|s| s.as_str()).map(str::to_string);
                                    let mode = v.get("auth_mode").and_then(|m| m.as_str()).unwrap_or("");
                                    if has_token && mode == "chatgpt" {
                                        Some((true, acc_id))
                                    } else {
                                        None
                                    }
                                })
                            }).unwrap_or((false, None))
                        } else {
                            (false, None)
                        }
                    } else {
                        (false, None)
                    };

                    let report = serde_json::json!({
                        "status": "healthy",
                        "port": port,
                        "base_url": format!("http://127.0.0.1:{port}/v1"),
                        "active_accounts": providers,
                        "account_count": files.len(),
                        "available_models_count": model_count,
                        "chatgpt_desktop_integrated": codex_status.as_ref().map(|s| s.managed_by_us).unwrap_or(false),
                        "chatgpt_desktop_current_model": codex_status.as_ref().and_then(|s| s.current_model.clone()),
                        "chatgpt_desktop_logged_in": chatgpt_desktop_logged_in,
                        "chatgpt_desktop_account_id": desktop_account,
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
            let app_data = agg_core::instance::default_app_data_dir().unwrap_or_else(|e| {
                eprintln!("{e}");
                exit(1);
            });
            let (_gw, management_key) = agg_core::instance::ensure_local_keys(&app_data).unwrap_or_else(|e| {
                eprintln!("無法讀取本機金鑰：{e}");
                exit(1);
            });
            let port = 50833u16;
            let mut all_models = Vec::new();
            if let Ok(index) = agg_core::attribution::single_active_index(port, &management_key, agg_core::registry::SourceKind::ChatGptCodex) {
                if let Ok((_, models)) = agg_core::attribution::discover_models(port, &management_key, &index) {
                    for m in models {
                        if !all_models.contains(&m) { all_models.push(m); }
                    }
                }
            }
            if let Ok(index) = agg_core::attribution::single_active_index(port, &management_key, agg_core::registry::SourceKind::Antigravity) {
                if let Ok((_, models)) = agg_core::attribution::discover_models(port, &management_key, &index) {
                    for m in models {
                        if !all_models.contains(&m) { all_models.push(m); }
                    }
                }
            }
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
        Some("sync-auth") => {
            let app_data = agg_core::instance::default_app_data_dir().unwrap_or_else(|e| {
                eprintln!("{e}");
                exit(1);
            });
            let runtime_auth = app_data.join("runtime").join("auth");
            let env_codex = std::env::var("CODEX_HOME").ok();
            let env_home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
            let os = std::env::consts::OS;
            let info = agg_core::codex::detect_codex_home(env_codex.as_deref(), env_home.as_deref(), os).expect("detect codex home");
            let codex_home = info.cli_config_path.parent().expect("codex home directory");
            match agg_core::codex::sync_chatgpt_desktop_auth(&runtime_auth, codex_home) {
                Ok(summary) => {
                    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
                }
                Err(e) => {
                    eprintln!("同步 ChatGPT Desktop 憑證失敗：{e}");
                    exit(1);
                }
            }
        }
        Some("start") => {
            let app_data = agg_core::instance::default_app_data_dir().unwrap_or_else(|e| {
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
            let runtime_dir = app_data.join("runtime");
            let auth_dir = runtime_dir.join("auth");
            let _ = std::fs::create_dir_all(&auth_dir);

            let policy = agg_core::corecfg::ListenPolicy::new("127.0.0.1", port).unwrap();
            let paths = agg_core::corecfg::write_managed_config(&runtime_dir, &policy, &gateway_key, &management_key).unwrap_or_else(|e| {
                eprintln!("無法寫入閘道配置：{e}");
                exit(1);
            });

            let stdout_log = runtime_dir.join("stdout.log");
            let stderr_log = runtime_dir.join("stderr.log");
            let stdout_file = std::fs::OpenOptions::new().create(true).append(true).open(&stdout_log).unwrap();
            let stderr_file = std::fs::OpenOptions::new().create(true).append(true).open(&stderr_log).unwrap();

            let mut cmd = std::process::Command::new(&binary);
            cmd.arg("-config").arg(&paths.config_path).arg("-local-model");
            cmd.current_dir(&runtime_dir);
            cmd.stdin(std::process::Stdio::null());
            cmd.stdout(stdout_file);
            cmd.stderr(stderr_file);

            let child = cmd.spawn().unwrap_or_else(|e| {
                eprintln!("無法啟動 sidecar 行程：{e}");
                exit(1);
            });

            let pid = child.id();
            let _ = std::fs::write(app_data.join("gateway.pid"), pid.to_string());

            let start = std::time::Instant::now();
            let mut ok = false;
            while start.elapsed() < std::time::Duration::from_secs(10) {
                std::thread::sleep(std::time::Duration::from_millis(300));
                if let Ok(resp) = agg_core::http::exchange(port, "GET", "/v8/management/credentials", &[("Authorization", &bearer)], std::time::Duration::from_millis(500), &redactor) {
                    if resp.status == 200 {
                        ok = true;
                        break;
                    }
                }
            }

            if ok {
                println!("閘道啟動成功！PID: {pid}，監聽 127.0.0.1:{port}");
                let env_codex = std::env::var("CODEX_HOME").ok();
                let env_home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
                let os = std::env::consts::OS;
                if let Ok(info) = agg_core::codex::detect_codex_home(env_codex.as_deref(), env_home.as_deref(), os) {
                    let cat_path = info.cli_config_path.with_file_name("model_catalog.json");
                    let req = agg_core::codex::ApplyRequest {
                        config_path: info.cli_config_path.clone(),
                        base_url: format!("http://127.0.0.1:{port}/v1"),
                        gateway_key: gateway_key.clone(),
                        model: None,
                        allow_foreign_override: true,
                        model_catalog_json: if cat_path.exists() { Some(cat_path) } else { None },
                    };
                    let _ = agg_core::codex::apply_config(&req);
                    println!("已自動將聚合端點套用至 {}", info.cli_config_path.display());
                    if let Some(parent) = info.cli_config_path.parent() {
                        if let Ok(summary) = agg_core::codex::sync_chatgpt_desktop_auth(&auth_dir, parent) {
                            println!("已自動同步 ChatGPT Desktop 登入狀態：{}", summary.message);
                        }
                    }
                }
            } else {
                eprintln!("閘道啟動後未在逾時前通過健康檢查，請檢視日誌：{}", stderr_log.display());
                exit(1);
            }
        }
        Some("stop") => {
            let app_data = agg_core::instance::default_app_data_dir().unwrap_or_else(|e| {
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
            eprintln!("usage: agg-poc [doctor|pin|core-check --archive FILE --binary FILE|apply --key KEY|status|sync|sync-auth|start|stop]");
            exit(2);
        }
    }
}
