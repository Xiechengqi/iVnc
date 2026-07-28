use super::app::{AppStatus, AppType, ManagedApp};
use super::datadir;
use super::desktop_entry;
use super::native;
use super::process::ProcessManager;
use super::service_process::{self, ServiceProcessManager};
use super::state_recovery::AppRunningState;
use super::store::AppStore;
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::Response,
    routing::{get, post},
    Router,
};
use serde_json::json;
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;

pub struct AppsState {
    pub store: Arc<AppStore>,
    pub process: ProcessManager,
    pub service: ServiceProcessManager,
}

impl AppsState {
    pub fn new() -> Result<Self, String> {
        let store = Arc::new(AppStore::new()?);
        ensure_builtin_apps(&store)?;
        let mut process = ProcessManager::new();
        process.set_store(store.clone());
        let mut service = ServiceProcessManager::new();
        service.set_store(store.clone());
        Ok(Self {
            store,
            process,
            service,
        })
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

    pub fn shutdown_running_apps_with_state(&self) -> Result<(), String> {
        let apps = self.store.list()?;
        let running_ids: Vec<String> = apps
            .iter()
            .filter(|app| matches!(self.app_status(app), AppStatus::Running))
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
                    if let Err(err) = self.stop_app_processes(&app) {
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
            .filter(|app| matches!(self.app_status(app), AppStatus::Running))
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
        for app in apps.iter().filter(|app| app.autostart) {
            let result = self.start_app_processes(app).await.map(|_| ());

            match result {
                Ok(()) => log::info!("Autostart app started: {} ({})", app.name, app.id),
                Err(e) if e == "App is already running" => {
                    log::debug!("Autostart app already running: {} ({})", app.name, app.id)
                }
                Err(e) => log::warn!("Failed to autostart app {} ({}): {}", app.name, app.id, e),
            }
        }
        Ok(())
    }

    async fn start_app_processes(&self, app: &ManagedApp) -> Result<Option<u32>, String> {
        match app.app_type {
            AppType::DesktopApp => Ok(Some(self.process.start(app)?)),
            AppType::BackgroundApp => {
                if !service_process::has_launch_command(app) {
                    return Err("background app requires launch_command".to_string());
                }
                self.service.start_and_wait(app).await
            }
            AppType::CliApp => Err("CLI apps are one-shot tools and cannot be started".to_string()),
        }
    }

    fn stop_app_processes(&self, app: &ManagedApp) -> Result<(), String> {
        match app.app_type {
            AppType::DesktopApp => self.process.stop(&app.id),
            AppType::BackgroundApp => self.service.stop(&app.id),
            AppType::CliApp => Ok(()),
        }
    }

    async fn restart_app_processes(&self, app: &ManagedApp) -> Result<Option<u32>, String> {
        let _ = self.stop_app_processes(app);
        self.start_app_processes(app).await
    }

    pub fn app_status(&self, app: &ManagedApp) -> AppStatus {
        match app.app_type {
            AppType::DesktopApp => self.process.status(&app.id),
            AppType::BackgroundApp => self.service.status(&app.id),
            AppType::CliApp => AppStatus::Stopped,
        }
    }

    fn app_pid(&self, app: &ManagedApp) -> Option<u32> {
        match app.app_type {
            AppType::DesktopApp => self.process.pid(&app.id),
            AppType::BackgroundApp => self.service.pid(&app.id),
            AppType::CliApp => None,
        }
    }

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
    match store.get("builtin-chrome") {
        Ok(mut app) => {
            if app.app_type != AppType::DesktopApp
                || app.exec_command.as_deref() != Some(chrome_command.as_str())
                || app.autostart
            {
                app.app_type = AppType::DesktopApp;
                app.autostart = false;
                app.url = None;
                app.launch_command = None;
                app.launch_env_vars = None;
                app.launch_cwd = None;
                app.launch_wait_timeout_secs = None;
                app.exec_command = Some(chrome_command.clone());
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
                exec_command: Some(chrome_command.clone()),
                env_vars: None,
                cli_binary_path: None,
                cli_env_vars: None,
                skill_paths: None,
                created_at: chrono_now(),
            };
            match store.add(&app) {
                Ok(()) => log::info!("Added built-in Chrome desktop app"),
                Err(err) if err.contains("already exists") => {}
                Err(err) => return Err(err),
            }
        }
    }
    desktop_entry::ensure_desktop_entry("Chrome", &chrome_command)?;
    ensure_builtin_agent_browser_cli(store)?;
    Ok(())
}

fn ensure_builtin_agent_browser_cli(store: &Arc<AppStore>) -> Result<(), String> {
    let skill_path = "/root/.config/ivnc/skills/agent-browser/SKILL.md".to_string();
    if let Err(err) = ensure_agent_browser_skill(&skill_path) {
        log::warn!("Failed to ensure default agent-browser skill: {}", err);
    }
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
            if app.skill_paths.as_ref().map_or(true, Vec::is_empty) {
                app.skill_paths = Some(vec![skill_path]);
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
                skill_paths: Some(vec![skill_path]),
                created_at: chrono_now(),
            };
            match store.add(&app) {
                Ok(()) => log::info!("Added built-in agent-browser CLI app"),
                Err(err) if err.contains("already exists") => {}
                Err(err) => return Err(err),
            }
        }
    }
    Ok(())
}

