//! Google Gemini `generateContent` provider.
//!
//! Targets `/v1beta/models/{model}:generateContent` with `x-goog-api-key` auth.
//! Like the Anthropic provider, this leans on `text_action_parser` to convert
//! assistant text (or `functionCall.args` JSON) into `Action`s.

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

pub const GEMINI_PROVIDER_NAME: &str = "gemini";
pub const GEMINI_MODEL_ENV: &str = "GEMINI_MODEL";
pub const GEMINI_ENDPOINT_ENV: &str = "GEMINI_BASE_URL";
pub const GEMINI_API_KEY_ENV: &str = "GEMINI_API_KEY";
pub const GEMINI_DEFAULT_MODEL: &str = "gemini-2.0-flash";
pub const GEMINI_DEFAULT_ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta";

pub struct GeminiProvider {
    endpoint: String,
    model: String,
    api_key: Option<String>,
    system_prompt: String,
    client: reqwest::Client,
}

impl GeminiProvider {
    pub fn new(
        endpoint: String,
        model: String,
        api_key: Option<String>,
        system_prompt: String,
    ) -> Self {
        Self {
            endpoint,
            model,
            api_key,
            system_prompt,
            client: reqwest::Client::new(),
        }
    }

    fn image_part(observation: &Observation) -> Result<Value, ProviderError> {
        let (mime, bytes) = match &observation.frame {
            ObservationFrame::JpegBytes { bytes, .. } => ("image/jpeg", bytes),
            ObservationFrame::PngBytes { bytes, .. } => ("image/png", bytes),
            ObservationFrame::RawXrgb { .. } => {
                return Err(ProviderError::UnsupportedObservation(
                    "raw xrgb frames are not supported by the Gemini provider".to_string(),
                ))
            }
        };
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        Ok(json!({
            "inlineData": {
                "mimeType": mime,
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
        let image_part = Self::image_part(observation)?;
        let user_text = json!({
            "text": build_user_text(task, display, &history_text, observation.page_text.as_deref())
        });
        Ok(json!({
            "systemInstruction": {
                "parts": [{ "text": self.system_prompt }]
            },
            "contents": [
                {
                    "role": "user",
                    "parts": [user_text, image_part]
                }
            ],
            "generationConfig": {
                "maxOutputTokens": 1024
            }
        }))
    }

    fn parse_body(&self, body: Value, elapsed_ms: u64) -> Result<ProviderTurn, ProviderError> {
        let response_id = body
            .get("responseId")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let usage = body.get("usageMetadata").map(|u| {
            let input_tokens = u
                .get("promptTokenCount")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let output_tokens = u
                .get("candidatesTokenCount")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let mut parsed = ProviderUsage {
                input_tokens,
                output_tokens,
                provider_latency_ms: elapsed_ms,
                cost_usd_micros: None,
            };
            parsed.cost_usd_micros = crate::agent::budget::estimate_cost_usd_micros(
                GEMINI_PROVIDER_NAME,
                &self.model,
                &parsed,
            );
            parsed
        });

        let mut text_buf = String::new();
        let mut actions: Vec<Action> = Vec::new();

        if let Some(candidates) = body.get("candidates").and_then(Value::as_array) {
            for cand in candidates {
                if let Some(parts) = cand
                    .get("content")
                    .and_then(|c| c.get("parts"))
                    .and_then(Value::as_array)
                {
                    for part in parts {
                        if let Some(t) = part.get("text").and_then(Value::as_str) {
                            text_buf.push_str(t);
                            text_buf.push('\n');
                        }
                        if let Some(call) = part.get("functionCall") {
                            if let Some(args) = call.get("args") {
                                if let Ok(action) =
                                    serde_json::from_value::<Action>(args.clone())
                                {
                                    actions.push(action);
                                } else {
                                    text_buf.push_str(&args.to_string());
                                    text_buf.push('\n');
                                }
                            }
                        }
                    }
                }
            }
        }

        if actions.is_empty() && !text_buf.trim().is_empty() {
            actions = text_action_parser::parse_text_actions(text_buf.trim())
                .map_err(ProviderError::InvalidResponse)?;
        }
        if actions.is_empty() {
            return Err(ProviderError::InvalidResponse(
                "Gemini response yielded no parseable actions".to_string(),
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
impl BrainProvider for GeminiProvider {
    fn name(&self) -> &'static str {
        GEMINI_PROVIDER_NAME
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            accepts_history_frames: 3,
            native_action_grammar: ActionGrammar::GeminiComputerUse,
            coordinate_space: CoordinateSpace::ImagePixels,
            supports_streaming: false,
            max_screenshot_megapixels: 4.0,
            requires_api_key_env: Some(GEMINI_API_KEY_ENV),
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
        let url = format!(
            "{}/models/{}:generateContent",
            self.endpoint.trim_end_matches('/'),
            self.model
        );
        let body = self.build_body(task, observation, history)?;
        let started = Instant::now();
        let mut request = self
            .client
            .post(&url)
            .header(CONTENT_TYPE, "application/json")
            .json(&body);
        if let Some(key) = &self.api_key {
            request = request.header("x-goog-api-key", key);
            // Some gateways accept either x-goog-api-key or Authorization: Bearer.
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
                "Gemini upstream {}: {}",
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

pub fn build_gemini_provider(model: Option<String>) -> GeminiProvider {
    let saved = crate::console_config::provider(GEMINI_PROVIDER_NAME);
    let model = model
        .or(saved.model)
        .or_else(|| std::env::var(GEMINI_MODEL_ENV).ok())
        .unwrap_or_else(|| GEMINI_DEFAULT_MODEL.to_string());
    let endpoint = saved
        .endpoint
        .or_else(|| std::env::var(GEMINI_ENDPOINT_ENV).ok())
        .unwrap_or_else(|| GEMINI_DEFAULT_ENDPOINT.to_string());
    let api_key = saved
        .api_key
        .or_else(|| std::env::var(GEMINI_API_KEY_ENV).ok());
    let system_prompt = saved.system_prompt.unwrap_or_else(default_system_prompt);
    GeminiProvider::new(endpoint, model, api_key, system_prompt)
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
