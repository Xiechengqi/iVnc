//! MCP (Model Context Protocol) server for iVnc.
//!
//! Exposes desktop control tools (screenshot, mouse, keyboard, clipboard,
//! window management) via the MCP protocol over stdio or Streamable HTTP.

pub mod frame_capture;
pub mod input_exec;
pub mod keyboard;
pub mod tools;

use crate::web::SharedState;
use base64::Engine;
use input_exec::{
    guard_mcp_action_allowed, key_chord, mouse_click as exec_mouse_click,
    mouse_move as exec_mouse_move, mouse_scroll as exec_mouse_scroll, type_text, validate_coords,
    window_close as exec_window_close, window_focus as exec_window_focus, McpActionKind,
};
use rmcp::{
    handler::server::tool::ToolCallContext,
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::*,
    service::RequestContext,
    tool, tool_router, ErrorData as McpError, RoleServer, ServerHandler,
};
use std::sync::Arc;
use tools::*;

#[derive(Clone)]
pub struct McpServer {
    pub state: Arc<SharedState>,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl McpServer {
    pub fn new(state: Arc<SharedState>) -> Self {
        Self {
            state,
            tool_router: Self::combined_tool_router(),
        }
    }

    #[cfg(feature = "agent")]
    async fn start_agent_run(
        &self,
        params: AgentStartParams,
        wait: bool,
    ) -> Result<crate::agent::types::RunReport, McpError> {
        let mut options = crate::console_config::agent_defaults().options;
        if let Some(explicit) = params.options {
            options = explicit;
        }
        if let Some(budget) = params.budget {
            options.budget = budget;
        }
        if let Some(active_run_id) = self.state.agent_runs.running_run_id() {
            return Err(McpError::invalid_request(
                format!("agent_run_already_active: {}", active_run_id),
                None,
            ));
        }
        let run_id = format!("run_{}", uuid::Uuid::new_v4());
        let replay_actions = match (params.replay_actions, params.replay_path) {
            (Some(actions), _) => Some(actions),
            (None, Some(path)) => Some(load_replay_actions(&path)?),
            (None, None) => None,
        };
        let provider =
            crate::agent::registry::build_provider(&params.provider, params.model, replay_actions)
                .ok_or_else(|| {
                    McpError::invalid_params(
                        format!(
                            "provider '{}' is not available in this build",
                            params.provider
                        ),
                        None,
                    )
                })?;
        let state = self.state.clone();
        let task = params.task.clone();
        let run_id_for_task = run_id.clone();
        let options_for_task = options.clone();
        let initial = crate::agent::types::RunReport {
            run_id: run_id.clone(),
            task: params.task,
            success: None,
            finish_reason: crate::agent::types::FinishReason::Running,
            steps_taken: 0,
            tokens_in: 0,
            tokens_out: 0,
            wall_ms: 0,
            trajectory_path: options
                .record_trajectory
                .then(|| crate::agent::trajectory::default_trajectory_path(&run_id)),
            pending_question: None,
            pending_safety_checks: Vec::new(),
            last_action: None,
            last_result: None,
        };
        self.state.agent_runs.insert(initial.clone());
        let handle = tokio::spawn(async move {
            let result = crate::agent::run_agent(
                state.clone(),
                provider,
                task,
                options_for_task,
                run_id_for_task.clone(),
            )
            .await;
            match result {
                Ok(report) => report,
                Err(crate::agent::r#loop::RunError::Interrupted(report))
                | Err(crate::agent::r#loop::RunError::BudgetExceeded(report))
                | Err(crate::agent::r#loop::RunError::MaxStepsReached(report)) => report,
                Err(crate::agent::r#loop::RunError::Provider(err)) => {
                    let mut report = state.agent_runs.get(&run_id_for_task).unwrap_or_else(|| {
                        crate::agent::types::RunReport {
                            run_id: run_id_for_task.clone(),
                            task: String::new(),
                            success: None,
                            finish_reason: crate::agent::types::FinishReason::ProviderError,
                            steps_taken: 0,
                            tokens_in: 0,
                            tokens_out: 0,
                            wall_ms: 0,
                            trajectory_path: None,
                            pending_question: None,
                            pending_safety_checks: Vec::new(),
                            last_action: None,
                            last_result: None,
                        }
                    });
                    report.finish_reason = crate::agent::types::FinishReason::ProviderError;
                    report.pending_question = Some(err.to_string());
                    state.agent_runs.update(report.clone());
                    report
                }
                Err(crate::agent::r#loop::RunError::Capture(err)) => {
                    let mut report = state.agent_runs.get(&run_id_for_task).unwrap_or_else(|| {
                        crate::agent::types::RunReport {
                            run_id: run_id_for_task.clone(),
                            task: String::new(),
                            success: None,
                            finish_reason: crate::agent::types::FinishReason::ProviderError,
                            steps_taken: 0,
                            tokens_in: 0,
                            tokens_out: 0,
                            wall_ms: 0,
                            trajectory_path: None,
                            pending_question: None,
                            pending_safety_checks: Vec::new(),
                            last_action: None,
                            last_result: None,
                        }
                    });
                    report.finish_reason = crate::agent::types::FinishReason::ProviderError;
                    report.pending_question = Some(err);
                    state.agent_runs.update(report.clone());
                    report
                }
            }
        });
        if wait {
            handle
                .await
                .map_err(|e| McpError::internal_error(e.to_string(), None))
        } else {
            Ok(initial)
        }
    }

    fn combined_tool_router() -> ToolRouter<Self> {
        let router = Self::base_tool_router();
        #[cfg(feature = "agent")]
        let router = router + Self::agent_tool_router();
        router
    }
}

#[tool_router(router = base_tool_router)]
impl McpServer {
    #[tool(
        description = "Capture the current desktop as a JPEG image. Use delay_ms to wait for UI updates before capturing."
    )]
    pub async fn screenshot(
        &self,
        Parameters(params): Parameters<ScreenshotParams>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(delay) = params.delay_ms {
            let delay = delay.min(30000);
            if delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
        }
        let (w, h, pixels) = frame_capture::capture_frame(&self.state)
            .await
            .map_err(|e| McpError::internal_error(e, None))?;
        let b64 = frame_capture::xrgb_to_jpeg_base64(w, h, &pixels, 80, 800_000)
            .map_err(|e| McpError::internal_error(e, None))?;
        Ok(CallToolResult::success(vec![Content::image(
            b64,
            "image/jpeg",
        )]))
    }

    #[tool(description = "Move the mouse cursor to the specified coordinates.")]
    pub async fn mouse_move(
        &self,
        Parameters(params): Parameters<MouseMoveParams>,
    ) -> Result<CallToolResult, McpError> {
        guard_mcp_action_allowed(&self.state, McpActionKind::Mouse)?;
        validate_coords(&self.state, params.x, params.y)?;
        exec_mouse_move(&self.state, params.x, params.y).await;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Moved to ({}, {})",
            params.x, params.y
        ))]))
    }

    #[tool(
        description = "Click a mouse button at coordinates. Supports left/right/middle and double-click."
    )]
    pub async fn mouse_click(
        &self,
        Parameters(params): Parameters<MouseClickParams>,
    ) -> Result<CallToolResult, McpError> {
        guard_mcp_action_allowed(&self.state, McpActionKind::Mouse)?;
        validate_coords(&self.state, params.x, params.y)?;
        let clicks = if params.double { 2 } else { 1 };
        exec_mouse_click(&self.state, params.x, params.y, &params.button, clicks).await?;
        let action = if params.double {
            "Double-clicked"
        } else {
            "Clicked"
        };
        Ok(CallToolResult::success(vec![Content::text(format!(
            "{} {} at ({}, {})",
            action, params.button, params.x, params.y
        ))]))
    }

    #[tool(description = "Scroll the mouse wheel. Positive dy scrolls down, negative scrolls up.")]
    pub async fn mouse_scroll(
        &self,
        Parameters(params): Parameters<MouseScrollParams>,
    ) -> Result<CallToolResult, McpError> {
        guard_mcp_action_allowed(&self.state, McpActionKind::Mouse)?;
        exec_mouse_scroll(&self.state, params.dx, params.dy);
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Scrolled dx={} dy={}",
            params.dx, params.dy
        ))]))
    }

    #[tool(
        description = "Type text using the keyboard. Supports ASCII and non-ASCII (CJK, emoji, etc.) text. Non-ASCII text is sent via IME/text input."
    )]
    pub async fn keyboard_type(
        &self,
        Parameters(params): Parameters<KeyboardTypeParams>,
    ) -> Result<CallToolResult, McpError> {
        guard_mcp_action_allowed(&self.state, McpActionKind::Keyboard)?;
        type_text(&self.state, &params.text, params.enter).await;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Typed {} chars{}",
            params.text.chars().count(),
            if params.enter { " + Enter" } else { "" }
        ))]))
    }

    #[tool(
        description = "Type multiple lines of text. Enter is pressed after each line. Supports non-ASCII (CJK, emoji, etc.) text via IME."
    )]
    pub async fn keyboard_type_multiline(
        &self,
        Parameters(params): Parameters<KeyboardTypeMultilineParams>,
    ) -> Result<CallToolResult, McpError> {
        let count = params.lines.len();
        for (i, line) in params.lines.iter().enumerate() {
            guard_mcp_action_allowed(&self.state, McpActionKind::Keyboard)?;
            type_text(&self.state, line, true).await;
            if i < count - 1 {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Typed {} lines",
            count
        ))]))
    }

    #[tool(
        description = "Press a key or key combination. Use '+' for combos: 'Ctrl+c', 'Alt+F4', 'Ctrl+Shift+t'. Single keys: 'Return', 'Escape', 'Tab', 'F1'-'F12', arrows, etc."
    )]
    pub async fn keyboard_key(
        &self,
        Parameters(params): Parameters<KeyboardKeyParams>,
    ) -> Result<CallToolResult, McpError> {
        guard_mcp_action_allowed(&self.state, McpActionKind::Keyboard)?;
        key_chord(&self.state, &params.key).await?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Pressed {}",
            params.key
        ))]))
    }

    #[tool(description = "Read the current clipboard text content.")]
    pub async fn clipboard_read(&self) -> Result<CallToolResult, McpError> {
        let clip = self.state.clipboard.lock().unwrap().clone();
        match clip {
            Some(b64) => {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(&b64)
                    .map_err(|e| McpError::internal_error(format!("base64 decode: {}", e), None))?;
                let text = String::from_utf8_lossy(&decoded).into_owned();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            None => Ok(CallToolResult::success(vec![Content::text(
                "(clipboard empty)",
            )])),
        }
    }

    #[tool(description = "Write text to the clipboard.")]
    pub async fn clipboard_write(
        &self,
        Parameters(params): Parameters<ClipboardWriteParams>,
    ) -> Result<CallToolResult, McpError> {
        guard_mcp_action_allowed(&self.state, McpActionKind::ClipboardWrite)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(params.text.as_bytes());
        self.state.set_clipboard(b64.clone());
        let _ = self.state.clipboard_incoming_tx.send(b64);
        self.state
            .clipboard_incoming_dirty
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(CallToolResult::success(vec![Content::text(
            "Clipboard updated",
        )]))
    }

    #[tool(description = "Get screen dimensions, FPS, bandwidth, and connection statistics.")]
    pub async fn get_screen_info(&self) -> Result<CallToolResult, McpError> {
        let (w, h) = self.state.display_size();
        let stats = self.state.stats.lock().unwrap().clone();
        let sessions = self.state.webrtc_sessions();
        let uptime = self.state.uptime().as_secs();
        let info = serde_json::json!({
            "width": w, "height": h,
            "fps": format!("{:.1}", stats.fps),
            "bandwidth_bps": stats.bandwidth,
            "webrtc_sessions": sessions,
            "uptime_seconds": uptime,
            "cpu_percent": format!("{:.1}", stats.cpu_percent),
            "mem_bytes": stats.mem_used,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&info).unwrap(),
        )]))
    }

    #[tool(description = "List all open windows with their IDs, titles, and focus state.")]
    pub async fn list_windows(&self) -> Result<CallToolResult, McpError> {
        let json = self.state.last_taskbar_json.lock().unwrap().clone();
        match json {
            Some(j) => Ok(CallToolResult::success(vec![Content::text(j)])),
            None => Ok(CallToolResult::success(vec![Content::text(
                r#"{"windows":[]}"#,
            )])),
        }
    }

    #[tool(description = "Focus a window by its ID (from list_windows).")]
    pub async fn window_focus(
        &self,
        Parameters(params): Parameters<WindowIdParams>,
    ) -> Result<CallToolResult, McpError> {
        guard_mcp_action_allowed(&self.state, McpActionKind::WindowFocus)?;
        exec_window_focus(&self.state, params.window_id);
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Focused window {}",
            params.window_id
        ))]))
    }

    #[tool(description = "Close a window by its ID (from list_windows).")]
    pub async fn window_close(
        &self,
        Parameters(params): Parameters<WindowIdParams>,
    ) -> Result<CallToolResult, McpError> {
        guard_mcp_action_allowed(&self.state, McpActionKind::WindowClose)?;
        exec_window_close(&self.state, params.window_id);
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Closed window {}",
            params.window_id
        ))]))
    }
}

