//! Starts and stops one pinned sidecar process.
//!
//! There is no restart loop. A crash or a failed health check is returned
//! to the caller. The lock is a local file lock so a second start in the
//! same runtime directory fails.

use crate::corecfg::{write_managed_config, ListenPolicy};
use crate::redact::Redactor;
use fs2::FileExt;
use std::fs::{self, File, OpenOptions};
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct StartRequest {
    pub program: PathBuf,
    pub work_dir: PathBuf,
    pub port: Option<u16>,
    pub extra_env: Vec<(String, String)>,
    pub health_timeout: Duration,
    pub gateway_key: String,
    pub management_password: String,
}

pub struct Supervisor {
    child: Option<Child>,
    lock: Option<File>,
    pub port: u16,
    pub config_path: PathBuf,
    pub auth_dir: PathBuf,
    stderr_path: PathBuf,
    stdout_path: PathBuf,
    redactor: Redactor,
}

impl std::fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Supervisor")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl Supervisor {
    pub fn start(req: StartRequest) -> Result<Self, String> {
        if req.gateway_key.is_empty() || req.management_password.is_empty() {
            return Err("missing local gateway key or management password".into());
        }
        let redactor = Redactor::new([req.gateway_key.clone(), req.management_password.clone()]);
        fs::create_dir_all(&req.work_dir).map_err(|e| e.to_string())?;
        let lock = lock_exclusive(&req.work_dir.join("instance.lock"))?;
        let port = match req.port {
            Some(port) => port,
            None => free_port()?,
        };
        ensure_port_free(port)?;
        let policy = ListenPolicy::new("127.0.0.1", port)?;
        let paths = write_managed_config(
            &req.work_dir,
            &policy,
            &req.gateway_key,
            &req.management_password,
        )?;
        let stdout_path = req.work_dir.join("stdout.log");
        let stderr_path = req.work_dir.join("stderr.log");
        let stdout = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stdout_path)
            .map_err(|e| e.to_string())?;
        let stderr = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stderr_path)
            .map_err(|e| e.to_string())?;
        let mut command = Command::new(&req.program);
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &req.work_dir)
            .env("TMPDIR", &req.work_dir)
            .env("XDG_CONFIG_HOME", &req.work_dir)
            .env("XDG_DATA_HOME", &req.work_dir)
            .current_dir(&req.work_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .arg("-config")
            .arg(&paths.config_path)
            .arg("-local-model");
        if Path::new("/etc/ssl/certs/ca-certificates.crt").exists() {
            command.env(
                "SSL_CERT_FILE",
                "/etc/ssl/certs/ca-certificates.crt",
            );
        }
        for (key, value) in &req.extra_env {
            command.env(key, value);
        }
        let child = command.spawn().map_err(|e| {
            redactor.redact(&format!("spawn {}: {e}", req.program.display()))
        })?;
        let mut supervisor = Self {
            child: Some(child),
            lock: Some(lock),
            port,
            config_path: paths.config_path,
            auth_dir: paths.auth_dir,
            stderr_path,
            stdout_path,
            redactor,
        };
        match supervisor.wait_healthy(req.health_timeout) {
            Ok(()) => Ok(supervisor),
            Err(err) => {
                supervisor.stop();
                Err(err)
            }
        }
    }

    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.lock.take();
    }

    pub fn redacted_logs(&self) -> String {
        let stdout = fs::read_to_string(&self.stdout_path).unwrap_or_default();
        let stderr = fs::read_to_string(&self.stderr_path).unwrap_or_default();
        let combined = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
        truncate(self.redactor.redact(&combined), 2000)
    }

    fn wait_healthy(&mut self, timeout: Duration) -> Result<(), String> {
        let started = Instant::now();
        loop {
            if started.elapsed() > timeout {
                return Err(format!("health timed out\n{}", self.redacted_logs()));
            }
            if let Some(child) = self.child.as_mut() {
                if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                    return Err(format!(
                        "core exited {status}\n{}",
                        self.redacted_logs()
                    ));
                }
            }
            match crate::http::exchange(
                self.port,
                "GET",
                "/healthz",
                &[],
                Duration::from_millis(400),
                &self.redactor,
            ) {
                Ok(resp) if resp.status == 200 && resp.body.contains("ok") => return Ok(()),
                _ => std::thread::sleep(Duration::from_millis(40)),
            }
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn ensure_port_free(port: u16) -> Result<(), String> {
    match TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
        Ok(listener) => {
            drop(listener);
            Ok(())
        }
        Err(err) => Err(format!("port conflict on 127.0.0.1:{port}: {err}")),
    }
}

pub fn free_port() -> Result<u16, String> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    drop(listener);
    Ok(port)
}

fn lock_exclusive(path: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.try_lock_exclusive()
        .map_err(|_| "single instance lock is already held".to_string())?;
    Ok(file)
}

fn truncate(text: String, max: usize) -> String {
    let mut out = text;
    if out.len() > max {
        out.truncate(max);
        out.push_str("\n<truncated>");
    }
    out
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ListenerReport {
    pub loopback_v4: bool,
    pub wildcard_v4: bool,
    pub loopback_v6: bool,
    pub wildcard_v6: bool,
}

pub fn inspect_listeners(port: u16) -> Result<ListenerReport, String> {
    let mut report = ListenerReport {
        loopback_v4: false,
        wildcard_v4: false,
        loopback_v6: false,
        wildcard_v6: false,
    };
    inspect_proc("/proc/net/tcp", port, false, &mut report)?;
    if Path::new("/proc/net/tcp6").exists() {
        inspect_proc("/proc/net/tcp6", port, true, &mut report)?;
    }
    Ok(report)
}

fn inspect_proc(path: &str, port: u16, v6: bool, report: &mut ListenerReport) -> Result<(), String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    for line in text.lines().skip(1) {
        let mut cols = line.split_whitespace();
        let _slot = cols.next();
        let local = cols.next().unwrap_or("");
        let _remote = cols.next();
        let state = cols.next().unwrap_or("");
        if state != "0A" {
            continue;
        }
        let Some((addr, hex_port)) = local.split_once(':') else {
            continue;
        };
        let Ok(seen) = u16::from_str_radix(hex_port, 16) else {
            continue;
        };
        if seen != port {
            continue;
        }
        if v6 {
            if addr.chars().all(|c| c == '0') {
                report.wildcard_v6 = true;
            } else if addr.ends_with("00000001") && addr.chars().filter(|c| *c != '0').count() <= 1 {
                report.loopback_v6 = true;
            }
        } else if addr.eq_ignore_ascii_case("0100007F") {
            report.loopback_v4 = true;
        } else if addr.chars().all(|c| c == '0') {
            report.wildcard_v4 = true;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occupied_port_is_a_conflict() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(ensure_port_free(port).is_err());
        drop(listener);
    }

    #[test]
    fn linux_proc_sees_loopback_only() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let report = inspect_listeners(port).unwrap();
        assert!(report.loopback_v4);
        assert!(!report.wildcard_v4);
        assert!(!report.wildcard_v6);
    }
}
