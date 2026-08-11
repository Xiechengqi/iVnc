use log::{info, warn};
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::Mutex as AsyncMutex;

const MIAO_BIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/miao-rust-linux-amd64"));
pub const MIAO_PORT: u16 = 6161;

#[derive(Debug)]
struct RunningProxy {
    child: Child,
    pid: u32,
}

#[derive(Debug, Serialize)]
pub struct ProxyPanelStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub port: u16,
    pub url: String,
}

#[derive(Debug)]
pub struct ProxyPanelManager {
    process: Mutex<Option<RunningProxy>>,
    start_lock: AsyncMutex<()>,
}

impl ProxyPanelManager {
    pub fn new() -> Self {
        Self {
            process: Mutex::new(None),
            start_lock: AsyncMutex::new(()),
        }
    }

    pub fn start_watchdog(self: Arc<Self>) {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(3)).await;
                if let Some(pid) = self.running_pid() {
                    if let Err(err) = self.wait_until_ready(pid).await {
                        warn!("miao proxy panel watchdog check failed: {}", err);
                    }
                }
            }
        });
    }

    pub async fn ensure_running(&self) -> Result<Option<u32>, String> {
        let _guard = self.start_lock.lock().await;
        if let Some(pid) = self.running_pid() {
            self.wait_until_ready(pid).await?;
            return Ok(Some(pid));
        }
        let pid = self.start()?;
        if let Some(pid) = pid {
            self.wait_until_ready(pid).await?;
        }
        Ok(pid)
    }

    pub async fn restart(&self) -> Result<Option<u32>, String> {
        let _guard = self.start_lock.lock().await;
        self.stop();
        let pid = self.start()?;
        if let Some(pid) = pid {
            self.wait_until_ready(pid).await?;
        }
        Ok(pid)
    }

    pub fn status(&self) -> ProxyPanelStatus {
        let pid = self.running_pid();
        ProxyPanelStatus {
            running: pid.is_some(),
            pid,
            port: MIAO_PORT,
            url: "/proxy/".to_string(),
        }
    }

    fn running_pid(&self) -> Option<u32> {
        let mut guard = self.process.lock().unwrap();
        if let Some(running) = guard.as_mut() {
            match running.child.try_wait() {
                Ok(None) => return Some(running.pid),
                Ok(Some(status)) => info!("miao proxy panel exited: {}", status),
                Err(err) => warn!("failed to poll miao proxy panel: {}", err),
            }
            *guard = None;
        }
        None
    }

    fn start(&self) -> Result<Option<u32>, String> {
        if let Some(pid) = self.running_pid() {
            return Ok(Some(pid));
        }

        let dir = data_dir()?;
        let expected_bin = dir.join("miao-rust-linux-amd64");
        cleanup_stale_miao_processes(&expected_bin)?;
        let bin = ensure_binary()?;
        cleanup_legacy_runtime_link()?;
        let home = crate::paths::host_home();
        fs::create_dir_all(home.join(".miao"))
            .map_err(|err| format!("failed to create miao data dir: {}", err))?;
        let log_path = dir.join("miao.log");
        let stdout = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|err| format!("failed to open miao log: {}", err))?;
        let stderr = stdout
            .try_clone()
            .map_err(|err| format!("failed to clone miao log: {}", err))?;

        let mut cmd = Command::new(&bin);
        cmd.current_dir(&dir)
            .env("HOME", &home)
            .env("MIAO_PORT", MIAO_PORT.to_string())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        unsafe {
            cmd.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = cmd
            .spawn()
            .map_err(|err| format!("failed to start miao proxy panel: {}", err))?;
        let pid = child.id();
        info!(
            "Started miao proxy panel (pid={}, port={}, log={})",
            pid,
            MIAO_PORT,
            log_path.display()
        );
        *self.process.lock().unwrap() = Some(RunningProxy { child, pid });
        Ok(Some(pid))
    }

    async fn wait_until_ready(&self, expected_pid: u32) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        loop {
            if self.running_pid() != Some(expected_pid) {
                return Err(format!(
                    "miao proxy panel pid {} exited before port {} became ready\n{}",
                    expected_pid,
                    MIAO_PORT,
                    log_tail()
                ));
            }

            match TcpStream::connect(("127.0.0.1", MIAO_PORT)).await {
                Ok(_) if self.running_pid() == Some(expected_pid) => return Ok(()),
                Ok(_) => {
                    return Err(format!(
                        "miao proxy panel pid changed while waiting for port {}\n{}",
                        MIAO_PORT,
                        log_tail()
                    ));
                }
                Err(err) => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(format!(
                            "miao proxy panel pid {} did not listen on 127.0.0.1:{} within 8s: {}\n{}",
                            expected_pid,
                            MIAO_PORT,
                            err,
                            log_tail()
                        ));
                    }
                }
            }

            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    fn stop(&self) {
        let mut guard = self.process.lock().unwrap();
        if let Some(mut running) = guard.take() {
            let pgid = -(running.pid as i32);
            if unsafe { libc::kill(pgid, libc::SIGTERM) } != 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() != Some(libc::ESRCH) {
                    warn!("failed to stop miao process group {}: {}", pgid, err);
                }
            }
            let _ = running.child.wait();
        }
    }
}