#[cfg(feature = "agent")]
#[tool_router(router = agent_tool_router)]
impl McpServer {
    #[tool(description = "Start an in-process VLM agent run. Returns immediately with a run_id.")]
    pub async fn agent_start(
        &self,
        Parameters(params): Parameters<AgentStartParams>,
    ) -> Result<CallToolResult, McpError> {
        let report = self.start_agent_run(params, false).await?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&report).unwrap(),
        )]))
    }

    #[tool(
        description = "Run an in-process VLM agent and wait for completion. Prefer agent_start for long tasks."
    )]
    pub async fn agent_run(
        &self,
        Parameters(params): Parameters<AgentRunParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = AgentStartParams {
            task: params.task,
            provider: params.provider,
            model: params.model,
            budget: params.budget,
            options: params.options,
            replay_actions: params.replay_actions,
            replay_path: params.replay_path,
        };
        let report = self.start_agent_run(params, true).await?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&report).unwrap(),
        )]))
    }

    #[tool(description = "Get the status of an agent run.")]
    pub async fn agent_status(
        &self,
        Parameters(params): Parameters<AgentStatusParams>,
    ) -> Result<CallToolResult, McpError> {
        let report = self
            .state
            .agent_runs
            .get(&params.run_id)
            .ok_or_else(|| McpError::invalid_params("unknown run_id".to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&report).unwrap(),
        )]))
    }

    #[tool(description = "Stop the active agent run and return control to the user.")]
    pub async fn agent_stop(
        &self,
        Parameters(params): Parameters<AgentStopParams>,
    ) -> Result<CallToolResult, McpError> {
        let already_terminal = params
            .run_id
            .as_deref()
            .map(|id| self.state.agent_runs.is_terminal(id))
            .unwrap_or(false);
        if already_terminal {
            return Ok(CallToolResult::success(vec![Content::text(
                "Agent run already terminal; no-op",
            )]));
        }
        self.state.request_agent_stop();
        self.state
            .agent_runs
            .mark_interrupted(params.run_id.as_deref());
        Ok(CallToolResult::success(vec![Content::text(
            "Agent stop requested",
        )]))
    }

    #[tool(description = "List compiled and currently configured agent providers.")]
    pub async fn provider_list(&self) -> Result<CallToolResult, McpError> {
        let providers = crate::agent::registry::provider_infos();
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&providers).unwrap(),
        )]))
    }

    #[tool(description = "Read the recorded JSONL trajectory for an agent run.")]
    pub async fn agent_history_get(
        &self,
        Parameters(params): Parameters<AgentHistoryGetParams>,
    ) -> Result<CallToolResult, McpError> {
        let report = self
            .state
            .agent_runs
            .get(&params.run_id)
            .ok_or_else(|| McpError::invalid_params("unknown run_id".to_string(), None))?;
        let path = report.trajectory_path.ok_or_else(|| {
            McpError::invalid_params("run has no trajectory_path".to_string(), None)
        })?;
        let text = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| McpError::internal_error(format!("read trajectory: {}", e), None))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Report local configuration health for an agent provider.")]
    pub async fn provider_health(
        &self,
        Parameters(params): Parameters<ProviderHealthParams>,
    ) -> Result<CallToolResult, McpError> {
        let provider = crate::agent::registry::provider_presets()
            .into_iter()
            .find(|p| p.name == params.provider)
            .ok_or_else(|| McpError::invalid_params("unknown provider".to_string(), None))?;
        let configured = crate::agent::registry::provider_infos()
            .iter()
            .any(|p| p.name == provider.name);
        let info = serde_json::json!({
            "name": provider.name,
            "configured": configured,
            "api_key_env": provider.api_key_env,
            "api_key_present": provider.api_key_env.map(|env| std::env::var_os(env).is_some()),
            "default_endpoint": provider.default_endpoint,
            "default_model": provider.default_model,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&info).unwrap(),
        )]))
    }
}

