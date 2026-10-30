use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::net::IpAddr;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;

const MIAO_BIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/miao-rust-linux-amd64"));
pub const BUILTIN_MIAO_ID: &str = "builtin-miao";
pub const MIAO_PORT: u16 = 6161;

#[derive(Debug)]
struct RunningProxy {
    child: Child,
    pid: u32,
    exited: bool,
    // Keep the configuration lock until the owned process group stops.
    _instance_lock: File,
}

impl Drop for RunningProxy {
    fn drop(&mut self) {
        terminate_group(&mut self.child, self.pid);
    }
}

#[derive(Debug, Default)]
struct Runtime {
    process: Option<RunningProxy>,
    ready: bool,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProxyPanelStatus {
    pub running: bool,
    pub ready: bool,
    pub enabled: bool,
    pub desired_running: bool,
    pub pid: Option<u32>,
    pub port: u16,
    pub url: &'static str,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HealthResponse {
    success: bool,
    data: Health,
}

#[derive(Debug, Deserialize)]
struct Health {
    service: String,
    service_pid: u32,
    embedded: bool,
    integration_version: u32,
    initializing: bool,
    proxy_running: bool,
    socks_listen: String,
    socks_port: u16,
}

#[derive(Debug)]
pub struct ProxyPanelManager {
    runtime: Mutex<Runtime>,
    operation: AsyncMutex<()>,
    desired_running: AtomicBool,
    shutting_down: AtomicBool,
    watchdog_started: AtomicBool,
    enabled: bool,
    port: u16,
    dir: PathBuf,
    home: PathBuf,
    #[cfg(test)]
    binary_override: Option<PathBuf>,
}

impl ProxyPanelManager {
    pub fn new(enabled: bool) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        Self {
            runtime: Mutex::new(Runtime::default()),
            operation: AsyncMutex::new(()),
            desired_running: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
            watchdog_started: AtomicBool::new(false),
            enabled,
            port: std::env::var("IVNC_MIAO_PORT")
                .ok()
                .and_then(|value| value.parse::<u16>().ok())
                .filter(|port| *port > 0)
                .unwrap_or(MIAO_PORT),
            dir: cwd.join(crate::paths::miao_dir()),
            home: cwd.join(crate::paths::host_home()),
            #[cfg(test)]
            binary_override: None,
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    #[cfg(test)]
    pub(crate) fn isolated(enabled: bool, root: &Path) -> Self {
        let mut manager = Self::new(enabled);
        manager.dir = root.join("miao");
        manager.home = root.join("home");
        manager
    }
    pub fn log_path(&self) -> PathBuf {
        self.dir.join("miao.log")
    }
    pub fn data_path(&self) -> PathBuf {
        self.home.join(".miao")
    }
    fn runtime_dir(&self) -> PathBuf {
        self.dir.join("runtime")
    }
    fn chrome_proxy_path(&self) -> PathBuf {
        self.dir.join("chrome-proxy")
    }

    pub fn start_watchdog(self: &Arc<Self>) {
        if self.watchdog_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut failures = 0u32;
            loop {
                tokio::time::sleep(Duration::from_secs(3 * (1u64 << failures.min(4)))).await;
                let Some(manager) = weak.upgrade() else {
                    break;
                };
                if manager.shutting_down.load(Ordering::SeqCst) {
                    break;
                }
                if !manager.desired_running.load(Ordering::SeqCst) {
                    continue;
                }
                let _guard = manager.operation.lock().await;
                if !manager.desired_running.load(Ordering::SeqCst) {
                    continue;
                }
                if manager.status().running {
                    match manager.refresh_chrome_proxy().await {
                        Ok(()) => {
                            failures = 0;
                            continue;
                        }
                        Err(error) => {
                            let mut runtime = manager.runtime.lock().unwrap();
                            runtime.ready = false;
                            runtime.error = Some(error);
                        }
                    }
                    failures = failures.saturating_add(1);
                    if failures < 3 {
                        continue;
                    }
                    manager.stop_process().await;
                }
                match manager.start_locked().await {
                    Ok(_) => failures = 0,
                    Err(err) => {
                        failures = failures.saturating_add(1);
                        warn!("miao recovery failed: {}", err);
                    }
                }
            }
        });
    }

