use super::app::{AppStatus, AppType, ManagedApp};
use super::datadir;
use super::desktop_entry;
use super::error::ApiError;
use super::native;
use super::process::ProcessManager;
use super::service_process::{self, ServiceProcessManager};
use super::state_recovery::AppRunningState;
use super::store::{validate_app_name, AppStore, OrderedApps};
use crate::proxy_panel::{ProxyPanelManager, BUILTIN_MIAO_ID, MIAO_START_TIMEOUT_SECS};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::Response,
    routing::{get, post, put},
    Router,
};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct AppsState {
    pub shared: std::sync::OnceLock<std::sync::Weak<crate::web::SharedState>>,
    pub remote: Arc<crate::remote::store::RemoteStore>,
    pub remote_health: Arc<crate::remote::gateway::RemoteHealthCache>,
    pub proxy_panel: Arc<ProxyPanelManager>,
    pub store: Arc<AppStore>,
    pub process: ProcessManager,
    pub service: ServiceProcessManager,
    /// Remote applications the user started; only these are connected.
    started: Mutex<HashSet<String>>,
    /// Wakes the monitor early, e.g. to connect a remote application just started.
    wake: tokio::sync::Notify,
    /// Set when the application list may have changed; the monitor reads it again.
    apps_changed: std::sync::atomic::AtomicBool,
}

impl AppsState {
    pub fn new(proxy_panel: Arc<ProxyPanelManager>) -> Result<Self, String> {
        crate::paths::migrate_legacy();
        crate::paths::ensure_layout()?;
        let store = Arc::new(AppStore::new()?);
        ensure_builtin_apps(&store)?;
        ensure_builtin_miao(&store)?;
        proxy_panel.start_watchdog();
        let remote = Arc::new(crate::remote::store::RemoteStore::new()?);
        let mut process =
            ProcessManager::new(remote.clone(), crate::remote::worker::runtimes_dir());
        process.set_store(store.clone());
        let mut service = ServiceProcessManager::new();
        service.set_store(store.clone());
        Ok(Self {
            shared: std::sync::OnceLock::new(),
            remote,
            remote_health: Default::default(),
            proxy_panel,
            store,
            process,
            service,
            started: Mutex::new(HashSet::new()),
            wake: tokio::sync::Notify::new(),
            apps_changed: std::sync::atomic::AtomicBool::new(true),
        })
    }