fn ensure_agent_browser_skill(path: &str) -> Result<(), String> {
    let path = PathBuf::from(path);
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create skill directory {}: {}",
                parent.display(),
                e
            )
        })?;
    }
    let content = r#"Use the registered CLI app `agent-browser` through `cli_app_run`.

Always prefer JSON output when available. Do not parse human-readable output if a `--json` flag can be used.

Common commands:
- Inspect browser state: `cli_app_run(app="agent-browser", args=["snapshot", "--json"])`
- Open a page: `cli_app_run(app="agent-browser", args=["open", "https://example.com", "--json"])`
- Click an accessibility ref from a snapshot: `cli_app_run(app="agent-browser", args=["click", "@e3", "--json"])`
- Fill text into an accessibility ref: `cli_app_run(app="agent-browser", args=["fill", "@e3", "text", "--json"])`
- Press a key: `cli_app_run(app="agent-browser", args=["press", "Enter", "--json"])`

After every mutating command, inspect stdout/stderr and verify browser state with another snapshot before calling done.
"#;
    std::fs::write(&path, content)
        .map_err(|e| format!("Failed to write default agent-browser skill: {}", e))
}

fn ivnc_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/root/.config"))
        .join("ivnc")
}

fn builtin_chrome_data_dir() -> Result<PathBuf, String> {
    let dir = ivnc_config_dir().join("chrome");
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
    let data_dir = shell_quote(&data_dir.to_string_lossy());
    let debug_port = chrome_devtools_port();
    Ok(format!(
        "profile_dir={data_dir}; \
rm -f \"$profile_dir/SingletonLock\" \"$profile_dir/SingletonSocket\" \"$profile_dir/SingletonCookie\"; \
proxy_arg=''; \
if [ \"${{IVNC_PROXY_PANEL_ENABLED:-1}}\" != '0' ] && grep -qiE '^[[:space:]]*[0-9]+:[[:space:]]+[0-9A-Fa-f]+:0438[[:space:]]+[0-9A-Fa-f]+:[0-9A-Fa-f]+[[:space:]]+0A[[:space:]]' /proc/net/tcp /proc/net/tcp6 2>/dev/null; then \
  proxy_arg='--proxy-server=socks5://127.0.0.1:1080'; \
fi; \
exec google-chrome ${{proxy_arg:+$proxy_arg}} --user-data-dir=\"$profile_dir\" --remote-debugging-host=0.0.0.0 --remote-debugging-port={debug_port} --ozone-platform=wayland --class=ivnc-chrome-windowed --test-type --no-first-run --no-default-browser-check --disable-features=MediaRouter --disable-background-networking --disable-process-singleton --no-sandbox --disable-setuid-sandbox --disable-dev-shm-usage",
    ))
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

fn json_response(status: StatusCode, body: serde_json::Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn err_response(status: StatusCode, msg: &str) -> Response {
    json_response(status, json!({"error": msg}))
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

fn parse_string_list(value: Option<&serde_json::Value>) -> Option<Vec<String>> {
    let values = value.and_then(|v| v.as_array()).map(|items| {
        items
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    })?;
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
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
    app: &ManagedApp,
    status: &str,
    pid: Option<u32>,
    data_bytes: u64,
) -> serde_json::Value {
    let mut obj = json!({
        "id": app.id,
        "name": app.name,
        "app_type": app.app_type.as_str(),
        "autostart": app.autostart,
        "status": status,
        "pid": pid,
        "data_size_bytes": data_bytes,
        "data_size_human": datadir::size_human(data_bytes),
        "created_at": app.created_at,
        "skill_paths": app.skill_paths,
    });

    match app.app_type {
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
    match state.store.list() {
        Ok(apps) => {
            let items: Vec<_> = apps
                .iter()
                .map(|app| {
                    let st = state.app_status(app);
                    let pid = state.app_pid(app);
                    let size = datadir::dir_size(&datadir::data_dir(app));
                    app_json(app, &format!("{:?}", st).to_lowercase(), pid, size)
                })
                .collect();
            json_response(StatusCode::OK, json!({"apps": items}))
        }
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn add_app(
    State(state): State<Arc<AppsState>>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Response {
    let name = match body.get("name").and_then(|v| v.as_str()) {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => return err_response(StatusCode::BAD_REQUEST, "missing name"),
    };

    let app_type_str = body
        .get("app_type")
        .and_then(|v| v.as_str())
        .unwrap_or("background");
    let app_type = AppType::from_str(app_type_str).unwrap_or(AppType::BackgroundApp);

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
        skill_paths,
    ) = match app_type {
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
                        "missing launch_command for background",
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
                parse_string_list(body.get("skill_paths")),
            )
        }
        AppType::DesktopApp => {
            let exec_command = match body.get("exec_command").and_then(|v| v.as_str()) {
                Some(e) if !e.trim().is_empty() => Some(e.trim().to_string()),
                _ => {
                    return err_response(
                        StatusCode::BAD_REQUEST,
                        "missing exec_command for desktop app",
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
                parse_string_list(body.get("skill_paths")),
            )
        }
        AppType::CliApp => {
            let cli_binary_path = match body.get("cli_binary_path").and_then(|v| v.as_str()) {
                Some(path) if !path.trim().is_empty() => Some(path.trim().to_string()),
                _ => return err_response(StatusCode::BAD_REQUEST, "missing cli_binary_path"),
            };
            let cli_env_vars = parse_env_vars(body.get("cli_env_vars"));
            let skill_paths = parse_string_list(body.get("skill_paths"));
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
                skill_paths,
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
        skill_paths,
        created_at: chrono_now(),
    };

    if let Err(e) = state.store.add(&app) {
        return err_response(StatusCode::CONFLICT, &e);
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
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, &e);
        }
    }

    let status = state.app_status(&app);
    let pid = state.app_pid(&app);
    let size = datadir::dir_size(&datadir::data_dir(&app));
    json_response(
        StatusCode::CREATED,
        json!({"ok": true, "app": app_json(&app, &format!("{:?}", status).to_lowercase(), pid, size)}),
    )
}

async fn get_app(State(state): State<Arc<AppsState>>, Path(id): Path<String>) -> Response {
    match state.store.get(&id) {
        Ok(app) => {
            let st = state.app_status(&app);
            let pid = state.app_pid(&app);
            let size = datadir::dir_size(&datadir::data_dir(&app));
            json_response(
                StatusCode::OK,
                app_json(&app, &format!("{:?}", st).to_lowercase(), pid, size),
            )
        }
        Err(e) => err_response(StatusCode::NOT_FOUND, &e),
    }
}

async fn update_app(
    State(state): State<Arc<AppsState>>,
    Path(id): Path<String>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Response {
    let mut app = match state.store.get(&id) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::NOT_FOUND, &e),
    };

    match app.app_type {
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
                    "missing launch_command for background",
                );
            }
        }
        AppType::DesktopApp => {
            if let Some(exec) = body.get("exec_command").and_then(|v| v.as_str()) {
                let exec = exec.trim();
                if exec.is_empty() {
                    return err_response(
                        StatusCode::BAD_REQUEST,
                        "missing exec_command for desktop app",
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
                    return err_response(StatusCode::BAD_REQUEST, "missing cli_binary_path");
                }
                app.cli_binary_path = Some(path.to_string());
            }
            app.cli_env_vars = parse_env_vars(body.get("cli_env_vars"));
        }
    }
    if body.get("skill_paths").is_some() {
        app.skill_paths = parse_string_list(body.get("skill_paths"));
    }

    if let Some(autostart) = body.get("autostart").and_then(|v| v.as_bool()) {
        app.autostart = app.app_type != AppType::CliApp && autostart;
    }

    if let Err(e) = state.store.update(&app) {
        return err_response(StatusCode::INTERNAL_SERVER_ERROR, &e);
    }
    json_response(StatusCode::OK, json!({"ok": true}))
}

async fn delete_app(State(state): State<Arc<AppsState>>, Path(id): Path<String>) -> Response {
    // Clean up data directory before removing from store
    if let Ok(app) = state.store.get(&id) {
        let _ = state.stop_app_processes(&app);
        let _ = std::fs::remove_dir_all(datadir::app_root(&app));
    }
    match state.store.delete(&id) {
        Ok(()) => json_response(StatusCode::OK, json!({"ok": true})),
        Err(e) => err_response(StatusCode::NOT_FOUND, &e),
    }
}

async fn start_app(State(state): State<Arc<AppsState>>, Path(id): Path<String>) -> Response {
    let app = match state.store.get(&id) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::NOT_FOUND, &e),
    };

    match state.start_app_processes(&app).await {
        Ok(pid) => json_response(StatusCode::OK, json!({"ok": true, "pid": pid})),
        Err(e) => {
            let _ = state.stop_app_processes(&app);
            err_response(StatusCode::INTERNAL_SERVER_ERROR, &e)
        }
    }
}

