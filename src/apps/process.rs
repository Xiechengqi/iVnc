use super::app::{AppStatus, ManagedApp};
use super::native;
use crate::remote::store::RemoteStore;
use log::{info, warn};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
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
    /// Runs in its own runtime rather than in the main desktop.
    worker: bool,
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

/// Ends the process group led by `pid`: SIGTERM, then SIGKILL unless `exited`
/// reports the leader gone within five seconds. The leader is the application's
/// reaper or runtime supervisor, which stays until the application has exited.
fn terminate_group(pid: u32, app_id: &str, mut exited: impl FnMut() -> std::io::Result<bool>) {
    let pgid = -(pid as i32);
    info!(
        "Stopping managed app process group (pid={}, pgid={})",
        pid, pgid
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

    for _ in 0..50 {
        match exited() {
            Ok(true) => return,
            Ok(false) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => {
                warn!("Failed to poll app {} during shutdown: {}", app_id, e);
                break;
            }
        }
    }

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

#[derive(Clone)]
pub struct ProcessManager {
    operations: Arc<Mutex<()>>,
    processes: Arc<Mutex<HashMap<String, RunningApp>>>,
    /// App IDs that were explicitly stopped by the user (should not auto-restart)
    stopped_by_user: Arc<Mutex<HashSet<String>>>,
    restarts: Arc<Mutex<HashMap<String, Restart>>>,
    /// App store reference for watchdog restarts
    store: Option<Arc<super::store::AppStore>>,
    /// Decides whether an app runs in its own runtime.
    remote: Arc<RemoteStore>,
    /// Where runtimes publish their state; see `worker::runtimes_dir`.
    runtimes: PathBuf,
}

impl ProcessManager {
    pub fn new(remote: Arc<RemoteStore>, runtimes: PathBuf) -> Self {
        Self {
            operations: Arc::new(Mutex::new(())),
            processes: Arc::new(Mutex::new(HashMap::new())),
            stopped_by_user: Arc::new(Mutex::new(HashSet::new())),
            restarts: Arc::new(Mutex::new(HashMap::new())),
            store: None,
            remote,
            runtimes,
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
                // Building a command opens databases.
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
            let mut restarts = self.restarts.lock().unwrap();
            let next = next_restart(restarts.get(&app_id).copied(), ran, now);
            if let Some(due) = next.due {
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
            match store
                .get(&app_id)
                .and_then(|app| self.spawn(&app).map(|r| (app, r)))
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

    fn spawn(&self, app: &ManagedApp) -> Result<RunningApp, String> {
        let worker = native::runs_in_worker(app, &self.remote)?;
        let child = native::build_command(app, worker)?
            .spawn()
            .map_err(|e| format!("Failed to start: {}", e))?;
        Ok(RunningApp {
            pid: child.id(),
            child,
            started: Instant::now(),
            worker,
        })
    }

    pub fn start(&self, app: &ManagedApp) -> Result<u32, String> {
        let _operation = self.operations.lock().unwrap();
        if self.is_running(&app.id) {
            return Err("App is already running".into());
        }

        // Remove from user-stopped set so watchdog will restart if it crashes
        self.stopped_by_user.lock().unwrap().remove(&app.id);
        self.restarts.lock().unwrap().remove(&app.id);

        let running = self.spawn(app)?;
        let pid = running.pid;
        info!("Started managed app '{}' (pid={})", app.name, pid);

        self.processes
            .lock()
            .unwrap()
            .insert(app.id.clone(), running);
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
            terminate_group(running.pid, app_id, || {
                running.child.try_wait().map(|s| s.is_some())
            });
            let _ = running.child.wait();
            Ok(())
        } else if pending {
            Ok(())
        } else {
            Err("App is not running".into())
        }
    }

    pub fn status(&self, app_id: &str) -> AppStatus {
        // The endpoint is read without holding the process table.
        let running = self
            .processes
            .lock()
            .unwrap()
            .get_mut(app_id)
            .map(|r| (matches!(r.child.try_wait(), Ok(None)), r.worker, r.pid));
        if let Some((alive, worker, pid)) = running {
            match (alive, worker) {
                (false, _) => AppStatus::Crashed,
                // The supervisor publishes its endpoint once the runtime is ready.
                (true, true)
                    if !crate::remote::worker::endpoint_in(&self.runtimes, app_id)
                        .is_ok_and(|e| e.supervisor == pid) =>
                {
                    AppStatus::Starting
                }
                (true, _) => AppStatus::Running,
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

    pub fn runtime_dir(&self, app_id: &str) -> PathBuf {
        self.runtimes.join(app_id)
    }

    /// Whether a live process of the app runs in its own runtime.
    pub fn in_worker(&self, app_id: &str) -> Option<bool> {
        let mut procs = self.processes.lock().unwrap();
        let running = procs.get_mut(app_id)?;
        matches!(running.child.try_wait(), Ok(None)).then_some(running.worker)
    }

    fn is_running(&self, app_id: &str) -> bool {
        matches!(
            self.status(app_id),
            AppStatus::Running | AppStatus::Starting
        )
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

    #[test]
    fn stopping_ends_application_processes_that_ignore_sigterm() {
        let dir = std::env::temp_dir().join(format!("ivnc-reaper-{}", crate::remote::token()));
        std::fs::create_dir_all(&dir).unwrap();
        // One process stays in the application's process group, one leaves it.
        let script = format!(
            "trap '' TERM; setsid sh -c 'echo $$ > {0}/outside; exec sleep 60' & echo $$ > {0}/inside; exec sleep 60",
            dir.display()
        );
        // The reaper must be the only thread of its process, as in `main`.
        let reaper = unsafe { libc::fork() };
        if reaper == 0 {
            let code = std::panic::catch_unwind(|| {
                unsafe { libc::setpgid(0, 0) };
                crate::remote::worker::reap(vec!["sh".into(), "-c".into(), script.into()])
            });
            unsafe { libc::_exit(code.unwrap_or(101)) };
        }
        let pid = |name: &str| -> i32 {
            for _ in 0..100 {
                if let Some(pid) = std::fs::read_to_string(dir.join(name))
                    .ok()
                    .and_then(|s| s.trim().parse().ok())
                {
                    return pid;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            panic!("{name} did not start");
        };
        let (inside, outside) = (pid("inside"), pid("outside"));
        // Wait until both run `sleep`, which then ignores SIGTERM.
        std::thread::sleep(Duration::from_millis(300));
        let alive = |pid: i32| {
            std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
                !matches!(
                    stat.rsplit_once(')')
                        .and_then(|(_, rest)| rest.trim_start().chars().next()),
                    Some('Z' | 'X')
                )
            })
        };
        assert!(alive(inside) && alive(outside));

        let started = Instant::now();
        terminate_group(reaper as u32, "app", || {
            match unsafe { libc::waitpid(reaper, std::ptr::null_mut(), libc::WNOHANG) } {
                0 => Ok(false),
                -1 => Err(std::io::Error::last_os_error()),
                _ => Ok(true),
            }
        });
        let _ = unsafe { libc::waitpid(reaper, std::ptr::null_mut(), 0) };
        // The reaper outlived SIGTERM, so the group was killed after the grace period.
        assert!(started.elapsed() >= Duration::from_secs(4));
        std::thread::sleep(Duration::from_millis(200));
        assert!(!alive(inside));
        assert!(!alive(outside));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn shared_apps_start_until_their_runtime_is_ready() {
        let runtimes =
            std::env::temp_dir().join(format!("ivnc-runtimes-{}", crate::remote::token()));
        let manager = ProcessManager::new(Arc::new(RemoteStore::in_memory()), runtimes.clone());
        let run = |id: &str, worker| {
            let child = std::process::Command::new("sleep")
                .arg("30")
                .spawn()
                .unwrap();
            let running = RunningApp {
                pid: child.id(),
                child,
                started: Instant::now(),
                worker,
            };
            let pid = running.pid;
            manager.processes.lock().unwrap().insert(id.into(), running);
            pid
        };
        let local = format!("test-{}", crate::remote::token());
        run(&local, false);
        assert_eq!(manager.status(&local), AppStatus::Running);
        assert_eq!(manager.in_worker(&local), Some(false));

        let shared = format!("test-{}", crate::remote::token());
        let pid = run(&shared, true);
        assert_eq!(manager.status(&shared), AppStatus::Starting);
        assert!(manager.is_running(&shared));
        assert_eq!(manager.in_worker(&shared), Some(true));
        let root = manager.runtime_dir(&shared);
        std::fs::create_dir_all(&root).unwrap();
        let publish = |supervisor| {
            let endpoint = crate::remote::worker::Endpoint {
                port: 1,
                password: String::new(),
                generation: String::new(),
                supervisor,
            };
            std::fs::write(
                root.join("endpoint.json"),
                serde_json::to_vec(&endpoint).unwrap(),
            )
            .unwrap();
        };
        // An endpoint left by another supervisor does not count.
        publish(std::process::id());
        assert_eq!(manager.status(&shared), AppStatus::Starting);
        publish(pid);
        assert_eq!(manager.status(&shared), AppStatus::Running);

        for id in [local, shared] {
            let mut running = manager.processes.lock().unwrap().remove(&id).unwrap();
            let _ = running.child.kill();
            let _ = running.child.wait();
            assert_eq!(manager.in_worker(&id), None);
        }
        std::fs::remove_dir_all(runtimes).unwrap();
    }
}
