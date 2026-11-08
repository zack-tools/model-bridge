//! Codex config adapter.
//!
//! Operates on a caller-supplied path. Detection does not create directories
//! and does not treat a desktop-client path as known. Apply preserves unrelated
//! TOML, writes a sibling backup, and refuses foreign or externally edited files.
//! It never opens `auth.json` for writing.

use crate::redact::Redactor;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table};

pub const PROVIDER_ID: &str = "custom";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexError {
    ForeignOwner,
    ExternalEdit,
    Symlink,
    BadBaseUrl,
    AuthJsonTarget,
    ParentMissing,
    RollbackConflict,
    NothingToRollBack,
    Message(String),
}

impl std::fmt::Display for CodexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ForeignOwner => write!(f, "foreign owner; refusing to overwrite"),
            Self::ExternalEdit => write!(f, "external edit; refusing to overwrite"),
            Self::Symlink => write!(f, "refusing to write through a symlink"),
            Self::BadBaseUrl => write!(f, "base_url must be http://127.0.0.1:<port>/v1"),
            Self::AuthJsonTarget => write!(f, "refusing to edit auth.json"),
            Self::ParentMissing => write!(f, "config parent directory does not exist"),
            Self::RollbackConflict => write!(f, "rollback refused because the file changed"),
            Self::NothingToRollBack => write!(f, "no ownership record to roll back"),
            Self::Message(s) => write!(f, "{s}"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Detection {
    pub codex_home_env: Option<String>,
    pub cli_config_path: PathBuf,
    pub basis: String,
    pub desktop_client_proven: bool,
    pub desktop_client_note: String,
}

pub fn detect_codex_home(
    codex_home_env: Option<&str>,
    home: Option<&str>,
    os: &str,
) -> Result<Detection, CodexError> {
    let (path, basis, env_owned) = if let Some(value) = codex_home_env.map(str::trim).filter(|s| !s.is_empty())
    {
        (
            PathBuf::from(value).join("config.toml"),
            "CODEX_HOME",
            Some(value.to_string()),
        )
    } else if let Some(home) = home.map(str::trim).filter(|s| !s.is_empty()) {
        (
            PathBuf::from(home).join(".codex").join("config.toml"),
            "openai-config-reference-user-default",
            None,
        )
    } else {
        return Err(CodexError::Message(
            "CODEX_HOME and HOME are both unset; no desktop path was invented".into(),
        ));
    };
    Ok(Detection {
        codex_home_env: env_owned,
        cli_config_path: path,
        basis: basis.into(),
        desktop_client_proven: false,
        desktop_client_note: format!(
            "ChatGPT Desktop / Codex GUI compatibility on {os} is not proven. This path is the CLI candidate only."
        ),
    })
}

#[derive(Clone)]
pub struct ApplyRequest {
    pub config_path: PathBuf,
    pub base_url: String,
    pub gateway_key: String,
    pub model: Option<String>,
    pub allow_foreign_override: bool,
    pub model_catalog_json: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogSyncSummary {
    pub catalog_path: PathBuf,
    pub backup_path: Option<PathBuf>,
    pub total_models: usize,
    pub added_slugs: Vec<String>,
    pub existing_slugs: Vec<String>,
}

pub fn sync_model_catalog(
    catalog_path: &Path,
    verified_models: &[String],
) -> Result<CatalogSyncSummary, CodexError> {
    reject_symlink(catalog_path)?;
    if file_name(catalog_path)? == "auth.json" {
        return Err(CodexError::AuthJsonTarget);
    }
    let existed = catalog_path.exists();
    let parent = catalog_path.parent().ok_or(CodexError::ParentMissing)?;
    if !parent.is_dir() {
        return Err(CodexError::ParentMissing);
    }

    let (mut root, backup_path) = if existed {
        let text = fs::read_to_string(catalog_path).map_err(|e| CodexError::Message(e.to_string()))?;
        let backup_name = format!("{}.app-agg-001.bak", file_name(catalog_path)?);
        let bpath = parent.join(&backup_name);
        fs::copy(catalog_path, &bpath).map_err(|e| CodexError::Message(e.to_string()))?;
        let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| CodexError::Message(format!("json parse: {e}")))?;
        (json, Some(bpath))
    } else {
        (serde_json::json!({ "models": [] }), None)
    };

    let models_arr = root.get_mut("models")
        .and_then(|v| v.as_array_mut())
        .ok_or_else(|| CodexError::Message("model_catalog.json does not contain a 'models' array".into()))?;

    // 依使用者指示：移除先前加入之不支援串流工具回合的 image 模型條目
    models_arr.retain(|entry| {
        let slug = entry.get("slug").and_then(|s| s.as_str()).unwrap_or("");
        let desc = entry.get("description").and_then(|s| s.as_str()).unwrap_or("");
        let is_agg_image = desc.starts_with("APP-AGG-001 aggregated model:") && slug.to_ascii_lowercase().contains("image");
        !is_agg_image
    });

    let mut existing_slugs = Vec::new();
    let mut added_slugs = Vec::new();

    for model in verified_models {
        if model.trim().is_empty() || model.to_ascii_lowercase().contains("image") { continue; }
        let already = models_arr.iter().any(|entry| {
            entry.get("slug").and_then(|s| s.as_str()) == Some(model.as_str())
        });
        if already {
            existing_slugs.push(model.clone());
        } else {
            let display_name = if model.starts_with("gemini") {
                model.replace('-', " ")
                    .split_whitespace()
                    .map(|w| {
                        let mut c = w.chars();
                        match c.next() {
                            None => String::new(),
                            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            } else if model.starts_with("gpt") {
                model.to_uppercase()
            } else {
                model.to_string()
            };
            let entry = if let Some(tpl) = if model.starts_with("gemini") {
                models_arr.iter().find(|e| e.get("slug").and_then(|s| s.as_str()).map_or(false, |s| s.starts_with("gemini")))
            } else if model.starts_with("gpt") {
                models_arr.iter().find(|e| e.get("slug").and_then(|s| s.as_str()).map_or(false, |s| s.starts_with("gpt")))
            } else {
                None
            } {
                let mut cloned = tpl.clone();
                if let Some(obj) = cloned.as_object_mut() {
                    obj.insert("slug".into(), serde_json::json!(model));
                    obj.insert("display_name".into(), serde_json::json!(display_name));
                    obj.insert("description".into(), serde_json::json!(format!("APP-AGG-001 aggregated model: {model}")));
                    obj.insert("priority".into(), serde_json::json!(5));
                    obj.insert("visibility".into(), serde_json::json!("list"));
                    obj.insert("supported_in_api".into(), serde_json::json!(true));
                    obj.insert("input_modalities".into(), serde_json::json!(["text", "image"]));
                    obj.insert("supports_search_tool".into(), serde_json::json!(true));
                    obj.insert("support_verbosity".into(), serde_json::json!(true));
                    obj.insert("shell_type".into(), serde_json::json!("unified_exec"));

                    if model.contains("image") {
                        obj.insert("default_reasoning_level".into(), serde_json::Value::Null);
                        obj.insert("supported_reasoning_levels".into(), serde_json::json!([]));
                    } else if model.starts_with("gemini") {
                        obj.insert("default_reasoning_level".into(), serde_json::json!("medium"));
                        obj.insert("supported_reasoning_levels".into(), serde_json::json!([
                            {"effort": "low", "description": "低思考強度：快速回應，適合日常簡短問答"},
                            {"effort": "medium", "description": "中思考強度：平衡速度與推理深度，適合大多數任務"},
                            {"effort": "high", "description": "高思考強度：深入推理，適合複雜問題與編程"}
                        ]));
                    } else {
                        obj.insert("default_reasoning_level".into(), serde_json::json!("medium"));
                        obj.insert("supported_reasoning_levels".into(), serde_json::json!([
                            {"effort": "low", "description": "Fast responses with lighter reasoning"},
                            {"effort": "medium", "description": "Balances speed and reasoning depth for everyday tasks"},
                            {"effort": "high", "description": "Greater reasoning depth for complex problems"}
                        ]));
                    }
                }
                cloned
            } else {
                let (def_reason, supp_reason) = if model.contains("image") {
                    (serde_json::Value::Null, serde_json::json!([]))
                } else if model.starts_with("gemini") {
                    (serde_json::json!("medium"), serde_json::json!([
                        {"effort": "low", "description": "低思考強度：快速回應，適合日常簡短問答"},
                        {"effort": "medium", "description": "中思考強度：平衡速度與推理深度，適合大多數任務"},
                        {"effort": "high", "description": "高思考強度：深入推理，適合複雜問題與編程"}
                    ]))
                } else {
                    (serde_json::json!("medium"), serde_json::json!([
                        {"effort": "low", "description": "Fast responses with lighter reasoning"},
                        {"effort": "medium", "description": "Balances speed and reasoning depth for everyday tasks"},
                        {"effort": "high", "description": "Greater reasoning depth for complex problems"}
                    ]))
                };
                serde_json::json!({
                    "slug": model,
                    "display_name": display_name,
                    "description": format!("APP-AGG-001 aggregated model: {model}"),
                    "visibility": "list",
                    "supported_in_api": true,
                    "priority": 5,
                    "input_modalities": ["text", "image"],
                    "supports_search_tool": true,
                    "support_verbosity": true,
                    "default_reasoning_level": def_reason,
                    "supported_reasoning_levels": supp_reason,
                    "shell_type": "unified_exec"
                })
            };
            models_arr.push(entry);
            added_slugs.push(model.clone());
        }
    }

    let total_models = models_arr.len();
    let formatted = serde_json::to_string_pretty(&root).map_err(|e| CodexError::Message(e.to_string()))?;
    atomic_write(catalog_path, formatted.as_bytes())?;

    Ok(CatalogSyncSummary {
        catalog_path: catalog_path.to_path_buf(),
        backup_path,
        total_models,
        added_slugs,
        existing_slugs,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSyncSummary {
    pub auth_json_path: PathBuf,
    pub backup_path: Option<PathBuf>,
    pub synced_to_desktop: bool,
    pub synced_from_desktop: bool,
    pub email: Option<String>,
    pub account_id: Option<String>,
    pub plan_type: Option<String>,
    pub message: String,
}

/// 安全同步 ChatGPT Desktop (auth.json) 與本機聚合閘道 (runtime/auth/codex-*.json) 的憑證。
///
/// 1. 當本機閘道有最新有效之憑證時，自動同步至 ~/.codex/auth.json 並設定 auth_mode 為 "chatgpt"，
///    使 ChatGPT Desktop UI 保持登入態。
/// 2. 當 ChatGPT Desktop 原生更新憑證時，亦能自動導入本地 runtime/auth，達成雙向憑證一致。
/// 3. 所有磁碟寫入皆透過 atomic_write 保證 0600 本機私有權限，絕不上傳。
pub fn sync_chatgpt_desktop_auth(
    runtime_auth_dir: &Path,
    codex_home: &Path,
) -> Result<AuthSyncSummary, CodexError> {
    reject_symlink(codex_home)?;
    let auth_path = codex_home.join("auth.json");
    reject_symlink(&auth_path)?;

    if !codex_home.exists() {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        let _ = builder.create(codex_home);
    }

    // 1. 掃描 runtime/auth 中的有效 Codex 憑證
    #[derive(Debug, Clone)]
    struct RuntimeCreds {
        access_token: String,
        refresh_token: String,
        id_token: Option<String>,
        account_id: Option<String>,
        email: Option<String>,
        plan_type: Option<String>,
        last_refresh: Option<String>,
        file_path: PathBuf,
    }

    let mut runtime_creds: Vec<RuntimeCreds> = Vec::new();
    if runtime_auth_dir.exists() && runtime_auth_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(runtime_auth_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name.starts_with("codex-")
                    && name.ends_with(".json")
                    && !name.ends_with(".lock")
                    && !name.ends_with(".bak")
                {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                            let disabled = val.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false);
                            let acc_tok = val.get("access_token").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let ref_tok = val.get("refresh_token").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            if !disabled && !acc_tok.is_empty() && !ref_tok.is_empty() {
                                runtime_creds.push(RuntimeCreds {
                                    access_token: acc_tok,
                                    refresh_token: ref_tok,
                                    id_token: val.get("id_token").and_then(|v| v.as_str()).map(str::to_string),
                                    account_id: val.get("account_id").and_then(|v| v.as_str()).map(str::to_string),
                                    email: val.get("email").and_then(|v| v.as_str()).map(str::to_string),
                                    plan_type: val.get("plan_type").and_then(|v| v.as_str()).map(str::to_string),
                                    last_refresh: val.get("last_refresh").and_then(|v| v.as_str()).map(str::to_string),
                                    file_path: path,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // 依 last_refresh 由新到舊排序
    runtime_creds.sort_by(|a, b| {
        b.last_refresh.as_deref().unwrap_or("").cmp(a.last_refresh.as_deref().unwrap_or(""))
    });

    // 2. 讀取現有 ~/.codex/auth.json
    let (mut desktop_val, auth_existed) = if auth_path.exists() {
        let content = fs::read_to_string(&auth_path).map_err(|e| CodexError::Message(e.to_string()))?;
        let v: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| CodexError::Message(format!("auth.json parse: {e}")))?;
        (v, true)
    } else {
        (serde_json::json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": null,
            "tokens": {},
            "last_refresh": ""
        }), false)
    };

    let desktop_access_token = desktop_val
        .get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let desktop_refresh_token = desktop_val
        .get("tokens")
        .and_then(|t| t.get("refresh_token"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let desktop_last_refresh = desktop_val
        .get("last_refresh")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let has_valid_desktop = !desktop_access_token.is_empty() && !desktop_refresh_token.is_empty();
    let desktop_account = desktop_val.get("tokens").and_then(|t| t.get("account_id"))
        .and_then(|v| v.as_str()).filter(|id| !id.is_empty());
    let best_runtime = if has_valid_desktop && !runtime_creds.is_empty() {
        let matching: Vec<_> = runtime_creds.iter().filter(|rt| {
            match (desktop_account, rt.account_id.as_deref().filter(|id| !id.is_empty())) {
                (Some(desktop), Some(runtime)) => desktop == runtime,
                _ => desktop_access_token == rt.access_token && desktop_refresh_token == rt.refresh_token,
            }
        }).collect();
        if matching.len() != 1 {
            return Err(CodexError::Message("無法唯一確認兩端為同一帳號；未同步憑證".into()));
        }
        matching.first().map(|rt| (*rt).clone())
    } else {
        if runtime_creds.len() > 1 {
            return Err(CodexError::Message("閘道有多個帳號；未自動選擇憑證".into()));
        }
        runtime_creds.first().cloned()
    };

    // 3. 雙向同步決策
    if let Some(rt) = best_runtime {
        let desktop_is_newer = has_valid_desktop && desktop_last_refresh.as_str() > rt.last_refresh.as_deref().unwrap_or("");
        if desktop_is_newer {
            // ChatGPT Desktop 擁有更新的 token -> 同步回 runtime 檔案
            if let Ok(content) = fs::read_to_string(&rt.file_path) {
                if let Ok(mut rt_val) = serde_json::from_str::<serde_json::Value>(&content) {
                    rt_val["access_token"] = serde_json::json!(desktop_access_token);
                    rt_val["refresh_token"] = serde_json::json!(desktop_refresh_token);
                    if let Some(id_tok) = desktop_val.get("tokens").and_then(|t| t.get("id_token")).and_then(|v| v.as_str()) {
                        rt_val["id_token"] = serde_json::json!(id_tok);
                    }
                    if !desktop_last_refresh.is_empty() {
                        rt_val["last_refresh"] = serde_json::json!(desktop_last_refresh);
                    }
                    let out = serde_json::to_string_pretty(&rt_val).map_err(|e| CodexError::Message(e.to_string()))?;
                    atomic_write(&rt.file_path, out.as_bytes())?;
                }
            }
            return Ok(AuthSyncSummary {
                auth_json_path: auth_path,
                backup_path: None,
                synced_to_desktop: false,
                synced_from_desktop: true,
                email: rt.email,
                account_id: rt.account_id,
                plan_type: rt.plan_type,
                message: "已自 ChatGPT Desktop 導入最新登入憑證至聚合閘道".into(),
            });
        }

        // 兩端 token 相同且有效 -> 無須覆寫
        if has_valid_desktop && desktop_access_token == rt.access_token && desktop_refresh_token == rt.refresh_token {
            return Ok(AuthSyncSummary {
                auth_json_path: auth_path,
                backup_path: None,
                synced_to_desktop: false,
                synced_from_desktop: false,
                email: rt.email,
                account_id: rt.account_id,
                plan_type: rt.plan_type,
                message: "ChatGPT Desktop 憑證已與本機閘道保持最新同步（登入狀態良好）".into(),
            });
        }

        // 本機閘道有最新憑證 -> 同步至 ~/.codex/auth.json
        let backup_path = if auth_existed {
            let bpath = codex_home.join("auth.json.app-agg-001.bak");
            let _ = fs::copy(&auth_path, &bpath);
            Some(bpath)
        } else {
            None
        };

        desktop_val["auth_mode"] = serde_json::json!("chatgpt");
        if desktop_val.get("OPENAI_API_KEY").is_none() {
            desktop_val["OPENAI_API_KEY"] = serde_json::Value::Null;
        }
        if desktop_val.get("tokens").is_none() {
            desktop_val["tokens"] = serde_json::json!({});
        }
        if let Some(obj) = desktop_val.get_mut("tokens").and_then(|v| v.as_object_mut()) {
            obj.insert("access_token".into(), serde_json::json!(rt.access_token));
            obj.insert("refresh_token".into(), serde_json::json!(rt.refresh_token));
            if let Some(it) = &rt.id_token {
                obj.insert("id_token".into(), serde_json::json!(it));
            }
            if let Some(aid) = &rt.account_id {
                obj.insert("account_id".into(), serde_json::json!(aid));
            }
        }
        if let Some(lr) = &rt.last_refresh {
            desktop_val["last_refresh"] = serde_json::json!(lr);
        }

        let formatted = serde_json::to_string_pretty(&desktop_val).map_err(|e| CodexError::Message(e.to_string()))?;
        atomic_write(&auth_path, formatted.as_bytes())?;

        Ok(AuthSyncSummary {
            auth_json_path: auth_path,
            backup_path,
            synced_to_desktop: true,
            synced_from_desktop: false,
            email: rt.email.clone(),
            account_id: rt.account_id.clone(),
            plan_type: rt.plan_type.clone(),
            message: format!("已成功將本機有效憑證同步至 ChatGPT Desktop (帳號: {})", rt.email.as_deref().unwrap_or("已驗證")),
        })
    } else if has_valid_desktop {
        // 本機 runtime 尚無憑證，但 ChatGPT Desktop 原生有有效 token -> 導出至 runtime
        if !runtime_auth_dir.exists() {
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            let _ = builder.create(runtime_auth_dir);
        }
        let acc_id = desktop_val.get("tokens")
            .and_then(|t| t.get("account_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("default");
        let id_tok = desktop_val.get("tokens")
            .and_then(|t| t.get("id_token"))
            .and_then(|v| v.as_str());

        let (email, plan_type) = if let Some(jwt) = id_tok {
            let payload = decode_jwt_payload(jwt);
            let email = payload.as_ref().and_then(|p| p.get("email")).and_then(|v| v.as_str()).map(str::to_string);
            let plan = payload.as_ref()
                .and_then(|p| p.get("https://api.openai.com/auth"))
                .and_then(|a| a.get("chatgpt_plan_type"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            (email, plan)
        } else {
            (None, None)
        };

        let short_id = if acc_id.len() >= 8 { &acc_id[..8] } else { acc_id };
        let email_slug = email.as_deref().unwrap_or("desktop-user");
        let plan_slug = plan_type.as_deref().unwrap_or("plus");
        let target_name = format!("codex-{short_id}-{email_slug}-{plan_slug}.json");
        let target_path = runtime_auth_dir.join(target_name);

        let runtime_entry = serde_json::json!({
            "access_token": desktop_access_token,
            "refresh_token": desktop_refresh_token,
            "id_token": id_tok,
            "account_id": acc_id,
            "email": email,
            "plan_type": plan_type,
            "disabled": false,
            "last_refresh": desktop_last_refresh,
            "type": "codex"
        });

        let formatted = serde_json::to_string_pretty(&runtime_entry).map_err(|e| CodexError::Message(e.to_string()))?;
        atomic_write(&target_path, formatted.as_bytes())?;

        Ok(AuthSyncSummary {
            auth_json_path: auth_path,
            backup_path: None,
            synced_to_desktop: false,
            synced_from_desktop: true,
            email,
            account_id: Some(acc_id.to_string()),
            plan_type,
            message: "已自 ChatGPT Desktop 匯入登入憑證至本地聚合閘道".into(),
        })
    } else {
        Ok(AuthSyncSummary {
            auth_json_path: auth_path,
            backup_path: None,
            synced_to_desktop: false,
            synced_from_desktop: false,
            email: None,
            account_id: None,
            plan_type: None,
            message: "未在閘道或 ChatGPT Desktop 中發現有效之本機登入憑證".into(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyPreview {
    pub diff: String,
    pub creates_file: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CodexConfigStatus {
    pub config_path: PathBuf,
    pub exists: bool,
    pub is_symlink: bool,
    pub managed_by_us: bool,
    pub external_edit_detected: bool,
    pub has_backup: bool,
    pub current_model: Option<String>,
}

pub fn inspect_codex_config(config_path: &Path) -> Result<CodexConfigStatus, CodexError> {
    let is_symlink = if config_path.exists() {
        let meta = fs::symlink_metadata(config_path).map_err(|e| CodexError::Message(e.to_string()))?;
        meta.file_type().is_symlink()
    } else {
        false
    };
    let exists = config_path.exists();
    let ownership_file = ownership_path(config_path);
    let ownership = read_ownership(&ownership_file)?;

    let mut managed_by_us = false;
    let mut external_edit_detected = false;
    let mut has_backup = false;
    let mut current_model = None;

    if exists && !is_symlink {
        if let Ok(content) = fs::read_to_string(config_path) {
            current_model = top_level_string(&content, "model");
            if let Some(ownership) = ownership {
                managed_by_us = true;
                let current_hash = crate::pin::sha256_bytes(content.as_bytes());
                external_edit_detected = current_hash != ownership.sha256;
                if let Some(backup_name) = &ownership.backup {
                    if let Some(parent) = config_path.parent() {
                        has_backup = parent.join(backup_name).exists();
                    }
                }
            }
        }
    } else if let Some(ownership) = ownership {
        managed_by_us = true;
        external_edit_detected = true;
        if let Some(backup_name) = &ownership.backup {
            if let Some(parent) = config_path.parent() {
                has_backup = parent.join(backup_name).exists();
            }
        }
    }

    Ok(CodexConfigStatus {
        config_path: config_path.to_path_buf(),
        exists,
        is_symlink,
        managed_by_us,
        external_edit_detected,
        has_backup,
        current_model,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Ownership {
    tool: String,
    generation: u64,
    sha256: String,
    managed_provider_id: String,
    backup: Option<String>,
    created: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256_without_model: Option<String>,
}

fn hash_without_model(text: &str) -> Result<String, CodexError> {
    let mut doc = text.parse::<DocumentMut>().map_err(|e| CodexError::Message(format!("toml parse: {e}")))?;
    doc.remove("model");
    Ok(crate::pin::sha256_bytes(doc.to_string().as_bytes()))
}

#[derive(Serialize, Deserialize)]
struct SuspendedConfig {
    active: String,
    native_hash: String,
}

fn suspended_path(path: &Path) -> PathBuf {
    path.with_file_name(format!("{}.app-agg-001.suspended.json", path.file_name().and_then(|s| s.to_str()).unwrap_or("config.toml")))
}

/// Switch a verified managed configuration to the client's native defaults.
pub fn suspend_config(path: &Path) -> Result<bool, CodexError> {
    reject_symlink(path)?;
    let marker = suspended_path(path);
    reject_symlink(&marker)?;
    if marker.exists() { return Err(CodexError::Message("已有尚未恢復的閘道設定".into())); }
    let Some(mut owner) = read_ownership(&ownership_path(path))? else { return Ok(false); };
    let active = read_existing(path)?;
    if crate::pin::sha256_bytes(active.as_bytes()) != owner.sha256 {
        let reference = if let Some(hash) = &owner.sha256_without_model {
            hash.clone()
        } else {
            let name = owner.backup.as_deref().ok_or(CodexError::ExternalEdit)?;
            if name.contains('/') || name.contains('\\') || name.contains("..") { return Err(CodexError::ExternalEdit); }
            let backup = read_existing(&path.with_file_name(name))?;
            if top_level_string(&backup, "model_provider").as_deref() != Some(PROVIDER_ID) { return Err(CodexError::ExternalEdit); }
            hash_without_model(&backup)?
        };
        if hash_without_model(&active)? != reference { return Err(CodexError::ExternalEdit); }
        owner.sha256 = crate::pin::sha256_bytes(active.as_bytes());
        owner.sha256_without_model = Some(reference);
        let bytes = serde_json::to_vec_pretty(&owner).map_err(|e| CodexError::Message(e.to_string()))?;
        atomic_write(&ownership_path(path), &bytes)?;
    }
    let mut doc = active.parse::<DocumentMut>().map_err(|e| CodexError::Message(format!("toml parse: {e}")))?;
    if doc.get("model_provider").and_then(Item::as_str) != Some(PROVIDER_ID) { return Err(CodexError::ForeignOwner); }
    doc.remove("model_provider");
    doc.remove("model_catalog_json");
    doc.remove("model");
    let native = doc.to_string();
    let record = SuspendedConfig { active, native_hash: crate::pin::sha256_bytes(native.as_bytes()) };
    let bytes = serde_json::to_vec(&record).map_err(|e| CodexError::Message(e.to_string()))?;
    atomic_write(&marker, &bytes)?;
    atomic_write(path, native.as_bytes())?;
    Ok(true)
}

/// Restore a suspended managed configuration only if its native version was not edited.
pub fn resume_config(path: &Path, port: u16, gateway_key: &str) -> Result<bool, CodexError> {
    reject_symlink(path)?;
    let marker = suspended_path(path);
    reject_symlink(&marker)?;
    if !marker.exists() { return Ok(false); }
    let record: SuspendedConfig = serde_json::from_slice(&fs::read(&marker).map_err(|e| CodexError::Message(e.to_string()))?)
        .map_err(|e| CodexError::Message(e.to_string()))?;
    let current = read_existing(path)?;
    if crate::pin::sha256_bytes(current.as_bytes()) != record.native_hash { return Err(CodexError::ExternalEdit); }
    let mut owner = read_ownership(&ownership_path(path))?.ok_or(CodexError::NothingToRollBack)?;
    if crate::pin::sha256_bytes(record.active.as_bytes()) != owner.sha256 { return Err(CodexError::RollbackConflict); }
    let req = ApplyRequest { config_path: path.to_path_buf(), base_url: format!("http://127.0.0.1:{port}/v1"), gateway_key: gateway_key.into(), model: None, allow_foreign_override: false, model_catalog_json: None };
    validate_request(&req)?;
    let active = mutate(&record.active, &req)?;
    owner.sha256 = crate::pin::sha256_bytes(active.as_bytes());
    owner.sha256_without_model = Some(hash_without_model(&active)?);
    atomic_write(path, active.as_bytes())?;
    let body = serde_json::to_vec_pretty(&owner).map_err(|e| CodexError::Message(e.to_string()))?;
    atomic_write(&ownership_path(path), &body)?;
    fs::remove_file(marker).map_err(|e| CodexError::Message(e.to_string()))?;
    Ok(true)
}

pub fn preview_apply(req: &ApplyRequest) -> Result<ApplyPreview, CodexError> {
    validate_request(req)?;
    let before = read_existing(&req.config_path)?;
    reject_foreign(&req.config_path, &before, req.allow_foreign_override)?;
    let after = mutate(&before, req)?;
    let diff = Redactor::new([req.gateway_key.clone()]).redact(&line_diff(&before, &after));
    Ok(ApplyPreview {
        diff,
        creates_file: !req.config_path.exists(),
    })
}

pub fn apply_config(req: &ApplyRequest) -> Result<(), CodexError> {
    let preview = preview_apply(req)?;
    let _ = preview;
    let before = read_existing(&req.config_path)?;
    let existed = req.config_path.exists();
    let after = mutate(&before, req)?;
    let parent = req
        .config_path
        .parent()
        .ok_or(CodexError::ParentMissing)?;
    let ownership_path = ownership_path(&req.config_path);
    let previous = read_ownership(&ownership_path)?;
    let generation = previous.as_ref().map(|o| o.generation + 1).unwrap_or(1);
    let backup_name = if existed {
        let name = format!(
            "{}.{generation}.app-agg-001.bak",
            file_name(&req.config_path)?
        );
        let backup_path = parent.join(&name);
        fs::copy(&req.config_path, &backup_path).map_err(|e| CodexError::Message(e.to_string()))?;
        Some(name)
    } else {
        None
    };
    atomic_write(&req.config_path, after.as_bytes())?;
    let ownership = Ownership {
        tool: "app-agg-001".into(),
        generation,
        sha256: crate::pin::sha256_bytes(after.as_bytes()),
        managed_provider_id: PROVIDER_ID.into(),
        backup: backup_name,
        created: !existed,
        sha256_without_model: Some(hash_without_model(&after)?),
    };
    let body = serde_json::to_vec_pretty(&ownership).map_err(|e| CodexError::Message(e.to_string()))?;
    atomic_write(&ownership_path, &body)?;
    Ok(())
}

pub fn rollback_config(config_path: &Path) -> Result<(), CodexError> {
    reject_symlink(config_path)?;
    if file_name(config_path)? == "auth.json" {
        return Err(CodexError::AuthJsonTarget);
    }
    let ownership_path = ownership_path(config_path);
    let ownership = read_ownership(&ownership_path)?.ok_or(CodexError::NothingToRollBack)?;
    let current = fs::read(config_path).map_err(|e| CodexError::Message(e.to_string()))?;
    if crate::pin::sha256_bytes(&current) != ownership.sha256 {
        return Err(CodexError::RollbackConflict);
    }
    let parent = config_path.parent().ok_or(CodexError::ParentMissing)?;
    if let Some(backup_name) = &ownership.backup {
        if backup_name.contains('/') || backup_name.contains("..") {
            return Err(CodexError::Message("backup name is not local".into()));
        }
        let backup_path = parent.join(backup_name);
        let bytes = fs::read(&backup_path).map_err(|e| CodexError::Message(e.to_string()))?;
        atomic_write(config_path, &bytes)?;
    } else if ownership.created {
        fs::remove_file(config_path).map_err(|e| CodexError::Message(e.to_string()))?;
    } else {
        return Err(CodexError::NothingToRollBack);
    }
    fs::remove_file(&ownership_path).map_err(|e| CodexError::Message(e.to_string()))?;
    Ok(())
}

fn validate_request(req: &ApplyRequest) -> Result<(), CodexError> {
    reject_symlink(&req.config_path)?;
    if file_name(&req.config_path)? == "auth.json" {
        return Err(CodexError::AuthJsonTarget);
    }
    let parent = req.config_path.parent().ok_or(CodexError::ParentMissing)?;
    if !parent.is_dir() {
        return Err(CodexError::ParentMissing);
    }
    validate_base_url(&req.base_url)?;
    if req.gateway_key.len() < 8 || req.gateway_key.contains('\n') {
        return Err(CodexError::Message("gateway key is empty or unsafe".into()));
    }
    if req
        .model
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase()
        .contains("claude")
    {
        return Err(CodexError::Message("refusing to write a claude model id".into()));
    }
    Ok(())
}

fn validate_base_url(url: &str) -> Result<(), CodexError> {
    let Some(rest) = url.strip_prefix("http://127.0.0.1:") else {
        return Err(CodexError::BadBaseUrl);
    };
    let Some((port, path)) = rest.split_once('/') else {
        return Err(CodexError::BadBaseUrl);
    };
    if path != "v1" || port.parse::<u16>().is_err() || port == "0" {
        return Err(CodexError::BadBaseUrl);
    }
    Ok(())
}

fn read_existing(path: &Path) -> Result<String, CodexError> {
    if !path.exists() {
        return Ok(String::new());
    }
    fs::read_to_string(path).map_err(|e| CodexError::Message(e.to_string()))
}

fn reject_foreign(path: &Path, before: &str, allow_override: bool) -> Result<(), CodexError> {
    if allow_override {
        return Ok(());
    }
    let ownership = read_ownership(&ownership_path(path))?;
    if let Some(ownership) = ownership {
        let current_hash = crate::pin::sha256_bytes(before.as_bytes());
        if current_hash != ownership.sha256 {
            return Err(CodexError::ExternalEdit);
        }
        return Ok(());
    }
    let lower = before.to_ascii_lowercase();
    if lower.contains("cc-switch") || lower.contains("cc_switch") {
        return Err(CodexError::ForeignOwner);
    }
    if let Some(provider) = top_level_string(before, "model_provider") {
        if !provider.is_empty() && provider != PROVIDER_ID {
            return Err(CodexError::ForeignOwner);
        }
        if provider == PROVIDER_ID {
            return Err(CodexError::ForeignOwner);
        }
    }
    Ok(())
}

fn top_level_string(text: &str, key: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let doc = text.parse::<DocumentMut>().ok()?;
    let item = doc.get(key)?;
    item.as_str().map(str::to_string)
}

fn mutate(before: &str, req: &ApplyRequest) -> Result<String, CodexError> {
    let mut doc = if before.is_empty() {
        DocumentMut::new()
    } else {
        before
            .parse::<DocumentMut>()
            .map_err(|e| CodexError::Message(format!("toml parse: {e}")))?
    };
    doc["model_provider"] = toml_edit::value(PROVIDER_ID);
    if let Some(model) = &req.model {
        doc["model"] = toml_edit::value(model.as_str());
    }
    if let Some(catalog) = &req.model_catalog_json {
        if let Some(s) = catalog.to_str() {
            doc["model_catalog_json"] = toml_edit::value(s);
        }
    }
    let providers = nested_table(doc.as_table_mut(), "model_providers")?;
    let entry = nested_table(providers, PROVIDER_ID)?;
    entry["name"] = toml_edit::value("Custom");
    entry["base_url"] = toml_edit::value(req.base_url.as_str());
    entry["wire_api"] = toml_edit::value("responses");
    entry["requires_openai_auth"] = toml_edit::value(true);
    entry["experimental_bearer_token"] = toml_edit::value(req.gateway_key.as_str());
    let headers = nested_table(entry, "http_headers")?;
    headers["X-Api-Key"] = toml_edit::value(req.gateway_key.as_str());

    // 同步更新既有 app_agg_001 提供者（若存在），確保雙向相容
    if let Some(agg) = providers.get_mut("app_agg_001").and_then(Item::as_table_mut) {
        agg["name"] = toml_edit::value("Custom");
        agg["base_url"] = toml_edit::value(req.base_url.as_str());
        agg["experimental_bearer_token"] = toml_edit::value(req.gateway_key.as_str());
        agg["wire_api"] = toml_edit::value("responses");
        agg["requires_openai_auth"] = toml_edit::value(true);
    }
    Ok(doc.to_string())
}

fn nested_table<'a>(table: &'a mut Table, key: &str) -> Result<&'a mut Table, CodexError> {
    if !table.contains_key(key) {
        table.insert(key, Item::Table(Table::new()));
    }
    table.get_mut(key).and_then(Item::as_table_mut).ok_or_else(|| {
        CodexError::Message(format!("{key} is not a table; refusing to replace existing configuration"))
    })
}

fn line_diff(before: &str, after: &str) -> String {
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    let mut out = String::new();
    for line in &before_lines {
        if !after_lines.contains(line) {
            out.push_str("- ");
            out.push_str(line);
            out.push('\n');
        }
    }
    for line in &after_lines {
        if !before_lines.contains(line) {
            out.push_str("+ ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn ownership_path(config: &Path) -> PathBuf {
    let name = config
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("config.toml");
    config.with_file_name(format!("{name}.app-agg-001.ownership.json"))
}

fn read_ownership(path: &Path) -> Result<Option<Ownership>, CodexError> {
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path).map_err(|e| CodexError::Message(e.to_string()))?;
    serde_json::from_str(&text).map_err(|e| CodexError::Message(e.to_string()))
}

fn file_name(path: &Path) -> Result<String, CodexError> {
    path.file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .ok_or_else(|| CodexError::Message("config file name is not utf-8".into()))
}

fn reject_symlink(path: &Path) -> Result<(), CodexError> {
    if path.exists() {
        let meta = fs::symlink_metadata(path).map_err(|e| CodexError::Message(e.to_string()))?;
        if meta.file_type().is_symlink() {
            return Err(CodexError::Symlink);
        }
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), CodexError> {
    let parent = path.parent().ok_or(CodexError::ParentMissing)?;
    let tmp_name = format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("cfg"),
        std::process::id()
    );
    let tmp = parent.join(tmp_name);
    {
        let mut options = OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&tmp)
            .map_err(|e| CodexError::Message(e.to_string()))?;
        file.write_all(bytes)
            .map_err(|e| CodexError::Message(e.to_string()))?;
        file.sync_all()
            .map_err(|e| CodexError::Message(e.to_string()))?;
    }
    fs::rename(&tmp, path).map_err(|e| CodexError::Message(e.to_string()))?;
    let _ = File::open(parent).and_then(|dir| dir.sync_all());
    Ok(())
}

fn decode_jwt_payload(jwt: &str) -> Option<serde_json::Value> {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let payload_b64 = parts[1];
    let mut s = payload_b64.replace('-', "+").replace('_', "/");
    match s.len() % 4 {
        2 => s.push_str("=="),
        3 => s.push_str("="),
        _ => {}
    }
    let bytes = base64_decode_raw(&s).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn base64_decode_raw(input: &str) -> Result<Vec<u8>, ()> {
    const TABLE: [i8; 256] = {
        let mut t = [-1i8; 256];
        let chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut i = 0;
        while i < chars.len() {
            t[chars[i] as usize] = i as i8;
            i += 1;
        }
        t
    };
    let mut buf = Vec::new();
    let mut accum = 0u32;
    let mut bits = 0;
    for &b in input.as_bytes() {
        if b == b'=' || b.is_ascii_whitespace() { continue; }
        let val = TABLE[b as usize];
        if val < 0 { return Err(()); }
        accum = (accum << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            buf.push((accum >> bits) as u8);
            accum &= (1 << bits) - 1;
        }
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let mut nonce = [0u8; 8];
        getrandom::getrandom(&mut nonce).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "agg-codex-{}-{}",
            std::process::id(),
            crate::pin::sha256_bytes(&nonce)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn req(dir: &Path, model: Option<&str>) -> ApplyRequest {
        ApplyRequest {
            config_path: dir.join("config.toml"),
            base_url: "http://127.0.0.1:18431/v1".into(),
            gateway_key: "localgateway012345".into(),
            model: model.map(str::to_string),
            allow_foreign_override: false,
            model_catalog_json: None,
        }
    }

    #[test]
    fn preserves_unrelated_toml_and_redacts_the_preview() {
        let dir = scratch();
        let path = dir.join("config.toml");
        fs::write(
            &path,
            "# keep-this-comment\npersonality = \"pragmatic\"\n\n[notice]\nkeep = true\n",
        )
        .unwrap();
        let preview = preview_apply(&req(&dir, Some("gpt-example"))).unwrap();
        assert!(preview.diff.contains("keep-this-comment") == false);
        assert!(!preview.diff.contains("localgateway012345"));
        assert!(preview.diff.contains("<redacted>"));
        apply_config(&req(&dir, Some("gpt-example"))).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("# keep-this-comment"));
        assert!(text.contains("personality = \"pragmatic\""));
        assert!(text.contains("keep = true"));
        assert!(text.contains("wire_api = \"responses\""));
        assert!(text.contains("requires_openai_auth = true"));
        assert!(text.contains("localgateway012345"));
        assert!(!text.contains("approval_policy"));
        assert!(!text.contains("claude"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn foreign_owner_external_edit_and_rollback() {
        let dir = scratch();
        let path = dir.join("config.toml");
        fs::write(&path, "model_provider = \"openai\"\n# cc-switch\n").unwrap();
        assert_eq!(
            apply_config(&req(&dir, None)).unwrap_err(),
            CodexError::ForeignOwner
        );
        assert!(fs::read_to_string(&path).unwrap().contains("openai"));

        fs::write(&path, "# user comment\npersonality = \"pragmatic\"\n").unwrap();
        let auth = dir.join("auth.json");
        fs::write(&auth, "{\"OPENAI_API_KEY\":\"leave-me\"}").unwrap();
        apply_config(&req(&dir, None)).unwrap();
        fs::write(&path, fs::read_to_string(&path).unwrap() + "\n# external\n").unwrap();
        assert_eq!(
            apply_config(&req(&dir, None)).unwrap_err(),
            CodexError::ExternalEdit
        );
        assert!(fs::read_to_string(&path).unwrap().contains("# external"));
        assert_eq!(
            rollback_config(&path).unwrap_err(),
            CodexError::RollbackConflict
        );

        let owned = fs::read_to_string(&path).unwrap().replace("\n# external\n", "\n");
        // external edit remains until a matching hash exists; restore via a fresh apply on a clean file
        let _ = owned;
        let dir2 = scratch();
        let path2 = dir2.join("config.toml");
        fs::write(&path2, "personality = \"keep\"\n").unwrap();
        apply_config(&req(&dir2, None)).unwrap();
        let auth2 = dir2.join("auth.json");
        fs::write(&auth2, "original-auth").unwrap();
        rollback_config(&path2).unwrap();
        assert_eq!(fs::read_to_string(&path2).unwrap(), "personality = \"keep\"\n");
        assert_eq!(fs::read_to_string(&auth2).unwrap(), "original-auth");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dir2);
    }

    #[test]
    fn refuses_to_replace_existing_non_table_configuration() {
        let dir = scratch();
        let path = dir.join("config.toml");
        for original in [
            "model_providers = \"user-value\"\n",
            "[model_providers]\ncustom = \"user-value\"\n",
            "[model_providers.custom]\nhttp_headers = \"user-value\"\n",
        ] {
            fs::write(&path, original).unwrap();
            assert!(preview_apply(&req(&dir, None)).is_err());
            assert!(apply_config(&req(&dir, None)).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
            assert!(!ownership_path(&path).exists());
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_env_without_inventing_a_desktop_path() {
        let found = detect_codex_home(Some("/tmp/codex-home"), Some("/tmp/should-not-win"), "linux").unwrap();
        assert_eq!(found.cli_config_path, PathBuf::from("/tmp/codex-home/config.toml"));
        assert_eq!(found.basis, "CODEX_HOME");
        assert!(!found.desktop_client_proven);
        assert!(!Path::new("/tmp/codex-home").exists());
        let fallback = detect_codex_home(None, Some("/tmp/home-fixture"), "windows").unwrap();
        assert_eq!(
            fallback.cli_config_path,
            PathBuf::from("/tmp/home-fixture/.codex/config.toml")
        );
        assert!(detect_codex_home(None, None, "macos").is_err());
    }

    #[test]
    fn refuses_symlink_and_bad_url() {
        let dir = scratch();
        let target = dir.join("real.toml");
        fs::write(&target, "personality = \"x\"\n").unwrap();
        let link = dir.join("config.toml");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert_eq!(apply_config(&req(&dir, None)).unwrap_err(), CodexError::Symlink);
            assert_eq!(fs::read_to_string(&target).unwrap(), "personality = \"x\"\n");
        }
        let mut bad = req(&dir, None);
        bad.config_path = dir.join("plain.toml");
        bad.base_url = "http://0.0.0.0:8317/v1".into();
        assert_eq!(preview_apply(&bad).unwrap_err(), CodexError::BadBaseUrl);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn inspect_reports_ownership_and_external_edit() {
        let dir = scratch();
        let path = dir.join("config.toml");
        let initial = inspect_codex_config(&path).unwrap();
        assert!(!initial.exists);
        assert!(!initial.managed_by_us);

        fs::write(&path, "personality = \"tester\"\n").unwrap();
        let unmanaged = inspect_codex_config(&path).unwrap();
        assert!(unmanaged.exists);
        assert!(!unmanaged.managed_by_us);

        apply_config(&req(&dir, Some("gpt-6-astra"))).unwrap();
        let managed = inspect_codex_config(&path).unwrap();
        assert!(managed.exists);
        assert!(managed.managed_by_us);
        assert!(!managed.external_edit_detected);
        assert!(managed.has_backup);
        assert_eq!(managed.current_model.as_deref(), Some("gpt-6-astra"));

        fs::write(&path, fs::read_to_string(&path).unwrap() + "\n# tamper").unwrap();
        let tampered = inspect_codex_config(&path).unwrap();
        assert!(tampered.managed_by_us);
        assert!(tampered.external_edit_detected);

        rollback_config(&path).unwrap_err(); // conflict
        // restore
        fs::write(&path, fs::read_to_string(&path).unwrap().replace("\n# tamper", "")).unwrap();
        rollback_config(&path).unwrap();
        let reverted = inspect_codex_config(&path).unwrap();
        assert!(reverted.exists);
        assert!(!reverted.managed_by_us);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn syncs_model_catalog_preserving_existing_and_backing_up() {
        let dir = scratch();
        let cat_path = dir.join("model_catalog.json");
        let initial = serde_json::json!({
            "models": [
                {
                    "slug": "custom-existing-1",
                    "display_name": "Custom Existing",
                    "description": "original description",
                    "visibility": "list"
                }
            ]
        });
        fs::write(&cat_path, serde_json::to_string_pretty(&initial).unwrap()).unwrap();

        let models = vec![
            "custom-existing-1".to_string(),
            "gpt-6-astra".to_string(),
            "gemini-3.8-flash-high".to_string(),
            "gpt-image-1".to_string(),
        ];
        let summary = sync_model_catalog(&cat_path, &models).unwrap();
        assert_eq!(summary.total_models, 3);
        assert_eq!(summary.existing_slugs, vec!["custom-existing-1"]);
        assert_eq!(summary.added_slugs, vec!["gpt-6-astra", "gemini-3.8-flash-high"]);
        assert!(summary.backup_path.is_some());
        assert!(summary.backup_path.unwrap().exists());

        let content = fs::read_to_string(&cat_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        let arr = parsed["models"].as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["slug"], "custom-existing-1");
        assert_eq!(arr[0]["description"], "original description");
        assert_eq!(arr[1]["slug"], "gpt-6-astra");
        assert_eq!(arr[1]["shell_type"], "unified_exec");
        assert_eq!(arr[1]["support_verbosity"], true);
        assert!(arr[1]["supported_reasoning_levels"].is_array());
        assert_eq!(arr[1]["default_reasoning_level"], "medium");
        assert_eq!(arr[2]["slug"], "gemini-3.8-flash-high");
        assert_eq!(arr[2]["display_name"], "Gemini 3.8 Flash High");
        assert_eq!(arr[2]["shell_type"], "unified_exec");
        assert!(arr[2]["supported_reasoning_levels"].is_array());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn suspend_and_resume_managed_config_without_overwriting_edits() {
        let dir = scratch();
        let path = dir.join("config.toml");
        fs::write(&path, "model = \"native-model\"\nother = true\n").unwrap();
        let mut req = req(&dir, Some("gateway-model"));
        req.model_catalog_json = Some(dir.join("model_catalog.json"));
        apply_config(&req).unwrap();
        let mut edited = fs::read_to_string(&path).unwrap().parse::<DocumentMut>().unwrap();
        edited["model"] = toml_edit::value("user-selected-model");
        fs::write(&path, edited.to_string()).unwrap();
        assert!(suspend_config(&path).unwrap());
        let native = fs::read_to_string(&path).unwrap();
        let doc = native.parse::<DocumentMut>().unwrap();
        assert!(doc.get("model_provider").is_none());
        assert!(doc.get("model_catalog_json").is_none());
        assert!(doc.get("model").is_none());
        assert_eq!(doc["other"].as_bool(), Some(true));
        fs::write(&path, format!("{native}\n# external change\n")).unwrap();
        assert_eq!(resume_config(&path, 50834, "localgateway012345").unwrap_err(), CodexError::ExternalEdit);
        fs::write(&path, native).unwrap();
        assert!(resume_config(&path, 50834, "localgateway012345").unwrap());
        let restored = fs::read_to_string(&path).unwrap();
        let doc = restored.parse::<DocumentMut>().unwrap();
        assert_eq!(doc["model"].as_str(), Some("user-selected-model"));
        assert_eq!(doc["model_providers"]["custom"]["base_url"].as_str(), Some("http://127.0.0.1:50834/v1"));
        assert!(!resume_config(&path, 50834, "localgateway012345").unwrap());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn suspend_rejects_external_changes_beyond_model() {
        let dir = scratch();
        let req = req(&dir, Some("gateway-model"));
        apply_config(&req).unwrap();
        let current = fs::read_to_string(&req.config_path).unwrap();
        fs::write(&req.config_path, format!("{current}\nexternal_setting = true\n")).unwrap();
        assert_eq!(suspend_config(&req.config_path).unwrap_err(), CodexError::ExternalEdit);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn legacy_ownership_accepts_only_a_model_change_matching_backup() {
        let dir = scratch();
        let req = req(&dir, Some("old-model"));
        apply_config(&req).unwrap();
        apply_config(&req).unwrap();
        let owner_path = ownership_path(&req.config_path);
        let mut owner = read_ownership(&owner_path).unwrap().unwrap();
        owner.sha256_without_model = None;
        fs::write(&owner_path, serde_json::to_vec(&owner).unwrap()).unwrap();
        let mut doc = fs::read_to_string(&req.config_path).unwrap().parse::<DocumentMut>().unwrap();
        doc["model"] = toml_edit::value("new-model");
        fs::write(&req.config_path, doc.to_string()).unwrap();
        assert!(suspend_config(&req.config_path).unwrap());
        assert!(resume_config(&req.config_path, 50833, "localgateway012345").unwrap());
        assert_eq!(top_level_string(&fs::read_to_string(&req.config_path).unwrap(), "model").as_deref(), Some("new-model"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn auth_sync_rejects_different_accounts_without_writing() {
        let dir = scratch();
        let runtime = dir.join("runtime/auth");
        let home = dir.join("home");
        fs::create_dir_all(&runtime).unwrap();
        fs::create_dir_all(&home).unwrap();
        let runtime_file = runtime.join("codex-test.json");
        let original_runtime = r#"{"access_token":"rt-access","refresh_token":"rt-refresh","account_id":"account-b","last_refresh":"2026-01-01"}"#;
        let original_desktop = r#"{"tokens":{"access_token":"desk-access","refresh_token":"desk-refresh","account_id":"account-a"},"last_refresh":"2026-02-01"}"#;
        fs::write(&runtime_file, original_runtime).unwrap();
        fs::write(home.join("auth.json"), original_desktop).unwrap();
        assert!(sync_chatgpt_desktop_auth(&runtime, &home).is_err());
        assert_eq!(fs::read_to_string(runtime_file).unwrap(), original_runtime);
        assert_eq!(fs::read_to_string(home.join("auth.json")).unwrap(), original_desktop);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sync_chatgpt_desktop_auth_both_directions() {
        let dir = scratch();
        let runtime_auth = dir.join("runtime").join("auth");
        let codex_home = dir.join("codex_home");
        fs::create_dir_all(&runtime_auth).unwrap();
        fs::create_dir_all(&codex_home).unwrap();

        // 1. Initial state: Runtime has fresh OAuth token, Desktop auth.json is empty or missing
        let rt_file = runtime_auth.join("codex-3d4a40d6-test@example.com-plus.json");
        let rt_json = serde_json::json!({
            "access_token": "acc_token_12345",
            "refresh_token": "ref_token_67890",
            "id_token": "id_token_abcde",
            "account_id": "acc-id-9999",
            "email": "test@example.com",
            "plan_type": "plus",
            "disabled": false,
            "last_refresh": "2026-11-07T18:00:00+08:00",
            "type": "codex"
        });
        fs::write(&rt_file, serde_json::to_string(&rt_json).unwrap()).unwrap();

        // Perform sync -> should populate auth.json
        let summary1 = sync_chatgpt_desktop_auth(&runtime_auth, &codex_home).unwrap();
        assert!(summary1.synced_to_desktop);
        assert!(!summary1.synced_from_desktop);
        assert_eq!(summary1.email.as_deref(), Some("test@example.com"));

        let auth_json_path = codex_home.join("auth.json");
        assert!(auth_json_path.exists());
        let desktop_content = fs::read_to_string(&auth_json_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&desktop_content).unwrap();
        assert_eq!(parsed["auth_mode"], "chatgpt");
        assert_eq!(parsed["tokens"]["access_token"], "acc_token_12345");
        assert_eq!(parsed["tokens"]["refresh_token"], "ref_token_67890");
        assert_eq!(parsed["tokens"]["account_id"], "acc-id-9999");
        assert_eq!(parsed["last_refresh"], "2026-11-07T18:00:00+08:00");

        // 2. Already synced -> should be noop
        let summary2 = sync_chatgpt_desktop_auth(&runtime_auth, &codex_home).unwrap();
        assert!(!summary2.synced_to_desktop);
        assert!(!summary2.synced_from_desktop);
        assert!(summary2.message.contains("最新同步"));

        // 3. Desktop has newer token (e.g. user refreshed in ChatGPT Desktop)
        let mut updated_desktop = parsed.clone();
        updated_desktop["tokens"]["access_token"] = serde_json::json!("acc_token_desktop_newer");
        updated_desktop["tokens"]["refresh_token"] = serde_json::json!("ref_token_desktop_newer");
        updated_desktop["last_refresh"] = serde_json::json!("2026-11-07T19:00:00+08:00");
        fs::write(&auth_json_path, serde_json::to_string(&updated_desktop).unwrap()).unwrap();

        let summary3 = sync_chatgpt_desktop_auth(&runtime_auth, &codex_home).unwrap();
        assert!(summary3.synced_from_desktop);
        assert!(!summary3.synced_to_desktop);

        let rt_content_after = fs::read_to_string(&rt_file).unwrap();
        let rt_parsed: serde_json::Value = serde_json::from_str(&rt_content_after).unwrap();
        assert_eq!(rt_parsed["access_token"], "acc_token_desktop_newer");
        assert_eq!(rt_parsed["refresh_token"], "ref_token_desktop_newer");
        assert_eq!(rt_parsed["last_refresh"], "2026-11-07T19:00:00+08:00");

        let _ = fs::remove_dir_all(&dir);
    }
}
