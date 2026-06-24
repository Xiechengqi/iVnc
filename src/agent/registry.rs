use super::provider::{ActionGrammar, BrainProvider, ProviderCapabilities};
use super::providers::{
    openai_compat, AnthropicProvider, OpenAiCompatibleProvider, ReplayProvider,
};
use super::types::{Action, CoordinateSpace};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
pub struct ProviderInfo {
    pub name: String,
    pub display_name: String,
    pub provider_type: String,
    pub default_model: String,
    pub default_endpoint: String,
    pub api_key_env: Option<&'static str>,
    pub configured: bool,
    pub capabilities: ProviderCapabilities,
}

pub fn provider_infos() -> Vec<ProviderInfo> {
    crate::console_config::load()
        .providers
        .into_iter()
        .filter_map(|(id, cfg)| provider_info_from_config(&id, &cfg))
        .collect()
}

pub fn build_provider(
    name: &str,
    model: Option<String>,
    replay_actions: Option<Vec<Action>>,
) -> Option<Arc<dyn BrainProvider>> {
    if name == "replay" {
        return Some(Arc::new(ReplayProvider::new(
            replay_actions.unwrap_or_else(|| {
                vec![Action::Done {
                    success: true,
                    reason: "empty replay".to_string(),
                    output: String::new(),
                }]
            }),
        )));
    }

    let config = crate::console_config::load();
    let cfg = config.providers.get(name)?.clone();
    if !cfg.enabled.unwrap_or(true) {
        return None;
    }
    let provider_type = normalized_provider_type(cfg.provider_type.as_deref());
    match provider_type.as_deref() {
        Some("anthropic") => {
            build_dynamic_anthropic(name, cfg, model).map(|p| Arc::new(p) as Arc<dyn BrainProvider>)
        }
        Some("openai") => {
            build_dynamic_openai(name, cfg, model).map(|p| Arc::new(p) as Arc<dyn BrainProvider>)
        }
        _ => None,
    }
}

pub fn provider_info_from_config(
    id: &str,
    cfg: &crate::console_config::ProviderConsoleConfig,
) -> Option<ProviderInfo> {
    if !cfg.enabled.unwrap_or(true) {
        return None;
    }
    let provider_type = normalized_provider_type(cfg.provider_type.as_deref())?;
    let (default_endpoint, default_model, capabilities) = defaults_for_type(&provider_type, cfg);
    let endpoint = cfg
        .endpoint
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(default_endpoint);
    let model = cfg
        .model
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(default_model);
    let api_key_configured = cfg.api_key.as_ref().is_some_and(|v| !v.trim().is_empty())
        || cfg
            .api_key_env
            .as_deref()
            .and_then(|env| std::env::var(env).ok())
            .is_some_and(|v| !v.trim().is_empty());
    let configured = !endpoint.is_empty()
        && !model.is_empty()
        && (provider_type == "openai" || api_key_configured);
    Some(ProviderInfo {
        name: id.to_string(),
        display_name: cfg
            .name
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(id)
            .to_string(),
        provider_type,
        default_model: default_model.to_string(),
        default_endpoint: default_endpoint.to_string(),
        api_key_env: None,
        configured,
        capabilities,
    })
}

fn build_dynamic_openai(
    id: &str,
    cfg: crate::console_config::ProviderConsoleConfig,
    model_override: Option<String>,
) -> Option<OpenAiCompatibleProvider> {
    let model = model_override
        .or(cfg.model)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "gpt-4o-mini".to_string());
    let endpoint = cfg
        .endpoint
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let system_prompt = cfg
        .system_prompt
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(openai_compat::default_system_prompt);
    Some(OpenAiCompatibleProvider::new(
        openai_compat::OpenAiCompatConfig {
            provider_name: id.to_string(),
            endpoint,
            model,
            api_format: openai_compat::OpenAiApiFormat::from_config(cfg.api_format.as_deref()),
            api_key: cfg
                .api_key
                .or_else(|| cfg.api_key_env.and_then(|env| std::env::var(env).ok())),
            api_key_env: None,
            coordinate_space: parse_coordinate_space(cfg.coord_space.as_deref()),
            system_prompt,
        },
    ))
}

fn build_dynamic_anthropic(
    id: &str,
    cfg: crate::console_config::ProviderConsoleConfig,
    model_override: Option<String>,
) -> Option<AnthropicProvider> {
    let model = model_override
        .or(cfg.model)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "claude-haiku-4-5".to_string());
    let endpoint = cfg
        .endpoint
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "https://api.anthropic.com/v1".to_string());
    let system_prompt = cfg
        .system_prompt
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(openai_compat::default_system_prompt);
    Some(AnthropicProvider::with_name(
        id.to_string(),
        endpoint,
        model,
        cfg.api_key
            .or_else(|| cfg.api_key_env.and_then(|env| std::env::var(env).ok())),
        None,
        system_prompt,
    ))
}

pub fn normalized_provider_type(value: Option<&str>) -> Option<String> {
    match value
        .unwrap_or("openai")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "openai" | "openai_compat" | "openai-compatible" | "openai_compatible" => {
            Some("openai".to_string())
        }
        "anthropic" | "claude" => Some("anthropic".to_string()),
        _ => None,
    }
}

pub fn defaults_for_type(
    provider_type: &str,
    cfg: &crate::console_config::ProviderConsoleConfig,
) -> (&'static str, &'static str, ProviderCapabilities) {
    match provider_type {
        "anthropic" => (
            "https://api.anthropic.com/v1",
            "claude-haiku-4-5",
            anthropic_capabilities(),
        ),
        _ => {
            let coordinate_space = parse_coordinate_space(cfg.coord_space.as_deref());
            (
                "https://api.openai.com/v1",
                "gpt-4o-mini",
                openai_compatible_capabilities(coordinate_space),
            )
        }
    }
}

fn openai_compatible_capabilities(coordinate_space: CoordinateSpace) -> ProviderCapabilities {
    ProviderCapabilities {
        accepts_history_frames: 3,
        native_action_grammar: ActionGrammar::OpenAiToolCall,
        coordinate_space,
        supports_streaming: false,
        max_screenshot_megapixels: 4.0,
        requires_api_key_env: None,
        supports_safety_ack: false,
        supports_window_actions: false,
    }
}

fn anthropic_capabilities() -> ProviderCapabilities {
    ProviderCapabilities {
        accepts_history_frames: 3,
        native_action_grammar: ActionGrammar::AnthropicComputerUse,
        coordinate_space: CoordinateSpace::ImagePixels,
        supports_streaming: false,
        max_screenshot_megapixels: 4.0,
        requires_api_key_env: None,
        supports_safety_ack: false,
        supports_window_actions: false,
    }
}

fn parse_coordinate_space(saved: Option<&str>) -> CoordinateSpace {
    match saved
        .unwrap_or("image")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "screen" | "screen_pixels" => CoordinateSpace::ScreenPixels,
        "normalized" | "normalized_unit" | "gemini" => CoordinateSpace::NormalizedUnit,
        _ => CoordinateSpace::ImagePixels,
    }
}
