//! Shared state for ivnc
//!
//! Manages shared configuration and WebRTC sessions.

use crate::audio::AudioPacket;
use crate::config::ui::UiConfig;
use crate::config::Config;
use crate::input::InputEventData;
use crate::proxy_panel::ProxyPanelManager;
use crate::runtime_settings::RuntimeSettings;
use base64::Engine;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use xxhash_rust::xxh64::xxh64;

/// Connection information for a WebRTC session
#[derive(Debug)]
pub struct ConnectionInfo {
    pub id: String,
    pub peer_ip: String,
    pub connected_at: i64,
    pub connection_type: String,
    pub client_info: Option<Value>,
    pub client_visible: bool,
    pub client_focused: bool,
    pub last_client_input_ms: u64,
    pub last_client_focus_ms: u64,
    pub last_client_hidden_ms: u64,
    pub last_client_heartbeat_ms: u64,
    pub last_client_wakeup_ms: u64,
    pub shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ClientViewerSummary {
    pub has_connections: bool,
    pub has_active_viewer: bool,
    pub all_clients_inactive: bool,
    pub has_recent_client_wakeup: bool,
}

#[derive(Debug, Deserialize)]
struct ClientActivityReport {
    visible: Option<bool>,
    focused: Option<bool>,
    input: Option<bool>,
    #[allow(dead_code)]
    event: Option<String>,
}

/// How long a session may stay unable to accept media before it is disconnected.
const CONGESTION_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// A per-session queue. Brief congestion drops items; sustained congestion ends
/// the session instead of buffering without bound.
pub struct Subscriber<T> {
    tx: mpsc::Sender<T>,
    congested_since: Option<std::time::Instant>,
}
impl<T> Subscriber<T> {
    fn new(tx: mpsc::Sender<T>) -> Self {
        Self {
            tx,
            congested_since: None,
        }
    }
}

/// Delivers an item to every subscriber; returns whether any recovered from congestion.
fn deliver<T: Clone>(subs: &mut Vec<Subscriber<T>>, item: &T) -> bool {
    let mut recovered = false;
    subs.retain_mut(|sub| match sub.tx.try_send(item.clone()) {
        Ok(()) => {
            recovered |= sub.congested_since.take().is_some();
            true
        }
        Err(mpsc::error::TrySendError::Closed(_)) => false,
        Err(mpsc::error::TrySendError::Full(_)) => {
            sub.congested_since
                .get_or_insert_with(std::time::Instant::now)
                .elapsed()
                < CONGESTION_LIMIT
        }
    });
    recovered
}

/// An application streamed in its own taskbar tab rather than as a desktop window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StreamApp {
    pub id: String,
    pub name: String,
    pub name_revision: u64,
    pub can_rename: bool,
    /// "shared": a local application in its own runtime; "remote": an application of another instance.
    pub kind: &'static str,
    pub status: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskbarAppName {
    pub name: String,
    pub name_revision: u64,
    pub can_rename: bool,
}

/// The taskbar message body: desktop windows and application tabs.
pub fn taskbar_json(windows: Vec<Value>, snapshot: &TaskbarApps) -> String {
    json!({ "windows": windows, "apps": snapshot.apps,
        "ordered_ids": snapshot.order.ordered_ids, "order_revision": snapshot.order.order_revision }).to_string()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskbarApps {
    pub apps: Vec<StreamApp>,
    pub order: crate::apps::store::AppOrder,
    pub window_apps: BTreeMap<i32, String>,
    pub names: BTreeMap<String, TaskbarAppName>,
}

/// Shared state for the application
#[derive(Clone)]
pub struct SharedState {
    pub media_router: Arc<crate::remote::media::MediaRouter>,
    pub app_views: Arc<crate::remote::gateway::Views>,

    /// Applications shown as their own taskbar tabs, kept current by the apps
    /// manager so that the compositor never queries it.
    taskbar_apps: Arc<Mutex<TaskbarApps>>,
    window_pids: Arc<Mutex<HashSet<i32>>>,
    /// Bumped whenever the taskbar app/order/identity snapshot changes.
    pub apps_generation: Arc<AtomicU64>,
    /// Configuration
    pub config: Arc<Config>,

    /// Input event sender
    pub input_sender: mpsc::UnboundedSender<InputEventData>,

    /// Display dimensions
    pub display_size: Arc<Mutex<(u32, u32)>>,

    /// Clipboard content (base64 text)
    pub clipboard: Arc<Mutex<Option<String>>>,

    /// Request keyframe flag (for WebRTC)
    pub force_keyframe: Arc<AtomicBool>,

    /// Pending display resize target (width, height); pipeline thread will apply it
    pub pending_resize: Arc<Mutex<Option<(u32, u32)>>>,

    /// Runtime stats
    pub stats: Arc<Mutex<RuntimeStats>>,

    /// Current compositor/encoder render state.
    render_state: Arc<AtomicU64>,

    /// UI configuration
    pub ui_config: Arc<UiConfig>,

    /// Server start time
    pub start_time: std::time::Instant,

    /// WebRTC session count
    pub webrtc_session_count: Arc<AtomicU64>,

    /// Bumped each time a DataChannel opens (used to trigger taskbar resend)
    pub datachannel_open_count: Arc<AtomicU64>,

    /// Runtime settings updated from client
    pub runtime_settings: Arc<RuntimeSettings>,

    /// Last received WebRTC stats (raw JSON)
    pub last_webrtc_stats_video: Arc<Mutex<Option<String>>>,
    pub last_webrtc_stats_audio: Arc<Mutex<Option<String>>>,

    /// Last clipboard hash written by server (to suppress echo)
    pub last_clipboard_write_hash: Arc<Mutex<Option<u64>>>,

    /// Flag: browser sent new clipboard content, compositor should pick it up
    #[allow(dead_code)]
    pub clipboard_incoming_dirty: Arc<AtomicBool>,

    /// Channel for browser→compositor clipboard content (replaces dirty flag for new data)
    pub clipboard_incoming_tx: mpsc::UnboundedSender<String>,
    pub clipboard_incoming_rx: Arc<Mutex<mpsc::UnboundedReceiver<String>>>,

    /// Cached keyframe RTP packets for new session replay
    pub keyframe_cache: Arc<Mutex<Vec<Vec<u8>>>>,

    /// Per-session mpsc senders for RTP (reliable cross-thread wakeup)
    pub rtp_subscribers: Arc<Mutex<Vec<Subscriber<Vec<u8>>>>>,
    /// Per-session mpsc senders for audio
    pub audio_subscribers: Arc<Mutex<Vec<Subscriber<AudioPacket>>>>,
    /// Per-session mpsc senders for text
    pub text_subscribers: Arc<Mutex<Vec<Subscriber<String>>>>,

    /// Password override (set via /api/change-password, takes precedence over config)
    pub password_override: Arc<RwLock<Option<String>>>,

    /// Cached latest taskbar JSON for the window list
    pub last_taskbar_json: Arc<Mutex<Option<String>>>,

    /// Active WebRTC connections
    pub connections: Arc<Mutex<HashMap<String, ConnectionInfo>>>,

    /// Current IPv4 address
    pub ipv4_address: Arc<RwLock<String>>,

    /// Built-in miao proxy panel process manager
    pub proxy_panel: Arc<ProxyPanelManager>,

    /// Managed-apps manager, attached after construction (apps subsystem is
    /// built later in startup than SharedState).
    apps_state: std::sync::OnceLock<Arc<crate::apps::api::AppsState>>,

    /// Main loop shutdown flag, attached after the compositor loop creates it.
    shutdown_flag: std::sync::OnceLock<Arc<AtomicBool>>,
}

impl std::fmt::Debug for SharedState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedState")
            .field("config", &self.config)
            .field("display_size", &self.display_size)
            .field("webrtc_sessions", &self.webrtc_sessions())
            .finish()
    }
}

