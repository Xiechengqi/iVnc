//! Anthropic Messages API provider.
//!
//! Targets `/v1/messages` with `x-api-key` auth and the standard
//! `anthropic-version: 2023-06-01` header. The provider relies on
//! `text_action_parser` to convert assistant text (or JSON-shaped tool_use
//! arguments) into `Action`s, mirroring the openai_compat fallback path.

use super::openai_compat::default_system_prompt;
use super::text_action_parser;
use crate::agent::provider::{
    ActionGrammar, BrainProvider, ProviderCapabilities, ProviderError, ProviderSession,
    ProviderTurn,
};
use crate::agent::types::{
    Action, CoordinateSpace, DisplayMetadata, History, Observation, ObservationFrame, ProviderUsage,
};
use async_trait::async_trait;
use base64::Engine;
use reqwest::header::CONTENT_TYPE;
use serde_json::{json, Value};
use std::time::Instant;

pub const ANTHROPIC_PROVIDER_NAME: &str = "anthropic";
pub const ANTHROPIC_MODEL_ENV: &str = "ANTHROPIC_MODEL";
pub const ANTHROPIC_ENDPOINT_ENV: &str = "ANTHROPIC_BASE_URL";
pub const ANTHROPIC_API_KEY_ENV: &str = "ANTHROPIC_API_KEY";
pub const ANTHROPIC_DEFAULT_MODEL: &str = "claude-haiku-4-5";
pub const ANTHROPIC_DEFAULT_ENDPOINT: &str = "https://api.anthropic.com/v1";
pub const ANTHROPIC_VERSION_HEADER: &str = "2023-06-01";

pub struct AnthropicProvider {
    endpoint: String,
    model: String,
    api_key: Option<String>,
    system_prompt: String,
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(endpoint: String, model: String, api_key: Option<String>, system_prompt: String) -> Self {
        Self {
            endpoint,
            model,
            api_key,
            system_prompt,
            client: reqwest::Client::new(),
        }
    }

