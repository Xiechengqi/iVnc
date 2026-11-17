use super::app::{AppStatus, ManagedApp};
use log::{info, warn};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct RunningService {
    child: Child,
    pid: u32,
}

pub struct ServiceProcessManager {
    processes: Arc<Mutex<HashMap<String, RunningService>>>,
    stopped_by_user: Arc<Mutex<HashSet<String>>>,
}

impl ServiceProcessManager {
    pub fn new() -> Self {
        Self {
            processes: Arc::new(Mutex::new(HashMap::new())),
            stopped_by_user: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn set_store(&mut self, store: Arc<super::store::AppStore>) {
        self.start_watchdog(store);
    }

    fn start_watchdog(&self, store: Arc<super::store::AppStore>) {
        let processes = self.processes.clone();
        let stopped_by_user = self.stopped_by_user.clone();

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(3)).await;

                let crashed: Vec<String> = {
                    let mut procs = processes.lock().unwrap();
                    let user_stopped = stopped_by_user.lock().unwrap();
                    let mut crashed = Vec::new();
                    procs.retain(|app_id, running| match running.child.try_wait() {
                        Ok(Some(_)) | Err(_) => {
                            if !user_stopped.contains(app_id) {
                                crashed.push(app_id.clone());
                            }
                            false
                        }
                        Ok(None) => true,
                    });
                    crashed
                };

                for app_id in crashed {
                    if let Ok(app) = store.get(&app_id) {
                        if !has_launch_command(&app) {
                            continue;
                        }
                        info!("Watchdog: service for app {} exited unexpectedly", app_id);
                        match build_service_command(&app).and_then(|mut cmd| {
                            cmd.spawn()
                                .map_err(|e| format!("Failed to restart service: {}", e))
                        }) {
                            Ok(child) => {
                                let pid = child.id();
                                info!(
                                    "Watchdog: restarted service for app '{}' (pid={})",
                                    app.name, pid
                                );
                                processes
                                    .lock()
                                    .unwrap()
                                    .insert(app_id, RunningService { child, pid });
                            }
                            Err(e) => warn!(
                                "Watchdog: failed to restart service for app {}: {}",
                                app.id, e
                            ),
                        }
                    }
                }
            }
        });
    }

    pub async fn start_and_wait(&self, app: &ManagedApp) -> Result<Option<u32>, String> {
        if !has_launch_command(app) {
            return Ok(None);
        }
        if self.is_running(&app.id) {
            return Ok(self.pid(&app.id));
        }

        self.stopped_by_user.lock().unwrap().remove(&app.id);

        let mut cmd = build_service_command(app)?;
        let child = cmd
            .spawn()
            .map_err(|e| format!("Failed to start service: {}", e))?;
        let pid = child.id();
        info!("Started service for app '{}' (pid={})", app.name, pid);
        self.processes
            .lock()
            .unwrap()
            .insert(app.id.clone(), RunningService { child, pid });

        if let Err(err) = self.wait_until_ready(app).await {
            let _ = self.stop(&app.id);
            return Err(err);
        }

        Ok(Some(pid))
    }

    pub fn stop(&self, app_id: &str) -> Result<(), String> {
        self.stopped_by_user
            .lock()
            .unwrap()
            .insert(app_id.to_string());

        let mut procs = self.processes.lock().unwrap();
        if let Some(mut running) = procs.remove(app_id) {
            terminate_process_group(app_id, running.pid);
            let _ = running.child.wait();
            Ok(())
        } else {
            Ok(())
        }
    }

    pub fn status(&self, app_id: &str) -> AppStatus {
        let mut procs = self.processes.lock().unwrap();
        if let Some(running) = procs.get_mut(app_id) {
            match running.child.try_wait() {
                Ok(Some(_)) => {
                    procs.remove(app_id);
                    AppStatus::Crashed
                }
                Ok(None) => AppStatus::Running,
                Err(_) => {
                    procs.remove(app_id);
                    AppStatus::Crashed
                }
            }
        } else {
            AppStatus::Stopped
        }
    }

    pub fn pid(&self, app_id: &str) -> Option<u32> {
        let procs = self.processes.lock().unwrap();
        procs.get(app_id).map(|running| running.pid)
    }

    fn is_running(&self, app_id: &str) -> bool {
        self.status(app_id) == AppStatus::Running
    }

    async fn wait_until_ready(&self, app: &ManagedApp) -> Result<(), String> {
        let Some(url) = app.url.as_deref() else {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if self.status(&app.id) != AppStatus::Running {
                return Err("service exited immediately after start".to_string());
            }
            info!("Service for app '{}' is running", app.name);
            return Ok(());
        };
        let timeout_secs = app.launch_wait_timeout_secs.unwrap_or(30).max(1);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| format!("Failed to create readiness client: {}", e))?;

        loop {
            if self.status(&app.id) != AppStatus::Running {
                return Err("service exited before it became ready".to_string());
            }

            match client.get(url).send().await {
                Ok(resp) if resp.status().is_success() || resp.status().is_redirection() => {
                    info!("Service for app '{}' is ready at {}", app.name, url);
                    return Ok(());
                }
                Ok(resp) => {
                    log::debug!("Service readiness returned {}", resp.status());
                }
                Err(err) => {
                    log::debug!("Service readiness check failed: {}", err);
                }
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "service did not become ready at {} within {}s",
                    url, timeout_secs
                ));
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

