//! VS Code Language Models Config Adapter.
//!
//! Generates and applies OpenAI Compatible endpoint settings to VS Code's
//! `chatLanguageModels.json`.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VSCodeStatusInfo {
    pub target_path: String,
    pub exists: bool,
    pub has_backup: bool,
    pub model_count: usize,
    pub configured_by_bridge: bool,
}

pub fn inspect_vscode_status(target_path: &Path) -> VSCodeStatusInfo {
    let exists = target_path.exists();
    let bak = target_path.with_extension("json.bak");
    let has_backup = bak.exists();
    let (model_count, configured_by_bridge) = if exists {
        let content = fs::read_to_string(target_path).unwrap_or_default();
        let val: Value = serde_json::from_str(&content).unwrap_or_default();
        let is_bridge = content.contains("Model-Bridge") || content.contains("customendpoint");
        let count = val.as_array()
            .and_then(|arr| arr.first())
            .and_then(|grp| grp.get("models"))
            .and_then(|m| m.as_array())
            .map(|arr| arr.len())
            .unwrap_or(0);
        (count, is_bridge)
    } else {
        (0, false)
    };
    VSCodeStatusInfo {
        target_path: target_path.to_string_lossy().to_string(),
        exists,
        has_backup,
        model_count,
        configured_by_bridge,
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VSCodeDeployResult {
    pub target_path: String,
    pub backup_path: Option<String>,
    pub model_count: usize,
    pub message: String,
}

pub fn detect_vscode_models_path(custom: Option<&Path>, os: &str) -> Option<PathBuf> {
    if let Some(p) = custom {
        return Some(p.to_path_buf());
    }
    match os {
        "windows" => {
            let appdata = std::env::var("APPDATA").ok()?;
            Some(PathBuf::from(appdata).join("Code").join("User").join("chatLanguageModels.json"))
        }
        "macos" => {
            let home = std::env::var("HOME").ok()?;
            Some(PathBuf::from(home).join("Library").join("Application Support").join("Code").join("User").join("chatLanguageModels.json"))
        }
        _ => {
            let home = std::env::var("HOME").ok()?;
            Some(PathBuf::from(home).join(".config").join("Code").join("User").join("chatLanguageModels.json"))
        }
    }
}

pub fn generate_vscode_config(base_url: &str, gateway_key: &str) -> Value {
    let auth_header = format!("Bearer {gateway_key}");
    json!([
        {
            "name": "Model-Bridge",
            "vendor": "customendpoint",
            "apiKey": gateway_key,
            "apiType": "chat-completions",
            "models": [
                {
                    "id": "gemini-3.8-flash-high",
                    "name": "Gemini 3.8 Flash",
                    "url": base_url,
                    "toolCalling": true,
                    "vision": true,
                    "thinking": true,
                    "supportsReasoningEffort": ["low", "medium", "high"],
                    "defaultReasoningEffort": "medium",
                    "maxInputTokens": 128000,
                    "maxOutputTokens": 16000,
                    "requestHeaders": {
                        "Authorization": auth_header
                    }
                },
                {
                    "id": "gemini-3-flash",
                    "name": "Gemini 3 Flash",
                    "url": base_url,
                    "toolCalling": true,
                    "vision": true,
                    "thinking": true,
                    "supportsReasoningEffort": ["low", "medium", "high"],
                    "defaultReasoningEffort": "medium",
                    "maxInputTokens": 128000,
                    "maxOutputTokens": 16000,
                    "requestHeaders": {
                        "Authorization": auth_header
                    }
                },
                {
                    "id": "gpt-6.1-terra",
                    "name": "GPT-6.1 Terra",
                    "url": base_url,
                    "toolCalling": true,
                    "vision": true,
                    "thinking": true,
                    "supportsReasoningEffort": ["low", "medium", "high", "max"],
                    "defaultReasoningEffort": "medium",
                    "maxInputTokens": 128000,
                    "maxOutputTokens": 16000,
                    "requestHeaders": {
                        "Authorization": auth_header
                    }
                },
                {
                    "id": "gpt-6-astra",
                    "name": "GPT-6 Astra",
                    "url": base_url,
                    "toolCalling": true,
                    "vision": true,
                    "thinking": true,
                    "supportsReasoningEffort": ["low", "medium", "high", "max"],
                    "defaultReasoningEffort": "medium",
                    "maxInputTokens": 128000,
                    "maxOutputTokens": 16000,
                    "requestHeaders": {
                        "Authorization": auth_header
                    }
                },
                {
                    "id": "gpt-5.6-terra",
                    "name": "GPT-5.6 Terra",
                    "url": base_url,
                    "toolCalling": true,
                    "vision": true,
                    "thinking": true,
                    "supportsReasoningEffort": ["low", "medium", "high", "max"],
                    "defaultReasoningEffort": "medium",
                    "maxInputTokens": 128000,
                    "maxOutputTokens": 16000,
                    "requestHeaders": {
                        "Authorization": auth_header
                    }
                },
                {
                    "id": "claude-sonnet-4-6",
                    "name": "Claude Sonnet 4.6",
                    "url": base_url,
                    "toolCalling": true,
                    "vision": true,
                    "thinking": true,
                    "supportsReasoningEffort": ["low", "medium", "high", "max"],
                    "defaultReasoningEffort": "medium",
                    "maxInputTokens": 128000,
                    "maxOutputTokens": 16000,
                    "requestHeaders": {
                        "Authorization": auth_header
                    }
                },
                {
                    "id": "claude-opus-4-6-thinking",
                    "name": "Claude Opus 4.6 Thinking",
                    "url": base_url,
                    "toolCalling": true,
                    "vision": true,
                    "thinking": true,
                    "supportsReasoningEffort": ["low", "medium", "high", "max"],
                    "defaultReasoningEffort": "high",
                    "maxInputTokens": 128000,
                    "maxOutputTokens": 16000,
                    "requestHeaders": {
                        "Authorization": auth_header
                    }
                }
            ]
        }
    ])
}

pub fn apply_vscode_config(
    target_path: &Path,
    base_url: &str,
    gateway_key: &str,
) -> Result<VSCodeDeployResult, String> {
    if let Some(parent) = target_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| format!("建立目錄失敗: {e}"))?;
        }
    }

    let backup_path = if target_path.exists() {
        let bak = target_path.with_extension("json.bak");
        fs::copy(target_path, &bak).map_err(|e| format!("備份現有設定檔失敗: {e}"))?;
        Some(bak.to_string_lossy().to_string())
    } else {
        None
    };

    let config_val = generate_vscode_config(base_url, gateway_key);
    let json_text = serde_json::to_string_pretty(&config_val).map_err(|e| e.to_string())?;

    fs::write(target_path, json_text).map_err(|e| format!("寫入 VS Code 設定檔失敗: {e}"))?;

    let model_count = config_val
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|grp| grp.get("models"))
        .and_then(|m| m.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);

    Ok(VSCodeDeployResult {
        target_path: target_path.to_string_lossy().to_string(),
        backup_path,
        model_count,
        message: format!("已成功部署至 VS Code ({model_count} 款模型已配置)"),
    })
}

