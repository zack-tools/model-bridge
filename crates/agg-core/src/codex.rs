//! Codex config adapter.
//!
//! Operates on a caller-supplied path. Detection does not create directories
//! and does not treat a desktop-client path as known. Apply preserves unrelated
//! TOML, writes a sibling backup, and refuses foreign or externally edited files.
//!
//! Detect, preview, apply, rollback, catalog sync, gateway start, and the daemon
//! follow-up never open `auth.json`. A person can preview that file and then
//! confirm a separation step. That step never copies a refresh token. The gateway
//! keeps the login it obtained itself. A generation-numbered backup is written
//! first, and a backup failure leaves `auth.json` unchanged.

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


fn is_cc_switch_model(entry: &serde_json::Value) -> bool {
    let slug = entry.get("slug").and_then(|s| s.as_str()).unwrap_or("");
    let desc = entry.get("description").and_then(|s| s.as_str()).unwrap_or("");
    let comp = entry.get("comp_hash").and_then(|s| s.as_str()).unwrap_or("");
    let display = entry.get("display_name").and_then(|s| s.as_str()).unwrap_or("");

    comp == "cc-switch"
        || slug.starts_with("ccs-")
        || display.contains("Gemini-OAuth")
        || display.contains("cc-switch")
        || desc.contains("cc-switch")
}

fn is_model_bridge_model(entry: &serde_json::Value) -> bool {
    let desc = entry.get("description").and_then(|s| s.as_str()).unwrap_or("");
    desc.starts_with("model-bridge aggregated model:")
}

/// Restore catalog by removing any non-native (ModelBridge or CC Switch) models.
pub fn restore_native_catalog(catalog_path: &Path) -> Result<bool, CodexError> {
    reject_symlink(catalog_path)?;
    if !catalog_path.exists() {
        return Ok(false);
    }
    let text = fs::read_to_string(catalog_path).map_err(|e| CodexError::Message(e.to_string()))?;
    let mut root: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| CodexError::Message(format!("json parse: {e}")))?;

    let models_arr = match root.get_mut("models").and_then(|v| v.as_array_mut()) {
        Some(arr) => arr,
        None => return Ok(false),
    };

    let before_len = models_arr.len();
    models_arr.retain(|entry| {
        !is_cc_switch_model(entry) && !is_model_bridge_model(entry)
    });

    if models_arr.len() != before_len {
        let new_text = serde_json::to_string_pretty(&root)
            .map_err(|e| CodexError::Message(e.to_string()))?;
        atomic_write(catalog_path, new_text.as_bytes())?;
        Ok(true)
    } else {
        Ok(false)
    }
}


