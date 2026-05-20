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
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
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

/// Shared state for the application
#[derive(Clone)]
pub struct SharedState {
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
    pub clipboard_incoming_dirty: Arc<AtomicBool>,

    /// Channel for browser→compositor clipboard content (replaces dirty flag for new data)
    pub clipboard_incoming_tx: mpsc::UnboundedSender<String>,
    pub clipboard_incoming_rx: Arc<Mutex<mpsc::UnboundedReceiver<String>>>,

    /// Cached keyframe RTP packets for new session replay
    pub keyframe_cache: Arc<Mutex<Vec<Vec<u8>>>>,

    /// Per-session mpsc senders for RTP (reliable cross-thread wakeup)
    pub rtp_subscribers: Arc<Mutex<Vec<mpsc::UnboundedSender<Vec<u8>>>>>,
    /// Per-session mpsc senders for audio
    pub audio_subscribers: Arc<Mutex<Vec<mpsc::UnboundedSender<AudioPacket>>>>,
    /// Per-session mpsc senders for text
    pub text_subscribers: Arc<Mutex<Vec<mpsc::UnboundedSender<String>>>>,

    /// Password override (set via /api/change-password, takes precedence over config)
    pub password_override: Arc<RwLock<Option<String>>>,

    /// MCP frame capture channel: MCP tools send oneshot senders here,
    /// main loop responds with (width, height, xrgb_pixels)
    #[cfg(feature = "mcp")]
    pub frame_capture_tx: mpsc::UnboundedSender<tokio::sync::oneshot::Sender<(u32, u32, Vec<u8>)>>,
    #[cfg(feature = "mcp")]
    pub frame_capture_rx:
        Arc<Mutex<mpsc::UnboundedReceiver<tokio::sync::oneshot::Sender<(u32, u32, Vec<u8>)>>>>,

    /// Cached latest taskbar JSON for MCP list_windows tool
    pub last_taskbar_json: Arc<Mutex<Option<String>>>,

    /// Active WebRTC connections
    pub connections: Arc<Mutex<HashMap<String, ConnectionInfo>>>,

    /// Current IPv4 address
    pub ipv4_address: Arc<RwLock<String>>,

    /// Built-in miao proxy panel process manager
    pub proxy_panel: Arc<ProxyPanelManager>,
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
    ) -> Self {
        let (clipboard_incoming_tx, clipboard_incoming_rx) = mpsc::unbounded_channel();
        #[cfg(feature = "mcp")]
        let (frame_capture_tx, frame_capture_rx) = mpsc::unbounded_channel();
        let display_size = Arc::new(Mutex::new((config.display.width, config.display.height)));

        Self {
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
            #[cfg(feature = "mcp")]
            frame_capture_tx,
            #[cfg(feature = "mcp")]
            frame_capture_rx: Arc::new(Mutex::new(frame_capture_rx)),
            last_taskbar_json: Arc::new(Mutex::new(None)),
            connections: Arc::new(Mutex::new(HashMap::new())),
            ipv4_address: Arc::new(RwLock::new(String::new())),
            proxy_panel: Arc::new(ProxyPanelManager::new()),
        }
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
        let mut subs = self.rtp_subscribers.lock().unwrap();
        subs.retain(|tx| tx.send(packet.clone()).is_ok());
    }

    /// Get current RTP subscriber count
    pub fn rtp_receiver_count(&self) -> usize {
        self.rtp_subscribers.lock().unwrap().len()
    }

    /// Subscribe to RTP packets via mpsc (reliable cross-thread wakeup)
    pub fn subscribe_rtp_mpsc(&self) -> mpsc::UnboundedReceiver<Vec<u8>> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.rtp_subscribers.lock().unwrap().push(tx);
        rx
    }

    /// Subscribe to audio packets via mpsc
    pub fn subscribe_audio_mpsc(&self) -> mpsc::UnboundedReceiver<AudioPacket> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.audio_subscribers.lock().unwrap().push(tx);
        rx
    }

    /// Subscribe to text messages via mpsc
    pub fn subscribe_text_mpsc(&self) -> mpsc::UnboundedReceiver<String> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.text_subscribers.lock().unwrap().push(tx);
        rx
    }

    /// Broadcast audio to all subscribers
    pub fn broadcast_audio(&self, packet: AudioPacket) {
        let mut subs = self.audio_subscribers.lock().unwrap();
        subs.retain(|tx| tx.send(packet.clone()).is_ok());
    }

    /// Broadcast text to all subscribers
    pub fn broadcast_text(&self, msg: String) {
        let mut subs = self.text_subscribers.lock().unwrap();
        subs.retain(|tx| tx.send(msg.clone()).is_ok());
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
        self.connections.lock().unwrap().remove(id);
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
