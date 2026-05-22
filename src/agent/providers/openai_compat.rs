use super::text_action_parser;
use crate::agent::provider::{
    ActionGrammar, BrainProvider, ProviderCapabilities, ProviderError, ProviderSession,
    ProviderTurn,
};
use crate::agent::types::{
    Action, CoordinateSpace, DisplayMetadata, History, MouseButton, Observation, ObservationFrame,
    ProviderUsage,
};
use async_trait::async_trait;
use base64::Engine;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct OpenAiCompatConfig {
    pub provider_name: &'static str,
    pub endpoint: String,
    pub model: String,
    pub api_format: OpenAiApiFormat,
    pub api_key: Option<String>,
    pub api_key_env: Option<&'static str>,
    pub coordinate_space: CoordinateSpace,
    pub system_prompt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiApiFormat {
    ChatCompletions,
    Responses,
}

impl OpenAiApiFormat {
    pub fn from_config(value: Option<&str>) -> Self {
        match value
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "responses" | "openai_responses" | "openai-responses" | "response" => Self::Responses,
            _ => Self::ChatCompletions,
        }
    }

    fn endpoint_path(self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat/completions",
            Self::Responses => "responses",
        }
    }
}

pub struct OpenAiCompatibleProvider {
    cfg: OpenAiCompatConfig,
    client: reqwest::Client,
}

impl OpenAiCompatibleProvider {
    pub fn new(cfg: OpenAiCompatConfig) -> Self {
        Self {
            cfg,
            client: reqwest::Client::new(),
        }
    }

    fn image_data_url(observation: &Observation) -> Result<String, ProviderError> {
        match &observation.frame {
            ObservationFrame::JpegBytes { bytes, .. } => Ok(format!(
                "data:image/jpeg;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            )),
            ObservationFrame::PngBytes { bytes, .. } => Ok(format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            )),
            ObservationFrame::RawXrgb { .. } => Err(ProviderError::UnsupportedObservation(
                "raw xrgb frames are not supported by OpenAI-compatible HTTP providers".to_string(),
            )),
        }
    }

    fn request_body(
        &self,
        task: &str,
        observation: &Observation,
        history: &History,
    ) -> Result<Value, ProviderError> {
        let image_url = Self::image_data_url(observation)?;
        let display = &observation.display;
        let history_text = history_text(history);
        Ok(json!({
            "model": self.cfg.model,
            "messages": [
                {
                    "role": "system",
                    "content": self.cfg.system_prompt
                },
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": format!(
                                "Task: {}\nScreenshot image size: {}x{}.\nReturn actions in this image coordinate space.\n{}",
                                task,
                                display.image_width,
                                display.image_height,
                                history_text
                            )
                        },
                        {
                            "type": "image_url",
                            "image_url": { "url": image_url }
                        }
                    ]
                }
            ],
            "tools": tool_schema(),
            "tool_choice": "auto",
            "temperature": 0.0
        }))
    }

    fn responses_request_body(
        &self,
        task: &str,
        observation: &Observation,
        history: &History,
    ) -> Result<Value, ProviderError> {
        let image_url = Self::image_data_url(observation)?;
        let display = &observation.display;
        let history_text = history_text(history);
        Ok(json!({
            "model": self.cfg.model,
            "instructions": self.cfg.system_prompt,
            "input": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": format!(
                                "Task: {}\nScreenshot image size: {}x{}.\nReturn actions in this image coordinate space.\n{}",
                                task,
                                display.image_width,
                                display.image_height,
                                history_text
                            )
                        },
                        {
                            "type": "input_image",
                            "image_url": image_url
                        }
                    ]
                }
            ],
            "tools": responses_tool_schema(),
            "tool_choice": "auto"
        }))
    }

    fn parse_response(
        &self,
        body: Value,
        display: &DisplayMetadata,
        elapsed_ms: u64,
    ) -> Result<ProviderTurn, ProviderError> {
        let response_id = body
            .get("id")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let usage = parse_usage(body.get("usage"), elapsed_ms);
        let message = body
            .pointer("/choices/0/message")
            .ok_or_else(|| ProviderError::InvalidResponse("missing choices[0].message".into()))?;

        let mut actions = Vec::new();
        if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
            for call in tool_calls {
                actions.push(self.parse_tool_call(call, display)?);
            }
        }

        if actions.is_empty() {
            if let Some(content) = message.get("content").and_then(Value::as_str) {
                actions = text_action_parser::parse_text_actions(content).map_err(|e| {
                    ProviderError::InvalidResponse(format!("could not parse text action: {}", e))
                })?;
            }
        }

        if actions.is_empty() {
            return Err(ProviderError::InvalidResponse(
                "provider returned no actions".to_string(),
            ));
        }

        Ok(ProviderTurn {
            actions,
            usage,
            pending_safety_checks: Vec::new(),
            provider_response_id: response_id,
        })
    }

    fn parse_responses_response(
        &self,
        body: Value,
        display: &DisplayMetadata,
        elapsed_ms: u64,
    ) -> Result<ProviderTurn, ProviderError> {
        let response_id = body
            .get("id")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let usage = parse_usage(body.get("usage"), elapsed_ms);
        let output = body
            .get("output")
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderError::InvalidResponse("missing output".into()))?;

        let mut actions = Vec::new();
        let mut text = String::new();
        for item in output {
            if item.get("type").and_then(Value::as_str) == Some("function_call") {
                actions.push(self.parse_tool_call(item, display)?);
                continue;
            }
            if let Some(content) = item.get("content").and_then(Value::as_array) {
                for part in content {
                    if matches!(
                        part.get("type").and_then(Value::as_str),
                        Some("output_text" | "summary_text" | "text")
                    ) {
                        if let Some(part_text) = part.get("text").and_then(Value::as_str) {
                            text.push_str(part_text);
                            text.push('\n');
                        }
                    }
                }
            }
            if let Some(item_text) = item.get("text").and_then(Value::as_str) {
                text.push_str(item_text);
                text.push('\n');
            }
        }
        if text.is_empty() {
            if let Some(output_text) = body.get("output_text").and_then(Value::as_str) {
                text.push_str(output_text);
            }
        }

        if actions.is_empty() && !text.trim().is_empty() {
            actions = text_action_parser::parse_text_actions(text.trim()).map_err(|e| {
                ProviderError::InvalidResponse(format!("could not parse text action: {}", e))
            })?;
        }

        if actions.is_empty() {
            return Err(ProviderError::InvalidResponse(
                "provider returned no actions".to_string(),
            ));
        }

        Ok(ProviderTurn {
            actions,
            usage,
            pending_safety_checks: Vec::new(),
            provider_response_id: response_id,
        })
    }

    fn parse_tool_call(
        &self,
        call: &Value,
        display: &DisplayMetadata,
    ) -> Result<Action, ProviderError> {
        let function = call.get("function").unwrap_or(call);
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::InvalidResponse("tool call missing name".into()))?;
        let args = match function.get("arguments") {
            Some(Value::String(s)) => serde_json::from_str(s).map_err(|e| {
                ProviderError::InvalidResponse(format!("invalid tool arguments: {}", e))
            })?,
            Some(value) => value.clone(),
            None => Value::Object(Default::default()),
        };
        tool_call_to_action(name, &args, display, self.cfg.coordinate_space)
    }
}