impl SharedState {
    /// Create a new shared state
    pub fn new(
        config: Config,
        ui_config: UiConfig,
        input_sender: mpsc::UnboundedSender<InputEventData>,
        runtime_settings: Arc<RuntimeSettings>,
        proxy_ip: Option<std::net::IpAddr>,
        proxy_autostart: Option<bool>,
    ) -> Self {
        let (clipboard_incoming_tx, clipboard_incoming_rx) = mpsc::unbounded_channel();
        let display_size = Arc::new(Mutex::new((config.display.width, config.display.height)));
        let proxy_panel = Arc::new(ProxyPanelManager::new(
            config.ui_features.proxy_enabled,
            proxy_ip,
            proxy_autostart,
        ));

        Self {
            media_router: Arc::new(crate::remote::media::MediaRouter::default()),
            app_views: Arc::new(crate::remote::gateway::Views::default()),
            taskbar_apps: Arc::new(Mutex::new(TaskbarApps::default())),
            window_pids: Arc::new(Mutex::new(HashSet::new())),
            apps_generation: Arc::new(AtomicU64::new(0)),
            config: Arc::new(config),
            ui_config: Arc::new(ui_config),
            input_sender,
            display_size,
            clipboard: Arc::new(Mutex::new(None)),
            force_keyframe: Arc::new(AtomicBool::new(false)),
            pending_resize: Arc::new(Mutex::new(None)),
            stats: Arc::new(Mutex::new(RuntimeStats::default())),
            render_state: Arc::new(AtomicU64::new(RenderState::Active as u64)),
            start_time: std::time::Instant::now(),
            webrtc_session_count: Arc::new(AtomicU64::new(0)),
            datachannel_open_count: Arc::new(AtomicU64::new(0)),
            runtime_settings,
            last_webrtc_stats_video: Arc::new(Mutex::new(None)),
            last_webrtc_stats_audio: Arc::new(Mutex::new(None)),
            last_clipboard_write_hash: Arc::new(Mutex::new(None)),
            clipboard_incoming_dirty: Arc::new(AtomicBool::new(false)),
            clipboard_incoming_tx,
            clipboard_incoming_rx: Arc::new(Mutex::new(clipboard_incoming_rx)),
            keyframe_cache: Arc::new(Mutex::new(Vec::new())),
            rtp_subscribers: Arc::new(Mutex::new(Vec::new())),
            audio_subscribers: Arc::new(Mutex::new(Vec::new())),
            text_subscribers: Arc::new(Mutex::new(Vec::new())),
            password_override: Arc::new(RwLock::new(None)),
            last_taskbar_json: Arc::new(Mutex::new(None)),
            connections: Arc::new(Mutex::new(HashMap::new())),
            ipv4_address: Arc::new(RwLock::new(String::new())),
            proxy_panel,
            apps_state: std::sync::OnceLock::new(),
            shutdown_flag: std::sync::OnceLock::new(),
        }
    }

