use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, Sender, Receiver};
use std::thread;
use log::info;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::unix::EventLoopBuilderExtUnix;
use super::webview::WebViewInstance;
use super::app::{PakeApp, AppStatus};

/// Shared state for webview windows (for compositor integration)
pub struct WebViewWindowInfo {
    pub app_id: String,
    pub app_name: String,
    pub is_focused: bool,
}

lazy_static::lazy_static! {
    /// Global registry of webview windows for compositor integration
    pub static ref WEBVIEW_WINDOWS: Arc<Mutex<HashMap<String, WebViewWindowInfo>>> =
        Arc::new(Mutex::new(HashMap::new()));
}

enum WebViewCommand {
    Start(PakeApp, Sender<Result<(), String>>),
    Stop(String, Sender<Result<(), String>>),
    Status(String, Sender<AppStatus>),
}

/// Manager for WebView instances
/// Uses a dedicated tao/GTK event loop thread
pub struct WebViewManager {
    command_tx: Sender<WebViewCommand>,
    instances: Arc<Mutex<HashMap<String, Arc<Mutex<bool>>>>>, // app_id -> is_open flag
}

impl WebViewManager {
    pub fn new() -> Self {
        info!("Initializing WebViewManager");

        let (command_tx, command_rx) = channel();
        let instances = Arc::new(Mutex::new(HashMap::new()));
        let instances_clone = instances.clone();

        // Start tao event loop thread
        thread::spawn(move || {
            tao_thread_main(command_rx, instances_clone);
        });

        Self {
            command_tx,
            instances,
        }
    }

    /// Start a WebView for the given app
    pub fn start(&mut self, app: &PakeApp) -> Result<(), String> {
        info!("Starting WebView for app: {} ({})", app.name, app.id);

        // Check if already running
        {
            let instances = self.instances.lock().unwrap();
            if instances.contains_key(&app.id) {
                return Err(format!("WebView already running for app: {}", app.id));
            }
        }

        let (result_tx, result_rx) = channel();
        self.command_tx
            .send(WebViewCommand::Start(app.clone(), result_tx))
            .map_err(|e| format!("Failed to send start command: {}", e))?;

        result_rx
            .recv()
            .map_err(|e| format!("Failed to receive start result: {}", e))?
    }

    /// Stop a WebView
    pub fn stop(&self, app_id: &str) -> Result<(), String> {
        info!("Stopping WebView for app: {}", app_id);

        let (result_tx, result_rx) = channel();
        self.command_tx
            .send(WebViewCommand::Stop(app_id.to_string(), result_tx))
            .map_err(|e| format!("Failed to send stop command: {}", e))?;

        result_rx
            .recv()
            .map_err(|e| format!("Failed to receive stop result: {}", e))?
    }

    /// Restart a WebView
    pub fn restart(&mut self, app: &PakeApp) -> Result<(), String> {
        info!("Restarting WebView for app: {}", app.id);
        let _ = self.stop(&app.id); // Ignore error if not running
        self.start(app)
    }

    /// Get status of a WebView
    pub fn status(&self, app_id: &str) -> AppStatus {
        let (result_tx, result_rx) = channel();
        if self.command_tx
            .send(WebViewCommand::Status(app_id.to_string(), result_tx))
            .is_err()
        {
            return AppStatus::Stopped;
        }

        result_rx.recv().unwrap_or(AppStatus::Stopped)
    }

    /// Get PID (not applicable for WebView, returns None)
    pub fn pid(&self, _app_id: &str) -> Option<u32> {
        None // WebView runs in-process
    }

    /// Stop all WebViews
    pub fn stop_all(&self) {
        info!("Stopping all WebViews");
        let app_ids: Vec<String> = self.instances.lock().unwrap().keys().cloned().collect();
        for app_id in app_ids {
            let _ = self.stop(&app_id);
        }
    }
}

impl Drop for WebViewManager {
    fn drop(&mut self) {
        info!("Dropping WebViewManager");
        self.stop_all();
    }
}

/// Tao event loop thread main function
fn tao_thread_main(
    command_rx: Receiver<WebViewCommand>,
    instances: Arc<Mutex<HashMap<String, Arc<Mutex<bool>>>>>,
) {
    info!("Tao event loop thread starting");

    // Create event loop with any_thread support for non-main thread
    let event_loop = EventLoopBuilder::new()
        .with_any_thread(true)
        .build();

    // Store WebView instances (need to keep them alive)
    let mut webviews: HashMap<String, WebViewInstance> = HashMap::new();
    let mut window_to_app: HashMap<tao::window::WindowId, String> = HashMap::new();

    event_loop.run(move |event, event_loop, control_flow| {
        *control_flow = ControlFlow::Poll;

        // Check for commands (non-blocking)
        if let Ok(cmd) = command_rx.try_recv() {
            match cmd {
                WebViewCommand::Start(app, result_tx) => {
                    let result = WebViewInstance::new(&app, event_loop);
                    match result {
                        Ok(instance) => {
                            let is_open = instance.is_open.clone();
                            let app_id = app.id.clone();
                            let app_name = app.name.clone();
                            let window_id = instance.window_id;

                            instances.lock().unwrap().insert(app_id.clone(), is_open);
                            window_to_app.insert(window_id, app_id.clone());
                            webviews.insert(app_id.clone(), instance);

                            // Register in global webview windows for compositor
                            WEBVIEW_WINDOWS.lock().unwrap().insert(app_id.clone(), WebViewWindowInfo {
                                app_id: app_id.clone(),
                                app_name,
                                is_focused: false,
                            });

                            let _ = result_tx.send(Ok(()));
                        }
                        Err(e) => {
                            let _ = result_tx.send(Err(e));
                        }
                    }
                }
                WebViewCommand::Stop(app_id, result_tx) => {
                    if let Some(instance) = webviews.remove(&app_id) {
                        instance.mark_closed();
                        window_to_app.remove(&instance.window_id);
                        instances.lock().unwrap().remove(&app_id);

                        // Unregister from global webview windows
                        WEBVIEW_WINDOWS.lock().unwrap().remove(&app_id);

                        let _ = result_tx.send(Ok(()));
                    } else {
                        let _ = result_tx.send(Err(format!("WebView not found: {}", app_id)));
                    }
                }
                WebViewCommand::Status(app_id, result_tx) => {
                    let instances_lock = instances.lock().unwrap();
                    let status = if let Some(is_open) = instances_lock.get(&app_id) {
                        if *is_open.lock().unwrap() {
                            AppStatus::Running
                        } else {
                            AppStatus::Crashed
                        }
                    } else {
                        AppStatus::Stopped
                    };
                    let _ = result_tx.send(status);
                }
            }
        }

        // Handle window events
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                window_id,
                ..
            } => {
                // Find and mark the corresponding instance as closed
                if let Some(app_id) = window_to_app.get(&window_id) {
                    info!("Window close requested for app: {}", app_id);
                    let app_id = app_id.clone();
                    if let Some(instance) = webviews.remove(&app_id) {
                        instance.mark_closed();
                        window_to_app.remove(&window_id);
                        instances.lock().unwrap().remove(&app_id);
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::Destroyed,
                window_id,
                ..
            } => {
                // Clean up when window is destroyed
                if let Some(app_id) = window_to_app.remove(&window_id) {
                    info!("Window destroyed for app: {}", app_id);
                    webviews.remove(&app_id);
                    instances.lock().unwrap().remove(&app_id);
                }
            }
            _ => {}
        }
    });
}
