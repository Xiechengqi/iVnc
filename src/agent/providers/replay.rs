use crate::agent::provider::{
    ActionGrammar, BrainProvider, ProviderCapabilities, ProviderError, ProviderSession,
    ProviderTurn,
};
use crate::agent::types::{Action, CoordinateSpace, History, Observation};
use async_trait::async_trait;
use std::sync::Mutex;

pub struct ReplayProvider {
    actions: Mutex<Vec<Action>>,
}

impl ReplayProvider {
    pub fn new(actions: Vec<Action>) -> Self {
        let mut actions = actions;
        actions.reverse();
        Self {
            actions: Mutex::new(actions),
        }
    }
}

#[async_trait]
impl BrainProvider for ReplayProvider {
    fn name(&self) -> &'static str {
        "replay"
    }

    fn capabilities(&self) -> ProviderCapabilities {
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

    async fn next_action(
        &self,
        _task: &str,
        _observation: &Observation,
        _history: &History,
        _session: &mut ProviderSession,
    ) -> Result<ProviderTurn, ProviderError> {
        let action = self.actions.lock().unwrap().pop().unwrap_or(Action::Done {
            success: true,
            reason: "replay complete".to_string(),
        });
        Ok(ProviderTurn::actions(vec![action]))
    }
}
