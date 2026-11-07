use super::app::{AppStatus, ManagedApp};
use super::native;
use log::{info, warn};
use std::collections::{HashMap, HashSet};
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A run shorter than this counts as a failed start for restart backoff.
const STABLE_RUN: Duration = Duration::from_secs(30);
/// Consecutive failed starts after which the watchdog gives up.
const MAX_FAILURES: u32 = 5;

struct RunningApp {
    child: Child,
    pid: u32,
    started: Instant,
}

/// Watchdog restart state for an application that exited unexpectedly.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Restart {
    failures: u32,
    /// `None` once the watchdog has given up.
    due: Option<Instant>,
}

/// Delay before the next restart after `failures` consecutive failed starts.
fn backoff(failures: u32) -> Duration {
    Duration::from_secs(3u64.saturating_mul(1 << failures.min(7)).min(300))
}

/// Computes the next restart after an exit at `now` following a run of `ran`.
fn next_restart(previous: Option<Restart>, ran: Duration, now: Instant) -> Restart {
    let failures = if ran < STABLE_RUN {
        previous.map_or(0, |r| r.failures) + 1
    } else {
        0
    };
    Restart {
        failures,
        due: (failures < MAX_FAILURES).then(|| now + backoff(failures)),
    }
}

#[derive(Clone)]
pub struct ProcessManager {
    operations: Arc<Mutex<()>>,
    processes: Arc<Mutex<HashMap<String, RunningApp>>>,
    /// App IDs that were explicitly stopped by the user (should not auto-restart)
    stopped_by_user: Arc<Mutex<HashSet<String>>>,
    restarts: Arc<Mutex<HashMap<String, Restart>>>,
    /// App store reference for watchdog restarts
    store: Option<Arc<super::store::AppStore>>,
}

impl ProcessManager {
    pub fn new() -> Self {
        Self {
            operations: Arc::new(Mutex::new(())),
            processes: Arc::new(Mutex::new(HashMap::new())),
            stopped_by_user: Arc::new(Mutex::new(HashSet::new())),
            restarts: Arc::new(Mutex::new(HashMap::new())),
            store: None,
        }
    }

    pub fn set_store(&mut self, store: Arc<super::store::AppStore>) {
        self.store = Some(store.clone());
        self.start_watchdog(store);
    }

