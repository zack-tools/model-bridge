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
    #[cfg(windows)]
    adopted: Option<Box<dyn AdoptedProcess>>,
    lock: Option<File>,
    pub port: u16,
    pub config_path: PathBuf,
    pub auth_dir: PathBuf,
    stderr_path: PathBuf,
    stdout_path: PathBuf,
    redactor: Redactor,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProcessHealth { Healthy, Degraded, Exited }

#[cfg(windows)]
pub trait AdoptedProcess: Send {
    fn pid(&self) -> u32;
    fn image(&self) -> Result<PathBuf, String>;
    fn alive(&self) -> Result<bool, String>;
    fn stop(&self) -> Result<(), String>;
    fn verify(&self, port: u16, binary: &Path, config: &Path) -> Result<(), String>;
}

impl std::fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Supervisor")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct StartFailure {
    pub message: String,
    /// True only when spawning failed or the spawned child was confirmed stopped.
    pub child_absent: bool,
}

impl From<String> for StartFailure {
    fn from(message: String) -> Self { Self { message, child_absent: true } }
}
impl From<&str> for StartFailure {
    fn from(message: &str) -> Self { Self::from(message.to_string()) }
}

/// Result of the only production sidecar launch.
#[derive(Debug)]
pub struct VerifiedSpawn {
    pub binary_sha256: String,
    pub supervisor: Supervisor,
}

/// Pin failure happens before any process is created. Start failure happens
/// only after the pinned checksum and archive member both matched.
#[derive(Debug)]
pub enum SpawnVerifiedError {
    Pin(String),
    Start(StartFailure, String),
}

impl SpawnVerifiedError {
    pub fn child_absent(&self) -> bool {
        match self {
            Self::Pin(_) => true,
            Self::Start(failure, _) => failure.child_absent,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Pin(message) => message,
            Self::Start(failure, _) => failure.message.as_str(),
        }
    }
}

/// Every production launch uses one of these labels and the same
/// verify-then-spawn function. Mock tests call `Supervisor::start` directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarLaunch {
    DesktopGateway,
    HeadlessDaemon,
    PocStart,
    CoreCheck,
}

impl SidecarLaunch {
    pub fn spawn(self, archive: &Path, request: StartRequest) -> Result<VerifiedSpawn, SpawnVerifiedError> {
        let _ = self;
        Supervisor::spawn_verified(archive, request)
    }
}

impl Supervisor {
    #[cfg(windows)]
    pub fn adopt_desktop(dir: &Path, process: Box<dyn AdoptedProcess>, port: u16, gateway_key: &str, management_key: &str) -> Result<Self, String> {
        let runtime = dir.join("runtime");
        crate::instance::desktop_guard_runtime(&dir.join("gateway.running"), &runtime)?;
        let lock = lock_exclusive(&runtime.join("instance.lock"))?;
        let binary = process.image()?;
        if binary.file_name().and_then(|s| s.to_str()).map(|s| s.eq_ignore_ascii_case("cli-proxy-api.exe")) != Some(true) {
            return Err("背景程序不是預期的閘道核心".into());
        }
        let archive = crate::instance::archive_beside(&binary)?;
        crate::pin::verify_native_binary(&archive, &binary)?;
        process.verify(port, &binary, &runtime.join("config.yaml"))?;
        if !process.alive()? { return Err("背景程序已結束".into()); }
        let redactor = Redactor::new([gateway_key.to_owned(), management_key.to_owned()]);
        let bearer = format!("Bearer {management_key}");
        let response = crate::http::exchange(port, "GET", "/v8/management/credentials", &[("Authorization", &bearer)], Duration::from_secs(5), &redactor)?;
        if response.status != 200 || !process.alive()? { return Err("無法驗證背景閘道的管理身分".into()); }
        let access = format!("Bearer {gateway_key}");
        let models = crate::http::exchange(port, "GET", "/v1/models", &[("Authorization", &access)], Duration::from_secs(5), &redactor)?;
        if models.status != 200 || !process.alive()? { return Err("無法驗證背景閘道的存取金鑰".into()); }
        Ok(Self { child: None, adopted: Some(process), lock: Some(lock), port,
            config_path: runtime.join("config.yaml"), auth_dir: runtime.join("auth"),
            stdout_path: runtime.join("stdout.log"), stderr_path: runtime.join("stderr.log"), redactor })
    }

    pub fn start(req: StartRequest) -> Result<Self, String> {
        Self::start_detailed(req).map_err(|failure| failure.message)
    }

    /// Verify the pinned archive and the on-disk executable, then spawn.
    /// A checksum or member mismatch returns before `Command` runs.
    pub fn spawn_verified(archive: &Path, request: StartRequest) -> Result<VerifiedSpawn, SpawnVerifiedError> {
        let binary_sha256 = crate::pin::verify_native_binary(archive, &request.program)
            .map_err(SpawnVerifiedError::Pin)?;
        match Self::start_detailed(request) {
            Ok(supervisor) => Ok(VerifiedSpawn { binary_sha256, supervisor }),
            Err(failure) => Err(SpawnVerifiedError::Start(failure, binary_sha256)),
        }
    }