fn cleanup_stale_miao_processes(bin: &Path) -> Result<(), String> {
    let current_pid = std::process::id() as i32;
    let bin = bin.to_string_lossy();
    let mut candidates = Vec::new();

    for entry in fs::read_dir("/proc").map_err(|err| format!("failed to read /proc: {}", err))? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let pid = match entry.file_name().to_string_lossy().parse::<i32>() {
            Ok(pid) if pid > 1 && pid != current_pid => pid,
            _ => continue,
        };
        let cmdline_path = entry.path().join("cmdline");
        let cmdline = match fs::read(&cmdline_path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).replace('\0', " "),
            Err(_) => continue,
        };
        if cmdline.contains(bin.as_ref()) || cmdline.contains("miao-rust-linux-amd64") {
            candidates.push(pid);
        }
    }

    if candidates.is_empty() {
        return Ok(());
    }

    warn!("cleaning up {} stale miao process(es)", candidates.len());
    for pid in &candidates {
        terminate_process_group_or_pid(*pid, libc::SIGTERM);
    }
    std::thread::sleep(Duration::from_millis(300));
    for pid in &candidates {
        reap_child_if_possible(*pid);
        if PathBuf::from(format!("/proc/{pid}")).exists() {
            terminate_process_group_or_pid(*pid, libc::SIGKILL);
            reap_child_if_possible(*pid);
        }
    }
    std::thread::sleep(Duration::from_millis(100));

    Ok(())
}

fn terminate_process_group_or_pid(pid: i32, signal: i32) {
    unsafe {
        let pgid = libc::getpgid(pid);
        if pgid > 1 {
            let _ = libc::kill(-pgid, signal);
        } else {
            let _ = libc::kill(pid, signal);
        }
    }
}

fn reap_child_if_possible(pid: i32) {
    let mut status = 0;
    unsafe {
        let _ = libc::waitpid(pid, &mut status, libc::WNOHANG);
    }
}

impl Drop for ProxyPanelManager {
    fn drop(&mut self) {
        self.stop();
    }
}

fn ensure_binary() -> Result<PathBuf, String> {
    let dir = data_dir()?;
    let bin = dir.join("miao-rust-linux-amd64");
    if !bin.exists() || fs::read(&bin).map(|data| data != MIAO_BIN).unwrap_or(true) {
        fs::write(&bin, MIAO_BIN).map_err(|err| format!("failed to write miao binary: {}", err))?;
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755))
            .map_err(|err| format!("failed to chmod miao binary: {}", err))?;
    }
    Ok(bin)
}

fn cleanup_legacy_runtime_link() -> Result<(), String> {
    let link = PathBuf::from("/tmp/miao-sing-box");
    if let Ok(meta) = fs::symlink_metadata(&link) {
        let file_type = meta.file_type();
        if file_type.is_dir() && !file_type.is_symlink() {
            fs::remove_dir_all(&link)
                .map_err(|err| format!("failed to remove legacy {}: {}", link.display(), err))?;
        } else {
            fs::remove_file(&link)
                .map_err(|err| format!("failed to remove legacy {}: {}", link.display(), err))?;
        }
    }
    Ok(())
}

fn data_dir() -> Result<PathBuf, String> {
    let dir = crate::paths::miao_dir();
    fs::create_dir_all(&dir).map_err(|err| format!("failed to create miao dir: {}", err))?;
    Ok(dir)
}

fn log_tail() -> String {
    let path = data_dir()
        .unwrap_or_else(|_| crate::paths::miao_dir())
        .join("miao.log");
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|err| format!("failed to read {}: {}", path.display(), err));
    let lines: Vec<&str> = content.lines().rev().take(40).collect();
    let tail = lines.into_iter().rev().collect::<Vec<_>>().join("\n");
    format!("miao log tail ({}):\n{}", path.display(), tail)
}
