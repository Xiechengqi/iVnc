use super::openai_compat::{
    default_system_prompt, OpenAiApiFormat, OpenAiCompatConfig, OpenAiCompatibleProvider,
};
use crate::agent::types::CoordinateSpace;

pub const LOCAL_PROVIDER_NAME: &str = "local";
pub const LOCAL_MODEL_ENV: &str = "LOCAL_VLM_MODEL";
pub const LOCAL_ENDPOINT_ENV: &str = "LOCAL_VLM_URL";
pub const LOCAL_API_KEY_ENV: &str = "LOCAL_VLM_API_KEY";
pub const LOCAL_API_FORMAT_ENV: &str = "LOCAL_VLM_API_FORMAT";
pub const LOCAL_COORD_SPACE_ENV: &str = "LOCAL_VLM_COORD_SPACE";
pub const LOCAL_SYSTEM_PROMPT_ENV: &str = "LOCAL_VLM_SYSTEM_PROMPT";
pub const LOCAL_DEFAULT_MODEL: &str = "local-vlm";
pub const LOCAL_DEFAULT_ENDPOINT: &str = "http://localhost:8000/v1";

pub fn build_local_provider(model: Option<String>) -> OpenAiCompatibleProvider {
    let saved = crate::console_config::provider(LOCAL_PROVIDER_NAME);
    let model = model
        .or(saved.model)
        .or_else(|| std::env::var(LOCAL_MODEL_ENV).ok())
        .unwrap_or_else(|| LOCAL_DEFAULT_MODEL.to_string());
    let endpoint = saved
        .endpoint
        .or_else(|| std::env::var(LOCAL_ENDPOINT_ENV).ok())
        .unwrap_or_else(|| LOCAL_DEFAULT_ENDPOINT.to_string());
    let system_prompt = saved
        .system_prompt
        .or_else(|| std::env::var(LOCAL_SYSTEM_PROMPT_ENV).ok())
        .unwrap_or_else(default_system_prompt);
    let api_format = saved
        .api_format
        .or_else(|| std::env::var(LOCAL_API_FORMAT_ENV).ok());
    OpenAiCompatibleProvider::new(OpenAiCompatConfig {
        provider_name: LOCAL_PROVIDER_NAME.to_string(),
        endpoint,
        model,
        api_format: OpenAiApiFormat::from_config(api_format.as_deref()),
        api_key: saved
            .api_key
            .or_else(|| std::env::var(LOCAL_API_KEY_ENV).ok()),
        api_key_env: None,
        coordinate_space: parse_coordinate_space(saved.coord_space.as_deref()),
        system_prompt,
    })
}

fn parse_coordinate_space(saved: Option<&str>) -> CoordinateSpace {
    match saved
        .map(ToString::to_string)
        .or_else(|| std::env::var(LOCAL_COORD_SPACE_ENV).ok())
        .unwrap_or_else(|| "image".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "screen" | "screen_pixels" => CoordinateSpace::ScreenPixels,
        "normalized" | "normalized_unit" | "gemini" => CoordinateSpace::NormalizedUnit,
        _ => CoordinateSpace::ImagePixels,
    }
}
