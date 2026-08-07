//! Console-managed runtime settings.
//!
//! Persists the handful of streaming knobs the web console can change at
//! runtime to `$IVNC_HOME/console.json` (default `$HOME/.ivnc/console.json`).
//! Unknown fields are ignored on load, so config files written by older builds
//! still parse.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConsoleConfig {
    #[serde(default)]
    pub runtime: RuntimeConsoleConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimeConsoleConfig {
    #[serde(default)]
    pub video_bitrate_kbps: Option<u32>,
    #[serde(default)]
    pub audio_bitrate: Option<u32>,
    #[serde(default)]
    pub keyframe_interval: Option<u32>,
    #[serde(default)]
    pub target_fps: Option<u32>,
    #[serde(default)]
    pub binary_clipboard_enabled: Option<bool>,
}

pub fn config_path() -> PathBuf {
    crate::paths::console_json()
}

pub fn load() -> ConsoleConfig {
    let path = config_path();
    let Ok(text) = std::fs::read_to_string(path) else {
        return ConsoleConfig::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save(config: &ConsoleConfig) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}