#[async_trait]
impl BrainProvider for OpenAiCompatibleProvider {
    fn name(&self) -> &'static str {
        self.cfg.provider_name
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            accepts_history_frames: 3,
            native_action_grammar: ActionGrammar::OpenAiToolCall,
            coordinate_space: CoordinateSpace::ImagePixels,
            supports_streaming: false,
            max_screenshot_megapixels: 4.0,
            requires_api_key_env: self.cfg.api_key_env,
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
        let started = Instant::now();
        let use_responses_api = self.cfg.api_format == OpenAiApiFormat::Responses;
        let url = format!(
            "{}/{}",
            self.cfg.endpoint.trim_end_matches('/'),
            self.cfg.api_format.endpoint_path()
        );
        let body = if use_responses_api {
            self.responses_request_body(task, observation, history)?
        } else {
            self.request_body(task, observation, history)?
        };
        let mut request = self
            .client
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .json(&body);
        if let Some(api_key) = &self.cfg.api_key {
            request = request.header(AUTHORIZATION, format!("Bearer {}", api_key));
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
            let retry_after_ms = response
                .headers()
                .get("retry-after")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .map(|s| s * 1000)
                .unwrap_or(1000);
            return Err(ProviderError::RateLimited { retry_after_ms });
        }
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(ProviderError::NetworkError(format!(
                "upstream status {}: {}",
                status, text
            )));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;
        if use_responses_api {
            self.parse_responses_response(
                body,
                &observation.display,
                started.elapsed().as_millis() as u64,
            )
        } else {
            self.parse_response(
                body,
                &observation.display,
                started.elapsed().as_millis() as u64,
            )
        }
    }
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

