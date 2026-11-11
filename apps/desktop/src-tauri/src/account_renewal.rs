//! Isolated staged sidecar for safe same-provider account renewal.
//! Existing credentials remain untouched in target_runtime/auth until explicit confirmation.

use agg_core::{
    instance,
    oauth::Provider,
    supervisor::{StartRequest, Supervisor},
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub struct Renewal {
    supervisor: Option<Supervisor>,
    runtime: PathBuf,
    target_auth: PathBuf,
    staging_keys: (String, String),
    provider: Provider,
}

impl Renewal {
    pub fn start(target_runtime: &Path, core_binary: &Path, provider: Provider) -> Result<Self, String> {
        let mut nonce = [0u8; 8];
        getrandom::getrandom(&mut nonce).map_err(|_| "無法產生隨機目錄名稱")?;
        let nonce_str = agg_core::pin::sha256_bytes(&nonce);
        let runtime = target_runtime.join(format!("staging-{}", &nonce_str[..16]));
        let auth_dir = runtime.join("auth");
        std::fs::create_dir_all(&auth_dir).map_err(|e| format!("建立暫存授權目錄失敗：{e}"))?;

        let (gateway_key, management_key) = instance::ensure_local_keys(&runtime)?;

        let req = StartRequest {
            program: core_binary.to_path_buf(),
            work_dir: runtime.clone(),
            port: None,
            extra_env: vec![],
            health_timeout: Duration::from_secs(30),
            gateway_key: gateway_key.clone(),
            management_password: management_key.clone(),
        };

        let supervisor = Supervisor::start(req).map_err(|e| format!("啟動暫存授權核心失敗：{e}"))?;

        Ok(Self {
            supervisor: Some(supervisor),
            runtime,
            target_auth: target_runtime.join("auth"),
            staging_keys: (gateway_key, management_key),
            provider,
        })
    }

    pub fn port(&self) -> u16 {
        self.supervisor.as_ref().map(|s| s.port).unwrap_or(0)
    }

    pub fn management_key(&self) -> &str {
        &self.staging_keys.1
    }

    pub fn commit(&mut self) -> Result<(), String> {
        let staging_auth = self.runtime.join("auth");
        let entries = std::fs::read_dir(&staging_auth).map_err(|e| format!("無法讀取暫存憑證目錄：{e}"))?;
        let provider_name = self.provider.upstream_name();

        let mut staged_file = None;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    if name.contains(provider_name) && name.ends_with(".json") {
                        staged_file = Some(path);
                        break;
                    }
                }
            }
        }
        let staged_path = staged_file.ok_or_else(|| "未找到新完成授權之憑證檔案".to_string())?;
        let staged_content = std::fs::read(&staged_path).map_err(|e| format!("無法讀取新憑證：{e}"))?;

        if let Some(mut sup) = self.supervisor.take() {
            let _ = sup.stop_checked();
        }

        let mut target_file = None;
        if let Ok(entries) = std::fs::read_dir(&self.target_auth) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                        if name.contains(provider_name) && name.ends_with(".json") {
                            target_file = Some(path);
                            break;
                        }
                    }
                }
            }
        }

        std::fs::create_dir_all(&self.target_auth).map_err(|e| format!("無法建立憑證目錄：{e}"))?;
        let target_dest = target_file.unwrap_or_else(|| {
            let file_name = staged_path.file_name().unwrap();
            self.target_auth.join(file_name)
        });

        let tmp_dest = target_dest.with_extension("tmp");
        std::fs::write(&tmp_dest, &staged_content).map_err(|e| format!("寫入新憑證失敗：{e}"))?;
        std::fs::rename(&tmp_dest, &target_dest).map_err(|e| format!("更新憑證失敗：{e}"))?;

        let _ = std::fs::remove_dir_all(&self.runtime);
        Ok(())
    }
}

impl Drop for Renewal {
    fn drop(&mut self) {
        if let Some(mut sup) = self.supervisor.take() {
            let _ = sup.stop_checked();
        }
        let _ = std::fs::remove_dir_all(&self.runtime);
    }
}