/// Format model slug into standard user-facing display name, avoiding ALL-CAPS and simplifying trailing tier suffixes (-high, -medium, -low).
pub fn format_model_display_name(model: &str) -> String {
    let base_model = if let Some(stripped) = model.strip_suffix("-high") {
        stripped
    } else if let Some(stripped) = model.strip_suffix("-medium") {
        stripped
    } else if let Some(stripped) = model.strip_suffix("-low") {
        stripped
    } else {
        model
    };

    if base_model.starts_with("gemini") {
        base_model
            .replace('-', " ")
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
    } else if base_model.starts_with("gpt-") || base_model.starts_with("gpt") {
        base_model
            .split('-')
            .map(|part| {
                if part.eq_ignore_ascii_case("gpt") {
                    "GPT".to_string()
                } else if part.eq_ignore_ascii_case("oss") {
                    "OSS".to_string()
                } else {
                    let mut chars = part.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(first) => {
                            let mut s = first.to_uppercase().collect::<String>();
                            s.push_str(&part[first.len_utf8()..].to_lowercase());
                            s
                        }
                    }
                }
            })
            .collect::<Vec<_>>()
            .join("-")
    } else if base_model.starts_with("claude") {
        base_model
            .replace('-', " ")
            .split_whitespace()
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => {
                        let mut s = f.to_uppercase().collect::<String>();
                        s.push_str(&w[f.len_utf8()..].to_lowercase());
                        s
                    }
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        base_model
            .split('-')
            .map(|part| {
                let mut chars = part.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => {
                        let mut s = first.to_uppercase().collect::<String>();
                        s.push_str(&part[first.len_utf8()..].to_lowercase());
                        s
                    }
                }
            })
            .collect::<Vec<_>>()
            .join("-")
    }
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
        let backup_name = format!("{}.model-bridge.bak", file_name(catalog_path)?);
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

    // 依使用者指示：徹底過濾 CC Switch 模型條目與先前加入之 model-bridge 模型
    models_arr.retain(|entry| {
        let slug = entry.get("slug").and_then(|s| s.as_str()).unwrap_or("");
        let is_image = slug.to_ascii_lowercase().contains("image");
        !is_cc_switch_model(entry) && !is_model_bridge_model(entry) && !is_image
    });

    let mut existing_slugs = Vec::new();
    let mut added_slugs = Vec::new();

    for model in verified_models {
        if model.trim().is_empty() || model.to_ascii_lowercase().contains("image") { continue; }
        let display_name = format_model_display_name(model);
        if let Some(existing) = models_arr.iter_mut().find(|entry| {
            entry.get("slug").and_then(|s| s.as_str()) == Some(model.as_str())
        }) {
            if is_model_bridge_model(existing) {
                if let Some(obj) = existing.as_object_mut() {
                    obj.insert("display_name".into(), serde_json::json!(display_name));
                }
            }
            existing_slugs.push(model.clone());
        } else {
            let entry = if let Some(tpl) = if model.contains("gemini") {
                models_arr.iter().find(|e| e.get("slug").and_then(|s| s.as_str()).map_or(false, |s| s.contains("gemini")))
            } else if model.contains("gpt") {
                models_arr.iter().find(|e| e.get("slug").and_then(|s| s.as_str()).map_or(false, |s| s.contains("gpt")))
            } else {
                models_arr.first()
            }.or_else(|| models_arr.first()) {
                let mut cloned = tpl.clone();
                if let Some(obj) = cloned.as_object_mut() {
                    obj.remove("comp_hash");
                    obj.insert("slug".into(), serde_json::json!(model));
                    obj.insert("display_name".into(), serde_json::json!(display_name));
                    obj.insert("description".into(), serde_json::json!(format!("model-bridge aggregated model: {model}")));
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
                    "description": format!("model-bridge aggregated model: {model}"),
                    "visibility": "list",
                    "supported_in_api": true,
                    "priority": 5,
                    "input_modalities": ["text", "image"],
                    "supports_search_tool": true,
                    "support_verbosity": true,
                    "default_reasoning_level": def_reason,
                    "supported_reasoning_levels": supp_reason,
                    "shell_type": "unified_exec",
                    "truncation_policy": {
                        "limit": 10000,
                        "mode": "bytes"
                    },
                    "context_window": 272000,
                    "max_context_window": 272000,
                    "auto_compact_token_limit": 244800,
                    "effective_context_window_percent": 95,
                    "base_instructions": "You are Codex, a coding agent. You and the user share the same workspace and collaborate to achieve the user's goals.",
                    "additional_speed_tiers": []
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
pub struct AuthSyncPreview {
    pub auth_json_path: PathBuf,
    pub gateway_has_own_login: bool,
    pub desktop_file_present: bool,
    pub shared_refresh_token: bool,
    pub refresh_token_will_be_copied: bool,
    pub would_write: bool,
    pub next_backup_name: Option<String>,
    pub message: String,
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
    pub refresh_token_copied: bool,
    pub wrote: bool,
}

/// Gateway start does no Codex auth I/O. Callers on the default path use this
/// and must not open `auth.json` themselves.
pub fn after_gateway_start(codex_home: &Path) -> Result<(), CodexError> {
    let _ = codex_home;
    Ok(())
}

/// Daemon follow-up may resume `config.toml`. It does not open `auth.json`.
pub fn after_daemon_start(config_path: &Path, port: u16, gateway_key: &str) -> Result<bool, CodexError> {
    resume_config(config_path, port, gateway_key)
}

/// Read-only preview of an explicit separation step. Does not create directories
/// or backups and does not return token material.
pub fn preview_chatgpt_desktop_auth(
    runtime_auth_dir: &Path,
    codex_home: &Path,
) -> Result<AuthSyncPreview, CodexError> {
    let auth_path = codex_home.join("auth.json");
    reject_symlink(codex_home)?;
    reject_symlink(&auth_path)?;
    let gateway_tokens = gateway_refresh_tokens(runtime_auth_dir)?;
    let desktop = read_desktop_auth(&auth_path)?;
    let desktop_refresh = desktop.as_ref().and_then(desktop_refresh_token);
    let shared = desktop_refresh.as_deref().is_some_and(|token| gateway_tokens.iter().any(|item| item == token));
    let generation = if shared { Some(next_backup_generation(codex_home)?) } else { None };
    let message = if shared {
        "預覽：桌面 auth.json 的 refresh token 與閘道自己的 OAuth 檔相同。確認後只會從桌面檔移除該 refresh token，不會把權杖複製到另一端。會先寫入世代編號備份；備份失敗則不改檔。".to_string()
    } else if gateway_tokens.is_empty() && desktop_refresh.is_none() {
        "預覽：閘道與桌面都沒有可分離的 refresh token。確認後不會寫入 auth.json。閘道登入只來自它自己的 OAuth。".to_string()
    } else {
        "預覽：兩端沒有共用的 refresh token。確認後不會寫入 auth.json，也不會複製 refresh token。閘道使用自己的 OAuth 登入。".to_string()
    };
    Ok(AuthSyncPreview {
        auth_json_path: auth_path,
        gateway_has_own_login: !gateway_tokens.is_empty(),
        desktop_file_present: desktop.is_some(),
        shared_refresh_token: shared,
        refresh_token_will_be_copied: false,
        would_write: shared,
        next_backup_name: generation.map(|n| format!("auth.json.{n}.model-bridge.bak")),
        message,
    })
}

/// Confirmed separation. Refuses to copy refresh tokens in either direction.
/// When the desktop file contains the gateway's refresh token, that field is
/// removed only after a new `auth.json.<generation>.model-bridge.bak` is durable.
pub fn apply_chatgpt_desktop_auth(
    runtime_auth_dir: &Path,
    codex_home: &Path,
    confirmed: bool,
) -> Result<AuthSyncSummary, CodexError> {
    let auth_path = codex_home.join("auth.json");
    if !confirmed {
        return Err(CodexError::Message("需要先預覽並確認，才會處理 auth.json".into()));
    }
    let preview = preview_chatgpt_desktop_auth(runtime_auth_dir, codex_home)?;
    if !preview.would_write {
        return Ok(summary(&auth_path, None, false, preview.message));
    }
    let original = fs::read(&auth_path).map_err(|e| CodexError::Message(format!("backup failed; auth.json unchanged: {e}")))?;
    let backup = backup_auth_json(codex_home, &original)?;
    let mut value: serde_json::Value = serde_json::from_slice(&original)
        .map_err(|e| CodexError::Message(format!("auth.json parse: {e}")))?;
    if let Some(obj) = value.get_mut("tokens").and_then(|v| v.as_object_mut()) {
        obj.remove("refresh_token");
    }
    let formatted = serde_json::to_string_pretty(&value).map_err(|e| CodexError::Message(e.to_string()))?;
    atomic_write(&auth_path, formatted.as_bytes())?;
    Ok(summary(
        &auth_path,
        Some(backup),
        true,
        "已從桌面 auth.json 移除與閘道相同的 refresh token。閘道仍使用自己的 OAuth 檔，沒有複製 refresh token。".into(),
    ))
}

fn summary(auth_path: &Path, backup: Option<PathBuf>, wrote: bool, message: String) -> AuthSyncSummary {
    AuthSyncSummary {
        auth_json_path: auth_path.to_path_buf(),
        backup_path: backup,
        synced_to_desktop: false,
        synced_from_desktop: false,
        email: None,
        account_id: None,
        plan_type: None,
        message,
        refresh_token_copied: false,
        wrote,
    }
}

fn gateway_refresh_tokens(runtime_auth_dir: &Path) -> Result<Vec<String>, CodexError> {
    let mut out = Vec::new();
    if !runtime_auth_dir.exists() {
        return Ok(out);
    }
    if !runtime_auth_dir.is_dir() {
        return Err(CodexError::Message("runtime auth path is not a directory".into()));
    }
    let entries = fs::read_dir(runtime_auth_dir).map_err(|e| CodexError::Message(e.to_string()))?;
    for entry in entries {
        let path = entry.map_err(|e| CodexError::Message(e.to_string()))?.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !(name.starts_with("codex-") && name.ends_with(".json") && !name.ends_with(".lock") && !name.contains(".bak")) {
            continue;
        }
        let content = fs::read_to_string(&path).map_err(|e| CodexError::Message(e.to_string()))?;
        let val: serde_json::Value = serde_json::from_str(&content).map_err(|e| CodexError::Message(e.to_string()))?;
        if val.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false) {
            continue;
        }
        if let Some(token) = val.get("refresh_token").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            out.push(token.to_string());
        }
    }
    Ok(out)
}

fn read_desktop_auth(auth_path: &Path) -> Result<Option<serde_json::Value>, CodexError> {
    if !auth_path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(auth_path).map_err(|e| CodexError::Message(e.to_string()))?;
    let value = serde_json::from_str(&content).map_err(|e| CodexError::Message(format!("auth.json parse: {e}")))?;
    Ok(Some(value))
}

fn desktop_refresh_token(value: &serde_json::Value) -> Option<String> {
    value.get("tokens")
        .and_then(|t| t.get("refresh_token"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn next_backup_generation(codex_home: &Path) -> Result<u64, CodexError> {
    let mut generation = 1u64;
    loop {
        if generation > 10_000 {
            return Err(CodexError::Message("too many auth.json backups".into()));
        }
        if !codex_home.join(format!("auth.json.{generation}.model-bridge.bak")).exists() {
            return Ok(generation);
        }
        generation += 1;
    }
}

fn backup_auth_json(codex_home: &Path, original: &[u8]) -> Result<PathBuf, CodexError> {
    let generation = next_backup_generation(codex_home)?;
    let backup = codex_home.join(format!("auth.json.{generation}.model-bridge.bak"));
    reject_symlink(&backup)?;
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&backup).map_err(|e| {
        CodexError::Message(format!("backup failed; auth.json unchanged: {e}"))
    })?;
    if let Err(err) = file.write_all(original).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&backup);
        return Err(CodexError::Message(format!("backup failed; auth.json unchanged: {err}")));
    }
    Ok(backup)
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
                if current_hash != ownership.sha256 {
                    let norm_cur = hash_without_model(&content).unwrap_or_default();
                    let ref_norm = ownership.sha256_without_model.as_deref().unwrap_or(&ownership.sha256);
                    external_edit_detected = norm_cur != ref_norm;
                } else {
                    external_edit_detected = false;
                }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ConfigOrigin {
    created: bool,
    backup: Option<String>,
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
    /// Content from before the first ModelBridge apply. Later applies keep this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    origin: Option<ConfigOrigin>,
    /// Hash of the managed keys only. Unowned tables such as `[projects.*]` are omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256_managed: Option<String>,
}

fn hash_without_model(text: &str) -> Result<String, CodexError> {
    let mut doc = text.parse::<DocumentMut>().map_err(|e| CodexError::Message(format!("toml parse: {e}")))?;
    doc.remove("model");
    doc.remove("model_reasoning_effort");
    if let Some(mcp) = doc.get_mut("mcp_servers").and_then(Item::as_table_like_mut) {
        if let Some(node) = mcp.get_mut("node_repl").and_then(Item::as_table_like_mut) {
            if let Some(env) = node.get_mut("env").and_then(Item::as_table_like_mut) {
                if env.contains_key("SKY_CUA_NATIVE_PIPE_DIRECTORY") {
                    env.insert("SKY_CUA_NATIVE_PIPE_DIRECTORY", toml_edit::value("normalized_cua_pipe"));
                }
            }
        }
    }
    Ok(crate::pin::sha256_bytes(doc.to_string().as_bytes()))
}

fn is_toml_table(item: &Item) -> bool {
    item.is_table() || item.is_inline_table()
}

fn is_managed_provider(name: &str) -> bool {
    name == PROVIDER_ID || name == "app_agg_001"
}

/// Document reduced to the keys ModelBridge writes: `model_provider`, `model`,
/// `model_catalog_json`, and `[model_providers.custom]` / `app_agg_001`.
fn managed_canonical(text: &str) -> Result<String, CodexError> {
    if text.is_empty() {
        return Ok(String::new());
    }
    let mut doc = text.parse::<DocumentMut>().map_err(|e| CodexError::Message(format!("toml parse: {e}")))?;
    let keys: Vec<String> = doc.iter().map(|(key, _)| key.to_string()).collect();
    for key in keys {
        if key == "model_provider" || key == "model" || key == "model_catalog_json" {
            continue;
        }
        if key == "model_providers" {
            if let Some(table) = doc.get_mut("model_providers").and_then(Item::as_table_mut) {
                let subkeys: Vec<String> = table.iter().map(|(key, _)| key.to_string()).collect();
                for subkey in subkeys {
                    if !is_managed_provider(&subkey) {
                        table.remove(&subkey);
                    }
                }
            }
            continue;
        }
        if doc.get(&key).is_some_and(is_toml_table) {
            doc.remove(&key);
        }
    }
    Ok(doc.to_string())
}

fn parse_doc(text: &str) -> Result<DocumentMut, CodexError> {
    if text.is_empty() {
        return Ok(DocumentMut::new());
    }
    text.parse::<DocumentMut>().map_err(|e| CodexError::Message(format!("toml parse: {e}")))
}

/// Keep the pre-ModelBridge document and overlay tables this tool does not own.
fn merge_foreign_tables(baseline: &str, current: &str) -> Result<String, CodexError> {
    let mut base = parse_doc(baseline)?;
    let cur = parse_doc(current)?;
    let keys: Vec<String> = cur.iter().map(|(key, _)| key.to_string()).collect();
    for key in keys {
        if key == "model_providers" || key == "model_provider" || key == "model" || key == "model_catalog_json" {
            continue;
        }
        if cur.get(&key).is_some_and(is_toml_table) {
            if let Some(item) = cur.get(&key).cloned() {
                base.insert(&key, item);
            }
        }
    }
    if let Some(providers) = cur.get("model_providers").and_then(Item::as_table) {
        let foreign: Vec<(String, Item)> = providers
            .iter()
            .filter(|(key, _)| !is_managed_provider(key))
            .map(|(key, item)| (key.to_string(), item.clone()))
            .collect();
        if !foreign.is_empty() {
            let dest = nested_table(base.as_table_mut(), "model_providers")?;
            for (key, item) in foreign {
                dest.insert(&key, item);
            }
        }
    }
    Ok(base.to_string())
}

fn local_backup_name(name: &str) -> Result<(), CodexError> {
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(CodexError::Message("backup name is not local".into()));
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct SuspendedConfig {
    active: String,
    native_hash: String,
}

fn suspended_path(path: &Path) -> PathBuf {
    path.with_file_name(format!("{}.model-bridge.suspended.json", path.file_name().and_then(|s| s.to_str()).unwrap_or("config.toml")))
}

/// Switch a verified managed configuration to the client's native defaults.
pub fn suspend_config(path: &Path) -> Result<bool, CodexError> {
    reject_symlink(path)?;
    let marker = suspended_path(path);
    reject_symlink(&marker)?;
    if marker.exists() {
        if let Some(parent) = path.parent() {
            let cat_path = parent.join("model_catalog.json");
            let _ = restore_native_catalog(&cat_path);
        }
        return Ok(true);
    }
    let Some(mut owner) = read_ownership(&ownership_path(path))? else {
        if let Some(parent) = path.parent() {
            let cat_path = parent.join("model_catalog.json");
            let _ = restore_native_catalog(&cat_path);
        }
        return Ok(false);
    };
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
    doc.remove("model_reasoning_effort");
    let native = doc.to_string();
    let record = SuspendedConfig { active, native_hash: crate::pin::sha256_bytes(native.as_bytes()) };
    let bytes = serde_json::to_vec(&record).map_err(|e| CodexError::Message(e.to_string()))?;
    atomic_write(&marker, &bytes)?;
    atomic_write(path, native.as_bytes())?;

    if let Some(parent) = path.parent() {
        let cat_path = parent.join("model_catalog.json");
        let _ = restore_native_catalog(&cat_path);
    }

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
    if crate::pin::sha256_bytes(current.as_bytes()) != record.native_hash {
        let norm_cur = hash_without_model(&current)?;
        let mut rec_doc = record.active.parse::<DocumentMut>().map_err(|e| CodexError::Message(format!("toml parse: {e}")))?;
        rec_doc.remove("model_provider");
        rec_doc.remove("model_catalog_json");
        rec_doc.remove("model");
        rec_doc.remove("model_reasoning_effort");
        let norm_rec_native = hash_without_model(&rec_doc.to_string())?;
        if norm_cur != norm_rec_native {
            return Err(CodexError::ExternalEdit);
        }
    }
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
    let latest_backup = if existed {
        let name = format!(
            "{}.{generation}.model-bridge.bak",
            file_name(&req.config_path)?
        );
        fs::copy(&req.config_path, parent.join(&name)).map_err(|e| CodexError::Message(e.to_string()))?;
        Some(name)
    } else {
        None
    };
    let origin = if let Some(previous) = &previous {
        previous.origin.clone().unwrap_or(ConfigOrigin {
            created: previous.created,
            backup: previous.backup.clone(),
        })
    } else {
        ConfigOrigin { created: !existed, backup: latest_backup.clone() }
    };
    atomic_write(&req.config_path, after.as_bytes())?;
    let ownership = Ownership {
        tool: "model-bridge".into(),
        generation,
        sha256: crate::pin::sha256_bytes(after.as_bytes()),
        managed_provider_id: PROVIDER_ID.into(),
        backup: latest_backup.or_else(|| origin.backup.clone()),
        created: origin.created,
        sha256_without_model: Some(hash_without_model(&after)?),
        origin: Some(origin),
        sha256_managed: Some(crate::pin::sha256_bytes(managed_canonical(&after)?.as_bytes())),
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
    let full_match = crate::pin::sha256_bytes(&current) == ownership.sha256;
    let current_text = std::str::from_utf8(&current).map_err(|_| CodexError::RollbackConflict)?;
    let managed_match = match &ownership.sha256_managed {
        Some(expected) => crate::pin::sha256_bytes(managed_canonical(current_text)?.as_bytes()) == *expected,
        None => false,
    };
    if !full_match && !managed_match {
        return Err(CodexError::RollbackConflict);
    }
    let parent = config_path.parent().ok_or(CodexError::ParentMissing)?;
    let origin = ownership.origin.clone().unwrap_or(ConfigOrigin {
        created: ownership.created,
        backup: ownership.backup.clone(),
    });
    let baseline = if let Some(backup_name) = &origin.backup {
        local_backup_name(backup_name)?;
        fs::read(parent.join(backup_name)).map_err(|e| CodexError::Message(e.to_string()))?
    } else if origin.created {
        Vec::new()
    } else {
        return Err(CodexError::NothingToRollBack);
    };
    let restored = if full_match {
        baseline
    } else {
        let baseline_text = std::str::from_utf8(&baseline).map_err(|_| CodexError::Message("backup is not utf-8".into()))?;
        merge_foreign_tables(baseline_text, current_text)?.into_bytes()
    };
    if restored.iter().all(u8::is_ascii_whitespace) && origin.created && origin.backup.is_none() {
        fs::remove_file(config_path).map_err(|e| CodexError::Message(e.to_string()))?;
    } else {
        atomic_write(config_path, &restored)?;
    }
    fs::remove_file(&ownership_path).map_err(|e| CodexError::Message(e.to_string()))?;
    if let Some(parent) = config_path.parent() {
        let cat_path = parent.join("model_catalog.json");
        let _ = restore_native_catalog(&cat_path);
    }
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
    config.with_file_name(format!("{name}.model-bridge.ownership.json"))
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

    fn append_project_trust(path: &Path, project: &str) {
        let mut text = fs::read_to_string(path).unwrap();
        text.push_str(&format!("\n[projects.\"{project}\"]\ntrust_level = \"trusted\"\n"));
        fs::write(path, text).unwrap();
    }

    fn project_trust<'a>(doc: &'a DocumentMut, project: &str) -> Option<&'a str> {
        doc.get("projects")?.get(project)?.get("trust_level")?.as_str()
    }

    #[test]
    fn rollback_keeps_codex_project_trust_across_two_applies_that_created_the_file() {
        let dir = scratch();
        let path = dir.join("config.toml");
        let project = dir.display().to_string();
        let mut apply = req(&dir, Some("gemini-3-flash"));
        apply_config(&apply).unwrap();
        apply.model = Some("gpt-6-sol".into());
        apply_config(&apply).unwrap();
        let owner = read_ownership(&ownership_path(&path)).unwrap().unwrap();
        assert_eq!(owner.generation, 2);
        assert_eq!(owner.origin.as_ref().map(|origin| origin.created), Some(true));
        assert!(owner.origin.as_ref().unwrap().backup.is_none());
        append_project_trust(&path, &project);
        let before = fs::read_to_string(&path).unwrap();
        assert!(before.contains("gpt-6-sol"));
        rollback_config(&path).unwrap();
        let restored = fs::read_to_string(&path).unwrap().parse::<DocumentMut>().unwrap();
        assert!(restored.get("model").is_none());
        assert!(restored.get("model_provider").is_none());
        assert!(restored.get("model_providers").is_none());
        assert_eq!(project_trust(&restored, &project), Some("trusted"));
        assert!(!ownership_path(&path).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rollback_of_a_file_we_created_removes_it_after_a_later_apply() {
        let dir = scratch();
        let path = dir.join("config.toml");
        apply_config(&req(&dir, Some("gemini-3-flash"))).unwrap();
        apply_config(&req(&dir, Some("gpt-6-sol"))).unwrap();
        assert_eq!(read_ownership(&ownership_path(&path)).unwrap().unwrap().generation, 2);
        rollback_config(&path).unwrap();
        assert!(!path.exists());
        assert!(!ownership_path(&path).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rollback_two_applies_restore_pre_bridge_file_and_keep_project_trust() {
        let dir = scratch();
        let path = dir.join("config.toml");
        let project = dir.display().to_string();
        fs::write(&path, "personality = \"keep\"\n").unwrap();
        let mut apply = req(&dir, Some("gemini-3-flash"));
        apply_config(&apply).unwrap();
        apply.model = Some("gpt-6-sol".into());
        apply_config(&apply).unwrap();
        let owner = read_ownership(&ownership_path(&path)).unwrap().unwrap();
        assert_eq!(owner.generation, 2);
        assert_eq!(
            owner.origin.as_ref().and_then(|origin| origin.backup.as_deref()),
            Some("config.toml.1.model-bridge.bak")
        );
        append_project_trust(&path, &project);
        rollback_config(&path).unwrap();
        let restored = fs::read_to_string(&path).unwrap().parse::<DocumentMut>().unwrap();
        assert_eq!(restored["personality"].as_str(), Some("keep"));
        assert!(restored.get("model").is_none());
        assert!(restored.get("model_provider").is_none());
        assert!(restored.get("model_providers").is_none());
        assert_eq!(project_trust(&restored, &project), Some("trusted"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rollback_refuses_when_a_managed_field_changes_beside_project_trust() {
        let dir = scratch();
        let path = dir.join("config.toml");
        let project = dir.display().to_string();
        apply_config(&req(&dir, Some("gemini-3-flash"))).unwrap();
        append_project_trust(&path, &project);
        let mut doc = fs::read_to_string(&path).unwrap().parse::<DocumentMut>().unwrap();
        doc["model"] = toml_edit::value("gpt-6-sol");
        fs::write(&path, doc.to_string()).unwrap();
        let changed = fs::read_to_string(&path).unwrap();
        assert_eq!(rollback_config(&path).unwrap_err(), CodexError::RollbackConflict);
        assert_eq!(fs::read_to_string(&path).unwrap(), changed);

        fs::write(&path, changed.replace("gpt-6-sol", "gemini-3-flash")).unwrap();
        let mut doc = fs::read_to_string(&path).unwrap().parse::<DocumentMut>().unwrap();
        doc["model_providers"]["custom"]["base_url"] = toml_edit::value("http://127.0.0.1:9/v1");
        fs::write(&path, doc.to_string()).unwrap();
        let changed = fs::read_to_string(&path).unwrap();
        assert_eq!(rollback_config(&path).unwrap_err(), CodexError::RollbackConflict);
        assert_eq!(fs::read_to_string(&path).unwrap(), changed);
        let _ = fs::remove_dir_all(&dir);
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
    fn format_model_display_name_avoids_all_caps() {
        assert_eq!(format_model_display_name("gpt-6.1-terra"), "GPT-6.1-Terra");
        assert_eq!(format_model_display_name("gpt-6-sol"), "GPT-6-Sol");
        assert_eq!(format_model_display_name("gpt-6.1-sol"), "GPT-6.1-Sol");
        assert_eq!(format_model_display_name("gpt-6-luna"), "GPT-6-Luna");
        assert_eq!(format_model_display_name("gpt-5.6-sol"), "GPT-5.6-Sol");
        assert_eq!(format_model_display_name("gpt-5.4-mini"), "GPT-5.4-Mini");
        assert_eq!(format_model_display_name("gemini-3.8-flash-high"), "Gemini 3.8 Flash");
        assert_eq!(format_model_display_name("gemini-3-flash"), "Gemini 3 Flash");
        assert_eq!(format_model_display_name("gemini-3.1-flash-lite"), "Gemini 3.1 Flash Lite");
        assert_eq!(format_model_display_name("claude-sonnet-4-6"), "Claude Sonnet 4 6");
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
        assert_eq!(arr[2]["display_name"], "Gemini 3.8 Flash");
        assert_eq!(arr[2]["shell_type"], "unified_exec");
        assert!(arr[2]["supported_reasoning_levels"].is_array());

        let _ = fs::remove_dir_all(&dir);
    }


    #[test]
    fn syncs_model_catalog_filters_cc_switch_and_restores_native() {
        let dir = scratch();
        let cat_path = dir.join("model_catalog.json");
        let initial = serde_json::json!({
            "models": [
                {
                    "slug": "gpt-6-astra",
                    "display_name": "GPT-6-Astra",
                    "description": "Our most capable model",
                    "visibility": "list"
                },
                {
                    "slug": "ccs-gemini/gemini-3.7-flash",
                    "display_name": "Gemini 3.7 (Gemini-OAuth)",
                    "description": "from cc-switch",
                    "comp_hash": "cc-switch",
                    "visibility": "list"
                }
            ]
        });
        fs::write(&cat_path, serde_json::to_string_pretty(&initial).unwrap()).unwrap();

        let models = vec![
            "gemini-3.8-flash-high".to_string(),
        ];
        let summary = sync_model_catalog(&cat_path, &models).unwrap();
        assert_eq!(summary.total_models, 2);
        assert_eq!(summary.added_slugs, vec!["gemini-3.8-flash-high"]);

        let content = fs::read_to_string(&cat_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        let arr = parsed["models"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["slug"], "gpt-6-astra");
        assert_eq!(arr[1]["slug"], "gemini-3.8-flash-high");
        assert!(arr[1].get("comp_hash").is_none());

        // Test restore_native_catalog
        assert!(restore_native_catalog(&cat_path).unwrap());
        let restored_content = fs::read_to_string(&cat_path).unwrap();
        let restored: serde_json::Value = serde_json::from_str(&restored_content).unwrap();
        let restored_arr = restored["models"].as_array().unwrap();
        assert_eq!(restored_arr.len(), 1);
        assert_eq!(restored_arr[0]["slug"], "gpt-6-astra");

        let _ = fs::remove_dir_all(dir);
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

    fn write_gateway_login(runtime: &Path, refresh: &str) -> PathBuf {
        fs::create_dir_all(runtime).unwrap();
        let path = runtime.join("codex-own-login.json");
        let body = serde_json::json!({
            "access_token": "gateway-access",
            "refresh_token": refresh,
            "account_id": "gateway-account",
            "disabled": false,
            "type": "codex"
        });
        fs::write(&path, serde_json::to_string(&body).unwrap()).unwrap();
        path
    }

    #[test]
    fn explicit_auth_separation_never_copies_refresh_tokens() {
        let dir = scratch();
        let runtime = dir.join("runtime/auth");
        let home = dir.join("home");
        fs::create_dir_all(&home).unwrap();
        let runtime_file = write_gateway_login(&runtime, "gateway-refresh");
        let original_runtime = fs::read_to_string(&runtime_file).unwrap();
        let desktop = r#"{"auth_mode":"chatgpt","tokens":{"access_token":"desk-access","refresh_token":"desk-refresh","account_id":"account-a"},"last_refresh":"2026-02-01"}"#;
        let auth_path = home.join("auth.json");
        fs::write(&auth_path, desktop).unwrap();

        let preview = preview_chatgpt_desktop_auth(&runtime, &home).unwrap();
        assert!(!preview.refresh_token_will_be_copied);
        assert!(!preview.would_write);
        assert!(!preview.shared_refresh_token);
        assert!(!preview.message.contains("gateway-refresh"));
        assert!(!preview.message.contains("desk-refresh"));
        assert!(apply_chatgpt_desktop_auth(&runtime, &home, false).is_err());
        assert_eq!(fs::read_to_string(&auth_path).unwrap(), desktop);
        let confirmed = apply_chatgpt_desktop_auth(&runtime, &home, true).unwrap();
        assert!(!confirmed.wrote);
        assert!(!confirmed.refresh_token_copied);
        assert!(!confirmed.synced_to_desktop);
        assert!(!confirmed.synced_from_desktop);
        assert_eq!(fs::read_to_string(&auth_path).unwrap(), desktop);
        assert_eq!(fs::read_to_string(&runtime_file).unwrap(), original_runtime);
        assert!(fs::read_dir(&home).unwrap().all(|entry| {
            !entry.unwrap().file_name().to_string_lossy().contains(".model-bridge.bak")
        }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn confirmed_separation_strips_only_a_shared_refresh_token_after_numbered_backup() {
        let dir = scratch();
        let runtime = dir.join("runtime/auth");
        let home = dir.join("home");
        fs::create_dir_all(&home).unwrap();
        let runtime_file = write_gateway_login(&runtime, "shared-refresh");
        let original_runtime = fs::read_to_string(&runtime_file).unwrap();
        let desktop = r#"{"auth_mode":"chatgpt","tokens":{"access_token":"desk-access","refresh_token":"shared-refresh","account_id":"account-a"}}"#;
        let auth_path = home.join("auth.json");
        fs::write(&auth_path, desktop).unwrap();

        let preview = preview_chatgpt_desktop_auth(&runtime, &home).unwrap();
        assert!(preview.would_write);
        assert_eq!(preview.next_backup_name.as_deref(), Some("auth.json.1.model-bridge.bak"));
        assert!(!preview.refresh_token_will_be_copied);
        assert!(!preview.message.contains("shared-refresh"));

        let summary = apply_chatgpt_desktop_auth(&runtime, &home, true).unwrap();
        assert!(summary.wrote);
        assert!(!summary.refresh_token_copied);
        let backup = home.join("auth.json.1.model-bridge.bak");
        assert_eq!(summary.backup_path.as_deref(), Some(backup.as_path()));
        assert_eq!(fs::read_to_string(&backup).unwrap(), desktop);
        let updated: serde_json::Value = serde_json::from_str(&fs::read_to_string(&auth_path).unwrap()).unwrap();
        assert!(updated["tokens"].get("refresh_token").is_none());
        assert_eq!(updated["tokens"]["access_token"], "desk-access");
        assert_eq!(fs::read_to_string(&runtime_file).unwrap(), original_runtime);

        fs::write(&auth_path, desktop).unwrap();
        let again = apply_chatgpt_desktop_auth(&runtime, &home, true).unwrap();
        assert!(again.wrote);
        assert!(home.join("auth.json.2.model-bridge.bak").is_file());
        assert!(home.join("auth.json.1.model-bridge.bak").is_file());
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn backup_failure_aborts_the_auth_json_write() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch();
        let runtime = dir.join("runtime/auth");
        let home = dir.join("home");
        fs::create_dir_all(&home).unwrap();
        write_gateway_login(&runtime, "shared-refresh");
        let desktop = r#"{"tokens":{"access_token":"desk-access","refresh_token":"shared-refresh"}}"#;
        let auth_path = home.join("auth.json");
        fs::write(&auth_path, desktop).unwrap();
        let mut perms = fs::metadata(&home).unwrap().permissions();
        perms.set_mode(0o555);
        fs::set_permissions(&home, perms).unwrap();
        struct Restore(PathBuf);
        impl Drop for Restore {
            fn drop(&mut self) {
                let mut perms = fs::metadata(&self.0).unwrap().permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&self.0, perms).unwrap();
            }
        }
        let _restore = Restore(home.clone());
        let err = apply_chatgpt_desktop_auth(&runtime, &home, true).unwrap_err();
        assert!(err.to_string().contains("backup failed"));
        assert_eq!(fs::read_to_string(&auth_path).unwrap(), desktop);
        drop(_restore);
        assert!(!home.join("auth.json.1.model-bridge.bak").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn default_followups_do_not_open_auth_json() {
        use std::os::unix::fs::FileTypeExt;
        use std::sync::mpsc;
        use std::time::Duration;
        let dir = scratch();
        let auth = dir.join("auth.json");
        assert!(std::process::Command::new("mkfifo").arg(&auth).status().unwrap().success());
        fs::write(dir.join("config.toml"), "personality = \"keep\"\n").unwrap();
        let req = req(&dir, None);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(apply_config(&req));
        });
        let applied = rx.recv_timeout(Duration::from_secs(2)).expect("apply_config opened auth.json");
        applied.unwrap();
        assert!(fs::symlink_metadata(&auth).unwrap().file_type().is_fifo());

        let (tx, rx) = mpsc::channel();
        let home = dir.clone();
        std::thread::spawn(move || {
            let _ = tx.send(after_gateway_start(&home));
        });
        rx.recv_timeout(Duration::from_secs(2)).expect("after_gateway_start opened auth.json").unwrap();

        let config = dir.join("config.toml");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(after_daemon_start(&config, 50833, "localgateway012345"));
        });
        rx.recv_timeout(Duration::from_secs(2)).expect("after_daemon_start opened auth.json").unwrap();
        assert!(fs::symlink_metadata(&auth).unwrap().file_type().is_fifo());

        let runtime = dir.join("runtime");
        let (tx, rx) = mpsc::channel();
        let home = dir.clone();
        std::thread::spawn(move || {
            let _ = tx.send(apply_chatgpt_desktop_auth(&runtime, &home, false));
        });
        assert!(rx.recv_timeout(Duration::from_secs(2)).expect("unconfirmed auth apply opened auth.json").is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_desktop_auth_file_is_not_created() {
        let dir = scratch();
        let runtime = dir.join("runtime/auth");
        let home = dir.join("missing-home");
        write_gateway_login(&runtime, "gateway-refresh");
        let preview = preview_chatgpt_desktop_auth(&runtime, &home).unwrap();
        assert!(!preview.desktop_file_present);
        assert!(!preview.would_write);
        assert!(!home.exists());
        let summary = apply_chatgpt_desktop_auth(&runtime, &home, true).unwrap();
        assert!(!summary.wrote);
        assert!(!home.join("auth.json").exists());
        let _ = fs::remove_dir_all(dir);
    }
}
