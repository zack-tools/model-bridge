//! Conservative inspection of management credential metadata. Raw account details
//! and tokens never leave this module; matching alone is not live verification.
use crate::{http, redact::Redactor, registry::SourceKind};
use serde_json::Value;
use std::time::Duration;

/// Refuse a new login when any credential for this provider already exists.
pub fn ensure_provider_slot_empty(port: u16, key: &str, source: SourceKind) -> Result<(), String> {
    let bearer = format!("Bearer {key}");
    let redactor = Redactor::new([key.to_string()]);
    let response = http::exchange(port, "GET", "/v8/management/credentials", &[("Authorization", &bearer)], Duration::from_secs(20), &redactor)
        .map_err(|e| format!("無法檢查既有帳號：{e}"))?;
    if response.status != 200 { return Err("無法檢查既有帳號；未開始授權".into()); }
    ensure_slot_empty(&response.body, source)
}

fn ensure_slot_empty(body: &str, source: SourceKind) -> Result<(), String> {
    let value: Value = serde_json::from_str(body).map_err(|_| "帳號清單格式錯誤；未開始授權")?;
    let files = value.get("files").and_then(Value::as_array).ok_or("帳號清單不完整；未開始授權")?;
    let provider = match source { SourceKind::ChatGptCodex => "codex", SourceKind::Antigravity => "antigravity" };
    for entry in files {
        let kind = entry.get("provider").or_else(|| entry.get("type")).and_then(Value::as_str)
            .ok_or("帳號類型不明；未開始授權")?;
        if kind == provider { return Err("此類型已有本機憑證；暫不開放再次授權或新增同類帳號".into()); }
    }
    Ok(())
}

pub fn source_for_index(port: u16, management_key: &str, index: &str) -> Result<SourceKind, String> {
    if index.len() != 16 || !index.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("不支援的憑證索引格式（需為 16 位 16 進位）".into());
    }
    let bearer = format!("Bearer {management_key}");
    let redactor = Redactor::new([management_key.to_string()]);
    let response = http::exchange(port, "GET", "/v8/management/credentials", &[("Authorization", &bearer)], Duration::from_secs(20), &redactor)
        .map_err(|e| format!("憑證中繼資料請求失敗：{e}"))?;
    if response.status != 200 { return Err(format!("憑證中繼資料不可用 (HTTP {})", response.status)); }
    parse_source(&response.body, index)
}

/// Select one active credential only when the provider is unambiguous.
/// The index remains backend-only and does not prove which account was used.
pub fn single_active_index(port: u16, key: &str, source: SourceKind) -> Result<String, String> {
    let bearer = format!("Bearer {key}");
    let redactor = Redactor::new([key.to_string()]);
    let response = http::exchange(port, "GET", "/v8/management/credentials", &[("Authorization", &bearer)], Duration::from_secs(20), &redactor)
        .map_err(|e| format!("憑證中繼資料請求失敗：{e}"))?;
    if response.status != 200 { return Err(format!("憑證中繼資料不可用 (HTTP {})", response.status)); }
    single_index(&response.body, source)
}

fn single_index(body: &str, source: SourceKind) -> Result<String, String> {
    let value: Value = serde_json::from_str(body).map_err(|e| format!("憑證中繼資料非有效 JSON：{e}"))?;
    let files = value.get("files").and_then(Value::as_array).ok_or_else(|| "憑證中繼資料缺少 files 清單欄位".to_string())?;
    let provider = match source { SourceKind::ChatGptCodex => "codex", SourceKind::Antigravity => "antigravity" };
    let candidates: Vec<&Value> = files.iter().filter(|entry| {
        entry.get("provider").and_then(Value::as_str) == Some(provider)
            || entry.get("type").and_then(Value::as_str) == Some(provider)
    }).collect();
    if candidates.is_empty() {
        return Err(format!("未找到來源為 {provider} 的憑證（目前憑證總數：{}）", files.len()));
    }
    if candidates.len() > 1 {
        return Err(format!("來源為 {provider} 的憑證有多筆（共 {} 筆），無法確定唯一可用帳號", candidates.len()));
    }
    let index = candidates[0].get("auth_index").and_then(Value::as_str).ok_or_else(|| "憑證缺少 auth_index 欄位".to_string())?;
    if index.len() != 16 || !index.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("憑證索引格式不符合預期（長度：{}，需為 16 位 16 進位）", index.len()));
    }
    if parse_source(body, index)? != source { return Err("憑證來源種類不相符".to_string()); }
    Ok(index.to_string())
}

