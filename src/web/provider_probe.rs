//! Minimal connectivity probe for built-in providers.
//!
//! Sends a tiny "ping" request to the provider's chat/messages/generateContent
//! endpoint to validate that the configured endpoint + model + api_key tuple
//! actually round-trips. This is intentionally separate from the
//! `BrainProvider` trait — we don't need an `Observation`, we just want to
//! tell apart "network/auth/model is broken" from "agent prompt is broken".

use serde_json::{json, Value};
use std::time::{Duration, Instant};

const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const PREVIEW_MAX: usize = 200;
const ERROR_BODY_MAX: usize = 300;

#[derive(Debug)]
pub struct ProbeOk {
    pub latency_ms: u64,
    pub status_code: u16,
    pub preview: Option<String>,
}

#[derive(Debug)]
pub struct ProbeErr {
    pub latency_ms: u64,
    pub status_code: Option<u16>,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub enum ProbeShape {
    ChatCompletions,
    Responses,
    AnthropicMessages,
    GeminiGenerate,
}

pub fn shape_for(provider: &str, api_format: Option<&str>) -> Option<ProbeShape> {
    match provider {
        "anthropic" => Some(ProbeShape::AnthropicMessages),
        "gemini" => Some(ProbeShape::GeminiGenerate),
        "openai" | "holo3" | "local" | "openai_compat" => match api_format {
            Some("responses") => Some(ProbeShape::Responses),
            _ => Some(ProbeShape::ChatCompletions),
        },
        _ => None,
    }
}

pub async fn probe(
    shape: ProbeShape,
    endpoint: &str,
    model: &str,
    api_key: Option<&str>,
) -> Result<ProbeOk, ProbeErr> {
    let client = match reqwest::Client::builder().timeout(PROBE_TIMEOUT).build() {
        Ok(c) => c,
        Err(e) => {
            return Err(ProbeErr {
                latency_ms: 0,
                status_code: None,
                message: format!("client build failed: {}", e),
            });
        }
    };

    let endpoint = endpoint.trim_end_matches('/');
    let started = Instant::now();
    let result = match shape {
        ProbeShape::ChatCompletions => {
            let body = json!({
                "model": model,
                "messages": [{"role": "user", "content": "ping"}],
                "max_tokens": 4,
            });
            let mut req = client.post(format!("{}/chat/completions", endpoint)).json(&body);
            if let Some(k) = api_key {
                req = req.bearer_auth(k);
            }
            req.send().await
        }
        ProbeShape::Responses => {
            let body = json!({
                "model": model,
                "input": "ping",
                "max_output_tokens": 16,
            });
            let mut req = client.post(format!("{}/responses", endpoint)).json(&body);
            if let Some(k) = api_key {
                req = req.bearer_auth(k);
            }
            req.send().await
        }
        ProbeShape::AnthropicMessages => {
            let body = json!({
                "model": model,
                "max_tokens": 4,
                "messages": [{"role": "user", "content": "ping"}],
            });
            let mut req = client
                .post(format!("{}/messages", endpoint))
                .header("anthropic-version", "2023-06-01")
                .json(&body);
            if let Some(k) = api_key {
                req = req.header("x-api-key", k);
            }
            req.send().await
        }
        ProbeShape::GeminiGenerate => {
            let body = json!({
                "contents": [{"parts": [{"text": "ping"}]}],
                "generationConfig": {"maxOutputTokens": 8},
            });
            let mut req = client
                .post(format!("{}/models/{}:generateContent", endpoint, model))
                .json(&body);
            if let Some(k) = api_key {
                req = req.header("x-goog-api-key", k);
            }
            req.send().await
        }
    };

    let latency_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    let resp = match result {
        Ok(r) => r,
        Err(e) => {
            let kind = if e.is_timeout() {
                "timeout"
            } else if e.is_connect() {
                "connect error"
            } else {
                "network error"
            };
            return Err(ProbeErr {
                latency_ms,
                status_code: None,
                message: format!("{}: {}", kind, e),
            });
        }
    };

    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    if status.is_success() {
        let preview = extract_preview(&body_text);
        Ok(ProbeOk {
            latency_ms,
            status_code: status.as_u16(),
            preview,
        })
    } else {
        Err(ProbeErr {
            latency_ms,
            status_code: Some(status.as_u16()),
            message: truncate(&body_text, ERROR_BODY_MAX),
        })
    }
}

fn extract_preview(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    // chat_completions: choices[0].message.content
    if let Some(s) = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
    {
        return Some(truncate(s, PREVIEW_MAX));
    }
    // responses: output[].content[].text or output_text
    if let Some(s) = v.get("output_text").and_then(|c| c.as_str()) {
        return Some(truncate(s, PREVIEW_MAX));
    }
    if let Some(s) = v
        .get("output")
        .and_then(|o| o.get(0))
        .and_then(|o| o.get("content"))
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
    {
        return Some(truncate(s, PREVIEW_MAX));
    }
    // anthropic: content[0].text
    if let Some(s) = v
        .get("content")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
    {
        return Some(truncate(s, PREVIEW_MAX));
    }
    // gemini: candidates[0].content.parts[0].text
    if let Some(s) = v
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.get(0))
        .and_then(|p| p.get("text"))
        .and_then(|t| t.as_str())
    {
        return Some(truncate(s, PREVIEW_MAX));
    }
    None
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}