    /// Keeps the taskbar's application tabs current without any request: runtimes
    /// become ready, crash or are restarted by the watchdog, and remote
    /// applications change state on their instance.
    /// The monitor alone publishes the tabs. The application list is read again
    /// only after `refresh_stream_apps`; the periodic pass only checks runtimes.
    pub fn start_monitor(self: &Arc<Self>) {
        let apps = self.clone();
        tokio::spawn(async move {
            let mut listed = OrderedApps::default();
            loop {
                let started: Vec<String> = apps.started.lock().unwrap().iter().cloned().collect();
                crate::remote::gateway::refresh_remote_health(apps.clone(), started);
                // Listing applications opens databases, and runtimes publish
                // their state in files.
                let state = apps.clone();
                let previous = std::mem::take(&mut listed);
                listed = tokio::task::spawn_blocking(move || state.publish_stream_apps(previous))
                    .await
                    .unwrap_or_default();
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
                    _ = apps.wake.notified() => {}
                }
            }
        });
    }

    /// Asks the monitor to publish the taskbar's application tabs again, e.g.
    /// after an application was added, renamed, started or stopped. Cheap enough
    /// for any thread.
    pub fn refresh_stream_apps(&self) {
        self.apps_changed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.wake.notify_one();
    }

    pub fn refresh_window_apps(&self) {
        self.wake.notify_one();
    }

    /// Publishes the applications shown as taskbar tabs: local applications in
    /// their own runtime, and started remote applications. Returns the list it
    /// used, which is `listed` unless the applications changed.
    fn publish_stream_apps(&self, listed: OrderedApps) -> OrderedApps {
        let listed = if self
            .apps_changed
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            match self.store.list_ordered() {
                Ok(apps) => apps,
                Err(e) => {
                    log::warn!("Cannot list application tabs: {}", e);
                    // Retried on the next pass.
                    self.apps_changed
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    return listed;
                }
            }
        } else {
            listed
        };
        if let Some(shared) = self.shared.get().and_then(std::sync::Weak::upgrade) {
            shared.set_taskbar_apps(crate::web::shared::TaskbarApps {
                apps: self.stream_apps(&listed.apps),
                order: listed.order.clone(),
                window_apps: self.process.apps_for_clients(&shared.window_pids()),
                names: listed
                    .apps
                    .iter()
                    .map(|app| {
                        (
                            app.id.clone(),
                            crate::web::shared::TaskbarAppName {
                                name: app.name.clone(),
                                name_revision: app.name_revision,
                                can_rename: app.id != BUILTIN_MIAO_ID,
                            },
                        )
                    })
                    .collect(),
            });
        }
        listed
    }

    fn stream_apps(&self, apps: &[ManagedApp]) -> Vec<crate::web::shared::StreamApp> {
        apps.iter()
            .filter_map(|app| {
                let (kind, status) = match app.app_type {
                    AppType::DesktopApp if self.process.in_worker(&app.id) == Some(true) => {
                        match self.process.status(&app.id) {
                            AppStatus::Starting => ("shared", "starting"),
                            AppStatus::Running => ("shared", "running"),
                            _ => return None,
                        }
                    }
                    AppType::RemoteApp if self.remote_started(&app.id) => {
                        ("remote", self.remote_state(&app.id).0)
                    }
                    _ => return None,
                };
                Some(crate::web::shared::StreamApp {
                    id: app.id.clone(),
                    name: app.name.clone(),
                    name_revision: app.name_revision,
                    can_rename: app.id != BUILTIN_MIAO_ID,
                    kind,
                    status,
                })
            })
            .collect()
    }

    pub fn remote_started(&self, id: &str) -> bool {
        self.started.lock().unwrap().contains(id)
    }

    /// The connection state of a remote application and why it is not connected.
    pub fn remote_state(&self, id: &str) -> (&'static str, Option<ApiError>) {
        if !self.remote_started(id) {
            return ("disconnected", None);
        }
        self.remote_health
            .get(id)
            .map_or(("connecting", None), |h| (h.state, h.detail))
    }

    /// Whether the application runs differently than its sharing setting asks for.
    pub fn restart_required(&self, id: &str, sharing: bool) -> bool {
        self.process
            .in_worker(id)
            .is_some_and(|worker| worker != sharing)
    }

    pub fn revoke_views(&self, id: &str, reason: crate::remote::media::StopReason) {
        if let Some(shared) = self.shared.get().and_then(std::sync::Weak::upgrade) {
            shared.app_views.revoke(id, false, reason);
            shared.media_router.revoke(id, false, reason);
        }
    }

    /// Save current running apps state
    pub fn save_running_state(&self) -> Result<(), String> {
        let running_ids = self.running_app_ids()?;

        if running_ids.is_empty() {
            log::info!("No running apps to save");
            AppRunningState::clear()?;
            return Ok(());
        }

        let state = AppRunningState::new(running_ids);
        state.save()
    }

    pub async fn shutdown_running_apps_with_state(&self) -> Result<(), String> {
        let apps = self.store.list()?;
        let running_ids: Vec<String> = apps
            .iter()
            .filter(|app| app.id != BUILTIN_MIAO_ID)
            .filter(|app| {
                matches!(
                    self.app_status(app),
                    AppStatus::Running | AppStatus::Starting
                )
            })
            .map(|app| app.id.clone())
            .collect();

        if running_ids.is_empty() {
            log::info!("No running apps to save before shutdown");
            AppRunningState::clear()?;
        } else {
            AppRunningState::new(running_ids.clone()).save()?;
        }

        for app_id in running_ids {
            match self.store.get(&app_id) {
                Ok(app) => {
                    log::info!(
                        "Stopping managed app during shutdown: {} ({})",
                        app.name,
                        app.id
                    );
                    if let Err(err) = self.stop_app_processes(&app).await {
                        log::warn!("Failed to stop app {} during shutdown: {}", app.id, err);
                    }
                }
                Err(err) => log::warn!(
                    "Running app {} disappeared before shutdown: {}",
                    app_id,
                    err
                ),
            }
        }

        Ok(())
    }

    fn running_app_ids(&self) -> Result<Vec<String>, String> {
        let apps = self.store.list()?;
        Ok(apps
            .iter()
            .filter(|app| app.id != BUILTIN_MIAO_ID)
            .filter(|app| {
                matches!(
                    self.app_status(app),
                    AppStatus::Running | AppStatus::Starting
                )
            })
            .map(|app| app.id.clone())
            .collect())
    }

    /// Restore previously running apps
    pub async fn restore_running_state(&self) -> Result<(), String> {
        let state = match AppRunningState::load() {
            Ok(s) => s,
            Err(_) => {
                log::info!("No previous running state to restore");
                return Ok(());
            }
        };

        // Only restore if state is recent (within 5 minutes)
        if !state.is_recent() {
            log::info!("Running state is too old, skipping restore");
            AppRunningState::clear()?;
            return Ok(());
        }

        log::info!("Restoring {} running apps", state.app_ids.len());

        for app_id in &state.app_ids {
            if app_id == BUILTIN_MIAO_ID {
                continue;
            }
            match self.store.get(app_id) {
                Ok(app) => {
                    log::info!("Restoring app: {} ({})", app.name, app_id);
                    let result = self.start_app_processes(&app).await.map(|_| ());

                    if let Err(e) = result {
                        log::warn!("Failed to restore app {}: {}", app_id, e);
                    } else {
                        // Small delay between starts to avoid overwhelming the system
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                    }
                }
                Err(e) => {
                    log::warn!("App {} not found in store: {}", app_id, e);
                }
            }
        }

        // Clear the state file after restoration
        AppRunningState::clear()?;
        log::info!("Running state restoration completed");
        Ok(())
    }

    pub async fn start_autostart_apps(&self) -> Result<(), String> {
        let apps = self.store.list()?;
        for app in apps
            .iter()
            .filter(|app| app.autostart && app.id != BUILTIN_MIAO_ID)
        {
            let result = self.start_app_processes(app).await.map(|_| ());

            match result {
                Ok(()) => log::info!("Autostart app started: {} ({})", app.name, app.id),
                Err(e) if e.code == "app_already_running" => {
                    log::debug!("Autostart app already running: {} ({})", app.name, app.id)
                }
                Err(e) => log::warn!("Failed to autostart app {} ({}): {}", app.name, app.id, e),
            }
        }
        Ok(())
    }

    async fn start_app_processes(&self, app: &ManagedApp) -> Result<Option<u32>, ApiError> {
        let result = self.launch_app(app).await;
        self.refresh_stream_apps();
        result
    }

    async fn launch_app(&self, app: &ManagedApp) -> Result<Option<u32>, ApiError> {
        if app.id == BUILTIN_MIAO_ID {
            return Ok(self.proxy_panel.start().await?);
        }
        if app.id == "builtin-chrome" {
            self.proxy_panel.prepare_chrome_proxy().await;
        }
        match app.app_type {
            AppType::DesktopApp => {
                // Building a command opens databases.
                let (process, launch) = (self.process.clone(), app.clone());
                let pid = tokio::task::spawn_blocking(move || process.start(&launch))
                    .await
                    .map_err(|e| e.to_string())??;
                if self.process.in_worker(&app.id) != Some(true) {
                    return Ok(Some(pid));
                }
                // Show the tab while the runtime starts.
                self.refresh_stream_apps();
                for _ in 0..900 {
                    match self.process.status(&app.id) {
                        AppStatus::Running => return Ok(Some(pid)),
                        AppStatus::Starting => {}
                        _ => {
                            if let Some(error) = self.process.failure(&app.id) {
                                return Err(error);
                            }
                            return Err(match crate::remote::worker::failure(&app.id) {
                                Some(failure) => failure.message.into(),
                                None => ApiError::new(
                                    "app_runtime_failed",
                                    "Application runtime failed to start; see the application log",
                                ),
                            });
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                Err(ApiError::new(
                    "app_runtime_timeout",
                    "Application runtime did not become ready; check its log",
                ))
            }
            AppType::BackgroundApp => {
                if !service_process::has_launch_command(app) {
                    return Err(ApiError::new(
                        "launch_command_required",
                        "background app requires launch_command",
                    ));
                }
                Ok(self.service.start_and_wait(app).await?)
            }
            AppType::RemoteApp => {
                // The process runs on B; starting connects to it.
                self.remote.target(&app.id)?;
                if !self.started.lock().unwrap().insert(app.id.clone()) {
                    return Err(ApiError::new(
                        "app_already_running",
                        "App is already running",
                    ));
                }
                self.remote_health.forget(&app.id);
                self.wake.notify_one();
                Ok(None)
            }
            AppType::CliApp => Err(ApiError::new(
                "cli_not_startable",
                "CLI apps are one-shot tools and cannot be started",
            )),
        }
    }

    async fn stop_app_processes(&self, app: &ManagedApp) -> Result<(), ApiError> {
        self.revoke_views(&app.id, crate::remote::media::StopReason::Stopped);
        if app.id == BUILTIN_MIAO_ID {
            return Ok(self.proxy_panel.stop().await?);
        }
        let result = match app.app_type {
            AppType::DesktopApp => {
                let (process, id) = (self.process.clone(), app.id.clone());
                tokio::task::spawn_blocking(move || process.stop(&id))
                    .await
                    .map_err(|e| e.to_string())?
            }
            AppType::BackgroundApp => self.service.stop(&app.id).map_err(ApiError::from),
            AppType::RemoteApp => {
                self.started.lock().unwrap().remove(&app.id);
                self.remote_health.forget(&app.id);
                Ok(())
            }
            AppType::CliApp => Ok(()),
        };
        self.refresh_stream_apps();
        result
    }

    async fn restart_app_processes(&self, app: &ManagedApp) -> Result<Option<u32>, ApiError> {
        if app.id == BUILTIN_MIAO_ID {
            return Ok(self.proxy_panel.restart().await?);
        }
        let _ = self.stop_app_processes(app).await;
        self.start_app_processes(app).await
    }

    pub fn app_status(&self, app: &ManagedApp) -> AppStatus {
        if app.id == BUILTIN_MIAO_ID {
            let status = self.proxy_panel.status();
            return if status.running {
                AppStatus::Running
            } else if status.desired_running {
                AppStatus::Crashed
            } else {
                AppStatus::Stopped
            };
        }
        match app.app_type {
            AppType::DesktopApp => self.process.status(&app.id),
            AppType::BackgroundApp => self.service.status(&app.id),
            AppType::RemoteApp if self.remote_started(&app.id) => AppStatus::Running,
            AppType::CliApp | AppType::RemoteApp => AppStatus::Stopped,
        }
    }

    fn app_pid(&self, app: &ManagedApp) -> Option<u32> {
        if app.id == BUILTIN_MIAO_ID {
            return self.proxy_panel.status().pid;
        }
        match app.app_type {
            AppType::DesktopApp => self.process.pid(&app.id),
            AppType::BackgroundApp => self.service.pid(&app.id),
            AppType::CliApp | AppType::RemoteApp => None,
        }
    }

    pub async fn start_builtin_apps(&self) -> Result<(), String> {
        let app = self.store.get(BUILTIN_MIAO_ID)?;
        if self.proxy_panel.autostart(app.autostart) && self.proxy_panel.status().enabled {
            self.proxy_panel.start().await?;
        } else {
            self.proxy_panel.cleanup_inactive_instance().await?;
        }
        Ok(())
    }
}

#[cfg(test)]
impl AppsState {
    /// An application manager with in-memory stores and its files in a new
    /// temporary directory, never in `$IVNC_HOME`.
    pub(crate) fn for_test(
        proxy_ip: Option<std::net::IpAddr>,
        autostart_override: Option<bool>,
    ) -> Arc<Self> {
        let store = Arc::new(AppStore::in_memory());
        ensure_builtin_miao(&store).unwrap();
        let remote = Arc::new(crate::remote::store::RemoteStore::in_memory());
        let root =
            std::env::temp_dir().join(format!("ivnc-apps-{} space'quote", uuid::Uuid::new_v4()));
        Arc::new(Self {
            shared: std::sync::OnceLock::new(),
            remote: remote.clone(),
            remote_health: Default::default(),
            store,
            proxy_panel: Arc::new(ProxyPanelManager::isolated(
                false,
                &root,
                proxy_ip,
                autostart_override,
            )),
            process: ProcessManager::new(remote, root.join("runtimes")),
            service: ServiceProcessManager::new(),
            started: Mutex::new(HashSet::new()),
            wake: tokio::sync::Notify::new(),
            apps_changed: std::sync::atomic::AtomicBool::new(true),
        })
    }
}

fn ensure_builtin_miao(store: &AppStore) -> Result<(), String> {
    match store.get_optional(BUILTIN_MIAO_ID)? {
        Some(mut app) => {
            // Preserve the startup preference saved by older versions across upgrades.
            if app.app_type != AppType::BackgroundApp {
                app.app_type = AppType::BackgroundApp;
                store.update(&app)?;
            }
        }
        None => {
            // Names are unique; a user-created Miao app must not disable all app management.
            let names: Vec<_> = store.list()?.into_iter().map(|app| app.name).collect();
            let mut name = "Miao".to_string();
            let mut suffix = 1;
            while names.contains(&name) {
                name = format!("Miao (iVnc {suffix})");
                suffix += 1;
            }
            store.add(&ManagedApp {
                id: BUILTIN_MIAO_ID.into(),
                name,
                app_type: AppType::BackgroundApp,
                autostart: true,
                url: None,
                launch_command: None,
                launch_env_vars: None,
                launch_cwd: None,
                launch_wait_timeout_secs: None,
                exec_command: None,
                env_vars: None,
                cli_binary_path: None,
                cli_env_vars: None,
                created_at: chrono_now(),
                sort_order: 0,
                name_revision: 0,
            })?;
        }
    }
    Ok(())
}

fn ensure_builtin_apps(store: &Arc<AppStore>) -> Result<(), String> {
    let chrome_command = builtin_chrome_command()?;

    for app in store.list()? {
        let is_builtin_terminal = app.id == "builtin-terminal"
            || (app.name == "Terminal" && app.exec_command.as_deref() == Some("alacritty"));
        if is_builtin_terminal {
            match store.delete(&app.id) {
                Ok(()) => log::info!("Removed legacy built-in desktop terminal app: {}", app.id),
                Err(err) => log::warn!("Failed to remove legacy terminal app {}: {}", app.id, err),
            }
        }
    }
    ensure_builtin_chrome(store, &chrome_command)?;
    desktop_entry::ensure_desktop_entry("Chrome", &chrome_command)?;
    ensure_builtin_agent_browser_cli(store)?;
    Ok(())
}

fn ensure_builtin_chrome(store: &AppStore, chrome_command: &str) -> Result<(), String> {
    match store.get("builtin-chrome") {
        Ok(mut app) => {
            if app.app_type != AppType::DesktopApp
                || app.exec_command.as_deref() != Some(chrome_command)
            {
                app.app_type = AppType::DesktopApp;
                // Refresh managed launch settings without replacing the saved startup preference.
                app.url = None;
                app.launch_command = None;
                app.launch_env_vars = None;
                app.launch_cwd = None;
                app.launch_wait_timeout_secs = None;
                app.exec_command = Some(chrome_command.to_string());
                app.env_vars = None;
                app.cli_binary_path = None;
                app.cli_env_vars = None;
                store.update(&app)?;
                log::info!("Updated built-in Chrome desktop app");
            }
        }
        Err(_) => {
            let app = ManagedApp {
                id: "builtin-chrome".to_string(),
                name: "Chrome".to_string(),
                app_type: AppType::DesktopApp,
                autostart: false,
                url: None,
                launch_command: None,
                launch_env_vars: None,
                launch_cwd: None,
                launch_wait_timeout_secs: None,
                exec_command: Some(chrome_command.to_string()),
                env_vars: None,
                cli_binary_path: None,
                cli_env_vars: None,
                created_at: chrono_now(),
                sort_order: 0,
                name_revision: 0,
            };
            match store.add(&app) {
                Ok(()) => log::info!("Added built-in Chrome desktop app"),
                Err(err) if err.code == "app_name_exists" => {}
                Err(err) => return Err(err.into()),
            }
        }
    }
    Ok(())
}

fn ensure_builtin_agent_browser_cli(store: &Arc<AppStore>) -> Result<(), String> {
    match store.get("builtin-agent-browser") {
        Ok(mut app) => {
            let mut changed = false;
            if app.app_type != AppType::CliApp {
                app.app_type = AppType::CliApp;
                app.url = None;
                app.launch_command = None;
                app.launch_env_vars = None;
                app.launch_cwd = None;
                app.launch_wait_timeout_secs = None;
                app.exec_command = None;
                app.env_vars = None;
                changed = true;
            }
            if app.autostart {
                app.autostart = false;
                changed = true;
            }
            if app
                .cli_binary_path
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
            {
                app.cli_binary_path = Some("/usr/local/bin/agent-browser".to_string());
                changed = true;
            }
            if app.cli_env_vars.is_none() {
                app.cli_env_vars = Some(HashMap::from([("NO_COLOR".to_string(), "1".to_string())]));
                changed = true;
            }
            if changed {
                store.update(&app)?;
                log::info!("Updated built-in agent-browser CLI app defaults");
            }
        }
        Err(_) => {
            let app = ManagedApp {
                id: "builtin-agent-browser".to_string(),
                name: "agent-browser".to_string(),
                app_type: AppType::CliApp,
                autostart: false,
                url: None,
                launch_command: None,
                launch_env_vars: None,
                launch_cwd: None,
                launch_wait_timeout_secs: None,
                exec_command: None,
                env_vars: None,
                cli_binary_path: Some("/usr/local/bin/agent-browser".to_string()),
                cli_env_vars: Some(HashMap::from([("NO_COLOR".to_string(), "1".to_string())])),
                created_at: chrono_now(),
                sort_order: 0,
                name_revision: 0,
            };
            match store.add(&app) {
                Ok(()) => log::info!("Added built-in agent-browser CLI app"),
                Err(err) if err.code == "app_name_exists" => {}
                Err(err) => return Err(err.into()),
            }
        }
    }
    Ok(())
}

fn builtin_chrome_data_dir() -> Result<PathBuf, String> {
    let dir = crate::paths::chrome_profile();
    std::fs::create_dir_all(&dir).map_err(|err| {
        format!(
            "Failed to create Chrome data dir {}: {}",
            dir.display(),
            err
        )
    })?;
    Ok(dir)
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
    {
        return value.to_string();
    }

    format!("'{}'", value.replace('\'', "'\\''"))
}

fn builtin_chrome_command() -> Result<String, String> {
    let data_dir = builtin_chrome_data_dir()?;
    let crash_dir = crate::paths::chrome_crash_dumps_dir();
    std::fs::create_dir_all(&crash_dir).map_err(|err| {
        format!(
            "Failed to create Chrome crash dumps dir {}: {}",
            crash_dir.display(),
            err
        )
    })?;
    let data_dir = shell_quote(&data_dir.to_string_lossy());
    let crash_dir = shell_quote(&crash_dir.to_string_lossy());
    let debug_port = chrome_devtools_port();
    let proxy_file = shell_quote(
        &crate::paths::miao_dir()
            .join("chrome-proxy")
            .to_string_lossy(),
    );
    Ok(format!(
        r#"profile_dir={data_dir};
crash_dir={crash_dir};
rm -f "$profile_dir/SingletonLock" "$profile_dir/SingletonSocket" "$profile_dir/SingletonCookie";
set --;
if [ "${{IVNC_PROXY_PANEL_ENABLED:-1}}" != '0' ] && [ -s {proxy_file} ]; then
  IFS= read -r proxy_server < {proxy_file};
  set -- "--proxy-server=$proxy_server";
fi;
exec google-chrome "$@" --user-data-dir="$profile_dir" --crash-dumps-dir="$crash_dir" --remote-debugging-host=0.0.0.0 --remote-debugging-port={debug_port} --ozone-platform=wayland --class=ivnc-chrome-windowed --test-type --no-first-run --no-default-browser-check --disable-features=MediaRouter --disable-background-networking --disable-process-singleton --no-sandbox --disable-setuid-sandbox --disable-dev-shm-usage
"#
    ).replace('\n', " "))
}

/// DevTools remote-debugging port the built-in Chrome is launched with
/// (see `builtin_chrome_command`). Overridable via `IVNC_CHROME_DEBUG_PORT`
/// so multiple instances (or a sandboxed test instance) don't collide on 9222.
pub(crate) fn chrome_devtools_port() -> u16 {
    std::env::var("IVNC_CHROME_DEBUG_PORT")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(9222)
}

pub fn router(state: Arc<AppsState>) -> Router {
    Router::new()
        .route("/api/apps", get(list_apps).post(add_app))
        .route("/api/apps/order", get(get_order).put(reorder_apps))
        .route("/api/apps/launch-context", get(launch_context))
        .route("/api/apps/{id}/name", put(rename_app))
        .route(
            "/api/apps/{id}",
            get(get_app).put(update_app).delete(delete_app),
        )
        .route("/api/apps/{id}/start", post(start_app))
        .route("/api/apps/{id}/stop", post(stop_app))
        .route("/api/apps/{id}/restart", post(restart_app))
        .route("/api/apps/{id}/data-size", get(data_size))
        .route("/api/apps/{id}/clear-data", post(clear_data))
        .route("/api/apps/{id}/logs", get(get_logs))
        .with_state(state)
}

async fn launch_context() -> axum::Json<serde_json::Value> {
    axum::Json(json!({
        "identity": super::launch::identity(),
        "session_type": "wayland",
        "cross_user_supported": false,
    }))
}

fn json_response(status: StatusCode, body: serde_json::Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn err_response(status: StatusCode, error: impl Into<ApiError>) -> Response {
    json_response(status, error.into().body())
}

fn parse_env_vars(value: Option<&serde_json::Value>) -> Option<HashMap<String, String>> {
    value.and_then(|v| v.as_object()).and_then(|obj| {
        let vars: HashMap<String, String> = obj
            .iter()
            .filter_map(|(key, value)| {
                value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| (key.trim().to_string(), value.to_string()))
            })
            .filter(|(key, _)| !key.is_empty())
            .collect();
        if vars.is_empty() {
            None
        } else {
            Some(vars)
        }
    })
}

fn parse_optional_string(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

pub fn cli_install_status(app: &ManagedApp) -> (bool, &'static str, Option<String>) {
    if app.app_type != AppType::CliApp {
        return (false, "not_cli", None);
    }
    let Some(path) = app
        .cli_binary_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    else {
        return (
            false,
            "missing_path",
            Some("missing binary path".to_string()),
        );
    };
    let path = std::path::Path::new(path);
    if !path.is_absolute() {
        return (
            false,
            "invalid_path",
            Some("binary path must be absolute".to_string()),
        );
    }
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return (
                false,
                "missing",
                Some("binary path does not exist".to_string()),
            );
        }
        Err(err) => return (false, "invalid_path", Some(err.to_string())),
    };
    if !metadata.is_file() {
        return (
            false,
            "not_file",
            Some("binary path is not a regular file".to_string()),
        );
    }
    if metadata.permissions().mode() & 0o111 == 0 {
        return (
            false,
            "not_executable",
            Some("binary path is not executable".to_string()),
        );
    }
    (true, "installed", None)
}

fn app_json(
    state: &AppsState,
    app: &ManagedApp,
    status: &str,
    pid: Option<u32>,
    data_bytes: u64,
) -> serde_json::Value {
    let mut obj = json!({
        "id": app.id,
        "name": app.name,
        "name_revision": app.name_revision,
        "can_rename": app.id != BUILTIN_MIAO_ID,
        "app_type": app.app_type.as_str(),
        "autostart": app.autostart,
        "status": status,
        "pid": pid,
        "data_size_bytes": data_bytes,
        "data_size_human": datadir::size_human(data_bytes),
        "created_at": app.created_at,
        "sort_order": app.sort_order,
    });

    if app.id == BUILTIN_MIAO_ID {
        obj["autostart"] = json!(state.proxy_panel.autostart(app.autostart));
        let proxy = state.proxy_panel.status();
        let command = state.proxy_panel.launch_command();
        let launch_command = std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|arg| shell_quote(&arg.to_string_lossy()))
            .collect::<Vec<_>>()
            .join(" ");
        // Show only explicit overrides, never the inherited process environment.
        let launch_env: std::collections::BTreeMap<_, _> = command
            .get_envs()
            .filter_map(|(key, value)| {
                value.map(|value| {
                    (
                        key.to_string_lossy().into_owned(),
                        value.to_string_lossy().into_owned(),
                    )
                })
            })
            .collect();
        obj["launch_command"] = json!(launch_command);
        obj["launch_env_vars"] = json!(launch_env);
        obj["launch_cwd"] = json!(command.get_current_dir());
        obj["launch_wait_timeout_secs"] = json!(MIAO_START_TIMEOUT_SECS);
        obj["url"] = json!(proxy.url);
        obj["status"] = json!(if proxy.running {
            "running"
        } else if proxy.desired_running {
            "crashed"
        } else {
            "stopped"
        });
        obj["pid"] = json!(proxy.pid);
        obj["builtin"] = json!(true);
        obj["managed_by"] = json!("ivnc");
        obj["panel_url"] = json!("/proxy/");
        obj["enabled"] = json!(proxy.enabled);
        obj["ready"] = json!(proxy.ready);
        obj["error"] = json!(proxy.error);
        obj["can_delete"] = json!(false);
        obj["can_clear_data"] = json!(false);
        return obj;
    }

    match app.app_type {
        AppType::RemoteApp => {
            if let Ok(target) = state.remote.target(&app.id) {
                obj["url"] = json!(target.share_url());
                obj["remote"] = json!(target);
            }
            let (remote_state, remote_detail) = state.remote_state(&app.id);
            obj["started"] = json!(state.remote_started(&app.id));
            obj["remote_state"] = json!(remote_state);
            obj["remote_detail"] = json!(remote_detail.as_ref().map_or("", |e| &e.message));
            obj["remote_detail_code"] = json!(remote_detail.as_ref().map(|e| e.code));
            obj["remote_detail_params"] = json!(remote_detail
                .as_ref()
                .map(|e| &e.params)
                .filter(|p| !p.is_empty()));
            // Plain HTTP is only accepted on private networks; the list marks it.
            obj["remote_plaintext"] = json!(obj["url"]
                .as_str()
                .is_some_and(|url| url.starts_with("http://")));
            obj["can_clear_data"] = json!(false);
        }
        AppType::BackgroundApp => {
            obj["url"] = json!(app.url);
            obj["launch_command"] = json!(app.launch_command);
            obj["launch_env_vars"] = json!(app.launch_env_vars);
            obj["launch_cwd"] = json!(app.launch_cwd);
            obj["launch_wait_timeout_secs"] = json!(app.launch_wait_timeout_secs);
        }
        AppType::DesktopApp => {
            obj["exec_command"] = json!(app.exec_command);
            obj["env_vars"] = json!(app.env_vars);
            obj["launch_error"] = json!(state.process.failure(&app.id).map(|e| e.body()));
            // A running process is not proof of a mapped application window.
            // Worker status describes its session; it has a different compositor.
            obj["window_ready"] = json!(state.process.window_ready(&app.id));
            let enabled = state
                .remote
                .policy(&app.id)
                .map(|p| p.enabled)
                .unwrap_or(false);
            obj["sharing"] = json!({
                "enabled": enabled,
                "restart_required": state.restart_required(&app.id, enabled),
            });
        }
        AppType::CliApp => {
            let (installed, install_status, install_error) = cli_install_status(app);
            obj["cli_binary_path"] = json!(app.cli_binary_path);
            obj["cli_env_vars"] = json!(app.cli_env_vars);
            obj["installed"] = json!(installed);
            obj["install_status"] = json!(install_status);
            obj["install_error"] = json!(install_error);
        }
    }

    obj
}

