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
        let apps = self.store.list()?;
        let running_ids: Vec<String> = apps
            .iter()
            .filter(|app| {
                let status = self.app_status(app);
                matches!(status, AppStatus::Running)
            })
            .map(|app| app.id.clone())
            .collect();

        if running_ids.is_empty() {
            log::info!("No running apps to save");
            return Ok(());
        }

        let state = AppRunningState::new(running_ids);
        state.save()
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
        }
    }

    fn stop_app_processes(&self, app: &ManagedApp) -> Result<(), String> {
        match app.app_type {
            AppType::DesktopApp => self.process.stop(&app.id),
            AppType::BackgroundApp => self.service.stop(&app.id),
        }
    }

    async fn restart_app_processes(&self, app: &ManagedApp) -> Result<Option<u32>, String> {
        let _ = self.stop_app_processes(app);
        self.start_app_processes(app).await
    }

    fn app_status(&self, app: &ManagedApp) -> AppStatus {
        match app.app_type {
            AppType::DesktopApp => self.process.status(&app.id),
            AppType::BackgroundApp => self.service.status(&app.id),
        }
    }

    fn app_pid(&self, app: &ManagedApp) -> Option<u32> {
        match app.app_type {
            AppType::DesktopApp => self.process.pid(&app.id),
            AppType::BackgroundApp => self.service.pid(&app.id),
        }
    }
}

fn ensure_builtin_apps(store: &Arc<AppStore>) -> Result<(), String> {
    const CHROME_COMMAND: &str = "google-chrome --ozone-platform=wayland --no-first-run --no-default-browser-check --disable-features=MediaRouter --disable-background-networking --disable-process-singleton --no-sandbox --disable-setuid-sandbox --disable-gpu-sandbox --disable-dev-shm-usage";

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
                || app.exec_command.as_deref() != Some(CHROME_COMMAND)
                || app.autostart
            {
                app.app_type = AppType::DesktopApp;
                app.autostart = false;
                app.url = None;
                app.launch_command = None;
                app.launch_env_vars = None;
                app.launch_cwd = None;
                app.launch_wait_timeout_secs = None;
                app.exec_command = Some(CHROME_COMMAND.to_string());
                app.env_vars = None;
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
                exec_command: Some(CHROME_COMMAND.to_string()),
                env_vars: None,
                created_at: chrono_now(),
            };
            match store.add(&app) {
                Ok(()) => log::info!("Added built-in Chrome desktop app"),
                Err(err) if err.contains("already exists") => {}
                Err(err) => return Err(err),
            }
        }
    }
    desktop_entry::ensure_desktop_entry("Chrome", CHROME_COMMAND)?;
    Ok(())
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
            (None, None, None, None, None, exec_command, env_vars)
        }
    };
    let autostart = body
        .get("autostart")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

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

    json_response(StatusCode::CREATED, json!({"ok": true, "app": app}))
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
    }

    if let Some(autostart) = body.get("autostart").and_then(|v| v.as_bool()) {
        app.autostart = autostart;
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