#[cfg(feature = "agent")]
fn load_replay_actions(path: &str) -> Result<Vec<crate::agent::types::Action>, McpError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| McpError::invalid_params(format!("replay_path read failed: {}", e), None))?;
    let mut actions = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line).map_err(|e| {
            McpError::invalid_params(
                format!("invalid replay jsonl at line {}: {}", idx + 1, e),
                None,
            )
        })?;
        let action_value = value.get("action").cloned().unwrap_or(value);
        let action = serde_json::from_value(action_value).map_err(|e| {
            McpError::invalid_params(
                format!("invalid replay action at line {}: {}", idx + 1, e),
                None,
            )
        })?;
        actions.push(action);
    }
    if actions.is_empty() {
        return Err(McpError::invalid_params(
            "replay_path did not contain any actions".to_string(),
            None,
        ));
    }
    Ok(actions)
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability { list_changed: None }),
                ..Default::default()
            },
            server_info: Implementation {
                name: "ivnc-mcp".into(),
                title: None,
                version: env!("CARGO_PKG_VERSION").into(),
                description: Some("iVnc remote desktop MCP server".into()),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "iVnc remote desktop MCP server. Use screenshot to see the desktop, \
                 mouse/keyboard tools to interact, clipboard to read/write text, \
                 and window tools to manage windows."
                    .into(),
            ),
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(self.tool_router.list_all()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let ctx = ToolCallContext::new(self, request, context);
        self.tool_router.call(ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ui::UiConfig, Config};
    use crate::input::InputEventData;
    use crate::runtime_settings::RuntimeSettings;
    use std::sync::Arc;

    fn test_state() -> Arc<SharedState> {
        let config = Config::default();
        let ui_config = UiConfig::from_env(&config);
        let runtime_settings = Arc::new(RuntimeSettings::new(&config));
        let (input_sender, _input_rx) = tokio::sync::mpsc::unbounded_channel::<InputEventData>();
        Arc::new(SharedState::new(
            config,
            ui_config,
            input_sender,
            runtime_settings,
        ))
    }

    #[tokio::test]
    async fn clipboard_write_updates_read_cache() {
        let server = McpServer::new(test_state());
        let text = "mcp clipboard roundtrip";

        server
            .clipboard_write(Parameters(ClipboardWriteParams {
                text: text.to_string(),
            }))
            .await
            .unwrap();

        let encoded = server.state.clipboard.lock().unwrap().clone().unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), text);
    }
}
