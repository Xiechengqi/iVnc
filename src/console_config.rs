use crate::agent::types::{Budget, RunOptions};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConsoleConfig {
    #[serde(default)]
    pub agent: AgentConsoleConfig,
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConsoleConfig>,
    #[serde(default)]
    pub runtime: RuntimeConsoleConfig,
    #[serde(default)]
    pub schedules: Vec<ScheduledTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    pub name: String,
    /// Standard 5-field cron expression in the server local timezone.
    pub cron: String,
    pub task: String,
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub budget: Budget,
    #[serde(default)]
    pub options: Option<RunOptions>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConsoleConfig {
    #[serde(default = "default_provider")]
    pub default_provider: String,
    #[serde(default)]
    pub options: RunOptions,
}

impl Default for AgentConsoleConfig {
    fn default() -> Self {
        Self {
            default_provider: default_provider(),
            options: RunOptions::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderConsoleConfig {
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub api_format: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub coord_space: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
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
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/root/.config"))
        .join("ivnc")
        .join("console.json")
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

pub fn provider(name: &str) -> ProviderConsoleConfig {
    load().providers.remove(name).unwrap_or_default()
}

pub fn agent_defaults() -> AgentConsoleConfig {
    load().agent
}

pub fn default_provider() -> String {
    "local".to_string()
}