async fn stop_app(State(state): State<Arc<AppsState>>, Path(id): Path<String>) -> Response {
    let app = match state.store.get(&id) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::NOT_FOUND, &e),
    };

    match state.stop_app_processes(&app) {
        Ok(()) => json_response(StatusCode::OK, json!({"ok": true})),
        Err(e) => err_response(StatusCode::BAD_REQUEST, &e),
    }
}

async fn restart_app(State(state): State<Arc<AppsState>>, Path(id): Path<String>) -> Response {
    let app = match state.store.get(&id) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::NOT_FOUND, &e),
    };

    match state.restart_app_processes(&app).await {
        Ok(pid) => json_response(StatusCode::OK, json!({"ok": true, "pid": pid})),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn data_size(State(state): State<Arc<AppsState>>, Path(id): Path<String>) -> Response {
    let app = match state.store.get(&id) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::NOT_FOUND, &e),
    };
    let dir = datadir::data_dir(&app);
    let bytes = datadir::dir_size(&dir);
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
    let app = match state.store.get(&id) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::NOT_FOUND, &e),
    };
    // Stop app before clearing data
    let _ = state.stop_app_processes(&app);
    match datadir::clear(&app) {
        Ok(()) => json_response(StatusCode::OK, json!({"ok": true})),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn get_logs(State(state): State<Arc<AppsState>>, Path(id): Path<String>) -> Response {
    // Verify app exists
    if let Err(e) = state.store.get(&id) {
        return err_response(StatusCode::NOT_FOUND, &e);
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
