use super::types::{Action, CoordinateSpace, Observation, ProviderUsage, SafetyCheck};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[async_trait]
#[allow(dead_code)]
pub trait BrainProvider: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> ProviderCapabilities;

    async fn next_action(
        &self,
        task: &str,
        observation: &Observation,
        history: &super::types::History,
        session: &mut ProviderSession,
    ) -> Result<ProviderTurn, ProviderError>;

    async fn reset(&self, _task: &str) -> Result<(), ProviderError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub accepts_history_frames: u8,
    pub native_action_grammar: ActionGrammar,
    pub coordinate_space: CoordinateSpace,
    pub supports_streaming: bool,
    pub max_screenshot_megapixels: f32,
    pub requires_api_key_env: Option<&'static str>,
    pub supports_safety_ack: bool,
    pub supports_window_actions: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderTurn {
    pub actions: Vec<Action>,
    pub usage: Option<ProviderUsage>,
    pub pending_safety_checks: Vec<SafetyCheck>,
    pub provider_response_id: Option<String>,
}

impl ProviderTurn {
    pub fn actions(actions: Vec<Action>) -> Self {
        Self {
            actions,
            usage: None,
            pending_safety_checks: Vec::new(),
            provider_response_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSession {
    pub run_id: String,
    pub previous_response_id: Option<String>,
    pub provider_conversation_state: serde_json::Value,
    pub acknowledged_safety_checks: Vec<SafetyCheck>,
}

impl ProviderSession {
    pub fn new(run_id: String) -> Self {
        Self {
            run_id,
            previous_response_id: None,
            provider_conversation_state: serde_json::Value::Null,
            acknowledged_safety_checks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionGrammar {
    OpenAiToolCall,
    OpenAiComputerUsePreview,
    AnthropicComputerUse,
    GeminiComputerUse,
    Replay,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProviderError {
    NetworkError(String),
    AuthError(String),
    RateLimited { retry_after_ms: u64 },
    QuotaExceeded,
    InvalidResponse(String),
    UnsupportedObservation(String),
    UpstreamRefused(String),
    SafetyConfirmationRequired(Vec<SafetyCheck>),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NetworkError(e) => write!(f, "network error: {}", e),
            Self::AuthError(e) => write!(f, "auth error: {}", e),
            Self::RateLimited { retry_after_ms } => {
                write!(f, "rate limited; retry after {}ms", retry_after_ms)
            }
            Self::QuotaExceeded => write!(f, "quota exceeded"),
            Self::InvalidResponse(e) => write!(f, "invalid provider response: {}", e),
            Self::UnsupportedObservation(e) => write!(f, "unsupported observation: {}", e),
            Self::UpstreamRefused(e) => write!(f, "upstream refused: {}", e),
            Self::SafetyConfirmationRequired(_) => write!(f, "safety confirmation required"),
        }
    }
}

impl std::error::Error for ProviderError {}