async fn list_apps(State(state): State<Arc<AppsState>>) -> Response {
    match state.store.list_ordered() {
        Ok(listed) => {
            let items: Vec<_> = listed
                .apps
                .iter()
                .map(|app| {
                    let st = state.app_status(app);
                    let pid = state.app_pid(app);
                    let size = datadir::durable_size(app);
                    app_json(&state, app, &format!("{:?}", st).to_lowercase(), pid, size)
                })
                .collect();
            json_response(
                StatusCode::OK,
                json!({"apps": items, "order_revision": listed.order.order_revision}),
            )
        }
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn get_order(State(state): State<Arc<AppsState>>) -> Response {
    match state.store.order() {
        Ok(order) => json_response(StatusCode::OK, json!(order)),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReorderRequest {
    ordered_ids: Vec<String>,
    base_revision: u64,
}

async fn reorder_apps(
    State(state): State<Arc<AppsState>>,
    body: Result<axum::Json<ReorderRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Ok(axum::Json(body)) = body else {
        return err_response(
            StatusCode::BAD_REQUEST,
            ApiError::new(
                "app_order_invalid",
                "Expected ordered_ids and base_revision",
            ),
        );
    };
    match state.store.reorder(&body.ordered_ids, body.base_revision) {
        Ok(order) => {
            state.refresh_stream_apps();
            json_response(StatusCode::OK, json!(order))
        }
        Err(e) => {
            let status = match e.code {
                "app_order_invalid" => StatusCode::BAD_REQUEST,
                "app_order_conflict" => StatusCode::CONFLICT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            err_response(status, e)
        }
    }
}

fn name_error(error: ApiError) -> Response {
    let status = match error.code {
        "app_name_required" | "app_name_invalid" | "app_name_too_long" => StatusCode::BAD_REQUEST,
        "app_name_exists" | "app_name_conflict" => StatusCode::CONFLICT,
        "app_not_found" => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    err_response(status, error)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameRequest {
    name: String,
    base_revision: u64,
}

async fn rename_app(
    State(state): State<Arc<AppsState>>,
    Path(id): Path<String>,
    body: Result<axum::Json<RenameRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    if id == BUILTIN_MIAO_ID {
        return err_response(
            StatusCode::FORBIDDEN,
            ApiError::new("miao_read_only", "Miao application settings are read-only"),
        );
    }
    let Ok(axum::Json(body)) = body else {
        return name_error(ApiError::new(
            "app_name_invalid",
            "Expected name and base_revision",
        ));
    };
    match state.store.rename(&id, &body.name, body.base_revision) {
        Ok(app) => {
            state.refresh_stream_apps();
            json_response(StatusCode::OK, json!(app))
        }
        Err(e) => name_error(e),
    }
}

async fn add_app(
    State(state): State<Arc<AppsState>>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Response {
    let mut name = match body.get("name").and_then(|v| v.as_str()) {
        Some(n) if !n.trim().is_empty() => n.to_string(),
        _ if body["app_type"] == "remote" => String::new(),
        _ => {
            return err_response(
                StatusCode::BAD_REQUEST,
                ApiError::new("app_name_required", "missing name"),
            )
        }
    };

    let app_type_str = body
        .get("app_type")
        .and_then(|v| v.as_str())
        .unwrap_or("background");
    let app_type = match AppType::from_str(app_type_str) {
        Some(t) => t,
        None => {
            return err_response(
                StatusCode::BAD_REQUEST,
                ApiError::new("app_type_invalid", "Invalid application type"),
            )
        }
    };
    let remote_target = if app_type == AppType::RemoteApp {
        match crate::remote::gateway::connect_target(&body, &state.remote.node_id).await {
            Ok(target) => Some(target),
            Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
        }
    } else {
        None
    };

    if name.is_empty() {
        if let Some((target, _)) = &remote_target {
            name = target.name.clone();
        }
    }
    name = match validate_app_name(&name) {
        Ok(name) => name,
        Err(e) => return name_error(e),
    };

    let (
        url,
        launch_command,
        launch_env_vars,
        launch_cwd,
        launch_wait_timeout_secs,
        exec_command,
        env_vars,
        cli_binary_path,
        cli_env_vars,
    ) = match app_type {
        AppType::RemoteApp => (None, None, None, None, None, None, None, None, None),
        AppType::BackgroundApp => {
            let url = parse_optional_string(body.get("url"));
            let launch_command = match body
                .get("launch_command")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
            {
                Some(cmd) => Some(cmd),
                None => {
                    return err_response(
                        StatusCode::BAD_REQUEST,
                        ApiError::new(
                            "launch_command_required",
                            "missing launch_command for background",
                        ),
                    )
                }
            };
            let launch_env_vars = parse_env_vars(body.get("launch_env_vars"));
            let launch_cwd = body
                .get("launch_cwd")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let launch_wait_timeout_secs = body
                .get("launch_wait_timeout_secs")
                .and_then(|v| v.as_u64())
                .filter(|secs| *secs > 0);
            (
                url,
                launch_command,
                launch_env_vars,
                launch_cwd,
                launch_wait_timeout_secs,
                None,
                None,
                None,
                None,
            )
        }
        AppType::DesktopApp => {
            let exec_command = match body.get("exec_command").and_then(|v| v.as_str()) {
                Some(e) if !e.trim().is_empty() => Some(e.trim().to_string()),
                _ => {
                    return err_response(
                        StatusCode::BAD_REQUEST,
                        ApiError::new(
                            "exec_command_required",
                            "missing exec_command for desktop app",
                        ),
                    )
                }
            };
            let env_vars = parse_env_vars(body.get("env_vars"));
            (
                None,
                None,
                None,
                None,
                None,
                exec_command,
                env_vars,
                None,
                None,
            )
        }
        AppType::CliApp => {
            let cli_binary_path = match body.get("cli_binary_path").and_then(|v| v.as_str()) {
                Some(path) if !path.trim().is_empty() => Some(path.trim().to_string()),
                _ => {
                    return err_response(
                        StatusCode::BAD_REQUEST,
                        ApiError::new("cli_binary_path_required", "missing cli_binary_path"),
                    )
                }
            };
            let cli_env_vars = parse_env_vars(body.get("cli_env_vars"));
            (
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                cli_binary_path,
                cli_env_vars,
            )
        }
    };
    let autostart = body
        .get("autostart")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        && app_type != AppType::CliApp;

    let app = ManagedApp {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        app_type,
        autostart,
        url,
        launch_command,
        launch_env_vars,
        launch_cwd,
        launch_wait_timeout_secs,
        exec_command,
        env_vars,
        cli_binary_path,
        cli_env_vars,
        created_at: chrono_now(),
        sort_order: 0,
        name_revision: 0,
    };

    if let Err(e) = state.store.add(&app) {
        return err_response(StatusCode::CONFLICT, e);
    }

    if let Some((target, token)) = remote_target {
        if let Err(e) = state.remote.save_target(&app.id, &target) {
            let _ = state.store.delete(&app.id);
            state.refresh_stream_apps();
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, e);
        }
        state.remote_health.remember_token(&app.id, token);
    }
    if app.app_type == AppType::DesktopApp {
        let exec_command = app
            .exec_command
            .as_deref()
            .ok_or_else(|| "missing exec_command for desktop app".to_string());
        if let Err(e) =
            exec_command.and_then(|exec| desktop_entry::ensure_desktop_entry(&app.name, exec))
        {
            if let Err(delete_err) = state.store.delete(&app.id) {
                log::warn!(
                    "Failed to roll back app {} after desktop entry error: {}",
                    app.id,
                    delete_err
                );
            }
            state.refresh_stream_apps();
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, e);
        }
    }

    state.refresh_stream_apps();
    let app = state.store.get(&app.id).unwrap_or(app);
    let status = state.app_status(&app);
    let pid = state.app_pid(&app);
    let size = datadir::durable_size(&app);
    json_response(
        StatusCode::CREATED,
        json!({"ok": true, "app": app_json(&state, &app, &format!("{:?}", status).to_lowercase(), pid, size)}),
    )
}

async fn get_app(State(state): State<Arc<AppsState>>, Path(id): Path<String>) -> Response {
    match state.store.get(&id) {
        Ok(app) => {
            let st = state.app_status(&app);
            let pid = state.app_pid(&app);
            let size = datadir::durable_size(&app);
            json_response(
                StatusCode::OK,
                app_json(&state, &app, &format!("{:?}", st).to_lowercase(), pid, size),
            )
        }
        Err(e) => err_response(StatusCode::NOT_FOUND, e),
    }
}

async fn update_app(
    State(state): State<Arc<AppsState>>,
    Path(id): Path<String>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Response {
    let mut app = match state.store.get(&id) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::NOT_FOUND, e),
    };

    if id == BUILTIN_MIAO_ID {
        return err_response(
            StatusCode::FORBIDDEN,
            ApiError::new(
                "miao_read_only",
                "Miao application settings are read-only; configure startup through the iVnc CLI",
            ),
        );
    }

    let rename = if app.app_type == AppType::RemoteApp && body.get("name").is_some() {
        let Some(name) = body["name"].as_str() else {
            return name_error(ApiError::new("app_name_invalid", "Expected a string name"));
        };
        let name = match validate_app_name(name) {
            Ok(name) => name,
            Err(e) => return name_error(e),
        };
        let revision = match body.get("name_revision") {
            Some(value) => match value.as_u64() {
                Some(revision) => revision,
                None => {
                    return name_error(ApiError::new("app_name_invalid", "Expected name_revision"))
                }
            },
            // Older clients are protected against changes during this request.
            None => app.name_revision,
        };
        if revision != app.name_revision {
            return name_error(
                ApiError::new(
                    "app_name_conflict",
                    "Application name changed; review and retry",
                )
                .param("name", app.name.clone())
                .param("name_revision", app.name_revision),
            );
        }
        Some((name, revision))
    } else {
        None
    };
    match app.app_type {
        AppType::RemoteApp => {
            if body["password"].as_str().is_some_and(|s| !s.is_empty()) {
                match crate::remote::gateway::connect_target(&body, &state.remote.node_id).await {
                    Ok((target, token)) => {
                        if let Err(e) = state.remote.save_target(&app.id, &target) {
                            return err_response(StatusCode::INTERNAL_SERVER_ERROR, e);
                        }
                        state.revoke_views(&app.id, crate::remote::media::StopReason::Closed);
                        state.remote_health.forget(&app.id);
                        state.remote_health.remember_token(&app.id, token);
                        state.wake.notify_one();
                    }
                    Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
                }
            } else if let Some(url) = body["url"].as_str() {
                let old = state.remote.target(&app.id).ok();
                if old.as_ref().map(|t| t.share_url()).as_deref() != Some(url.trim()) {
                    return err_response(
                        StatusCode::BAD_REQUEST,
                        ApiError::new(
                            "remote_password_for_url",
                            "Enter the password when changing the remote address",
                        ),
                    );
                }
            }
        }
        AppType::BackgroundApp => {
            if body.get("url").is_some() {
                app.url = parse_optional_string(body.get("url"));
            }
            if let Some(command) = body
                .get("launch_command")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
            {
                app.launch_command = Some(command);
            }
            app.launch_env_vars = parse_env_vars(body.get("launch_env_vars"));
            app.launch_cwd = body
                .get("launch_cwd")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            app.launch_wait_timeout_secs = body
                .get("launch_wait_timeout_secs")
                .and_then(|v| v.as_u64())
                .filter(|secs| *secs > 0);
            if !service_process::has_launch_command(&app) {
                return err_response(
                    StatusCode::BAD_REQUEST,
                    ApiError::new(
                        "launch_command_required",
                        "missing launch_command for background",
                    ),
                );
            }
        }
        AppType::DesktopApp => {
            if let Some(exec) = body.get("exec_command").and_then(|v| v.as_str()) {
                let exec = exec.trim();
                if exec.is_empty() {
                    return err_response(
                        StatusCode::BAD_REQUEST,
                        ApiError::new(
                            "exec_command_required",
                            "missing exec_command for desktop app",
                        ),
                    );
                }
                app.exec_command = Some(exec.to_string());
            }
            app.env_vars = parse_env_vars(body.get("env_vars"));
        }
        AppType::CliApp => {
            if let Some(path) = body.get("cli_binary_path").and_then(|v| v.as_str()) {
                let path = path.trim();
                if path.is_empty() {
                    return err_response(
                        StatusCode::BAD_REQUEST,
                        ApiError::new("cli_binary_path_required", "missing cli_binary_path"),
                    );
                }
                app.cli_binary_path = Some(path.to_string());
            }
            app.cli_env_vars = parse_env_vars(body.get("cli_env_vars"));
        }
    }

    if let Some(autostart) = body.get("autostart").and_then(|v| v.as_bool()) {
        app.autostart = app.app_type != AppType::CliApp && autostart;
    }

    if let Err(e) = state.store.update_with_name(
        &app,
        rename
            .as_ref()
            .map(|(name, revision)| (name.as_str(), *revision)),
    ) {
        return name_error(e);
    }
    // Tabs show the name.
    state.refresh_stream_apps();
    let current = state.store.get(&id).unwrap_or(app);
    json_response(
        StatusCode::OK,
        json!({"ok": true, "id": id, "name": current.name, "name_revision": current.name_revision}),
    )
}

async fn delete_app(State(state): State<Arc<AppsState>>, Path(id): Path<String>) -> Response {
    if id == BUILTIN_MIAO_ID {
        return err_response(
            StatusCode::FORBIDDEN,
            ApiError::new(
                "miao_not_deletable",
                "Built-in Miao cannot be deleted; disable autostart instead",
            ),
        );
    }
    // Clean up data directory before removing from store
    if let Ok(app) = state.store.get(&id) {
        let _ = state.stop_app_processes(&app).await;
        let _ = std::fs::remove_dir_all(datadir::app_root(&app));
        if crate::remote::valid_id(&app.id) {
            let _ = std::fs::remove_dir_all(state.process.runtime_dir(&app.id));
        }
    }
    state.revoke_views(&id, crate::remote::media::StopReason::Stopped);
    state.remote_health.forget(&id);
    let _ = state.remote.delete(&id);
    let result = state.store.delete(&id);
    state.refresh_stream_apps();
    match result {
        Ok(()) => json_response(StatusCode::OK, json!({"ok": true})),
        Err(e) => err_response(StatusCode::NOT_FOUND, e),
    }
}

async fn start_app(State(state): State<Arc<AppsState>>, Path(id): Path<String>) -> Response {
    if id == BUILTIN_MIAO_ID && !state.proxy_panel.status().enabled {
        return err_response(
            StatusCode::FORBIDDEN,
            ApiError::new("proxy_disabled", "Proxy disabled by administrator"),
        );
    }
    let app = match state.store.get(&id) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::NOT_FOUND, e),
    };

    match state.start_app_processes(&app).await {
        Ok(pid) => json_response(StatusCode::OK, json!({"ok": true, "pid": pid})),
        Err(e) if e.code == "app_already_running" => {
            json_response(StatusCode::OK, json!({"ok":true,"pid":state.app_pid(&app)}))
        }
        Err(e) => {
            if id != BUILTIN_MIAO_ID && app.app_type != AppType::RemoteApp {
                let _ = state.stop_app_processes(&app).await;
            }
            if app.app_type == AppType::DesktopApp {
                state.process.record_failure(&id, e.clone());
            }
            err_response(StatusCode::INTERNAL_SERVER_ERROR, e.or("app_start_failed"))
        }
    }
}

async fn stop_app(State(state): State<Arc<AppsState>>, Path(id): Path<String>) -> Response {
    let app = match state.store.get(&id) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::NOT_FOUND, e),
    };

    match state.stop_app_processes(&app).await {
        Ok(()) => json_response(StatusCode::OK, json!({"ok": true})),
        Err(e) => err_response(StatusCode::BAD_REQUEST, e.or("app_stop_failed")),
    }
}

async fn restart_app(State(state): State<Arc<AppsState>>, Path(id): Path<String>) -> Response {
    if id == BUILTIN_MIAO_ID && !state.proxy_panel.status().enabled {
        return err_response(
            StatusCode::FORBIDDEN,
            ApiError::new("proxy_disabled", "Proxy disabled by administrator"),
        );
    }
    let app = match state.store.get(&id) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::NOT_FOUND, e),
    };

    match state.restart_app_processes(&app).await {
        Ok(pid) => json_response(StatusCode::OK, json!({"ok": true, "pid": pid})),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.or("app_start_failed")),
    }
}

