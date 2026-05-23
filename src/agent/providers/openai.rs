use super::openai_compat::{
    default_system_prompt, OpenAiApiFormat, OpenAiCompatConfig, OpenAiCompatibleProvider,
};
use crate::agent::types::CoordinateSpace;

pub const OPENAI_PROVIDER_NAME: &str = "openai";
pub const OPENAI_MODEL_ENV: &str = "OPENAI_MODEL";
pub const OPENAI_ENDPOINT_ENV: &str = "OPENAI_BASE_URL";
pub const OPENAI_API_KEY_ENV: &str = "OPENAI_API_KEY";
pub const OPENAI_DEFAULT_MODEL: &str = "gpt-4o-mini";
pub const OPENAI_DEFAULT_ENDPOINT: &str = "https://api.openai.com/v1";

pub fn build_openai_provider(model: Option<String>) -> OpenAiCompatibleProvider {
    let saved = crate::console_config::provider(OPENAI_PROVIDER_NAME);
    let model = model
        .or(saved.model)
        .or_else(|| std::env::var(OPENAI_MODEL_ENV).ok())
        .unwrap_or_else(|| OPENAI_DEFAULT_MODEL.to_string());
    let endpoint = saved
        .endpoint
        .or_else(|| std::env::var(OPENAI_ENDPOINT_ENV).ok())
        .unwrap_or_else(|| OPENAI_DEFAULT_ENDPOINT.to_string());
    let api_format = OpenAiApiFormat::from_config(saved.api_format.as_deref());
    OpenAiCompatibleProvider::new(OpenAiCompatConfig {
        provider_name: OPENAI_PROVIDER_NAME,
        endpoint,
        model,
        api_format,
        api_key: saved
            .api_key
            .or_else(|| std::env::var(OPENAI_API_KEY_ENV).ok()),
        api_key_env: Some(OPENAI_API_KEY_ENV),
        coordinate_space: CoordinateSpace::ImagePixels,
        system_prompt: saved.system_prompt.unwrap_or_else(default_system_prompt),
    })
}