    pub fn start_detailed(req: StartRequest) -> Result<Self, StartFailure> {
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
        let program = fs::canonicalize(&req.program)
            .map_err(|e| format!("resolve core executable: {e}"))?;
        let mut command = Command::new(&program);
        command.env_clear();
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            command.creation_flags(CREATE_NO_WINDOW);

            let system_root = std::env::var_os("SystemRoot")
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "SystemRoot is missing; cannot prepare Windows runtime".to_string())?;
            command.env("SystemRoot", &system_root)
                .env("PATH", PathBuf::from(&system_root).join("System32"))
                .env("USERPROFILE", &req.work_dir)
                .env("APPDATA", &req.work_dir)
                .env("LOCALAPPDATA", &req.work_dir)
                .env("TEMP", &req.work_dir)
                .env("TMP", &req.work_dir);
        }
        #[cfg(not(windows))]
        command.env("PATH", "/usr/bin:/bin")
            .env("HOME", &req.work_dir)
            .env("TMPDIR", &req.work_dir)
            .env("XDG_CONFIG_HOME", &req.work_dir)
            .env("XDG_DATA_HOME", &req.work_dir);
        command.current_dir(&req.work_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .arg("-config")
            .arg(&paths.config_path)
            .arg("-local-model");
        #[cfg(not(windows))]
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
            #[cfg(windows)]
            adopted: None,
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
                let child_absent = supervisor.stop_checked().is_ok();
                Err(StartFailure { message: err, child_absent })
            }
        }
    }

    pub fn pid(&self) -> Option<u32> {
        if let Some(child) = self.child.as_ref() { return Some(child.id()); }
        #[cfg(windows)]
        if let Some(process) = self.adopted.as_ref() { return Some(process.pid()); }
        None
    }

    /// Leave the child running when the owning application exits.
    pub fn detach(&mut self) {
        self.child.take();
        #[cfg(windows)]
        self.adopted.take();
        self.lock.take();
    }

    pub fn stop(&mut self) {
        let _ = self.stop_checked();
    }

    pub fn stop_checked(&mut self) -> Result<(), String> {
        #[cfg(windows)]
        if let Some(process) = self.adopted.as_ref() {
            process.stop()?;
            self.adopted.take();
        }
        if let Some(child) = self.child.as_mut() {
            if child.try_wait().map_err(|e| format!("process status: {e}"))?.is_none() {
                if let Err(err) = child.kill() {
                    if child.try_wait().map_err(|e| format!("process status: {e}"))?.is_none() {
                        return Err(format!("stop core: {err}"));
                    }
                }
                child.wait().map_err(|e| format!("wait for core: {e}"))?;
            }
            self.child.take();
        }
        self.lock.take();
        Ok(())
    }

    pub fn redacted_logs(&self) -> String {
        let stdout = fs::read_to_string(&self.stdout_path).unwrap_or_default();
        let stderr = fs::read_to_string(&self.stderr_path).unwrap_or_default();
        let combined = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
        truncate(self.redactor.redact(&combined), 2000)
    }

    pub fn health(&mut self) -> Result<ProcessHealth, String> {
        if let Some(child) = self.child.as_mut() {
            if child.try_wait().map_err(|e| format!("process status: {e}"))?.is_some() { return Ok(ProcessHealth::Exited); }
        } else {
            #[cfg(windows)]
            if self.adopted.as_ref().map(|p| p.alive()).transpose()?.unwrap_or(false) {
                return self.check_health();
            }
            return Ok(ProcessHealth::Exited);
        }
        self.check_health()
    }

    fn check_health(&self) -> Result<ProcessHealth, String> {
        match crate::http::exchange(self.port, "GET", "/healthz", &[], Duration::from_millis(500), &self.redactor) {
            Ok(resp) if resp.status == 200 && resp.body.contains("ok") => Ok(ProcessHealth::Healthy),
            _ => Ok(ProcessHealth::Degraded),
        }
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
    #[cfg(target_os = "macos")]
    { return inspect_macos_listeners(port); }
    #[cfg(not(target_os = "macos"))]
    {
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
}

#[cfg(target_os = "macos")]
fn inspect_macos_listeners(port: u16) -> Result<ListenerReport, String> {
    let output = Command::new("/usr/sbin/lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fn"])
        .output().map_err(|e| format!("lsof: {e}"))?;
    if !output.status.success() { return Err("lsof failed to inspect listeners".into()); }
    let mut report = ListenerReport { loopback_v4: false, wildcard_v4: false, loopback_v6: false, wildcard_v6: false };
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some(address) = line.strip_prefix('n') else { continue };
        let Some((host, port_text)) = address.rsplit_once(':') else { continue };
        if port_text.parse::<u16>().ok() != Some(port) { continue; }
        match host {
            "127.0.0.1" => report.loopback_v4 = true,
            "[::1]" => report.loopback_v6 = true,
            "*" | "0.0.0.0" => report.wildcard_v4 = true,
            "[::]" => report.wildcard_v6 = true,
            _ => return Err(format!("unexpected listener address for port {port}")),
        }
    }
    Ok(report)
}

#[cfg(not(target_os = "macos"))]
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

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_lsof_sees_loopback_only() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let report = inspect_listeners(listener.local_addr().unwrap().port()).unwrap();
        assert!(report.loopback_v4);
        assert!(!report.wildcard_v4 && !report.wildcard_v6);
    }

    #[cfg(target_os = "linux")]
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
