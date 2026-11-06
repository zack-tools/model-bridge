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

pub const PROVIDER_ID: &str = "app_agg_001";

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

#[derive(Debug, Clone)]
pub struct ApplyRequest {
    pub config_path: PathBuf,
    pub base_url: String,
    pub gateway_key: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyPreview {
    pub diff: String,
    pub creates_file: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Ownership {
    tool: String,
    generation: u64,
    sha256: String,
    managed_provider_id: String,
    backup: Option<String>,
    created: bool,
}

pub fn preview_apply(req: &ApplyRequest) -> Result<ApplyPreview, CodexError> {
    validate_request(req)?;
    let before = read_existing(&req.config_path)?;
    reject_foreign(&req.config_path, &before)?;
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

fn reject_foreign(path: &Path, before: &str) -> Result<(), CodexError> {
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
    let providers = nested_table(doc.as_table_mut(), "model_providers");
    let entry = nested_table(providers, PROVIDER_ID);
    entry["name"] = toml_edit::value("APP-AGG-001");
    entry["base_url"] = toml_edit::value(req.base_url.as_str());
    entry["wire_api"] = toml_edit::value("responses");
    entry["requires_openai_auth"] = toml_edit::value(false);
    let headers = nested_table(entry, "http_headers");
    headers["X-Api-Key"] = toml_edit::value(req.gateway_key.as_str());
    Ok(doc.to_string())
}

fn nested_table<'a>(table: &'a mut Table, key: &str) -> &'a mut Table {
    if !table.get(key).map(Item::is_table).unwrap_or(false) {
        table.insert(key, Item::Table(Table::new()));
    }
    table.get_mut(key).and_then(Item::as_table_mut).unwrap()
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
}