    fn image_part(observation: &Observation) -> Result<Value, ProviderError> {
        let (media_type, bytes) = match &observation.frame {
            ObservationFrame::JpegBytes { bytes, .. } => ("image/jpeg", bytes),
            ObservationFrame::PngBytes { bytes, .. } => ("image/png", bytes),
            ObservationFrame::RawXrgb { .. } => {
                return Err(ProviderError::UnsupportedObservation(
                    "raw xrgb frames are not supported by the Anthropic provider".to_string(),
                ))
            }
        };
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        Ok(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": media_type,
                "data": data,
            }
        }))
    }

    fn build_body(
        &self,
        task: &str,
        observation: &Observation,
        history: &History,
    ) -> Result<Value, ProviderError> {
        let display = &observation.display;
        let history_text = history_text(history);
        let user_text = build_user_text(task, display, &history_text, observation.page_text.as_deref());
        let image_part = Self::image_part(observation)?;
        Ok(json!({
            "model": self.model,
            "max_tokens": 1024,
            "system": self.system_prompt,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": user_text
                        },
                        image_part,
                    ]
                }
            ]
        }))
    }

    fn parse_body(&self, body: Value, elapsed_ms: u64) -> Result<ProviderTurn, ProviderError> {
        let response_id = body
            .get("id")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let usage = body.get("usage").map(|u| {
            let input_tokens = u
                .get("input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let output_tokens = u
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let mut parsed = ProviderUsage {
                input_tokens,
                output_tokens,
                provider_latency_ms: elapsed_ms,
                cost_usd_micros: None,
            };
            parsed.cost_usd_micros = crate::agent::budget::estimate_cost_usd_micros(
                ANTHROPIC_PROVIDER_NAME,
                &self.model,
                &parsed,
            );
            parsed
        });

        let mut text_buf = String::new();
        let mut actions: Vec<Action> = Vec::new();

        if let Some(content) = body.get("content").and_then(Value::as_array) {
            for part in content {
                match part.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = part.get("text").and_then(Value::as_str) {
                            text_buf.push_str(t);
                            text_buf.push('\n');
                        }
                    }
                    Some("tool_use") => {
                        if let Some(input) = part.get("input") {
                            // tool_use.input is the action JSON; try to deserialize directly.
                            if let Ok(action) = serde_json::from_value::<Action>(input.clone()) {
                                actions.push(action);
                            } else {
                                text_buf.push_str(&input.to_string());
                                text_buf.push('\n');
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if actions.is_empty() && !text_buf.trim().is_empty() {
            actions = text_action_parser::parse_text_actions(text_buf.trim())
                .map_err(ProviderError::InvalidResponse)?;
        }
        if actions.is_empty() {
            return Err(ProviderError::InvalidResponse(
                "Anthropic response yielded no parseable actions".to_string(),
            ));
        }
        Ok(ProviderTurn {
            actions,
            usage,
            pending_safety_checks: Vec::new(),
            provider_response_id: response_id,
        })
    }
}

#[async_trait]
impl BrainProvider for AnthropicProvider {
    fn name(&self) -> &'static str {
        ANTHROPIC_PROVIDER_NAME
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            accepts_history_frames: 3,
            native_action_grammar: ActionGrammar::AnthropicComputerUse,
            coordinate_space: CoordinateSpace::ImagePixels,
            supports_streaming: false,
            max_screenshot_megapixels: 4.0,
            requires_api_key_env: Some(ANTHROPIC_API_KEY_ENV),
            supports_safety_ack: false,
            supports_window_actions: false,
        }
    }

    async fn next_action(
        &self,
        task: &str,
        observation: &Observation,
        history: &History,
        _session: &mut ProviderSession,
    ) -> Result<ProviderTurn, ProviderError> {
        let url = format!("{}/messages", self.endpoint.trim_end_matches('/'));
        let body = self.build_body(task, observation, history)?;
        let started = Instant::now();
        let mut request = self
            .client
            .post(&url)
            .header(CONTENT_TYPE, "application/json")
            .header("anthropic-version", ANTHROPIC_VERSION_HEADER)
            .json(&body);
        if let Some(key) = &self.api_key {
            request = request.header("x-api-key", key);
            // Some gateways accept either x-api-key or Authorization: Bearer.
            request = request.header(reqwest::header::AUTHORIZATION, format!("Bearer {}", key));
        }
        let response = request
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;
        let status = response.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(ProviderError::AuthError(status.to_string()));
        }
        if status.as_u16() == 429 {
            return Err(ProviderError::RateLimited { retry_after_ms: 1000 });
        }
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(ProviderError::NetworkError(format!(
                "Anthropic upstream {}: {}",
                status, text
            )));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;
        self.parse_body(body, started.elapsed().as_millis() as u64)
    }
}

pub fn build_anthropic_provider(model: Option<String>) -> AnthropicProvider {
    let saved = crate::console_config::provider(ANTHROPIC_PROVIDER_NAME);
    let model = model
        .or(saved.model)
        .or_else(|| std::env::var(ANTHROPIC_MODEL_ENV).ok())
        .unwrap_or_else(|| ANTHROPIC_DEFAULT_MODEL.to_string());
    let endpoint = saved
        .endpoint
        .or_else(|| std::env::var(ANTHROPIC_ENDPOINT_ENV).ok())
        .unwrap_or_else(|| ANTHROPIC_DEFAULT_ENDPOINT.to_string());
    let api_key = saved
        .api_key
        .or_else(|| std::env::var(ANTHROPIC_API_KEY_ENV).ok());
    let system_prompt = saved.system_prompt.unwrap_or_else(default_system_prompt);
    AnthropicProvider::new(endpoint, model, api_key, system_prompt)
}

fn history_text(history: &History) -> String {
    if history.steps.is_empty() {
        return "No prior steps.".to_string();
    }
    let mut out = String::from("Prior steps:\n");
    for (idx, step) in history.steps.iter().enumerate() {
        out.push_str(&format!(
            "{}. action={:?}; result={:?}; image={}x{} sha256={}\n",
            idx + 1,
            step.action,
            step.result,
            step.observation.image_width,
            step.observation.image_height,
            step.observation.sha256
        ));
    }
    out
}

fn build_user_text(
    task: &str,
    display: &DisplayMetadata,
    history_text: &str,
    page_text: Option<&str>,
) -> String {
    let mut out = format!(
        "Task: {}\nScreenshot image size: {}x{}.\nReturn actions in this image coordinate space.\n{}",
        task, display.image_width, display.image_height, history_text
    );
    if display.read_only {
        out.push_str(
            "\n[Read-only full-page capture] This frame is taller than the live viewport and its \
             coordinates are NOT clickable. To interact, call screenshot to return to the live \
             viewport, then scroll and click on that frame.",
        );
    }
    if let Some(text) = page_text.map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str("\n[Extracted page text]\n");
        out.push_str(text);
    }
    out
}