fn parse_usage(value: Option<&Value>, elapsed_ms: u64) -> Option<ProviderUsage> {
    let usage = value?;
    Some(ProviderUsage {
        input_tokens: usage
            .get("prompt_tokens")
            .or_else(|| usage.get("input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: usage
            .get("completion_tokens")
            .or_else(|| usage.get("output_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        provider_latency_ms: elapsed_ms,
        cost_usd_micros: None,
    })
}

fn point_arg(
    args: &Value,
    display: &DisplayMetadata,
    space: CoordinateSpace,
) -> Result<(i32, i32), ProviderError> {
    if let Some(point) = args.get("point").or_else(|| args.get("coordinate")) {
        if let Some(values) = point.as_array() {
            if values.len() >= 2 {
                return Ok(display.provider_to_image(
                    (
                        values[0].as_f64().unwrap_or_default() as f32,
                        values[1].as_f64().unwrap_or_default() as f32,
                    ),
                    space,
                ));
            }
        }
        if let Some(s) = point.as_str() {
            let nums = integers(s);
            if nums.len() >= 2 {
                return Ok(display.provider_to_image((nums[0] as f32, nums[1] as f32), space));
            }
        }
    }
    let x = number_arg(args, &["x"])?;
    let y = number_arg(args, &["y"])?;
    Ok(display.provider_to_image((x, y), space))
}

fn tool_call_to_action(
    name: &str,
    args: &Value,
    display: &DisplayMetadata,
    space: CoordinateSpace,
) -> Result<Action, ProviderError> {
    let name = name.to_ascii_lowercase();
    match name.as_str() {
        "click" | "left_click" | "mouse_click" => {
            let (x, y) = point_arg(args, display, space)?;
            Ok(Action::MouseClick {
                x,
                y,
                button: mouse_button(args),
                click_count: args
                    .get("click_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    .clamp(1, 3) as u8,
                label: string_arg(args, &["label", "reason"]),
            })
        }
        "double_click" => {
            let (x, y) = point_arg(args, display, space)?;
            Ok(Action::MouseClick {
                x,
                y,
                button: mouse_button(args),
                click_count: 2,
                label: string_arg(args, &["label", "reason"]),
            })
        }
        "move" | "mouse_move" => {
            let (x, y) = point_arg(args, display, space)?;
            Ok(Action::MouseMove {
                x,
                y,
                label: string_arg(args, &["label", "reason"]),
            })
        }
        "drag" | "mouse_drag" => {
            let path = args
                .get("path")
                .and_then(Value::as_array)
                .ok_or_else(|| ProviderError::InvalidResponse("drag missing path".into()))?
                .iter()
                .map(|point| point_arg(&json!({ "point": point.clone() }), display, space))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Action::MouseDrag {
                path,
                button: mouse_button(args),
                label: string_arg(args, &["label", "reason"]),
            })
        }
        "scroll" => Ok(Action::Scroll {
            x: optional_number_arg(args, &["x"])
                .map(|x| display.provider_to_image((x, 0.0), space).0),
            y: optional_number_arg(args, &["y"])
                .map(|y| display.provider_to_image((0.0, y), space).1),
            dx: optional_i32_arg(args, &["dx", "scroll_x"]).unwrap_or(0),
            dy: optional_i32_arg(args, &["dy", "scroll_y"]).unwrap_or(0),
            label: string_arg(args, &["label", "reason"]),
        }),
        "type" | "type_text" => Ok(Action::TypeText {
            text: string_arg(args, &["text", "content"]).unwrap_or_default(),
            press_enter: args
                .get("press_enter")
                .or_else(|| args.get("enter"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }),
        "key" | "keypress" | "key_chord" => Ok(Action::KeyChord {
            combo: keys_arg(args).unwrap_or_default(),
        }),
        "wait" => Ok(Action::Wait {
            ms: optional_i32_arg(args, &["ms"]).unwrap_or(1000).max(0) as u32,
        }),
        "screenshot" => Ok(Action::Screenshot),
        "done" | "finish" => Ok(Action::Done {
            success: args.get("success").and_then(Value::as_bool).unwrap_or(true),
            reason: string_arg(args, &["reason"]).unwrap_or_default(),
        }),
        other => Err(ProviderError::InvalidResponse(format!(
            "unknown tool action: {}",
            other
        ))),
    }
}

fn mouse_button(args: &Value) -> MouseButton {
    match args
        .get("button")
        .and_then(Value::as_str)
        .unwrap_or("left")
        .to_ascii_lowercase()
        .as_str()
    {
        "middle" => MouseButton::Middle,
        "right" => MouseButton::Right,
        _ => MouseButton::Left,
    }
}

fn number_arg(args: &Value, keys: &[&str]) -> Result<f32, ProviderError> {
    optional_number_arg(args, keys).ok_or_else(|| {
        ProviderError::InvalidResponse(format!("missing numeric argument {:?}", keys))
    })
}

fn optional_number_arg(args: &Value, keys: &[&str]) -> Option<f32> {
    for key in keys {
        if let Some(v) = args.get(*key).and_then(Value::as_f64) {
            return Some(v as f32);
        }
    }
    None
}

fn optional_i32_arg(args: &Value, keys: &[&str]) -> Option<i32> {
    optional_number_arg(args, keys).map(|v| v.round() as i32)
}

fn string_arg(args: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(s) = args.get(*key).and_then(Value::as_str) {
            return Some(s.to_string());
        }
    }
    None
}

fn keys_arg(args: &Value) -> Option<String> {
    if let Some(keys) = args.get("keys").and_then(Value::as_array) {
        return Some(
            keys.iter()
                .filter_map(Value::as_str)
                .map(normalize_key_name)
                .collect::<Vec<_>>()
                .join("+"),
        );
    }
    string_arg(args, &["combo", "key", "keys"])
}

fn normalize_key_name(key: &str) -> String {
    match key.to_ascii_uppercase().as_str() {
        "CTRL" | "CONTROL" => "Ctrl".to_string(),
        "ALT" => "Alt".to_string(),
        "SHIFT" => "Shift".to_string(),
        "META" | "SUPER" | "CMD" | "COMMAND" => "Super".to_string(),
        "ENTER" | "RETURN" => "Return".to_string(),
        other => other.to_string(),
    }
}

fn integers(text: &str) -> Vec<i32> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() || (ch == '-' && current.is_empty()) {
            current.push(ch);
        } else if !current.is_empty() {
            if let Ok(n) = current.parse() {
                out.push(n);
            }
            current.clear();
        }
    }
    if !current.is_empty() {
        if let Ok(n) = current.parse() {
            out.push(n);
        }
    }
    out
}

fn tool_schema() -> Value {
    json!([
        function_schema(
            "click",
            json!({
                "type":"object",
                "properties":{
                    "x":{"type":"number"},"y":{"type":"number"},
                    "button":{"type":"string","enum":["left","middle","right"]},
                    "click_count":{"type":"integer","minimum":1,"maximum":3},
                    "label":{"type":"string"}
                },
                "required":["x","y"]
            })
        ),
        function_schema(
            "move",
            json!({
                "type":"object",
                "properties":{"x":{"type":"number"},"y":{"type":"number"},"label":{"type":"string"}},
                "required":["x","y"]
            })
        ),
        function_schema(
            "scroll",
            json!({
                "type":"object",
                "properties":{"x":{"type":"number"},"y":{"type":"number"},"dx":{"type":"integer"},"dy":{"type":"integer"},"label":{"type":"string"}},
                "required":["dy"]
            })
        ),
        function_schema(
            "type",
            json!({
                "type":"object",
                "properties":{"text":{"type":"string"},"press_enter":{"type":"boolean"}},
                "required":["text"]
            })
        ),
        function_schema(
            "key",
            json!({
                "type":"object",
                "properties":{"combo":{"type":"string"}},
                "required":["combo"]
            })
        ),
        function_schema(
            "wait",
            json!({
                "type":"object",
                "properties":{"ms":{"type":"integer","minimum":0}},
                "required":["ms"]
            })
        ),
        function_schema("screenshot", json!({"type":"object","properties":{}})),
        function_schema(
            "done",
            json!({
                "type":"object",
                "properties":{"success":{"type":"boolean"},"reason":{"type":"string"}},
                "required":["success"]
            })
        )
    ])
}

fn responses_tool_schema() -> Value {
    json!([
        responses_function_schema(
            "click",
            json!({
                "type":"object",
                "properties":{
                    "x":{"type":"number"},"y":{"type":"number"},
                    "button":{"type":"string","enum":["left","middle","right"]},
                    "click_count":{"type":"integer","minimum":1,"maximum":3},
                    "label":{"type":"string"}
                },
                "required":["x","y"]
            })
        ),
        responses_function_schema(
            "move",
            json!({
                "type":"object",
                "properties":{"x":{"type":"number"},"y":{"type":"number"},"label":{"type":"string"}},
                "required":["x","y"]
            })
        ),
        responses_function_schema(
            "scroll",
            json!({
                "type":"object",
                "properties":{"x":{"type":"number"},"y":{"type":"number"},"dx":{"type":"integer"},"dy":{"type":"integer"},"label":{"type":"string"}},
                "required":["dy"]
            })
        ),
        responses_function_schema(
            "type",
            json!({
                "type":"object",
                "properties":{"text":{"type":"string"},"press_enter":{"type":"boolean"}},
                "required":["text"]
            })
        ),
        responses_function_schema(
            "key",
            json!({
                "type":"object",
                "properties":{"combo":{"type":"string"}},
                "required":["combo"]
            })
        ),
        responses_function_schema(
            "wait",
            json!({
                "type":"object",
                "properties":{"ms":{"type":"integer","minimum":0}},
                "required":["ms"]
            })
        ),
        responses_function_schema("screenshot", json!({"type":"object","properties":{}})),
        responses_function_schema(
            "done",
            json!({
                "type":"object",
                "properties":{"success":{"type":"boolean"},"reason":{"type":"string"}},
                "required":["success"]
            })
        )
    ])
}

fn function_schema(name: &str, parameters: Value) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": format!("iVnc computer action: {}", name),
            "parameters": parameters
        }
    })
}

