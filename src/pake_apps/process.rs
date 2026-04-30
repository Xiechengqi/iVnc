use super::app::{AppStatus, PakeApp};
use super::native;
use log::{info, warn};
use std::collections::{HashMap, HashSet};
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct RunningApp {
    child: Child,
    pid: u32,
}

pub struct ProcessManager {
    processes: Arc<Mutex<HashMap<String, RunningApp>>>,
    /// App IDs that were explicitly stopped by the user (should not auto-restart)
    stopped_by_user: Arc<Mutex<HashSet<String>>>,
    /// App store reference for watchdog restarts
    store: Option<Arc<super::store::AppStore>>,
}

impl ProcessManager {
    pub fn new() -> Self {
        Self {
            processes: Arc::new(Mutex::new(HashMap::new())),
            stopped_by_user: Arc::new(Mutex::new(HashSet::new())),
            store: None,
        }
    }

    pub fn set_store(&mut self, store: Arc<super::store::AppStore>) {
        self.store = Some(store.clone());
        self.start_watchdog(store);
    }

    fn start_watchdog(&self, store: Arc<super::store::AppStore>) {
        let processes = self.processes.clone();
        let stopped_by_user = self.stopped_by_user.clone();

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;

                // Find crashed apps that were not stopped by user
                let crashed: Vec<String> = {
                    let mut procs = processes.lock().unwrap();
                    let user_stopped = stopped_by_user.lock().unwrap();
                    let mut crashed = Vec::new();
                    procs.retain(|app_id, running| {
                        match running.child.try_wait() {
                            Ok(Some(_)) | Err(_) => {
                                if !user_stopped.contains(app_id) {
                                    crashed.push(app_id.clone());
                                }
                                false // remove from map
                            }
                            Ok(None) => true, // still running
                        }
                    });
                    crashed
                };

                // Restart crashed apps
                for app_id in crashed {
                    info!("Watchdog: app {} exited unexpectedly, restarting", app_id);
                    if let Ok(app) = store.get(&app_id) {
                        match native::build_command(&app) {
                            Ok(mut cmd) => match cmd.spawn() {
                                Ok(child) => {
                                    let pid = child.id();
                                    info!("Watchdog: restarted app '{}' (pid={})", app.name, pid);
                                    processes
                                        .lock()
                                        .unwrap()
                                        .insert(app_id, RunningApp { child, pid });
                                }
                                Err(e) => warn!("Watchdog: failed to restart {}: {}", app_id, e),
                            },
                            Err(e) => {
                                warn!("Watchdog: failed to build command for {}: {}", app_id, e)
                            }
                        }
                    }
                }
            }
        });
    }

    pub fn start(&self, app: &PakeApp) -> Result<u32, String> {
        if self.is_running(&app.id) {
            return Err("App is already running".into());
        }

        // Remove from user-stopped set so watchdog will restart if it crashes
        self.stopped_by_user.lock().unwrap().remove(&app.id);

        let mut cmd = native::build_command(app)?;
        let child = cmd.spawn().map_err(|e| format!("Failed to start: {}", e))?;
        let pid = child.id();
        info!("Started Pake app '{}' (pid={})", app.name, pid);

        self.processes
            .lock()
            .unwrap()
            .insert(app.id.clone(), RunningApp { child, pid });
        Ok(pid)
    }

    pub fn stop(&self, app_id: &str) -> Result<(), String> {
        // Mark as user-stopped so watchdog won't restart it
        self.stopped_by_user
            .lock()
            .unwrap()
            .insert(app_id.to_string());

        let mut procs = self.processes.lock().unwrap();
        if let Some(mut running) = procs.remove(app_id) {
            let pgid = -(running.pid as i32);
            info!(
                "Stopping Pake app process group (pid={}, pgid={})",
                running.pid, pgid
            );

            if unsafe { libc::kill(pgid, libc::SIGTERM) } != 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() != Some(libc::ESRCH) {
                    warn!(
                        "Failed to send SIGTERM to process group {} for app {}: {}",
                        pgid, app_id, err
                    );
                }
            }

            let mut exited = false;
            for _ in 0..20 {
                match running.child.try_wait() {
                    Ok(Some(_)) => {
                        exited = true;
                        break;
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                    Err(e) => {
                        warn!("Failed to poll app {} during shutdown: {}", app_id, e);
                        break;
                    }
                }
            }

            if !exited {
                warn!(
                    "Process group {} for app {} did not exit after SIGTERM, sending SIGKILL",
                    pgid, app_id
                );
                if unsafe { libc::kill(pgid, libc::SIGKILL) } != 0 {
                    let err = std::io::Error::last_os_error();
                    if err.raw_os_error() != Some(libc::ESRCH) {
                        warn!(
                            "Failed to send SIGKILL to process group {} for app {}: {}",
                            pgid, app_id, err
                        );
                    }
                }
            }

            let _ = running.child.wait();
            Ok(())
        } else {
            Err("App is not running".into())
        }
    }

    pub fn restart(&self, app: &PakeApp) -> Result<u32, String> {
        let _ = self.stop(&app.id);
        // Remove from stopped_by_user so it stays alive after restart
        self.stopped_by_user.lock().unwrap().remove(&app.id);
        self.start(app)
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
        procs.get(app_id).map(|r| r.pid)
    }

    fn is_running(&self, app_id: &str) -> bool {
        self.status(app_id) == AppStatus::Running
    }
}
