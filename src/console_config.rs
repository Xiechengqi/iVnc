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
    pub provider_type: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// OpenAI-compatible request shape: `chat_completions` or `responses`.
    #[serde(default)]
    pub api_format: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(skip)]
    pub api_key_env: Option<String>,
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
        return with_provider_env_defaults(ConsoleConfig::default());
    };
    let config = serde_json::from_str(&text).unwrap_or_default();
    migrate_provider_config(config)
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

pub fn agent_defaults() -> AgentConsoleConfig {
    load().agent
}

pub fn default_provider() -> String {
    String::new()
}

fn migrate_provider_config(mut config: ConsoleConfig) -> ConsoleConfig {
    for (id, provider) in config.providers.iter_mut() {
        if provider.name.as_ref().is_none_or(|v| v.trim().is_empty()) {
            provider.name = Some(id.clone());
        }
        if provider.enabled.is_none() {
            provider.enabled = Some(true);
        }
        if provider
            .provider_type
            .as_ref()
            .is_none_or(|v| v.trim().is_empty())
        {
            let provider_type = match id.as_str() {
                "anthropic" => "anthropic",
                "openai" | "local" | "holo3" | "openai_compat" => "openai",
                "gemini" => "gemini",
                _ => "openai",
            };
            provider.provider_type = Some(provider_type.to_string());
        }
        if provider.provider_type.as_deref() == Some("gemini") {
            provider.enabled = Some(false);
        }
        apply_legacy_provider_defaults(id, provider);
        attach_legacy_provider_env(id, provider);
    }

    config = with_provider_env_defaults(config);

    if config.agent.default_provider.trim().is_empty()
        || !config
            .providers
            .contains_key(&config.agent.default_provider)
    {
        config.agent.default_provider = config
            .providers
            .iter()
            .find(|(_, p)| p.enabled.unwrap_or(true))
            .map(|(id, _)| id.clone())
            .unwrap_or_default();
    }
    config
}

fn attach_legacy_provider_env(id: &str, provider: &mut ProviderConsoleConfig) {
    let env = match id {
        "local" => "LOCAL_VLM_API_KEY",
        "holo3" => "HAI_API_KEY",
        "anthropic" => "ANTHROPIC_API_KEY",
        "openai" => "OPENAI_API_KEY",
        _ => return,
    };
    if std::env::var_os(env).is_some() {
        provider.api_key_env = Some(env.to_string());
    }
}

fn apply_legacy_provider_defaults(id: &str, provider: &mut ProviderConsoleConfig) {
    let (endpoint, model) = match id {
        "local" => ("http://localhost:8000/v1", "local-vlm"),
        "holo3" => ("https://api.hcompany.ai/v1", "holo3-35b-a3b"),
        "anthropic" => ("https://api.anthropic.com/v1", "claude-haiku-4-5"),
        "openai" => ("https://api.openai.com/v1", "gpt-4o-mini"),
        _ => return,
    };
    if provider
        .endpoint
        .as_ref()
        .is_none_or(|v| v.trim().is_empty())
    {
        provider.endpoint = Some(endpoint.to_string());
    }
    if provider.model.as_ref().is_none_or(|v| v.trim().is_empty()) {
        provider.model = Some(model.to_string());
    }
}

fn with_provider_env_defaults(mut config: ConsoleConfig) -> ConsoleConfig {
    seed_env_provider(
        &mut config,
        "openai",
        "openai",
        "OpenAI",
        "https://api.openai.com/v1",
        "gpt-4o-mini",
        "OPENAI_API_KEY",
    );
    seed_env_provider(
        &mut config,
        "anthropic",
        "anthropic",
        "Anthropic",
        "https://api.anthropic.com/v1",
        "claude-haiku-4-5",
        "ANTHROPIC_API_KEY",
    );
    seed_env_provider(
        &mut config,
        "holo3",
        "openai",
        "holo3",
        "https://api.hcompany.ai/v1",
        "holo3-35b-a3b",
        "HAI_API_KEY",
    );
    seed_env_provider(
        &mut config,
        "local",
        "openai",
        "local",
        "http://localhost:8000/v1",
        "local-vlm",
        "LOCAL_VLM_API_KEY",
    );
    config
}

fn seed_env_provider(
    config: &mut ConsoleConfig,
    id: &str,
    provider_type: &str,
    name: &str,
    endpoint: &str,
    model: &str,
    api_key_env: &str,
) {
    if config.providers.contains_key(id) || std::env::var_os(api_key_env).is_none() {
        return;
    }
    config.providers.insert(
        id.to_string(),
        ProviderConsoleConfig {
            provider_type: Some(provider_type.to_string()),
            name: Some(name.to_string()),
            enabled: Some(true),
            endpoint: Some(endpoint.to_string()),
            model: Some(model.to_string()),
            api_format: None,
            api_key: None,
            api_key_env: Some(api_key_env.to_string()),
            coord_space: None,
            system_prompt: None,
        },
    );
}