fn responses_function_schema(name: &str, parameters: Value) -> Value {
    json!({
        "type": "function",
        "name": name,
        "description": format!("iVnc computer action: {}", name),
        "parameters": parameters
    })
}

pub fn default_system_prompt() -> String {
    "You are an autonomous computer-use agent controlling a Linux desktop through iVnc. \
     Inspect the screenshot and return one or more tool calls. Coordinates must be in the \
     screenshot image pixel coordinate space, not browser or CSS coordinates. Prefer small, \
     verifiable steps and finish with done(success, reason) when the task is complete."
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display() -> DisplayMetadata {
        DisplayMetadata {
            screen_width: 1920,
            screen_height: 1080,
            image_width: 960,
            image_height: 540,
            image_to_screen_scale_x: 2.0,
            image_to_screen_scale_y: 2.0,
            client_dpr: None,
            monitors: vec![],
        }
    }

    #[test]
    fn parses_tool_click() {
        let action = tool_call_to_action(
            "click",
            &json!({"x": 100, "y": 50, "button": "right"}),
            &display(),
            CoordinateSpace::ImagePixels,
        )
        .unwrap();
        assert!(matches!(
            action,
            Action::MouseClick {
                x: 100,
                y: 50,
                button: MouseButton::Right,
                ..
            }
        ));
    }

    #[test]
    fn converts_normalized_tool_coordinates() {
        let action = tool_call_to_action(
            "move",
            &json!({"x": 500, "y": 500}),
            &display(),
            CoordinateSpace::NormalizedUnit,
        )
        .unwrap();
        assert!(matches!(action, Action::MouseMove { x: 480, y: 270, .. }));
    }

    #[test]
    fn api_format_is_explicit_config_not_model_name() {
        assert_eq!(
            OpenAiApiFormat::from_config(Some("responses")),
            OpenAiApiFormat::Responses
        );
        assert_eq!(
            OpenAiApiFormat::from_config(Some("openai_responses")),
            OpenAiApiFormat::Responses
        );
        assert_eq!(
            OpenAiApiFormat::from_config(Some("chat")),
            OpenAiApiFormat::ChatCompletions
        );
        assert_eq!(OpenAiApiFormat::from_config(None), OpenAiApiFormat::ChatCompletions);
    }

    #[test]
    fn parses_responses_function_call() {
        let provider = OpenAiCompatibleProvider::new(OpenAiCompatConfig {
            provider_name: "test",
            endpoint: "https://example.com/v1".to_string(),
            model: "future-model".to_string(),
            api_format: OpenAiApiFormat::Responses,
            api_key: None,
            api_key_env: None,
            coordinate_space: CoordinateSpace::ImagePixels,
            system_prompt: default_system_prompt(),
        });
        let turn = provider
            .parse_responses_response(
                json!({
                    "id": "resp_1",
                    "output": [
                        {
                            "type": "function_call",
                            "name": "done",
                            "arguments": "{\"success\":true,\"reason\":\"visible\"}"
                        }
                    ],
                    "usage": {
                        "input_tokens": 10,
                        "output_tokens": 2
                    }
                }),
                &display(),
                123,
            )
            .unwrap();
        assert_eq!(turn.provider_response_id.as_deref(), Some("resp_1"));
        assert_eq!(turn.usage.as_ref().unwrap().input_tokens, 10);
        assert!(matches!(
            turn.actions.first(),
            Some(Action::Done { success: true, .. })
        ));
    }
}
