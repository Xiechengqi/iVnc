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
    pub display: Option<crate::display::Size>,
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

/// Serialize read/modify/write so stream settings and geometry cannot erase each other.
pub fn update(change: impl FnOnce(&mut ConsoleConfig)) -> Result<(), String> {
    static WRITER: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = WRITER.lock().unwrap();
    let mut config = load();
    change(&mut config);
    save_at(&config_path(), &config)
}

fn save_at(path: &std::path::Path, config: &ConsoleConfig) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let parent = path.parent().ok_or("Invalid settings path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temp = parent.join(format!(".console-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&temp, path).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_file(temp);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_settings_keep_runtime_and_geometry_and_private_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ivnc-console-{}", uuid::Uuid::new_v4()));
        let path = dir.join("console.json");
        let mut config = ConsoleConfig::default();
        config.runtime.target_fps = Some(24);
        config.display = Some(crate::display::Size::new(1280, 720));
        save_at(&path, &config).unwrap();
        let restored: ConsoleConfig =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored.display, config.display);
        assert_eq!(restored.runtime.target_fps, Some(24));
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // Rename failure must not expose a partial file or leave temporary files.
        let blocked = dir.join("blocked");
        std::fs::create_dir(&blocked).unwrap();
        assert!(save_at(&blocked, &config).is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
