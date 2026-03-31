use super::app::{AppMode, AppType, PakeApp};
use super::datadir;
use log::info;
use std::fs;
use std::process::{Command, Stdio};

/// Detect Chrome/Chromium binary path on Linux
pub fn find_chrome() -> Option<String> {
    let candidates = [
        "google-chrome",
        "google-chrome-stable",
        "chromium-browser",
        "chromium",
    ];
    for name in &candidates {
        if let Ok(output) = Command::new("which").arg(name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// Log file path for a pake app
pub fn log_path(app_id: &str) -> std::path::PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/root/.config"))
        .join("ivnc")
        .join("pake-apps")
        .join(app_id);
    let _ = fs::create_dir_all(&dir);
    dir.join("app.log")
}

/// Build the launch command for a Pake app
pub fn build_command(app: &PakeApp) -> Result<Command, String> {
    match app.app_type {
        AppType::DesktopApp => build_desktop_command(app),
        AppType::WebApp => match app.mode {
            Some(AppMode::Native) => build_native_command(app),
            Some(AppMode::Webview) => build_webview_command(app),
            None => Err("WebApp must have a mode".to_string()),
        },
    }
}

/// Allocate a free TCP port for CDP remote debugging
fn alloc_debug_port() -> u16 {
    use std::net::TcpListener;
    TcpListener::bind("127.0.0.1:0")
        .map(|l| l.local_addr().unwrap().port())
        .unwrap_or(9222)
}

fn build_native_command(app: &PakeApp) -> Result<Command, String> {
    let chrome = find_chrome().ok_or("Chrome/Chromium not found")?;
    let data = datadir::ensure_data_dir(app)?;

    let url = app.url.as_ref().ok_or("WebApp must have url")?;

    // Log environment info
    let wayland = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "(not set)".into());
    let xdg_runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "(not set)".into());
    let display = std::env::var("DISPLAY").unwrap_or_else(|_| "(not set)".into());

    info!("Pake app '{}' launch info:", app.name);
    info!("  Chrome: {}", chrome);
    info!("  URL: {}", url);
    info!("  Data dir: {}", data.display());
    info!("  Show nav: {}", app.show_nav);
    info!("  WAYLAND_DISPLAY={}", wayland);
    info!("  XDG_RUNTIME_DIR={}", xdg_runtime);
    info!("  DISPLAY={}", display);

    let mut cmd = Command::new(&chrome);

    // Choose between app mode (no nav bar) and normal window mode (with nav bar)
    if app.show_nav {
        // Normal browser mode with full UI (address bar, toolbar, etc.)
        // Don't use --app or --new-window, just pass the URL
        cmd.arg(url);
        // Use a special WM_CLASS to identify windows that should not be fullscreened
        cmd.arg("--class=ivnc-pake-windowed");
        info!("  -> Using browser mode (with full navigation UI)");
    } else {
        // App mode without navigation bar
        cmd.arg(format!("--app={}", url));
        cmd.arg("--class=ivnc-pake-app");
        info!("  -> Using app mode (no navigation bar)");
    }

    cmd.arg(format!("--user-data-dir={}", data.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-features=MediaRouter")
        .arg("--disable-background-networking")
        .arg("--disable-process-singleton");

    // Add proxy server if configured (default: socks5://127.0.0.1:1080)
    let proxy = app
        .proxy_server
        .as_deref()
        .unwrap_or("socks5://127.0.0.1:1080");
    cmd.arg(format!("--proxy-server={}", proxy));
    info!("  Proxy: {}", proxy);

    // Add CDP debugging based on configuration
    let debug_port = if let Some(port) = app.remote_debugging_port {
        info!("  CDP debug port: {} (configured)", port);
        cmd.arg(format!("--remote-debugging-port={}", port));
        Some(port)
    } else if !app.show_nav {
        let port = alloc_debug_port();
        info!("  CDP debug port: {} (auto-allocated)", port);
        cmd.arg(format!("--remote-debugging-port={}", port));
        Some(port)
    } else {
        info!("  CDP debugging disabled");
        None
    };

    // Clear LD_PRELOAD to avoid issues with missing libraries (e.g. libgtk3-nocsd.so.0)
    cmd.env_remove("LD_PRELOAD");

    // Disable D-Bus to avoid error logs in headless environment
    cmd.env("DBUS_SYSTEM_BUS_ADDRESS", "disabled:");
    cmd.env("DBUS_SESSION_BUS_ADDRESS", "disabled:");

    // Ensure Wayland env vars are passed
    if let Ok(val) = std::env::var("WAYLAND_DISPLAY") {
        cmd.env("WAYLAND_DISPLAY", &val);
        cmd.arg("--ozone-platform=wayland");
        info!("  -> Using Wayland (ozone-platform=wayland)");
    }
    if let Ok(val) = std::env::var("XDG_RUNTIME_DIR") {
        cmd.env("XDG_RUNTIME_DIR", &val);
    }
    if let Ok(val) = std::env::var("DISPLAY") {
        cmd.env("DISPLAY", &val);
    }

    // Root needs extra sandbox-disabling flags
    if unsafe { libc::getuid() } == 0 {
        cmd.arg("--no-sandbox");
        cmd.arg("--disable-setuid-sandbox");
        cmd.arg("--disable-gpu-sandbox");
        cmd.arg("--disable-software-rasterizer");
        cmd.arg("--disable-dev-shm-usage");
        info!("  -> Running as root, added sandbox-disabling flags");
    }

    // Redirect stdout/stderr to log file
    let log_file = log_path(&app.id);
    info!("  Log file: {}", log_file.display());
    let stdout_file =
        fs::File::create(&log_file).map_err(|e| format!("Failed to create log file: {}", e))?;
    let stderr_file = stdout_file
        .try_clone()
        .map_err(|e| format!("Failed to clone log file: {}", e))?;
    cmd.stdout(Stdio::from(stdout_file));
    cmd.stderr(Stdio::from(stderr_file));

    // Spawn CDP injector in background only if in app mode
    if let Some(port) = debug_port {
        tokio::spawn(cdp_inject_nav(port));
    }

    Ok(cmd)
}

fn build_webview_command(app: &PakeApp) -> Result<Command, String> {
    // For webview mode, we also use Chrome in app mode for now.
    build_native_command(app)
}

fn build_desktop_command(app: &PakeApp) -> Result<Command, String> {
    let exec_cmd = app
        .exec_command
        .as_ref()
        .ok_or("DesktopApp must have exec_command")?;

    info!("Desktop app '{}' launch info:", app.name);
    info!("  Command: {}", exec_cmd);

    // Use shell to execute command (supports arguments and pipes)
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(exec_cmd);

    // Set environment variables
    if let Some(env_vars) = &app.env_vars {
        for (key, value) in env_vars {
            cmd.env(key, value);
            info!("  Env: {}={}", key, value);
        }
    }

    // Redirect stdout/stderr to log file
    let log_file = log_path(&app.id);
    info!("  Log file: {}", log_file.display());
    let stdout_file =
        fs::File::create(&log_file).map_err(|e| format!("Failed to create log file: {}", e))?;
    let stderr_file = stdout_file
        .try_clone()
        .map_err(|e| format!("Failed to clone log file: {}", e))?;
    cmd.stdout(Stdio::from(stdout_file));
    cmd.stderr(Stdio::from(stderr_file));

    Ok(cmd)
}

/// CDP-based nav button injection.
/// Waits for Chrome to start, then injects nav buttons into every page via Runtime.evaluate.
async fn cdp_inject_nav(port: u16) {
    use futures::SinkExt;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message;

    let nav_js = include_str!("../../extension/nav.js");
    let nav_css = include_str!("../../extension/nav.css");

    // Inject CSS via JS
    let inject_script = format!(
        r#"
(function() {{
    if (document.getElementById('pake-nav')) return;
    var style = document.createElement('style');
    style.id = 'pake-nav-style';
    style.textContent = {css_json};
    document.head.appendChild(style);
    {nav_js}
}})();
"#,
        css_json = serde_json::to_string(nav_css).unwrap_or_default(),
        nav_js = nav_js,
    );

    // Wait for Chrome to start (up to 15s)
    let cdp_url = format!("http://127.0.0.1:{}/json", port);
    let mut ws_url: Option<String> = None;
    for _ in 0..30 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if let Ok(resp) = reqwest_get(&cdp_url).await {
            if let Ok(tabs) = serde_json::from_str::<serde_json::Value>(&resp) {
                if let Some(tab) = tabs.as_array().and_then(|a| a.first()) {
                    if let Some(ws) = tab.get("webSocketDebuggerUrl").and_then(|v| v.as_str()) {
                        ws_url = Some(ws.to_string());
                        break;
                    }
                }
            }
        }
    }

    let ws_url = match ws_url {
        Some(u) => u,
        None => {
            info!("CDP: Chrome did not start in time on port {}", port);
            return;
        }
    };

    info!("CDP: connecting to {}", ws_url);

    let (mut ws, _) = match connect_async(&ws_url).await {
        Ok(v) => v,
        Err(e) => {
            info!("CDP: WebSocket connect failed: {}", e);
            return;
        }
    };

    // Enable Page events so we get loadEventFired
    let enable_msg = r#"{"id":1,"method":"Page.enable","params":{}}"#;
    let _ = ws.send(Message::Text(enable_msg.to_string().into())).await;

    // Also do an immediate inject in case page is already loaded
    let eval_msg = serde_json::json!({
        "id": 2,
        "method": "Runtime.evaluate",
        "params": { "expression": inject_script }
    });
    let _ = ws.send(Message::Text(eval_msg.to_string().into())).await;

    // Listen for page load events and re-inject
    let mut msg_id = 3u64;
    loop {
        use futures::StreamExt;
        match ws.next().await {
            Some(Ok(Message::Text(text))) => {
                let v: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");
                if method == "Page.loadEventFired" || method == "Page.frameNavigated" {
                    let eval = serde_json::json!({
                        "id": msg_id,
                        "method": "Runtime.evaluate",
                        "params": { "expression": inject_script }
                    });
                    msg_id += 1;
                    let _ = ws.send(Message::Text(eval.to_string().into())).await;
                }
            }
            Some(Err(e)) => {
                info!("CDP: WebSocket error: {}", e);
                break;
            }
            None => break,
            _ => {}
        }
    }
}

/// Simple HTTP GET using tokio (no reqwest dep needed — use std blocking in a spawn_blocking)
async fn reqwest_get(url: &str) -> Result<String, String> {
    let url = url.to_string();
    tokio::task::spawn_blocking(move || {
        // Use curl as a simple HTTP client
        let out = std::process::Command::new("curl")
            .arg("-s")
            .arg("--max-time")
            .arg("1")
            .arg(&url)
            .output()
            .map_err(|e| e.to_string())?;
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