async fn data_size(State(state): State<Arc<AppsState>>, Path(id): Path<String>) -> Response {
    let app = match state.store.get(&id) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::NOT_FOUND, e),
    };
    let dir = datadir::app_root(&app);
    let bytes = datadir::durable_size(&app);
    json_response(
        StatusCode::OK,
        json!({
            "data_dir": dir.display().to_string(),
            "size_bytes": bytes,
            "size_human": datadir::size_human(bytes),
        }),
    )
}

async fn clear_data(State(state): State<Arc<AppsState>>, Path(id): Path<String>) -> Response {
    if id == BUILTIN_MIAO_ID {
        return err_response(
            StatusCode::FORBIDDEN,
            ApiError::new(
                "miao_data_not_clearable",
                "Miao data is shared persistent proxy configuration and cannot be cleared here",
            ),
        );
    }
    let app = match state.store.get(&id) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::NOT_FOUND, e),
    };
    if app.app_type == AppType::RemoteApp {
        return err_response(
            StatusCode::BAD_REQUEST,
            ApiError::new(
                "remote_data_not_clearable",
                "Remote application data is managed on its source instance",
            ),
        );
    }
    // Stop app before clearing data
    let _ = state.stop_app_processes(&app).await;
    match datadir::clear(&app) {
        Ok(()) => json_response(StatusCode::OK, json!({"ok": true})),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn get_logs(State(state): State<Arc<AppsState>>, Path(id): Path<String>) -> Response {
    // Verify app exists
    if let Err(e) = state.store.get(&id) {
        return err_response(StatusCode::NOT_FOUND, e);
    }
    if id == BUILTIN_MIAO_ID {
        let path = state.proxy_panel.log_path();
        return json_response(
            StatusCode::OK,
            json!({"logs":crate::proxy_panel::read_log_tail(&path, 200), "path":path}),
        );
    }
    let log_file = native::log_path(&id);
    let service_log_file = service_process::service_log_path(&id);
    let app_content =
        std::fs::read_to_string(&log_file).unwrap_or_else(|_| "(no app logs yet)".into());
    let service_content = std::fs::read_to_string(&service_log_file).ok();
    let content = if let Some(service_content) = service_content {
        format!(
            "== service.log ==\n{}\n\n== app.log ==\n{}",
            service_content, app_content
        )
    } else {
        app_content
    };
    // Return last 200 lines
    let lines: Vec<&str> = content.lines().collect();
    let start = if lines.len() > 200 {
        lines.len() - 200
    } else {
        0
    };
    let tail = lines[start..].join("\n");
    json_response(
        StatusCode::OK,
        json!({"logs": tail, "path": log_file.display().to_string()}),
    )
}

fn chrono_now() -> String {
    // UTC timestamp without chrono dependency
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let h = time_secs / 3600;
    let m = (time_secs % 3600) / 60;
    let s = time_secs % 60;
    // Days since 1970-01-01
    let (y, mo, day) = days_to_ymd(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, day, h, m, s)
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut y = 1970;
    loop {
        let dy = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            366
        } else {
            365
        };
        if days < dy {
            break;
        }
        days -= dy;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let mdays = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut mo = 0;
    for (i, &md) in mdays.iter().enumerate() {
        if days < md {
            mo = i + 1;
            break;
        }
        days -= md;
    }
    (y, mo as u64, days + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(
        proxy_ip: Option<std::net::IpAddr>,
        autostart_override: Option<bool>,
    ) -> Arc<AppsState> {
        AppsState::for_test(proxy_ip, autostart_override)
    }

    #[tokio::test]
    async fn chrome_autostart_edits_survive_registration_and_command_refresh() {
        let state = state(None, None);
        let id = "builtin-chrome";
        ensure_builtin_chrome(&state.store, "google-chrome --original").unwrap();
        assert!(!state.store.get(id).unwrap().autostart);

        for (autostart, refreshed_command) in [
            (true, "google-chrome --updated-enabled"),
            (false, "google-chrome --updated-disabled"),
        ] {
            let response = update_app(
                State(state.clone()),
                Path(id.to_string()),
                axum::Json(json!({"autostart": autostart, "env_vars": {"LANG": "zh_CN.UTF-8"}})),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let saved = state.store.get(id).unwrap();
            let original_command = saved.exec_command.as_deref().unwrap();

            ensure_builtin_chrome(&state.store, original_command).unwrap();
            let unchanged = state.store.get(id).unwrap();
            assert_eq!(unchanged.autostart, autostart);
            assert_eq!(unchanged.env_vars, saved.env_vars);

            ensure_builtin_chrome(&state.store, refreshed_command).unwrap();
            let updated = state.store.get(id).unwrap();
            assert_eq!(updated.autostart, autostart);
            assert_eq!(updated.exec_command.as_deref(), Some(refreshed_command));
        }
    }

    #[test]
    fn chrome_legacy_type_migration_preserves_autostart() {
        let store = AppStore::in_memory();
        ensure_builtin_chrome(&store, "google-chrome").unwrap();
        for autostart in [true, false] {
            let mut legacy = store.get("builtin-chrome").unwrap();
            legacy.app_type = AppType::BackgroundApp;
            legacy.autostart = autostart;
            legacy.launch_command = Some("legacy launcher".into());
            legacy.url = Some("http://127.0.0.1:1234".into());
            store.update(&legacy).unwrap();

            ensure_builtin_chrome(&store, "google-chrome --wayland").unwrap();
            let app = store.get("builtin-chrome").unwrap();
            assert_eq!(app.app_type, AppType::DesktopApp);
            assert_eq!(app.autostart, autostart);
            assert_eq!(app.exec_command.as_deref(), Some("google-chrome --wayland"));
            assert!(app.launch_command.is_none());
            assert!(app.url.is_none());
        }
    }

    #[test]
    fn miao_registration_defaults_once_and_preserves_preference_and_name_collisions() {
        let store = AppStore::in_memory();
        ensure_builtin_miao(&store).unwrap();
        let mut app = store.get(BUILTIN_MIAO_ID).unwrap();
        assert!(app.autostart);
        assert_eq!(app.app_type, AppType::BackgroundApp);
        app.autostart = false;
        store.update(&app).unwrap();
        ensure_builtin_miao(&store).unwrap();
        assert!(!store.get(BUILTIN_MIAO_ID).unwrap().autostart);
        let other = AppStore::in_memory();
        app.id = "user-miao".into();
        other.add(&app).unwrap();
        ensure_builtin_miao(&other).unwrap();
        assert_eq!(other.list().unwrap().len(), 2);
        assert_ne!(other.get(BUILTIN_MIAO_ID).unwrap().name, "Miao");
    }

    #[tokio::test]
    async fn miao_api_rejects_all_configuration_edits() {
        let state = state(None, None);
        let id = || Path(BUILTIN_MIAO_ID.to_string());
        let update = |body| update_app(State(state.clone()), id(), axum::Json(body));
        for body in [
            json!({"launch_command":"false"}),
            json!({"app_type":"desktop"}),
            json!({"name":"Changed"}),
            json!({"env_vars":{"HOME":"/tmp"}}),
            json!({"autostart":false}),
            json!({"app_type":"background", "autostart":false}),
            json!({}),
        ] {
            assert_eq!(update(body).await.status(), StatusCode::FORBIDDEN);
        }
        ensure_builtin_miao(&state.store).unwrap();
        assert!(state.store.get(BUILTIN_MIAO_ID).unwrap().autostart);
        assert_eq!(
            delete_app(State(state.clone()), id()).await.status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            clear_data(State(state.clone()), id()).await.status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            start_app(State(state.clone()), id()).await.status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            restart_app(State(state.clone()), id()).await.status(),
            StatusCode::FORBIDDEN
        );
        state.start_builtin_apps().await.unwrap();
        assert!(!state.proxy_panel.status().desired_running);
        let app = state.store.get(BUILTIN_MIAO_ID).unwrap();
        let json = app_json(&state, &app, "stopped", None, 0);
        assert_eq!(json["panel_url"], "/proxy/");
        assert_eq!(json["can_delete"], false);
        assert_eq!(json["can_clear_data"], false);
        assert_eq!(json["enabled"], false);
    }

    #[test]
    fn miao_json_displays_effective_configuration_instead_of_stale_database_fields() {
        for ip in [None, Some("203.0.113.10"), Some("2001:db8::10")] {
            let state = state(ip.map(|ip| ip.parse().unwrap()), None);
            let mut app = state.store.get(BUILTIN_MIAO_ID).unwrap();
            app.launch_command = Some("obsolete-command".into());
            app.launch_cwd = Some("/obsolete-directory".into());
            app.launch_env_vars = Some(HashMap::from([("OLD_SECRET".into(), "hidden".into())]));
            app.launch_wait_timeout_secs = Some(999);
            app.url = Some("http://obsolete-url/".into());
            let json = app_json(&state, &app, "stopped", None, 0);
            let cwd = json["launch_cwd"].as_str().unwrap();
            let executable = format!("{cwd}/miao-rust-linux-amd64");
            let expected_command = match ip {
                Some(ip) => format!(
                    "{} --nopasswd --mode global --vps-host {}",
                    shell_quote(&executable),
                    shell_quote(ip)
                ),
                None => format!("{} --nopasswd", shell_quote(&executable)),
            };
            assert_eq!(json["launch_command"], expected_command);
            assert!(expected_command.starts_with('\''));
            assert!(expected_command.contains("'\\''"));
            assert_eq!(json["url"], "/proxy/");
            assert_eq!(json["launch_wait_timeout_secs"], 15);
            let env = json["launch_env_vars"].as_object().unwrap();
            assert_eq!(env.len(), 4);
            assert_eq!(env["MIAO_EMBEDDED"], "ivnc");
            assert_eq!(env["MIAO_PORT"], state.proxy_panel.port().to_string());
            assert_eq!(env["MIAO_RUNTIME_DIR"], format!("{cwd}/runtime"));
            assert!(env["HOME"].as_str().unwrap().ends_with("/home"));
        }
    }

    #[test]
    fn miao_autostart_cli_overrides_saved_preferences_without_writing_them() {
        for saved in [true, false] {
            for override_value in [None, Some(true), Some(false)] {
                let state = state(None, override_value);
                let mut app = state.store.get(BUILTIN_MIAO_ID).unwrap();
                app.autostart = saved;
                state.store.update(&app).unwrap();
                let json = app_json(&state, &app, "stopped", None, 0);
                assert_eq!(json["autostart"], override_value.unwrap_or(saved));
                assert_eq!(state.store.get(BUILTIN_MIAO_ID).unwrap().autostart, saved);
            }
        }
    }

    #[tokio::test]
    async fn deleting_a_desktop_app_removes_its_runtime() {
        let state = state(None, None);
        let app = ManagedApp::for_test(&uuid::Uuid::new_v4().to_string(), AppType::DesktopApp);
        state.store.add(&app).unwrap();
        assert_eq!(
            app_json(&state, &app, "stopped", None, 0)["sharing"],
            json!({"enabled": false, "restart_required": false})
        );
        let runtime = state.process.runtime_dir(&app.id);
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::write(runtime.join("failure.json"), "{}").unwrap();

        let response = delete_app(State(state.clone()), Path(app.id.clone())).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!runtime.exists());
        assert!(state.store.get_optional(&app.id).unwrap().is_none());
    }

    #[tokio::test]
    async fn remote_apps_are_started_and_stopped_on_this_instance() {
        let state = state(None, None);
        let app = ManagedApp {
            name: "Remote".into(),
            ..ManagedApp::for_test(&uuid::Uuid::new_v4().to_string(), AppType::RemoteApp)
        };
        state.store.add(&app).unwrap();
        state
            .remote
            .save_target(
                &app.id,
                &crate::remote::store::Target {
                    base_url: "http://192.168.1.2:8008".into(),
                    app_id: "shared".into(),
                    node_id: "node".into(),
                    name: "Remote".into(),
                    password: "password-one".into(),
                },
            )
            .unwrap();
        let id = || Path(app.id.clone());
        let json = || app_json(&state, &app, "stopped", None, 0);
        assert_eq!(json()["started"], false);
        assert_eq!(json()["remote_state"], "disconnected");
        assert_eq!(json()["remote_plaintext"], true);

        for _ in 0..2 {
            // Starting again is not an error.
            assert_eq!(
                start_app(State(state.clone()), id()).await.status(),
                StatusCode::OK
            );
        }
        assert_eq!(state.app_status(&app), AppStatus::Running);
        assert_eq!(json()["started"], true);
        assert_eq!(json()["remote_state"], "connecting");
        // Started remote applications are restored with running applications.
        assert_eq!(state.running_app_ids().unwrap(), vec![app.id.clone()]);
        let stream_apps = || state.stream_apps(&state.store.list().unwrap());
        assert_eq!(
            stream_apps(),
            [crate::web::shared::StreamApp {
                id: app.id.clone(),
                name: "Remote".into(),
                name_revision: 0,
                can_rename: true,
                kind: "remote",
                status: "connecting",
            }]
        );

        assert_eq!(
            stop_app(State(state.clone()), id()).await.status(),
            StatusCode::OK
        );
        assert_eq!(state.app_status(&app), AppStatus::Stopped);
        assert_eq!(json()["remote_state"], "disconnected");
        assert!(state.running_app_ids().unwrap().is_empty());
        assert!(stream_apps().is_empty());

        // Remote applications can start with this instance.
        let response = update_app(
            State(state.clone()),
            id(),
            axum::Json(json!({"autostart": true})),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(state.store.get(&app.id).unwrap().autostart);
        state.start_autostart_apps().await.unwrap();
        assert!(state.remote_started(&app.id));
    }

    #[tokio::test]
    async fn errors_keep_their_english_text_and_carry_codes() {
        let state = state(None, None);
        let body = |response: Response| async move {
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            (
                status,
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            )
        };
        let add = |body| add_app(State(state.clone()), axum::Json(body));
        let cli = json!({"name": "Tool", "app_type": "cli", "cli_binary_path": "/bin/true"});
        assert_eq!(add(cli.clone()).await.status(), StatusCode::CREATED);
        let (status, error) = body(add(cli).await).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            error,
            json!({
                "error": "App name 'Tool' already exists",
                "code": "app_name_exists",
                "params": {"name": "Tool"},
            })
        );
        let (status, error) = body(add(json!({"app_type": "cli"})).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            error,
            json!({"error": "missing name", "code": "app_name_required"})
        );
        let (_, error) = body(add(json!({"name": "x".repeat(257)})).await).await;
        assert_eq!(error["code"], "app_name_too_long");

        let tool = state
            .store
            .list()
            .unwrap()
            .into_iter()
            .find(|a| a.name == "Tool");
        let (status, error) =
            body(start_app(State(state.clone()), Path(tool.unwrap().id)).await).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error["code"], "cli_not_startable");
        let (status, error) =
            body(stop_app(State(state.clone()), Path("missing".into())).await).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(error["code"], "app_not_found");
    }

    #[test]
    fn tabs_reread_applications_only_after_a_change() {
        let state = state(None, None);
        let listed = state.publish_stream_apps(OrderedApps::default());
        assert_eq!(listed.apps.len(), 1);
        state
            .store
            .add(&ManagedApp::for_test("added", AppType::DesktopApp))
            .unwrap();
        // The periodic pass keeps the list it has.
        let listed = state.publish_stream_apps(listed);
        assert_eq!(listed.apps.len(), 1);
        state.refresh_stream_apps();
        assert_eq!(state.publish_stream_apps(listed).apps.len(), 2);
    }

    #[tokio::test]
    async fn rename_changes_only_the_name_and_respects_read_only_apps() {
        let state = state(None, None);
        ensure_builtin_chrome(&state.store, "test-chrome").unwrap();
        let mut app = ManagedApp::for_test("rename-desktop", AppType::DesktopApp);
        app.name = "Rename desktop".into();
        app.env_vars = Some(HashMap::from([("KEEP".into(), "value".into())]));
        app.autostart = true;
        state.store.add(&app).unwrap();
        let before = state.store.get(&app.id).unwrap();
        let order = state.store.order().unwrap();
        let rename = |id: String, name: &str, revision| {
            rename_app(
                State(state.clone()),
                Path(id),
                Ok(axum::Json(RenameRequest {
                    name: name.to_owned(),
                    base_revision: revision,
                })),
            )
        };
        assert_eq!(
            rename(app.id.clone(), "Renamed desktop", 0).await.status(),
            StatusCode::OK
        );
        let mut expected = serde_json::to_value(before).unwrap();
        expected["name"] = json!("Renamed desktop");
        expected["name_revision"] = json!(1);
        assert_eq!(
            serde_json::to_value(state.store.get(&app.id).unwrap()).unwrap(),
            expected
        );
        assert_eq!(state.store.order().unwrap(), order);
        assert_eq!(
            rename(app.id.clone(), "Stale", 0).await.status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            rename(app.id.clone(), "Chrome", 1).await.status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            rename(app.id.clone(), "x\ny", 1).await.status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            rename("missing".into(), "valid", 0).await.status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            rename(BUILTIN_MIAO_ID.into(), "Changed Miao", 0)
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        let chrome = state
            .store
            .list()
            .unwrap()
            .into_iter()
            .find(|a| a.name == "Chrome")
            .unwrap();
        assert_eq!(
            rename(chrome.id, "Browser alias", 0).await.status(),
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn remote_configuration_uses_name_versions_and_cannot_restore_a_stale_alias() {
        let state = state(None, None);
        let mut app = ManagedApp::for_test("rename-remote", AppType::RemoteApp);
        app.name = "Remote alias".into();
        state.store.add(&app).unwrap();
        state.store.rename(&app.id, "Toolbar alias", 0).unwrap();
        let update =
            |body| update_app(State(state.clone()), Path(app.id.clone()), axum::Json(body));
        assert_eq!(
            update(json!({"autostart": true})).await.status(),
            StatusCode::OK
        );
        assert_eq!(state.store.get(&app.id).unwrap().name, "Toolbar alias");
        assert_eq!(
            update(json!({"name": "Stale alias", "name_revision": 0, "autostart": false}))
                .await
                .status(),
            StatusCode::CONFLICT
        );
        assert!(state.store.get(&app.id).unwrap().autostart);
        assert_eq!(
            update(json!({"name": "Current alias", "name_revision": 1}))
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(state.store.get(&app.id).unwrap().name_revision, 2);
        // Legacy clients without revisions still pass through the same validator.
        assert_eq!(
            update(json!({"name": "bad\nname"})).await.status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(state.store.get(&app.id).unwrap().name, "Current alias");
    }
}
