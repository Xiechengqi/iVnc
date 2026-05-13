use log::{info, warn};
use serde::Serialize;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
}

impl ProxyPanelManager {
    pub fn new() -> Self {
        Self {
            process: Mutex::new(None),
        }
    }

    pub fn start_watchdog(self: Arc<Self>) {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(3)).await;
                if let Err(err) = self.ensure_running().await {
                    warn!("miao proxy panel watchdog failed: {}", err);
                }
            }
        });
    }

    pub async fn ensure_running(&self) -> Result<Option<u32>, String> {
        if let Some(pid) = self.running_pid() {
            return Ok(Some(pid));
        }
        self.start()
    }

    pub async fn restart(&self) -> Result<Option<u32>, String> {
        self.stop();
        self.start()
    }

    pub fn status(&self) -> ProxyPanelStatus {
        ProxyPanelStatus {
            running: self.running_pid().is_some(),
            pid: self.running_pid(),
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
        let bin = ensure_binary()?;
        ensure_runtime_links()?;
        let dir = data_dir()?;
        let log_path = dir.join("miao.log");
        let stdout = fs::File::create(&log_path)
            .map_err(|err| format!("failed to create miao log: {}", err))?;
        let stderr = stdout
            .try_clone()
            .map_err(|err| format!("failed to clone miao log: {}", err))?;

        let mut cmd = Command::new(&bin);
        cmd.current_dir(&dir)
            .env("HOME", home_dir())
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

fn ensure_runtime_links() -> Result<(), String> {
    let dir = data_dir()?;
    let target = dir.join("sing-box");
    let link = PathBuf::from("/tmp/miao-sing-box");
    if link.exists() || link.is_symlink() {
        let _ = fs::remove_file(&link);
    }
    std::os::unix::fs::symlink(&target, &link)
        .map_err(|err| format!("failed to create {}: {}", link.display(), err))?;
    Ok(())
}

fn data_dir() -> Result<PathBuf, String> {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/root/.config"))
        .join("ivnc")
        .join("miao");
    fs::create_dir_all(&dir).map_err(|err| format!("failed to create miao dir: {}", err))?;
    Ok(dir)
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/root".to_string())
}