/// Discover only; returned models are not verified or selectable.
pub fn discover_models(port: u16, management_key: &str, index: &str) -> Result<(SourceKind, Vec<String>), String> {
    if index.len() != 16 || !index.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("不支援的憑證索引格式（需為 16 位 16 進位）".into());
    }
    let bearer = format!("Bearer {management_key}");
    let redactor = Redactor::new([management_key.to_string()]);
    let request = |path: &str| http::exchange(port, "GET", path, &[("Authorization", &bearer)], Duration::from_secs(20), &redactor)
        .map_err(|e| format!("管理介面請求失敗：{e}"));
    let credentials = request("/v8/management/credentials")?;
    if credentials.status != 200 { return Err(format!("憑證中繼資料不可用 (HTTP {})", credentials.status)); }
    let source = parse_source(&credentials.body, index)?;
    let name = unique_name(&credentials.body, index)?;
    let encoded: String = name.bytes().map(|b| {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() }
        else { format!("%{b:02X}") }
    }).collect();
    let response = request(&format!("/v8/management/credentials/models?name={encoded}"))?;
    if response.status != 200 { return Err(format!("憑證模型清單不可用 (HTTP {})", response.status)); }
    let mut models = parse_models(&response.body)?;
    if source == SourceKind::Antigravity {
        models.retain(|id| id.rsplit('/').next().unwrap_or("").to_ascii_lowercase().starts_with("gemini"));
    }
    // 依使用者需求：不支援串流工具調用之 image 模型不顯示於可用模型清單
    models.retain(|id| !crate::registry::is_image_model(id));
    Ok((source, models))
}

fn unique_name(body: &str, index: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(body).map_err(|e| format!("憑證中繼資料非有效 JSON：{e}"))?;
    let files = value.get("files").and_then(Value::as_array).ok_or_else(|| "憑證中繼資料缺少 files 清單欄位".to_string())?;
    let entry = files.iter().find(|entry| entry.get("auth_index").and_then(Value::as_str) == Some(index)).ok_or_else(|| "未找到對應 auth_index 的憑證項目".to_string())?;
    let name = entry.get("name").or_else(|| entry.get("id")).and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 1024).ok_or_else(|| "憑證名稱缺失或超過長度上限".to_string())?;
    if files.iter().filter(|entry| entry.get("name").and_then(Value::as_str) == Some(name) || entry.get("id").and_then(Value::as_str) == Some(name)).count() != 1 {
        return Err("憑證名稱存在衝突或模糊重疊".to_string());
    }
    Ok(name.to_string())
}

fn parse_models(body: &str) -> Result<Vec<String>, String> {
    let value: Value = serde_json::from_str(body).map_err(|e| format!("模型清單非有效 JSON：{e}"))?;
    let models = value.get("models").and_then(Value::as_array).ok_or_else(|| "模型回應缺少 models 清單欄位".to_string())?;
    if models.len() > 10000 { return Err("模型清單項目數量超出安全上限".to_string()); }
    let mut result = Vec::new();
    for model in models {
        let id = model.get("id").and_then(Value::as_str).ok_or_else(|| "模型項目缺少 id 欄位".to_string())?;
        let owner = model.get("owned_by").and_then(Value::as_str);
        if id.is_empty() || id.len() > 256 || id.bytes().any(|b| b.is_ascii_control()) { return Err("模型 id 包含控制字元或長度不合法".to_string()); }
        if !crate::registry::is_claude(id, owner) && !crate::registry::is_image_model(id) && !result.iter().any(|existing| existing == id) { result.push(id.to_string()); }
    }
    Ok(result)
}