    /// Publishes one consistent app/order/identity snapshot; unchanged snapshots do not resend.
    pub fn set_taskbar_apps(&self, snapshot: TaskbarApps) {
        let mut current = self.taskbar_apps.lock().unwrap();
        if *current != snapshot {
            *current = snapshot;
            self.apps_generation.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn taskbar_apps(&self) -> TaskbarApps {
        self.taskbar_apps.lock().unwrap().clone()
    }

    pub fn window_pids(&self) -> HashSet<i32> {
        self.window_pids.lock().unwrap().clone()
    }

    pub fn set_window_pids(&self, pids: HashSet<i32>) {
        let mut current = self.window_pids.lock().unwrap();
        if *current != pids {
            *current = pids;
            if let Some(apps) = self.apps_state.get() {
                apps.refresh_window_apps();
            }
        }
    }

    /// Attach the apps manager once it has been constructed during startup.
    pub fn set_apps_state(&self, apps: Arc<crate::apps::api::AppsState>) {
        let _ = self.apps_state.set(apps);
    }

    /// The apps manager, if it has been attached.
    pub fn apps_state(&self) -> Option<&Arc<crate::apps::api::AppsState>> {
        self.apps_state.get()
    }

    pub fn set_shutdown_flag(&self, running: Arc<AtomicBool>) {
        let _ = self.shutdown_flag.set(running);
    }

    pub fn request_shutdown(&self) -> bool {
        if let Some(running) = self.shutdown_flag.get() {
            running.store(false, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    pub fn has_shutdown_flag(&self) -> bool {
        self.shutdown_flag.get().is_some()
    }

    pub fn update_webrtc_stats(&self, kind: &str, payload: &str) {
        match kind {
            "video" => {
                let mut last = self.last_webrtc_stats_video.lock().unwrap();
                *last = Some(payload.to_string());
            }
            "audio" => {
                let mut last = self.last_webrtc_stats_audio.lock().unwrap();
                *last = Some(payload.to_string());
            }
            _ => {}
        }
    }

    pub fn handle_command_message(&self, message: &str) -> bool {
        if !message.starts_with("cmd,") {
            return false;
        }
        if !self.config.input.enable_commands {
            warn!("Command execution disabled; ignoring cmd request");
            return true;
        }
        let cmd = message.trim_start_matches("cmd,").trim();
        if cmd.is_empty() {
            warn!("Received empty cmd request");
            return true;
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        match Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(home)
            .spawn()
        {
            Ok(_) => info!("Launched command: {}", cmd),
            Err(err) => warn!("Failed to launch command '{}': {}", cmd, err),
        }
        true
    }

    /// Send a text message to all sessions.
    pub fn send_text(&self, msg: String) {
        self.broadcast_text(msg.clone());
    }

    /// Store clipboard and broadcast to clients
    pub fn set_clipboard(&self, base64_text: String) {
        let mut clipboard = self.clipboard.lock().unwrap();
        *clipboard = Some(base64_text.clone());
        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&base64_text) {
            let total_size = decoded.len();
            if total_size > 8192 {
                self.send_text(format!("clipboard_start,text/plain,{}", total_size));
                for chunk in decoded.chunks(4096) {
                    let encoded = base64::engine::general_purpose::STANDARD.encode(chunk);
                    self.send_text(format!("clipboard_data,{}", encoded));
                }
                self.send_text("clipboard_finish".to_string());
                return;
            }
        }

        self.send_text(format!("clipboard,{}", base64_text));
    }

    /// Store binary clipboard and broadcast to clients
    pub fn set_clipboard_binary(&self, mime_type: String, data: Vec<u8>) {
        if data.len() > 8192 {
            self.send_text(format!("clipboard_start,{},{}", mime_type, data.len()));
            for chunk in data.chunks(4096) {
                let encoded = base64::engine::general_purpose::STANDARD.encode(chunk);
                self.send_text(format!("clipboard_data,{}", encoded));
            }
            self.send_text("clipboard_finish".to_string());
            return;
        }

        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        self.send_text(format!("clipboard_binary,{},{}", mime_type, encoded));
    }

    pub fn mark_clipboard_written(&self, mime_type: &str, data: &[u8]) {
        let mut hash = xxh64(mime_type.as_bytes(), 0);
        hash = xxh64(data, hash);
        let mut last = self.last_clipboard_write_hash.lock().unwrap();
        *last = Some(hash);
    }

    /// Update display size
    pub fn set_display_size(&self, width: u32, height: u32) {
        let mut size = self.display_size.lock().unwrap();
        *size = (width, height);
    }

    /// Get current display size
    pub fn display_size(&self) -> (u32, u32) {
        *self.display_size.lock().unwrap()
    }

    /// Request display resize
    pub fn resize_display(&self, width: u32, height: u32) {
        let current = self.display_size();
        if current == (width, height) {
            return;
        }
        info!("Queuing display resize to {}x{}", width, height);
        *self.pending_resize.lock().unwrap() = Some((width, height));
    }

    /// Take pending resize request (called by compositor thread)
    pub fn take_pending_resize(&self) -> Option<(u32, u32)> {
        self.pending_resize.lock().unwrap().take()
    }

    /// Update client-reported latency metric (ms)
    pub fn update_client_latency(&self, latency_ms: u64) {
        let mut stats = self.stats.lock().unwrap();
        stats.client_latency_ms = latency_ms;
    }

    /// Update client-reported FPS
    pub fn update_client_fps(&self, fps: u32) {
        let mut stats = self.stats.lock().unwrap();
        stats.client_fps = fps;
    }

    /// Record a protocol classification event
    pub fn record_protocol_classification(&self, kind: &str) {
        let mut stats = self.stats.lock().unwrap();
        match kind {
            "http" => stats.proto_http += 1,
            "ice_tcp" => stats.proto_ice_tcp += 1,
            "tls" => stats.proto_tls += 1,
            _ => stats.proto_unknown += 1,
        }
    }

    /// Build stats JSON payload
    pub fn stats_json(&self) -> String {
        let stats = self.stats.lock().unwrap().clone();
        format!(
            r#"{{"fps":{:.2},"bandwidth":{},"latency":{},"client_latency":{},"client_fps":{},"clients":{},"cpu_percent":{:.1},"mem_used":{},"render_state":"{}","ice_candidates_total":{},"ice_candidates_tcp":{}}}"#,
            stats.fps,
            stats.bandwidth,
            stats.latency_ms,
            stats.client_latency_ms,
            stats.client_fps,
            self.connection_count(),
            stats.cpu_percent,
            stats.mem_used,
            self.render_state().as_str(),
            stats.ice_candidates_total,
            stats.ice_candidates_tcp
        )
    }

    pub fn set_render_state(&self, state: RenderState) {
        self.render_state.store(state as u64, Ordering::Relaxed);
    }

    pub fn render_state(&self) -> RenderState {
        RenderState::from_u64(self.render_state.load(Ordering::Relaxed))
    }

    /// Build UI configuration JSON payload
    pub fn ui_config_json(&self) -> String {
        self.ui_config.to_json()
    }

    /// Get server uptime
    pub fn uptime(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }

    /// Get connection count (WebRTC sessions)
    pub fn connection_count(&self) -> u64 {
        self.webrtc_sessions()
    }

    // WebRTC methods

    /// Request a keyframe from the encoder
    pub fn request_keyframe(&self) {
        self.force_keyframe.store(true, Ordering::Relaxed);
    }

    /// Consume keyframe request flag
    pub fn take_keyframe_request(&self) -> bool {
        self.force_keyframe.swap(false, Ordering::Relaxed)
    }

    /// Broadcast an RTP packet to all WebRTC sessions
    pub fn broadcast_rtp(&self, packet: Vec<u8>) {
        let recovered = deliver(&mut self.rtp_subscribers.lock().unwrap(), &packet);
        // Video dropped during congestion is unusable until the next keyframe.
        if recovered {
            self.request_keyframe();
        }
    }

    /// Get current RTP subscriber count
    pub fn rtp_receiver_count(&self) -> usize {
        self.rtp_subscribers.lock().unwrap().len()
    }

    /// Subscribe to RTP packets via mpsc (reliable cross-thread wakeup)
    pub fn subscribe_rtp_mpsc(&self) -> mpsc::Receiver<Vec<u8>> {
        let (tx, rx) = mpsc::channel(2048);
        self.rtp_subscribers
            .lock()
            .unwrap()
            .push(Subscriber::new(tx));
        rx
    }

    /// Subscribe to audio packets via mpsc
    pub fn subscribe_audio_mpsc(&self) -> mpsc::Receiver<AudioPacket> {
        let (tx, rx) = mpsc::channel(512);
        self.audio_subscribers
            .lock()
            .unwrap()
            .push(Subscriber::new(tx));
        rx
    }

    /// Subscribe to text messages via mpsc
    pub fn subscribe_text_mpsc(&self) -> mpsc::Receiver<String> {
        let (tx, rx) = mpsc::channel(256);
        self.text_subscribers
            .lock()
            .unwrap()
            .push(Subscriber::new(tx));
        rx
    }

    /// Broadcast audio to all subscribers
    pub fn broadcast_audio(&self, packet: AudioPacket) {
        deliver(&mut self.audio_subscribers.lock().unwrap(), &packet);
    }

    /// Broadcast text to all subscribers
    pub fn broadcast_text(&self, msg: String) {
        deliver(&mut self.text_subscribers.lock().unwrap(), &msg);
    }

    /// Update the keyframe cache with a new set of RTP packets
    pub fn set_keyframe_cache(&self, packets: Vec<Vec<u8>>) {
        if let Ok(mut cache) = self.keyframe_cache.lock() {
            *cache = packets;
        }
    }

    /// Increment WebRTC session count
    pub fn increment_webrtc_sessions(&self) {
        self.webrtc_session_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement WebRTC session count (saturating to avoid underflow)
    pub fn decrement_webrtc_sessions(&self) {
        let mut current = self.webrtc_session_count.load(Ordering::Relaxed);
        loop {
            if current == 0 {
                break;
            }
            match self.webrtc_session_count.compare_exchange_weak(
                current,
                current - 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
    }

    /// Get WebRTC session count
    pub fn webrtc_sessions(&self) -> u64 {
        self.webrtc_session_count.load(Ordering::Relaxed)
    }

    /// Add a new connection
    pub fn add_connection(
        &self,
        id: String,
        peer_ip: String,
        shutdown_tx: tokio::sync::oneshot::Sender<()>,
    ) {
        let conn = ConnectionInfo {
            id: id.clone(),
            peer_ip,
            connected_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
            connection_type: "tcp".to_string(),
            client_info: None,
            client_visible: true,
            client_focused: true,
            last_client_input_ms: now_millis(),
            last_client_focus_ms: now_millis(),
            last_client_hidden_ms: 0,
            last_client_heartbeat_ms: now_millis(),
            last_client_wakeup_ms: now_millis(),
            shutdown_tx: Some(shutdown_tx),
        };
        self.connections.lock().unwrap().insert(id, conn);
    }

    /// Update client-reported browser feature information for a connection.
    pub fn update_connection_client_info(&self, id: &str, payload: &str) {
        if payload.len() > 8192 {
            warn!(
                "Ignoring oversized client info payload for connection {}",
                id
            );
            return;
        }
        let Ok(info) = serde_json::from_str::<Value>(payload) else {
            warn!("Ignoring invalid client info payload for connection {}", id);
            return;
        };
        if !info.is_object() {
            warn!(
                "Ignoring non-object client info payload for connection {}",
                id
            );
            return;
        }
        if let Ok(mut conns) = self.connections.lock() {
            if let Some(conn) = conns.get_mut(id) {
                conn.client_info = Some(info);
            }
        }
    }

    /// Update client page visibility/focus information for idle decisions.
    pub fn update_connection_client_activity(&self, id: &str, payload: &str) {
        if payload.len() > 2048 {
            warn!(
                "Ignoring oversized client activity payload for connection {}",
                id
            );
            return;
        }
        let Ok(report) = serde_json::from_str::<ClientActivityReport>(payload) else {
            warn!(
                "Ignoring invalid client activity payload for connection {}",
                id
            );
            return;
        };
        let now = now_millis();
        if let Ok(mut conns) = self.connections.lock() {
            if let Some(conn) = conns.get_mut(id) {
                conn.last_client_heartbeat_ms = now;
                if let Some(visible) = report.visible {
                    if conn.client_visible != visible && !visible {
                        conn.last_client_hidden_ms = now;
                    }
                    if visible && !conn.client_visible {
                        conn.last_client_wakeup_ms = now;
                    }
                    conn.client_visible = visible;
                }
                if let Some(focused) = report.focused {
                    if focused {
                        conn.last_client_focus_ms = now;
                        conn.last_client_wakeup_ms = now;
                    }
                    conn.client_focused = focused;
                }
                if report.input.unwrap_or(false) {
                    conn.last_client_input_ms = now;
                    conn.last_client_focus_ms = now;
                    conn.last_client_wakeup_ms = now;
                    conn.client_visible = true;
                    conn.client_focused = true;
                } else if report.event.as_deref() != Some("heartbeat") {
                    conn.last_client_wakeup_ms = now;
                }
            }
        }
    }

    pub fn record_connection_input_activity(&self, id: &str) {
        let now = now_millis();
        if let Ok(mut conns) = self.connections.lock() {
            if let Some(conn) = conns.get_mut(id) {
                conn.client_visible = true;
                conn.client_focused = true;
                conn.last_client_input_ms = now;
                conn.last_client_focus_ms = now;
                conn.last_client_heartbeat_ms = now;
                conn.last_client_wakeup_ms = now;
            }
        }
    }

    pub fn client_viewer_summary(&self) -> ClientViewerSummary {
        const HEARTBEAT_STALE_AFTER_MS: u64 = 30_000;
        const HIDDEN_DEEP_IDLE_AFTER_MS: u64 = 30_000;
        const UNFOCUSED_DEEP_IDLE_AFTER_MS: u64 = 5 * 60_000;
        const RECENT_WAKEUP_AFTER_MS: u64 = 2_000;

        let now = now_millis();
        let Ok(conns) = self.connections.lock() else {
            return ClientViewerSummary::default();
        };
        if conns.is_empty() {
            return ClientViewerSummary::default();
        }

        let mut has_active_viewer = false;
        let mut all_clients_inactive = true;
        let mut has_recent_client_wakeup = false;

        for conn in conns.values() {
            let heartbeat_age = now.saturating_sub(conn.last_client_heartbeat_ms);
            let input_age = now.saturating_sub(conn.last_client_input_ms);
            let focus_age = now.saturating_sub(conn.last_client_focus_ms);
            let wakeup_age = now.saturating_sub(conn.last_client_wakeup_ms);
            let hidden_age = if conn.last_client_hidden_ms == 0 {
                0
            } else {
                now.saturating_sub(conn.last_client_hidden_ms)
            };

            let stale = heartbeat_age > HEARTBEAT_STALE_AFTER_MS;
            let hidden_inactive = !conn.client_visible && hidden_age >= HIDDEN_DEEP_IDLE_AFTER_MS;
            let unfocused_inactive = conn.client_visible
                && !conn.client_focused
                && focus_age >= UNFOCUSED_DEEP_IDLE_AFTER_MS
                && input_age >= UNFOCUSED_DEEP_IDLE_AFTER_MS;
            let inactive = stale || hidden_inactive || unfocused_inactive;

            if !inactive {
                all_clients_inactive = false;
            }
            if !stale && conn.client_visible && conn.client_focused {
                has_active_viewer = true;
            }
            if !stale && wakeup_age <= RECENT_WAKEUP_AFTER_MS {
                has_recent_client_wakeup = true;
            }
        }

        ClientViewerSummary {
            has_connections: true,
            has_active_viewer,
            all_clients_inactive,
            has_recent_client_wakeup,
        }
    }

    /// Remove a connection
    pub fn remove_connection(&self, id: &str) {
        if self.connections.lock().unwrap().remove(id).is_some() {
            self.reset_input();
        }
    }

    pub fn disconnect_connection(&self, id: &str) {
        let Some(connection) = self.connections.lock().unwrap().remove(id) else {
            return;
        };
        if let Some(tx) = connection.shutdown_tx {
            let _ = tx.send(());
        }
        self.reset_input();
    }

    /// Releases every pressed key and pointer button. Every connection may
    /// control the desktop, and keys a connection pressed before it dropped can
    /// only be released here.
    fn reset_input(&self) {
        let _ = self.input_sender.send(InputEventData {
            event_type: crate::input::InputEvent::KeyboardReset,
            ..Default::default()
        });
    }

    /// Get all connections as JSON
    pub fn get_connections_json(&self) -> String {
        let conns = self.connections.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let items: Vec<Value> = conns
            .values()
            .map(|c| {
                let duration = now - c.connected_at;
                json!({
                    "id": c.id,
                    "peer_ip": c.peer_ip,
                    "connected_at": c.connected_at,
                    "connection_type": c.connection_type,
                    "duration_seconds": duration,
                    "client_info": c.client_info,
                    "client_activity": {
                        "visible": c.client_visible,
                        "focused": c.client_focused,
                        "last_input_ms": c.last_client_input_ms,
                        "last_focus_ms": c.last_client_focus_ms,
                        "last_hidden_ms": c.last_client_hidden_ms,
                        "last_heartbeat_ms": c.last_client_heartbeat_ms,
                        "last_wakeup_ms": c.last_client_wakeup_ms
                    }
                })
            })
            .collect();

        json!({ "connections": items }).to_string()
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderState {
    Active = 0,
    ViewerQuietIdle = 1,
    NoViewerIdle = 2,
}

impl RenderState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::ViewerQuietIdle => "viewer_quiet_idle",
            Self::NoViewerIdle => "no_viewer_idle",
        }
    }

    fn from_u64(value: u64) -> Self {
        match value {
            1 => Self::ViewerQuietIdle,
            2 => Self::NoViewerIdle,
            _ => Self::Active,
        }
    }
}

/// Runtime stats snapshot
#[derive(Debug, Clone)]
pub struct RuntimeStats {
    pub fps: f64,
    pub bandwidth: u64,
    pub latency_ms: u64,
    pub client_latency_ms: u64,
    pub client_fps: u32,
    pub total_frames: u64,
    pub total_bytes: u64,
    pub cpu_percent: f64,
    pub mem_used: u64,
    pub ice_candidates_total: u64,
    pub ice_candidates_tcp: u64,
    /// Protocol classification counters
    pub proto_http: u64,
    pub proto_ice_tcp: u64,
    pub proto_tls: u64,
    pub proto_unknown: u64,
}

impl Default for RuntimeStats {
    fn default() -> Self {
        Self {
            fps: 0.0,
            bandwidth: 0,
            latency_ms: 0,
            client_latency_ms: 0,
            client_fps: 0,
            total_frames: 0,
            total_bytes: 0,
            cpu_percent: 0.0,
            mem_used: 0,
            ice_candidates_total: 0,
            ice_candidates_tcp: 0,
            proto_http: 0,
            proto_ice_tcp: 0,
            proto_tls: 0,
            proto_unknown: 0,
        }
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::*;

    #[test]
    fn brief_congestion_drops_items_and_sustained_congestion_disconnects() {
        let (tx, mut rx) = mpsc::channel(1);
        let mut subs = vec![Subscriber::new(tx)];
        assert!(!deliver(&mut subs, &1));
        assert!(!deliver(&mut subs, &2)); // full: dropped, still subscribed
        assert_eq!(subs.len(), 1);
        assert_eq!(rx.try_recv().unwrap(), 1);
        assert!(deliver(&mut subs, &3)); // recovered
        assert!(!deliver(&mut subs, &4));
        subs[0].congested_since =
            Some(std::time::Instant::now() - CONGESTION_LIMIT - std::time::Duration::from_secs(1));
        deliver(&mut subs, &5);
        assert!(subs.is_empty());
        let (closed, receiver) = mpsc::channel::<i32>(1);
        drop(receiver);
        let mut subs = vec![Subscriber::new(closed)];
        deliver(&mut subs, &1);
        assert!(subs.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taskbar_lists_windows_and_application_tabs() {
        let empty: serde_json::Value =
            serde_json::from_str(&taskbar_json(vec![], &TaskbarApps::default())).unwrap();
        assert_eq!(
            empty,
            json!({"windows": [], "apps": [], "ordered_ids": [], "order_revision": 0})
        );
        let apps = [
            StreamApp {
                id: "local".into(),
                name: "Chrome".into(),
                name_revision: 0,
                can_rename: true,
                kind: "shared",
                status: "starting",
            },
            StreamApp {
                id: "mapping".into(),
                name: "Remote".into(),
                name_revision: 0,
                can_rename: true,
                kind: "remote",
                status: "auth_required",
            },
        ];
        let value: serde_json::Value = serde_json::from_str(&taskbar_json(
            vec![json!({"id": 0, "title": "Files"})],
            &TaskbarApps {
                apps: apps.to_vec(),
                ..Default::default()
            },
        ))
        .unwrap();
        assert_eq!(value["windows"][0]["title"], "Files");
        assert_eq!(
            value["apps"],
            json!([
                {"id": "local", "name": "Chrome", "name_revision": 0, "can_rename": true, "kind": "shared", "status": "starting"},
                {"id": "mapping", "name": "Remote", "name_revision": 0, "can_rename": true, "kind": "remote", "status": "auth_required"},
            ])
        );
    }

    #[test]
    fn only_changed_application_tabs_resend_the_taskbar() {
        let (input, _) = mpsc::unbounded_channel();
        let config = Config::default();
        let state = SharedState::new(
            config.clone(),
            UiConfig::from_env(&config),
            input,
            Arc::new(RuntimeSettings::new(&config)),
            None,
            None,
        );
        let app = StreamApp {
            id: "app".into(),
            name: "App".into(),
            name_revision: 0,
            can_rename: true,
            kind: "remote",
            status: "connecting",
        };
        state.set_taskbar_apps(TaskbarApps {
            apps: vec![app.clone()],
            ..Default::default()
        });
        assert_eq!(state.apps_generation.load(Ordering::Relaxed), 1);
        state.set_taskbar_apps(TaskbarApps {
            apps: vec![app.clone()],
            ..Default::default()
        });
        assert_eq!(state.apps_generation.load(Ordering::Relaxed), 1);
        state.set_taskbar_apps(TaskbarApps {
            apps: vec![StreamApp {
                status: "connected",
                ..app
            }],
            ..Default::default()
        });
        assert_eq!(state.apps_generation.load(Ordering::Relaxed), 2);
        assert_eq!(state.taskbar_apps().apps[0].status, "connected");
    }

    #[test]
    fn order_and_peer_identity_changes_resend_even_without_stream_tabs() {
        let (input, _) = mpsc::unbounded_channel();
        let config = Config::default();
        let state = SharedState::new(
            config.clone(),
            UiConfig::from_env(&config),
            input,
            Arc::new(RuntimeSettings::new(&config)),
            None,
            None,
        );
        let mut snapshot = TaskbarApps {
            order: crate::apps::store::AppOrder {
                ordered_ids: vec!["a".into(), "b".into()],
                order_revision: 2,
            },
            ..Default::default()
        };
        state.set_taskbar_apps(snapshot.clone());
        assert_eq!(state.apps_generation.load(Ordering::Relaxed), 1);
        snapshot.order.ordered_ids.reverse();
        snapshot.order.order_revision += 1;
        state.set_taskbar_apps(snapshot.clone());
        state.set_taskbar_apps(snapshot.clone());
        assert_eq!(state.apps_generation.load(Ordering::Relaxed), 2);
        snapshot.window_apps.insert(42, "a".into());
        state.set_taskbar_apps(snapshot.clone());
        assert_eq!(state.apps_generation.load(Ordering::Relaxed), 3);
        let message: Value = serde_json::from_str(&taskbar_json(vec![], &snapshot)).unwrap();
        assert_eq!(message["ordered_ids"], json!(["b", "a"]));
        assert_eq!(message["order_revision"], 3);
        assert_eq!(message["apps"], json!([]));
        snapshot.names.insert(
            "a".into(),
            TaskbarAppName {
                name: "Renamed".into(),
                name_revision: 1,
                can_rename: true,
            },
        );
        state.set_taskbar_apps(snapshot.clone());
        assert_eq!(state.apps_generation.load(Ordering::Relaxed), 4);
        state.set_taskbar_apps(snapshot);
        assert_eq!(state.apps_generation.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn dropped_connections_release_pressed_input() {
        let (input, mut events) = mpsc::unbounded_channel();
        let config = Config::default();
        let state = SharedState::new(
            config.clone(),
            UiConfig::from_env(&config),
            input,
            Arc::new(RuntimeSettings::new(&config)),
            None,
            None,
        );
        let reset = |events: &mut mpsc::UnboundedReceiver<InputEventData>| {
            let mut count = 0;
            while let Ok(event) = events.try_recv() {
                assert!(matches!(
                    event.event_type,
                    crate::input::InputEvent::KeyboardReset
                ));
                count += 1;
            }
            count
        };
        for disconnect in [false, true] {
            state.add_connection(
                "one".into(),
                "192.0.2.1".into(),
                tokio::sync::oneshot::channel().0,
            );
            if disconnect {
                state.disconnect_connection("one");
            }
            // Its session ends afterwards and removes it once more.
            state.remove_connection("one");
            assert_eq!(reset(&mut events), 1);
        }
        state.remove_connection("missing");
        assert_eq!(reset(&mut events), 0);
    }
}
