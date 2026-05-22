use super::openai_compat::{
    default_system_prompt, OpenAiApiFormat, OpenAiCompatConfig, OpenAiCompatibleProvider,
};
use crate::agent::types::CoordinateSpace;

pub const HOLO3_PROVIDER_NAME: &str = "holo3";
pub const HOLO3_MODEL_ENV: &str = "HOLO3_MODEL";
pub const HOLO3_ENDPOINT_ENV: &str = "HOLO3_BASE_URL";
pub const HOLO3_API_KEY_ENV: &str = "HAI_API_KEY";
pub const HOLO3_DEFAULT_MODEL: &str = "holo3-35b-a3b";
pub const HOLO3_DEFAULT_ENDPOINT: &str = "https://api.hcompany.ai/v1";

pub fn build_holo3_provider(model: Option<String>) -> OpenAiCompatibleProvider {
    let saved = crate::console_config::provider(HOLO3_PROVIDER_NAME);
    let model = model
        .or(saved.model)
        .or_else(|| std::env::var(HOLO3_MODEL_ENV).ok())
        .unwrap_or_else(|| HOLO3_DEFAULT_MODEL.to_string());
    let endpoint = saved
        .endpoint
        .or_else(|| std::env::var(HOLO3_ENDPOINT_ENV).ok())
        .unwrap_or_else(|| HOLO3_DEFAULT_ENDPOINT.to_string());
    let api_format = OpenAiApiFormat::from_config(saved.api_format.as_deref());
    OpenAiCompatibleProvider::new(OpenAiCompatConfig {
        provider_name: HOLO3_PROVIDER_NAME,
        endpoint,
        model,
        api_format,
        api_key: saved
            .api_key
            .or_else(|| std::env::var(HOLO3_API_KEY_ENV).ok()),
        api_key_env: Some(HOLO3_API_KEY_ENV),
        coordinate_space: CoordinateSpace::ImagePixels,
        system_prompt: default_system_prompt(),
    })
}