fn parse_source(body: &str, index: &str) -> Result<SourceKind, String> {
    let value: Value = serde_json::from_str(body).map_err(|e| format!("憑證中繼資料非有效 JSON：{e}"))?;
    let files = value.get("files").and_then(Value::as_array).ok_or_else(|| "憑證中繼資料缺少 files 清單欄位".to_string())?;
    let mut matches = files.iter().filter(|entry| entry.get("auth_index").and_then(Value::as_str) == Some(index));
    let entry = matches.next().ok_or_else(|| "未找到對應 auth_index 的憑證項目".to_string())?;
    if matches.next().is_some() { return Err("憑證索引重複或衝突".to_string()); }
    let disabled = entry.get("disabled").and_then(Value::as_bool);
    let unavailable = entry.get("unavailable").and_then(Value::as_bool);
    if disabled != Some(false) || unavailable != Some(false) {
        return Err(format!("憑證狀態不可用或未知（disabled={disabled:?}, unavailable={unavailable:?}）"));
    }
    match entry.get("provider").or_else(|| entry.get("type")).and_then(Value::as_str) {
        Some("codex") => Ok(SourceKind::ChatGptCodex),
        Some("antigravity") => Ok(SourceKind::Antigravity),
        Some(other) => Err(format!("不支援的憑證提供者種類：{other}")),
        None => Err("憑證缺少 provider 欄位".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_login_is_blocked_when_provider_has_any_credential() {
        assert!(ensure_slot_empty(r#"{"files":[]}"#, SourceKind::ChatGptCodex).is_ok());
        assert!(ensure_slot_empty(r#"{"files":[{"provider":"antigravity"}]}"#, SourceKind::ChatGptCodex).is_ok());
        assert!(ensure_slot_empty(r#"{"files":[{"provider":"codex","disabled":true}]}"#, SourceKind::ChatGptCodex).is_err());
        assert!(ensure_slot_empty(r#"{"files":[{}]}"#, SourceKind::ChatGptCodex).is_err());
        assert!(ensure_slot_empty("{}", SourceKind::ChatGptCodex).is_err());
    }
    #[test]
    fn selecting_account_requires_a_unique_active_provider() {
        let active = serde_json::json!({"auth_index":"0123456789abcdef","provider":"codex","disabled":false,"unavailable":false});
        let doc = |items: Vec<Value>| serde_json::json!({"files":items}).to_string();
        assert_eq!(single_index(&doc(vec![active.clone()]), SourceKind::ChatGptCodex).unwrap(), "0123456789abcdef");
        assert!(single_index(&doc(vec![active.clone(), active.clone()]), SourceKind::ChatGptCodex).is_err());
        let mut disabled = active.clone(); disabled["disabled"] = true.into();
        assert!(single_index(&doc(vec![disabled]), SourceKind::ChatGptCodex).is_err());
        assert!(single_index(&doc(vec![active]), SourceKind::Antigravity).is_err());
    }

    #[test]
    fn discovery_requests_encoded_name_and_returns_unverified_models() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = std::thread::spawn(move || {
            for (path, body) in [
                ("/v8/management/credentials", r#"{"files":[{"auth_index":"0123456789abcdef","name":"private name.json","id":"private name.json","provider":"antigravity","disabled":false,"unavailable":false}]}"#),
                ("/v8/management/credentials/models?name=private%20name.json", r#"{"models":[{"id":"gemini-example"},{"id":"claude-hidden"},{"id":"other-model"}]}"#),
            ] {
                let (mut socket, _) = listener.accept().unwrap();
                let mut bytes = [0u8; 4096];
                let n = socket.read(&mut bytes).unwrap();
                assert!(String::from_utf8_lossy(&bytes[..n]).starts_with(&format!("GET {path} HTTP/1.1")));
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
        });
        let (source, models) = discover_models(port, "fixture-management-key", "0123456789abcdef").unwrap();
        assert_eq!(source, SourceKind::Antigravity);
        assert_eq!(models, vec!["gemini-example"]);
        worker.join().unwrap();
    }

    #[test]
    fn discovered_models_are_filtered_and_names_must_be_unambiguous() {
        let entries = serde_json::json!({"files":[{"name":"private name.json","id":"private name.json","auth_index":"0123456789abcdef"}]}).to_string();
        assert_eq!(unique_name(&entries, "0123456789abcdef").unwrap(), "private name.json");
        let collision = serde_json::json!({"files":[{"name":"same","id":"id1","auth_index":"0123456789abcdef"},{"name":"other","id":"same","auth_index":"fedcba9876543210"}]}).to_string();
        assert!(unique_name(&collision, "0123456789abcdef").is_err());
        let models = r#"{"models":[{"id":"gemini-example"},{"id":"gemini-example"},{"id":"claude-hidden"},{"id":"other","owned_by":"anthropic"},{"id":"gpt-image-2.5"}]}"#;
        assert_eq!(parse_models(models).unwrap(), vec!["gemini-example"]);
    }

    #[test]
    fn requests_metadata_over_loopback_without_exposing_details() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let n = socket.read(&mut request).unwrap();
            let headers = String::from_utf8_lossy(&request[..n]);
            assert!(headers.starts_with("GET /v8/management/credentials HTTP/1.1"));
            assert!(headers.contains("Authorization: Bearer fixture-management-key"));
            let body = r#"{"files":[{"auth_index":"0123456789abcdef","provider":"antigravity","disabled":false,"unavailable":false,"email":"private@example.test","token":"secret"}]}"#;
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        assert_eq!(source_for_index(port, "fixture-management-key", "0123456789abcdef"), Ok(SourceKind::Antigravity));
        worker.join().unwrap();
    }

    #[test]
    fn matches_only_unique_active_supported_credentials() {
        let account = serde_json::json!({"auth_index":"0123456789abcdef", "provider":"codex", "disabled":false, "unavailable":false, "email":"private@example.test", "access_token":"fixture-secret"});
        let document = |files: Vec<Value>| serde_json::json!({"files":files}).to_string();
        assert_eq!(parse_source(&document(vec![account.clone()]), "0123456789abcdef"), Ok(SourceKind::ChatGptCodex));
        assert!(parse_source(&document(vec![account.clone(), account.clone()]), "0123456789abcdef").is_err());
        let mut disabled = account.clone(); disabled["disabled"] = true.into();
        assert!(parse_source(&document(vec![disabled]), "0123456789abcdef").is_err());
        let mut unknown = account.clone(); unknown.as_object_mut().unwrap().remove("unavailable");
        assert!(parse_source(&document(vec![unknown]), "0123456789abcdef").is_err());
        let mut unsupported = account; unsupported["provider"] = "claude".into();
        assert!(parse_source(&document(vec![unsupported]), "0123456789abcdef").is_err());
    }
}