pub fn preview_vscode_config(base_url: &str, gateway_key: &str) -> String {
    let masked_key = if gateway_key.len() > 16 {
        format!("{}...{}", &gateway_key[..8], &gateway_key[gateway_key.len()-6..])
    } else {
        "***".to_string()
    };
    let val = generate_vscode_config(base_url, &masked_key);
    serde_json::to_string_pretty(&val).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_and_apply_vscode_config() {
        let temp_dir = std::env::temp_dir().join(format!("agg-vscode-{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let target = temp_dir.join("chatLanguageModels.json");

        let res = apply_vscode_config(&target, "http://127.0.0.1:50833/v1", "test-key-1234567890abcdef").unwrap();
        assert_eq!(res.model_count, 7);
        assert!(target.exists());

        let text = fs::read_to_string(&target).unwrap();
        assert!(text.contains("Gemini 3.8 Flash"));
        assert!(!text.contains("Gemini 3.8 Flash High"));
        assert!(text.contains("supportsReasoningEffort"));

        // Second apply creates backup
        let res2 = apply_vscode_config(&target, "http://127.0.0.1:50833/v1", "test-key-1234567890abcdef").unwrap();
        assert!(res2.backup_path.is_some());
    }

    #[test]
    fn test_preview_masks_key() {
        let preview = preview_vscode_config("http://127.0.0.1:50833/v1", "agglocal003b7537d0ab643ff39ad576a9636e36d25875ea41da199efe6e16b43c4d0be7");
        assert!(preview.contains("agglocal...4d0be7"));
        assert!(!preview.contains("agglocal003b7537d0ab"));
    }
}

