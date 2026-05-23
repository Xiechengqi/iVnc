use super::provider::{ActionGrammar, BrainProvider, ProviderCapabilities};
use super::providers::{
    build_anthropic_provider, build_gemini_provider, build_holo3_provider, build_local_provider,
    build_openai_provider, ReplayProvider,
};
use super::types::{Action, CoordinateSpace};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct ProviderPreset {
    pub name: &'static str,
    pub default_model: &'static str,
    pub default_endpoint: &'static str,
    pub api_key_env: Option<&'static str>,
    pub capabilities: ProviderCapabilities,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderInfo {
    pub name: &'static str,
    pub default_model: &'static str,
    pub default_endpoint: &'static str,
    pub api_key_env: Option<&'static str>,
    pub configured: bool,
    pub capabilities: ProviderCapabilities,
}

pub fn provider_presets() -> Vec<ProviderPreset> {
    vec![
        ProviderPreset {
            name: "replay",
            default_model: "replay",
            default_endpoint: "",
            api_key_env: None,
            capabilities: replay_capabilities(),
        },
        ProviderPreset {
            name: "holo3",
            default_model: super::providers::holo3::HOLO3_DEFAULT_MODEL,
            default_endpoint: super::providers::holo3::HOLO3_DEFAULT_ENDPOINT,
            api_key_env: Some(super::providers::holo3::HOLO3_API_KEY_ENV),
            capabilities: openai_compatible_capabilities(Some(
                super::providers::holo3::HOLO3_API_KEY_ENV,
            )),
        },
        ProviderPreset {
            name: "local",
            default_model: super::providers::local_vlm::LOCAL_DEFAULT_MODEL,
            default_endpoint: super::providers::local_vlm::LOCAL_DEFAULT_ENDPOINT,
            api_key_env: None,
            capabilities: openai_compatible_capabilities(None),
        },
        ProviderPreset {
            name: super::providers::openai::OPENAI_PROVIDER_NAME,
            default_model: super::providers::openai::OPENAI_DEFAULT_MODEL,
            default_endpoint: super::providers::openai::OPENAI_DEFAULT_ENDPOINT,
            api_key_env: Some(super::providers::openai::OPENAI_API_KEY_ENV),
            capabilities: openai_compatible_capabilities(Some(
                super::providers::openai::OPENAI_API_KEY_ENV,
            )),
        },
        ProviderPreset {
            name: super::providers::anthropic::ANTHROPIC_PROVIDER_NAME,
            default_model: super::providers::anthropic::ANTHROPIC_DEFAULT_MODEL,
            default_endpoint: super::providers::anthropic::ANTHROPIC_DEFAULT_ENDPOINT,
            api_key_env: Some(super::providers::anthropic::ANTHROPIC_API_KEY_ENV),
            capabilities: anthropic_capabilities(),
        },
        ProviderPreset {
            name: super::providers::gemini::GEMINI_PROVIDER_NAME,
            default_model: super::providers::gemini::GEMINI_DEFAULT_MODEL,
            default_endpoint: super::providers::gemini::GEMINI_DEFAULT_ENDPOINT,
            api_key_env: Some(super::providers::gemini::GEMINI_API_KEY_ENV),
            capabilities: gemini_capabilities(),
        },
    ]
}

pub fn provider_infos() -> Vec<ProviderInfo> {
    provider_presets()
        .into_iter()
        .map(|p| {
            let configured = provider_configured(&p);
            ProviderInfo {
                name: p.name,
                default_model: p.default_model,
                default_endpoint: p.default_endpoint,
                api_key_env: p.api_key_env,
                capabilities: p.capabilities,
                configured,
            }
        })
        .collect()
}

pub fn build_provider(
    name: &str,
    model: Option<String>,
    replay_actions: Option<Vec<Action>>,
) -> Option<Arc<dyn BrainProvider>> {
    match name {
        "replay" => Some(Arc::new(ReplayProvider::new(
            replay_actions.unwrap_or_else(|| {
                vec![Action::Done {
                    success: true,
                    reason: "empty replay".to_string(),
                    output: String::new(),
                }]
            }),
        ))),
        "holo3"
            if std::env::var_os(super::providers::holo3::HOLO3_API_KEY_ENV).is_some()
                || crate::console_config::provider("holo3").api_key.is_some() =>
        {
            Some(Arc::new(build_holo3_provider(model)))
        }
        "local" => Some(Arc::new(build_local_provider(model))),
        "openai"
            if std::env::var_os(super::providers::openai::OPENAI_API_KEY_ENV).is_some()
                || crate::console_config::provider("openai").api_key.is_some() =>
        {
            Some(Arc::new(build_openai_provider(model)))
        }
        "anthropic"
            if std::env::var_os(super::providers::anthropic::ANTHROPIC_API_KEY_ENV).is_some()
                || crate::console_config::provider("anthropic").api_key.is_some() =>
        {
            Some(Arc::new(build_anthropic_provider(model)))
        }
        "gemini"
            if std::env::var_os(super::providers::gemini::GEMINI_API_KEY_ENV).is_some()
                || crate::console_config::provider("gemini").api_key.is_some() =>
        {
            Some(Arc::new(build_gemini_provider(model)))
        }
        _ => None,
    }
}

fn provider_configured(preset: &ProviderPreset) -> bool {
    match preset.name {
        "replay" => true,
        "local" => true,
        "holo3" => {
            std::env::var_os(super::providers::holo3::HOLO3_API_KEY_ENV).is_some()
                || crate::console_config::provider("holo3").api_key.is_some()
        }
        "openai" => {
            std::env::var_os(super::providers::openai::OPENAI_API_KEY_ENV).is_some()
                || crate::console_config::provider("openai").api_key.is_some()
        }
        "anthropic" => {
            std::env::var_os(super::providers::anthropic::ANTHROPIC_API_KEY_ENV).is_some()
                || crate::console_config::provider("anthropic").api_key.is_some()
        }
        "gemini" => {
            std::env::var_os(super::providers::gemini::GEMINI_API_KEY_ENV).is_some()
                || crate::console_config::provider("gemini").api_key.is_some()
        }
        _ => preset
            .api_key_env
            .map(std::env::var_os)
            .map_or(true, |v| v.is_some()),
    }
}

fn replay_capabilities() -> ProviderCapabilities {
    ProviderCapabilities {
        accepts_history_frames: 0,
        native_action_grammar: ActionGrammar::Replay,
        coordinate_space: CoordinateSpace::ImagePixels,
        supports_streaming: false,
        max_screenshot_megapixels: 16.0,
        requires_api_key_env: None,
        supports_safety_ack: false,
        supports_window_actions: true,
    }
}

fn openai_compatible_capabilities(api_key_env: Option<&'static str>) -> ProviderCapabilities {
    ProviderCapabilities {
        accepts_history_frames: 3,
        native_action_grammar: ActionGrammar::OpenAiToolCall,
        coordinate_space: CoordinateSpace::ImagePixels,
        supports_streaming: false,
        max_screenshot_megapixels: 4.0,
        requires_api_key_env: api_key_env,
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
        requires_api_key_env: Some(super::providers::anthropic::ANTHROPIC_API_KEY_ENV),
        supports_safety_ack: false,
        supports_window_actions: false,
    }
}

fn gemini_capabilities() -> ProviderCapabilities {
    ProviderCapabilities {
        accepts_history_frames: 3,
        native_action_grammar: ActionGrammar::GeminiComputerUse,
        coordinate_space: CoordinateSpace::ImagePixels,
        supports_streaming: false,
        max_screenshot_megapixels: 4.0,
        requires_api_key_env: Some(super::providers::gemini::GEMINI_API_KEY_ENV),
        supports_safety_ack: false,
        supports_window_actions: false,
    }
}