    fn start_watchdog(&self, store: Arc<super::store::AppStore>) {
        let manager = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(3)).await;
                // Building a runtime inspects the network and opens databases.
                let manager = manager.clone();
                let store = store.clone();
                let _ = tokio::task::spawn_blocking(move || manager.watchdog_tick(&store)).await;
            }
        });
    }

    fn watchdog_tick(&self, store: &super::store::AppStore) {
        let _operation = self.operations.lock().unwrap();
        let now = Instant::now();
        let mut exited = Vec::new();
        {
            let mut procs = self.processes.lock().unwrap();
            let user_stopped = self.stopped_by_user.lock().unwrap();
            procs.retain(|app_id, running| match running.child.try_wait() {
                Ok(Some(_)) | Err(_) => {
                    if !user_stopped.contains(app_id) {
                        exited.push((app_id.clone(), running.started.elapsed()));
                    }
                    false
                }
                Ok(None) => true,
            });
        }
        for (app_id, ran) in exited {
            // Restarting cannot change how the host or container runs iVnc.
            let environment = crate::remote::worker::failure(&app_id).filter(|f| f.environment);
            let mut restarts = self.restarts.lock().unwrap();
            let mut next = next_restart(restarts.get(&app_id).copied(), ran, now);
            if let Some(failure) = environment {
                next.due = None;
                warn!(
                    "Watchdog: app {} cannot start in this environment; not restarting: {}",
                    app_id, failure.message
                );
            } else if let Some(due) = next.due {
                info!(
                    "Watchdog: app {} exited unexpectedly; restarting in {}s",
                    app_id,
                    due.saturating_duration_since(now).as_secs()
                );
            } else {
                warn!(
                    "Watchdog: app {} failed to start {} times; giving up until it is started again",
                    app_id, next.failures
                );
            }
            restarts.insert(app_id, next);
        }
        let due: Vec<String> = self
            .restarts
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, r)| r.due.is_some_and(|d| d <= now))
            .map(|(id, _)| id.clone())
            .collect();
        for app_id in due {
            let restart = |app: &ManagedApp| -> Result<RunningApp, String> {
                let child = native::build_command(app)?
                    .spawn()
                    .map_err(|e| e.to_string())?;
                Ok(RunningApp {
                    pid: child.id(),
                    child,
                    started: Instant::now(),
                })
            };
            match store
                .get(&app_id)
                .and_then(|app| restart(&app).map(|r| (app, r)))
            {
                Ok((app, running)) => {
                    info!(
                        "Watchdog: restarted app '{}' (pid={})",
                        app.name, running.pid
                    );
                    if let Some(r) = self.restarts.lock().unwrap().get_mut(&app_id) {
                        r.due = None;
                    }
                    self.processes.lock().unwrap().insert(app_id, running);
                }
                Err(e) => {
                    warn!("Watchdog: failed to restart {}: {}", app_id, e);
                    let mut restarts = self.restarts.lock().unwrap();
                    let next = next_restart(restarts.get(&app_id).copied(), Duration::ZERO, now);
                    restarts.insert(app_id, next);
                }
            }
        }
    }

    pub fn start(&self, app: &ManagedApp) -> Result<u32, String> {
        let _operation = self.operations.lock().unwrap();
        if self.is_running(&app.id) {
            return Err("App is already running".into());
        }

        // Remove from user-stopped set so watchdog will restart if it crashes
        self.stopped_by_user.lock().unwrap().remove(&app.id);
        self.restarts.lock().unwrap().remove(&app.id);

        let mut cmd = native::build_command(app)?;
        let child = cmd.spawn().map_err(|e| format!("Failed to start: {}", e))?;
        let pid = child.id();
        info!("Started managed app '{}' (pid={})", app.name, pid);

        self.processes.lock().unwrap().insert(
            app.id.clone(),
            RunningApp {
                child,
                pid,
                started: Instant::now(),
            },
        );
        Ok(pid)
    }

    pub fn stop(&self, app_id: &str) -> Result<(), String> {
        let _operation = self.operations.lock().unwrap();
        // Mark as user-stopped so watchdog won't restart it
        self.stopped_by_user
            .lock()
            .unwrap()
            .insert(app_id.to_string());
        let pending = self.restarts.lock().unwrap().remove(app_id).is_some();

        let mut procs = self.processes.lock().unwrap();
        if let Some(mut running) = procs.remove(app_id) {
            drop(procs);
            let pgid = -(running.pid as i32);
            info!(
                "Stopping managed app process group (pid={}, pgid={})",
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
            for _ in 0..50 {
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
        } else if pending {
            Ok(())
        } else {
            Err("App is not running".into())
        }
    }

    pub fn status(&self, app_id: &str) -> AppStatus {
        let mut procs = self.processes.lock().unwrap();
        if let Some(running) = procs.get_mut(app_id) {
            match running.child.try_wait() {
                Ok(Some(_)) => AppStatus::Crashed,
                Ok(None) => AppStatus::Running,
                Err(_) => AppStatus::Crashed,
            }
        } else if self.restarts.lock().unwrap().contains_key(app_id) {
            // Waiting for a backoff restart, or given up after repeated failures.
            AppStatus::Crashed
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_exits_back_off_and_give_up_while_stable_runs_reset() {
        let now = Instant::now();
        let mut restart = None;
        let mut delays = Vec::new();
        for _ in 0..MAX_FAILURES {
            let next = next_restart(restart, Duration::from_secs(1), now);
            if let Some(due) = next.due {
                delays.push(due - now);
            }
            restart = Some(next);
        }
        assert_eq!(delays, [6, 12, 24, 48].map(Duration::from_secs).to_vec());
        assert_eq!(restart.unwrap().due, None);
        let stable = next_restart(restart, STABLE_RUN, now);
        assert_eq!(stable.failures, 0);
        assert_eq!(stable.due, Some(now + Duration::from_secs(3)));
        assert_eq!(backoff(30), Duration::from_secs(300));
    }
}
