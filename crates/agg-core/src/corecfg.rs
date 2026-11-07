//! Canonical v8 config for the managed sidecar.
//!
//! The file binds 127.0.0.1, keeps remote management off, and disables the
//! control-panel updater. The management key is written to the private
//! generated config because the pinned core requires it for management API access.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenPolicy {
    pub host: String,
    pub port: u16,
}

impl ListenPolicy {
    pub fn new(host: &str, port: u16) -> Result<Self, String> {
        if host != "127.0.0.1" {
            return Err(format!("refusing host {host}; only 127.0.0.1 is allowed"));
        }
        if port == 0 {
            return Err("refusing port 0; upstream treats it as the default 8317".into());
        }
        Ok(Self {
            host: host.to_string(),
            port,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedConfig {
    pub host: String,
    pub port: u16,
    pub allow_remote: bool,
    pub discovery_enabled: bool,
    pub gateway_key: String,
    pub management_key: String,
    pub auth_dir: String,
    pub disable_control_panel: bool,
    pub disable_auto_update_panel: bool,
    pub request_retry: i64,
    pub session_affinity: bool,
}

#[derive(Debug)]
pub struct CorePaths {
    pub config_path: PathBuf,
    pub auth_dir: PathBuf,
}

pub fn write_managed_config(
    dir: &Path,
    policy: &ListenPolicy,
    gateway_key: &str,
    management_key: &str,
) -> Result<CorePaths, String> {
    validate_secret(gateway_key)?;
    validate_secret(management_key)?;
    if !dir.is_dir() {
        return Err(format!("runtime dir does not exist: {}", dir.display()));
    }
    let auth_dir = dir.join("auth");
    fs::create_dir_all(&auth_dir).map_err(|e| e.to_string())?;
    let config_path = dir.join("config.yaml");
    let yaml = render(policy, gateway_key, management_key, &auth_dir)?;
    write_private(&config_path, yaml.as_bytes())?;
    Ok(CorePaths {
        config_path,
        auth_dir,
    })
}

pub fn parse_managed_config(text: &str) -> Result<ManagedConfig, String> {
    let host = quoted("host:", text)?;
    let port = bare_value("port:", text)?
        .parse::<u16>()
        .map_err(|_| "bad port".to_string())?;
    let allow_remote = parse_bool(&bare_value("allow-remote:", text)?)?;
    let discovery_enabled = parse_bool(&bare_value("enabled:", text)?)?;
    let gateway_key = quoted_list_item(text)?;
    let management_key = quoted("secret-key:", text)?;
    let auth_dir = quoted("auth-dir:", text)?;
    let disable_control_panel = parse_bool(&bare_value("disable-control-panel:", text)?)?;
    let disable_auto_update_panel = parse_bool(&bare_value("disable-auto-update-panel:", text)?)?;
    let request_retry = bare_value("request-retry:", text)?
        .parse::<i64>()
        .map_err(|_| "bad request-retry".to_string())?;
    let session_affinity = parse_bool(&bare_value("session-affinity:", text)?)?;
    Ok(ManagedConfig {
        host,
        port,
        allow_remote,
        discovery_enabled,
        gateway_key,
        management_key,
        auth_dir,
        disable_control_panel,
        disable_auto_update_panel,
        request_retry,
        session_affinity,
    })
}

fn render(
    policy: &ListenPolicy,
    gateway_key: &str,
    management_key: &str,
    auth_dir: &Path,
) -> Result<String, String> {
    let auth = auth_dir.to_str().ok_or("auth dir is not utf-8")?;
    let auth = serde_json::to_string(auth).map_err(|e| format!("encode auth dir: {e}"))?;
    Ok(format!(
        r#"config-version: 8
server:
  host: "127.0.0.1"
  port: {port}
  discovery:
    enabled: false
  tls:
    enable: false
management:
  allow-remote: false
  secret-key: "{management_key}"
  disable-control-panel: true
  disable-auto-update-panel: true
access:
  api-keys:
    - "{gateway_key}"
routing:
  strategy: "fill-first"
  session-affinity: false
  retry:
    request-retry: 0
oauth:
  auth-dir: {auth}
observability:
  logs:
    request-log: false
    logging-to-file: false
"#,
        port = policy.port,
        gateway_key = gateway_key,
        management_key = management_key,
        auth = auth,
    ))
}

fn validate_secret(value: &str) -> Result<(), String> {
    if value.len() < 16
        || value.contains("your-api-key")
        || value.contains("sk-dummy")
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric())
    {
        return Err("gateway key must be a long alphanumeric local token".into());
    }
    Ok(())
}

fn quoted(prefix: &str, text: &str) -> Result<String, String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let rest = rest.trim();
            if rest.starts_with('"') && rest.ends_with('"') {
                return serde_json::from_str(rest).map_err(|_| format!("invalid quoted {prefix}"));
            }
        }
    }
    Err(format!("missing quoted {prefix}"))
}

fn quoted_list_item(text: &str) -> Result<String, String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("- \"") {
            if let Some(inner) = rest.strip_suffix('"') {
                return Ok(inner.to_string());
            }
        }
    }
    Err("missing api key list item".into())
}

fn bare_value(prefix: &str, text: &str) -> Result<String, String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return Ok(rest.trim().trim_matches('"').to_string());
        }
    }
    Err(format!("missing {prefix}"))
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!("bad bool {other}")),
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_path_is_escaped_and_round_trips() {
        let path = Path::new(r#"C:\Users\測試 使用者\auth"#);
        let config = render(&ListenPolicy::new("127.0.0.1", 18431).unwrap(), "localgateway012345", "management012345", path).unwrap();
        assert!(config.contains(r#"C:\\Users\\測試 使用者\\auth"#));
        assert_eq!(parse_managed_config(&config).unwrap().auth_dir, path.to_str().unwrap());
    }

    #[test]
    fn rejects_wildcard_and_port_zero() {
        assert!(ListenPolicy::new("0.0.0.0", 8317).is_err());
        assert!(ListenPolicy::new("127.0.0.1", 0).is_err());
    }

    #[test]
    fn round_trip_is_loopback_only() {
        let dir = std::env::temp_dir().join(format!("agg-cfg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let policy = ListenPolicy::new("127.0.0.1", 18431).unwrap();
        let paths = write_managed_config(
            &dir,
            &policy,
            "agglocal0123456789abcdef",
            "mgmtlocal0123456789abcd",
        )
        .unwrap();
        let text = fs::read_to_string(&paths.config_path).unwrap();
        assert!(!text.contains("0.0.0.0"));
        assert!(!text.contains("allow-remote: true"));
        let parsed = parse_managed_config(&text).unwrap();
        assert_eq!(parsed.host, "127.0.0.1");
        assert!(!parsed.allow_remote);
        assert!(!parsed.discovery_enabled);
        assert!(parsed.disable_control_panel);
        assert!(parsed.disable_auto_update_panel);
        assert_eq!(parsed.request_retry, 0);
        assert!(!parsed.session_affinity);
        assert_eq!(parsed.gateway_key, "agglocal0123456789abcdef");
        assert_eq!(parsed.management_key, "mgmtlocal0123456789abcd");
        let _ = fs::remove_dir_all(&dir);
    }
}