pub fn has_launch_command(app: &ManagedApp) -> bool {
    app.launch_command
        .as_deref()
        .map(|cmd| !cmd.trim().is_empty())
        .unwrap_or(false)
}

pub fn service_log_path(app_id: &str) -> std::path::PathBuf {
    let dir = crate::paths::app_log_dir(app_id);
    let _ = fs::create_dir_all(&dir);
    dir.join("service.log")
}

/// Build the raw command for a background app: either the legacy `sh -c`, or
/// the shell-profile bootstrap when profile loading is enabled. Kept separate
/// from the filesystem/log setup so it can be tested without touching `$HOME`.
fn service_command(launch_command: &str, profile_homes: &[PathBuf]) -> Command {
    if profile_homes.is_empty() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(launch_command);
        cmd
    } else {
        // Loads $HOME/.profile and $HOME/.bashrc (real home first, app home
        // second) in a bash that has `source`, then runs the launch command in
        // that same shell with the loaded PATH.
        crate::shell_profile::app_command(launch_command, profile_homes)
    }
}

fn build_service_command(app: &ManagedApp) -> Result<Command, String> {
    let launch_command = app
        .launch_command
        .as_deref()
        .map(str::trim)
        .filter(|cmd| !cmd.is_empty())
        .ok_or("missing launch_command")?;

    // The manager's HOME is the real user home; apply_std_command_isolation
    // below replaces it with the app's isolated HOME. Compute both before that
    // so the bootstrap can source the right files in the right order.
    let real_home = std::env::var_os("HOME").map(PathBuf::from);
    let app_home = crate::paths::prepare_app_home(&app.id)?;
    let profile_homes = crate::shell_profile::app_profile_homes(real_home.as_deref(), &app_home);

    let mut cmd = service_command(launch_command, &profile_homes);
    unsafe {
        cmd.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    if let Some(cwd) = app
        .launch_cwd
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        cmd.current_dir(cwd);
    }

    if let Some(env_vars) = &app.launch_env_vars {
        for (key, value) in env_vars {
            cmd.env(key, value);
        }
    }

    crate::paths::apply_std_command_isolation(&mut cmd, &app.id)?;

    if let Ok(val) = std::env::var("WAYLAND_DISPLAY") {
        cmd.env("WAYLAND_DISPLAY", &val);
    }
    if let Ok(val) = std::env::var("XDG_RUNTIME_DIR") {
        cmd.env("XDG_RUNTIME_DIR", &val);
    }
    if let Ok(val) = std::env::var("DISPLAY") {
        cmd.env("DISPLAY", &val);
    }

    let log_file = service_log_path(&app.id);
    let stdout_file =
        fs::File::create(&log_file).map_err(|e| format!("Failed to create service log: {}", e))?;
    let stderr_file = stdout_file
        .try_clone()
        .map_err(|e| format!("Failed to clone service log: {}", e))?;
    cmd.stdout(Stdio::from(stdout_file));
    cmd.stderr(Stdio::from(stderr_file));

    Ok(cmd)
}

fn terminate_process_group(app_id: &str, pid: u32) {
    let pgid = -(pid as i32);
    info!(
        "Stopping service process group for app {} (pid={}, pgid={})",
        app_id, pid, pgid
    );

    if unsafe { libc::kill(pgid, libc::SIGTERM) } != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            warn!(
                "Failed to send SIGTERM to service process group {} for app {}: {}",
                pgid, app_id, err
            );
        }
    }

    for _ in 0..20 {
        if unsafe { libc::kill(pgid, 0) } != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ESRCH) {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    if unsafe { libc::kill(pgid, libc::SIGKILL) } != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            warn!(
                "Failed to send SIGKILL to service process group {} for app {}: {}",
                pgid, app_id, err
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMAND: &str = "ls /root/.nvm/versions/node/v26.13.1/bin/node; source /root/.bashrc; which node; which dsh; sleep 1000";

    #[test]
    fn no_profile_homes_uses_legacy_sh() {
        let cmd = service_command(COMMAND, &[]);
        assert_eq!(cmd.get_program(), "sh");
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(args[0], "-c");
        assert_eq!(args[1], COMMAND);
    }

    #[test]
    fn profile_homes_use_bash_bootstrap_with_real_home_first() {
        let homes = vec![
            PathBuf::from("/root"),
            PathBuf::from("/root/.ivnc/apps/x/home"),
        ];
        let cmd = service_command(COMMAND, &homes);
        assert!(cmd.get_program().to_string_lossy().ends_with("/bash"));
        let args: Vec<_> = cmd.get_args().collect();
        // $0 is the original command (with `source`), homes follow in order.
        assert_eq!(args[5], COMMAND);
        assert_eq!(args[6], "/root");
        assert_eq!(args[7], "/root/.ivnc/apps/x/home");
        let script = args[4].to_str().unwrap();
        assert!(script.contains("eval \"$0\""));
    }

    #[test]
    fn app_home_is_resolved_for_profile_homes() {
        // prepare_app_home returns the per-app isolated HOME under $IVNC_HOME.
        let home = crate::paths::prepare_app_home("svc-test").unwrap();
        assert!(home.is_absolute());
        assert!(home.to_string_lossy().contains("apps/svc-test/home"));
    }
}