    pub async fn start(&self) -> Result<Option<u32>, String> {
        let _guard = self.operation.lock().await;
        self.check_enabled()?;
        self.desired_running.store(true, Ordering::SeqCst);
        self.start_locked().await
    }

    pub async fn restart(&self) -> Result<Option<u32>, String> {
        let _guard = self.operation.lock().await;
        self.check_enabled()?;
        self.desired_running.store(true, Ordering::SeqCst);
        self.stop_process().await;
        self.start_locked().await
    }

    pub async fn stop(&self) -> Result<(), String> {
        self.desired_running.store(false, Ordering::SeqCst);
        let _guard = self.operation.lock().await;
        self.desired_running.store(false, Ordering::SeqCst);
        self.stop_process().await;
        self.runtime.lock().unwrap().error = None;
        Ok(())
    }

    pub fn begin_shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        self.desired_running.store(false, Ordering::SeqCst);
    }

    pub async fn shutdown(&self) {
        self.begin_shutdown();
        let _ = self.stop().await;
    }

    /// Clean this instance's orphans after an abrupt host exit, including when autostart is off.
    pub async fn cleanup_inactive_instance(&self) -> Result<(), String> {
        let _guard = self.operation.lock().await;
        if self.status().running || self.desired_running.load(Ordering::SeqCst) {
            return Ok(());
        }
        let _ = fs::remove_file(self.chrome_proxy_path());
        let data = self.data_path();
        let bin = self.binary_path();
        let runtime = self.runtime_dir();
        tokio::task::spawn_blocking(move || {
            if !data.exists() {
                return Ok(());
            }
            let _lock = lock_configuration(&data)?;
            cleanup_stale_processes(&bin, &runtime)
        })
        .await
        .map_err(|error| error.to_string())?
    }

    fn check_enabled(&self) -> Result<(), String> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err("miao is shutting down".into());
        }
        if !self.enabled {
            return Err("proxy disabled by administrator".into());
        }
        Ok(())
    }

    pub fn status(&self) -> ProxyPanelStatus {
        let mut state = self.runtime.lock().unwrap();
        let mut exit = None;
        let pid = state.process.as_mut().and_then(|running| {
            if running.exited {
                return None;
            }
            match running.child.try_wait() {
                Ok(None) => Some(running.pid),
                result => {
                    running.exited = true;
                    exit = Some(format!("miao exited: {:?}", result));
                    None
                }
            }
        });
        if let Some(error) = exit {
            state.error = Some(error);
            state.ready = false;
        }
        ProxyPanelStatus {
            running: pid.is_some(),
            ready: pid.is_some() && state.ready,
            enabled: self.enabled,
            desired_running: self.desired_running.load(Ordering::SeqCst),
            pid,
            port: self.port,
            url: "/proxy/",
            error: state.error.clone(),
        }
    }

    /// Read-only: opening a panel or a socket never resurrects a stopped app.
    pub fn require_running(&self) -> Result<(), String> {
        self.check_enabled()?;
        if !self.status().ready {
            return Err("Miao is stopped or starting; start it from the application list".into());
        }
        Ok(())
    }

    async fn start_locked(&self) -> Result<Option<u32>, String> {
        let result = async {
            let pid = if let Some(pid) = self.status().pid { pid } else {
                self.stop_process().await;
                self.spawn().await?
            };
            let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
            loop {
                if !self.desired_running.load(Ordering::SeqCst) || self.shutting_down.load(Ordering::SeqCst) {
                    return Err("miao startup cancelled".into());
                }
                if self.status().pid != Some(pid) { return Err(format!("miao pid {pid} exited before becoming ready")); }
                if let Ok(health) = self.health(pid).await {
                    self.runtime.lock().unwrap().ready = true;
                    self.write_chrome_proxy(&health)?;
                    return Ok(Some(pid));
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(format!("miao did not report embedded readiness at 127.0.0.1:{}; rebuild with the paired Miao source", self.port));
                }
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
        }.await;
        if let Err(error) = result {
            self.stop_process().await;
            let error = format!("{}\n{}", error, self.log_tail());
            self.runtime.lock().unwrap().error = Some(error.clone());
            return Err(error);
        }
        self.runtime.lock().unwrap().error = None;
        result
    }

    async fn spawn(&self) -> Result<u32, String> {
        let dir = self.dir.clone();
        let home = self.home.clone();
        let data_path = self.data_path();
        let runtime_dir = self.runtime_dir();
        let log_path = self.log_path();
        let bin = self.binary_path();
        let port = self.port;
        #[cfg(test)]
        let use_embedded = self.binary_override.is_none();
        #[cfg(not(test))]
        let use_embedded = true;
        // Extraction, stale cleanup and process waits must not block HTTP workers.
        let process = tokio::task::spawn_blocking(move || -> Result<RunningProxy, String> {
            if use_embedded && unsafe { libc::geteuid() } != 0 {
                return Err("Miao requires root privileges".into());
            }
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            fs::create_dir_all(&data_path).map_err(|e| e.to_string())?;
            let instance_lock = lock_configuration(&data_path)?;
            cleanup_stale_processes(&bin, &runtime_dir)?;
            if use_embedded {
                write_atomic(&bin, MIAO_BIN, 0o755)?;
            }
            let stdout = OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(log_path)
                .map_err(|e| e.to_string())?;
            let stderr = stdout.try_clone().map_err(|e| e.to_string())?;
            let mut command = Command::new(&bin);
            command
                .current_dir(dir)
                .env("HOME", home)
                .env("MIAO_EMBEDDED", "ivnc")
                .env("MIAO_PORT", port.to_string())
                .env("MIAO_RUNTIME_DIR", runtime_dir)
                .stdin(Stdio::null())
                .stdout(stdout)
                .stderr(stderr);
            unsafe {
                command.pre_exec(|| {
                    if libc::setpgid(0, 0) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let child = command
                .spawn()
                .map_err(|e| format!("failed to start miao: {e}"))?;
            let pid = child.id();
            Ok(RunningProxy {
                child,
                pid,
                exited: false,
                _instance_lock: instance_lock,
            })
        })
        .await
        .map_err(|error| format!("Miao process task failed: {error}"))??;
        let pid = process.pid;
        self.runtime.lock().unwrap().process = Some(process);
        info!("Started Miao (pid={}, port={})", pid, self.port);
        Ok(pid)
    }

    fn binary_path(&self) -> PathBuf {
        #[cfg(test)]
        if let Some(path) = &self.binary_override {
            return path.clone();
        }
        self.dir.join("miao-rust-linux-amd64")
    }

    async fn stop_process(&self) {
        let process = {
            let mut state = self.runtime.lock().unwrap();
            state.ready = false;
            state.process.take()
        };
        let _ = fs::remove_file(self.chrome_proxy_path());
        if let Some(process) = process {
            let _ = tokio::task::spawn_blocking(move || terminate_owned_group(process)).await;
        }
    }

    async fn health(&self, expected_pid: u32) -> Result<Health, String> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(800))
            .build()
            .map_err(|e| e.to_string())?;
        let response: HealthResponse = client
            .get(format!("http://127.0.0.1:{}/api/auth/status", self.port))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        if !response.success
            || response.data.service != "miao"
            || !response.data.embedded
            || response.data.integration_version != 1
            || response.data.service_pid != expected_pid
            || self.status().pid != Some(expected_pid)
        {
            return Err(
                "miao readiness response belongs to another process or incompatible build".into(),
            );
        }
        Ok(response.data)
    }

    async fn refresh_chrome_proxy(&self) -> Result<(), String> {
        let pid = self.status().pid.ok_or("miao is stopped")?;
        self.write_chrome_proxy(&self.health(pid).await?)?;
        // A disconnected start request can leave a live child awaiting its first probe.
        let mut runtime = self.runtime.lock().unwrap();
        runtime.ready = true;
        runtime.error = None;
        Ok(())
    }

    /// Chrome inspects the live kernel, not just the panel's listening port.
    pub async fn prepare_chrome_proxy(&self) {
        let _guard = self.operation.lock().await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        while let Some(pid) = self.status().pid {
            match self.health(pid).await {
                Ok(health) => {
                    let _ = self.write_chrome_proxy(&health);
                    if !health.initializing || tokio::time::Instant::now() >= deadline {
                        return;
                    }
                }
                Err(_) => break,
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let _ = fs::remove_file(self.chrome_proxy_path());
    }

    fn write_chrome_proxy(&self, health: &Health) -> Result<(), String> {
        if !health.proxy_running {
            let _ = fs::remove_file(self.chrome_proxy_path());
            return Ok(());
        }
        let address = health
            .socks_listen
            .parse::<IpAddr>()
            .map_err(|e| e.to_string())?;
        let address = match address {
            IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
            address => address,
        };
        let address = match address {
            IpAddr::V6(ip) => format!("[{ip}]"),
            address => address.to_string(),
        };
        if health.socks_port == 0 {
            return Err("invalid Miao SOCKS port".into());
        }
        write_atomic(
            &self.chrome_proxy_path(),
            format!("socks5://{}:{}\n", address, health.socks_port).as_bytes(),
            0o600,
        )
    }

    fn log_tail(&self) -> String {
        format!(
            "Miao log ({}):\n{}",
            self.log_path().display(),
            read_log_tail(&self.log_path(), 40)
        )
    }
}

impl Drop for ProxyPanelManager {
    fn drop(&mut self) {
        if let Some(process) = self.runtime.get_mut().unwrap().process.take() {
            terminate_owned_group(process);
        }
    }
}

fn terminate_owned_group(process: RunningProxy) {
    drop(process);
}

fn lock_configuration(data_path: &Path) -> Result<File, String> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(data_path.join("ivnc.lock"))
        .map_err(|error| error.to_string())?;
    if unsafe {
        libc::flock(
            std::os::fd::AsRawFd::as_raw_fd(&lock),
            libc::LOCK_EX | libc::LOCK_NB,
        )
    } != 0
    {
        return Err("Miao configuration is already managed by another iVnc instance".into());
    }
    Ok(lock)
}

fn terminate_group(child: &mut Child, pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let _ = child.try_wait();
        if unsafe { libc::kill(-(pid as i32), 0) } != 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    let _ = child.wait();
}

fn cleanup_stale_processes(bin: &Path, runtime: &Path) -> Result<(), String> {
    for entry in fs::read_dir("/proc").map_err(|e| e.to_string())?.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        if pid <= 1 || pid == std::process::id() as i32 {
            continue;
        }
        let Ok(exe) = fs::read_link(entry.path().join("exe")) else {
            continue;
        };
        let exe = PathBuf::from(exe.to_string_lossy().trim_end_matches(" (deleted)"));
        if exe != bin && exe != runtime.join("sing-box") {
            continue;
        }
        // Match executable paths, never names or the iVnc process group.
        let pgid = unsafe { libc::getpgid(pid) };
        let target = if pgid == pid && pgid != unsafe { libc::getpgrp() } {
            -pgid
        } else {
            pid
        };
        unsafe {
            libc::kill(target, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && Path::new(&format!("/proc/{pid}")).exists() {
            unsafe {
                libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if Path::new(&format!("/proc/{pid}")).exists() {
            unsafe {
                libc::kill(target, libc::SIGKILL);
            }
        }
    }
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    if fs::read(path).ok().as_deref() == Some(bytes) {
        return fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|e| e.to_string());
    }
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        use std::io::Write;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(mode)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e: std::io::Error| e.to_string())
}

pub fn read_log_tail(path: &Path, count: usize) -> String {
    let Ok(mut file) = File::open(path) else {
        return "(no Miao logs yet)".into();
    };
    let length = file.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = file.seek(SeekFrom::Start(length.saturating_sub(64 * 1024)));
    let mut bytes = Vec::new();
    let _ = file.take(64 * 1024).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes)
        .lines()
        .rev()
        .take(count)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("ivnc-miao-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn manager(&self, enabled: bool, instance: &str) -> Arc<ProxyPanelManager> {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let mut manager = ProxyPanelManager::new(enabled);
            manager.port = listener.local_addr().unwrap().port();
            manager.home = self.0.join("home");
            manager.dir = self.0.join(instance);
            let script = self.0.join("panel.py");
            fs::write(&script, r#"#!/usr/bin/env python3
import http.server, json, os, signal, subprocess, time
class Panel(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        payload = dict(success=True, data=dict(service='miao', service_pid=os.getpid(), embedded=True,
            integration_version=1, initializing=False, proxy_running=True, socks_listen='::', socks_port=19081))
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(json.dumps(payload).encode())
    def log_message(self, *args): pass
child = None
if os.path.exists('spawn-child'):
    child = subprocess.Popen(['sleep', '300'])
    with open('child.pid', 'w') as file: file.write(str(child.pid))
def stop(*_):
    if child is not None: child.wait(timeout=2)
    exit(0)
signal.signal(signal.SIGTERM, stop)
if os.path.exists('slow-ready'): time.sleep(1)
http.server.HTTPServer(('127.0.0.1', int(os.environ['MIAO_PORT'])), Panel).serve_forever()
"#).unwrap();
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
            manager.binary_override = Some(script);
            Arc::new(manager)
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn disabled_manager_never_starts_or_recovers() {
        let sandbox = Sandbox::new();
        let manager = sandbox.manager(false, "disabled");
        manager.start_watchdog();
        assert!(manager.start().await.is_err());
        assert!(manager.restart().await.is_err());
        assert!(manager.require_running().is_err());
        assert!(!manager.status().desired_running);
        assert!(!manager.log_path().exists());
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn lifecycle_serializes_starts_recovers_crashes_and_respects_stop_and_shutdown() {
        let sandbox = Sandbox::new();
        let manager = sandbox.manager(true, "lifecycle");
        manager.start_watchdog();
        let (first, second) = tokio::join!(manager.start(), manager.start());
        let pid = first.unwrap().unwrap();
        assert_eq!(second.unwrap(), Some(pid));
        assert!(manager.status().ready);
        assert_eq!(
            fs::read_to_string(manager.chrome_proxy_path()).unwrap(),
            "socks5://[::1]:19081\n"
        );
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                let status = manager.status();
                if status.ready && status.pid != Some(pid) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("watchdog should recover a crashed Miao");
        manager.stop().await.unwrap();
        assert!(manager.require_running().is_err());
        assert!(!manager.chrome_proxy_path().exists());
        tokio::time::sleep(Duration::from_millis(3300)).await;
        assert!(!manager.status().running);
        assert!(!manager.status().desired_running);
        manager.start().await.unwrap();
        manager.shutdown().await;
        assert!(!manager.status().running);
        assert!(manager.start().await.is_err());
        assert!(!manager.status().desired_running);
    }

    #[tokio::test]
    async fn cancelled_start_request_does_not_leave_a_healthy_child_unavailable() {
        let sandbox = Sandbox::new();
        let manager = sandbox.manager(true, "cancelled-start");
        fs::create_dir_all(&manager.dir).unwrap();
        fs::write(manager.dir.join("slow-ready"), "").unwrap();
        manager.start_watchdog();
        let starting_manager = manager.clone();
        let starting = tokio::spawn(async move { starting_manager.start().await });
        tokio::time::timeout(Duration::from_secs(5), async {
            while !manager.status().running {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(!manager.status().ready);
        starting.abort();
        assert!(starting.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(5), async {
            while !manager.status().ready {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("watchdog must finish the readiness probe after client cancellation");
        assert!(manager.require_running().is_ok());
        manager.stop().await.unwrap();
    }

    #[tokio::test]
    async fn instances_cannot_share_configuration_and_dropped_manager_reaps_its_child() {
        let sandbox = Sandbox::new();
        let first = sandbox.manager(true, "first");
        let second = sandbox.manager(true, "second");
        first.start().await.unwrap();
        assert!(second
            .start()
            .await
            .unwrap_err()
            .contains("another iVnc instance"));
        assert!(!second.status().running);
        first.stop().await.unwrap();
        fs::create_dir_all(&second.dir).unwrap();
        fs::write(second.dir.join("spawn-child"), "").unwrap();
        let pid = second.start().await.unwrap().unwrap();
        let child_pid: i32 = fs::read_to_string(second.dir.join("child.pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::getpgid(child_pid) }, pid as i32);
        drop(second);
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
        assert!(!Path::new(&format!("/proc/{child_pid}")).exists());
    }
}
