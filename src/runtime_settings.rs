//! Runtime-adjustable settings derived from client SETTINGS messages.

use crate::config::Config;
use log::debug;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub struct RuntimeSettings {
    console_updates: tokio::sync::Mutex<()>,
    target_fps: AtomicU32,
    max_fps: u32,
    binary_clipboard_enabled: AtomicBool,
    video_bitrate_kbps: AtomicU32,
    audio_bitrate: AtomicU32,
    keyframe_interval: AtomicU32,
    keyframe_request: AtomicBool,
    audio_bitrate_dirty: AtomicBool,
}

impl RuntimeSettings {
    pub fn new(config: &Config) -> Self {
        Self {
            console_updates: Default::default(),
            target_fps: AtomicU32::new(config.encoding.target_fps.max(1)),
            max_fps: config.encoding.max_fps.max(1),
            binary_clipboard_enabled: AtomicBool::new(config.input.enable_binary_clipboard),
            video_bitrate_kbps: AtomicU32::new(config.webrtc.video_bitrate),
            audio_bitrate: AtomicU32::new(config.audio.bitrate.max(1)),
            keyframe_interval: AtomicU32::new(config.webrtc.keyframe_interval.max(1)),
            keyframe_request: AtomicBool::new(false),
            audio_bitrate_dirty: AtomicBool::new(false),
        }
    }

    pub async fn console_update_lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.console_updates.lock().await
    }

    pub fn binary_clipboard_enabled(&self) -> bool {
        self.binary_clipboard_enabled.load(Ordering::Relaxed)
    }

    pub fn set_binary_clipboard_enabled(&self, enabled: bool) {
        self.binary_clipboard_enabled
            .store(enabled, Ordering::Relaxed);
    }

    pub fn video_bitrate_kbps(&self) -> u32 {
        self.video_bitrate_kbps.load(Ordering::Relaxed)
    }

    #[allow(dead_code)]
    pub fn target_fps(&self) -> u32 {
        self.target_fps.load(Ordering::Relaxed)
    }

    #[allow(dead_code)]
    pub fn audio_bitrate(&self) -> u32 {
        self.audio_bitrate.load(Ordering::Relaxed)
    }

    pub fn keyframe_interval(&self) -> u32 {
        self.keyframe_interval.load(Ordering::Relaxed)
    }

    pub fn take_keyframe_request(&self) -> bool {
        self.keyframe_request.swap(false, Ordering::Relaxed)
    }

    pub fn set_target_fps(&self, fps: u32) {
        let clamped = fps.max(1).min(self.max_fps);
        self.target_fps.store(clamped, Ordering::Relaxed);
    }

    pub fn set_video_bitrate_kbps(&self, bitrate: u32) {
        let clamped = bitrate.max(1);
        self.video_bitrate_kbps.store(clamped, Ordering::Relaxed);
    }

    pub fn set_audio_bitrate(&self, bitrate: u32) {
        let clamped = bitrate.max(1);
        self.audio_bitrate.store(clamped, Ordering::Relaxed);
        self.audio_bitrate_dirty.store(true, Ordering::Relaxed);
    }

    pub fn set_keyframe_interval(&self, interval: u32) {
        let clamped = interval.max(1);
        self.keyframe_interval.store(clamped, Ordering::Relaxed);
    }

    pub fn request_keyframe(&self) {
        self.keyframe_request.store(true, Ordering::Relaxed);
    }

    pub fn handle_simple_message(&self, message: &str) -> bool {
        if message == "keyframe" || message == "_k" {
            self.request_keyframe();
            return true;
        }
        if message.starts_with("vb,") {
            let payload = message.trim_start_matches("vb,");
            if let Ok(bitrate) = payload.parse::<u32>() {
                self.set_video_bitrate_kbps(bitrate);
            }
            return true;
        }
        if message.starts_with("ab,") {
            let payload = message.trim_start_matches("ab,");
            if let Ok(bitrate) = payload.parse::<u32>() {
                self.set_audio_bitrate(bitrate);
            }
            return true;
        }
        false
    }

    pub fn apply_settings_json(&self, json_str: &str) {
        let value: Value = match serde_json::from_str(json_str) {
            Ok(value) => value,
            Err(err) => {
                debug!("SETTINGS parse failed: {}", err);
                return;
            }
        };

        if let Some(fps) = value.get("framerate").and_then(|v| v.as_u64()) {
            self.set_target_fps(fps as u32);
        }

        // Clipboard policy belongs to the instance owner. Older clients send
        // their cached value on reconnect; it must not override console policy.

        if let Some(bitrate) = value.get("video_bitrate").and_then(|v| v.as_u64()) {
            self.set_video_bitrate_kbps(bitrate as u32);
        }

        if let Some(bitrate) = value.get("audio_bitrate").and_then(|v| v.as_u64()) {
            self.set_audio_bitrate(bitrate as u32);
        }

        if let Some(interval) = value.get("keyframe_interval").and_then(|v| v.as_u64()) {
            self.set_keyframe_interval(interval as u32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_policy_cannot_be_overridden_by_an_old_viewer_setting() {
        let config = Config::default();
        assert!(config.input.enable_binary_clipboard);
        let explicit: crate::config::InputConfig =
            toml::from_str("enable_binary_clipboard = false").unwrap();
        assert!(!explicit.enable_binary_clipboard);
        let unset: crate::config::InputConfig = toml::from_str("").unwrap();
        assert!(unset.enable_binary_clipboard);
        let settings = RuntimeSettings::new(&config);
        settings.set_binary_clipboard_enabled(false);
        settings.apply_settings_json(r#"{"enable_binary_clipboard":true}"#);
        assert!(!settings.binary_clipboard_enabled());
        settings.set_binary_clipboard_enabled(true);
        settings.apply_settings_json(r#"{"enable_binary_clipboard":false}"#);
        assert!(settings.binary_clipboard_enabled());
    }
}
