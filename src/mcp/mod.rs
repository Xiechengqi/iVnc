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
        let replay_actions = match (params.replay_actions, params.replay_path) {
            (Some(actions), _) => Some(actions),
            (None, Some(path)) => Some(load_replay_actions(&path)?),
            (None, None) => None,
        };
        let req = crate::agent::launch::LaunchRequest {
            task: params.task,
            provider: params.provider,
            model: params.model,
            budget: params.budget,
            options: params.options,
            replay_actions,
            source: None,
        };
        crate::agent::launch::launch_agent_run(self.state.clone(), req, wait)
            .await
            .map_err(|err| match err {
                crate::agent::launch::LaunchError::AlreadyActive(id) => {
                    McpError::invalid_request(format!("agent_run_already_active: {}", id), None)
                }
                crate::agent::launch::LaunchError::ProviderUnavailable(name) => {
                    McpError::invalid_params(
                        format!("provider '{}' is not available in this build", name),
                        None,
                    )
                }
                crate::agent::launch::LaunchError::InvalidRequest(msg) => {
                    McpError::invalid_request(msg, None)
                }
                crate::agent::launch::LaunchError::Internal(msg) => {
                    McpError::internal_error(msg, None)
                }
            })
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

    #[tool(description = "List iVNC managed apps, tools, skills, and diagnostics.")]
    pub async fn ivnc_capabilities_list(
        &self,
        Parameters(params): Parameters<CapabilityListParams>,
    ) -> Result<CallToolResult, McpError> {
        let apps = self
            .state
            .apps_state()
            .cloned()
            .ok_or_else(|| McpError::internal_error("apps manager is not initialized", None))?;
        let snapshot = crate::capabilities::build_snapshot(&apps).await;
        let kind = params.kind.as_deref().unwrap_or("all");
        let value = match kind {
            "apps" => {
                serde_json::json!({ "apps": snapshot.apps, "diagnostics": snapshot.diagnostics })
            }
            "tools" => {
                serde_json::json!({ "tools": snapshot.tools, "diagnostics": snapshot.diagnostics })
            }
            "skills" => {
                serde_json::json!({ "skills": snapshot.skills, "diagnostics": snapshot.diagnostics })
            }
            _ => serde_json::to_value(snapshot)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        };
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&value).unwrap(),
        )]))
    }

    #[tool(description = "List iVNC capability tools available through ivnc_tool_call.")]
    pub async fn ivnc_tools_list(
        &self,
        Parameters(_params): Parameters<CapabilityListParams>,
    ) -> Result<CallToolResult, McpError> {
        let apps = self
            .state
            .apps_state()
            .cloned()
            .ok_or_else(|| McpError::internal_error("apps manager is not initialized", None))?;
        let snapshot = crate::capabilities::build_snapshot(&apps).await;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(
                &serde_json::json!({ "tools": snapshot.tools, "diagnostics": snapshot.diagnostics }),
            )
            .unwrap(),
        )]))
    }

    #[tool(description = "Call an iVNC capability tool by tool_id with JSON arguments.")]
    pub async fn ivnc_tool_call(
        &self,
        Parameters(params): Parameters<CapabilityToolCallParams>,
    ) -> Result<CallToolResult, McpError> {
        let apps = self
            .state
            .apps_state()
            .cloned()
            .ok_or_else(|| McpError::internal_error("apps manager is not initialized", None))?;
        let caller = crate::capabilities::CallerContext {
            client_id: params.client_id.unwrap_or_else(|| "mcp-client".to_string()),
            session_id: params
                .session_id
                .unwrap_or_else(|| format!("session_{}", uuid::Uuid::new_v4())),
            user: None,
            source: "mcp".to_string(),
        };
        let request = crate::capabilities::CapabilityCallRequest {
            arguments: params.arguments,
            confirm: params.confirm,
            caller: None,
        };
        let outcome = crate::capabilities::call_tool(apps, &params.tool_id, request, caller).await;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&outcome).unwrap(),
        )]))
    }

    #[tool(description = "List iVNC capability skills.")]
    pub async fn ivnc_skills_list(
        &self,
        Parameters(_params): Parameters<CapabilityListParams>,
    ) -> Result<CallToolResult, McpError> {
        let apps = self
            .state
            .apps_state()
            .cloned()
            .ok_or_else(|| McpError::internal_error("apps manager is not initialized", None))?;
        let snapshot = crate::capabilities::build_snapshot(&apps).await;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(
                &serde_json::json!({ "skills": snapshot.skills, "diagnostics": snapshot.diagnostics }),
            )
            .unwrap(),
        )]))
    }

    #[tool(description = "Read a capability skill by skill_id.")]
    pub async fn ivnc_skill_get(
        &self,
        Parameters(params): Parameters<CapabilitySkillGetParams>,
    ) -> Result<CallToolResult, McpError> {
        let apps = self
            .state
            .apps_state()
            .cloned()
            .ok_or_else(|| McpError::internal_error("apps manager is not initialized", None))?;
        let snapshot = crate::capabilities::build_snapshot(&apps).await;
        let skill = snapshot
            .skills
            .into_iter()
            .find(|skill| skill.id == params.skill_id)
            .ok_or_else(|| McpError::invalid_params("unknown skill", None))?;
        let mut skill = skill;
        if let crate::capabilities::SkillSource::LocalPath { path } = &skill.source {
            let content = crate::capabilities::read_skill_content(path)
                .await
                .map_err(|e| McpError::invalid_params(e, None))?;
            skill.content = Some(content);
        }
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&skill).unwrap(),
        )]))
    }

    #[tool(description = "Read recent audited iVNC capability tool calls.")]
    pub async fn ivnc_call_history(
        &self,
        Parameters(params): Parameters<CapabilityCallHistoryParams>,
    ) -> Result<CallToolResult, McpError> {
        let calls =
            crate::capabilities::call_log::read_recent(params.limit.unwrap_or(50).min(200)).await;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&serde_json::json!({ "calls": calls })).unwrap(),
        )]))
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

    #[tool(description = "Read the structured JSONL event stream for an agent run.")]
    pub async fn agent_events_get(
        &self,
        Parameters(params): Parameters<AgentHistoryGetParams>,
    ) -> Result<CallToolResult, McpError> {
        let report = self
            .state
            .agent_runs
            .get(&params.run_id)
            .ok_or_else(|| McpError::invalid_params("unknown run_id".to_string(), None))?;
        let path = report
            .event_path
            .unwrap_or_else(|| crate::agent::trajectory::default_event_path(&params.run_id));
        let text = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| McpError::internal_error(format!("read agent events: {}", e), None))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        description = "Execute a single agent Action against the live desktop, honoring the same destructive-action guard as agent_run. Returns the ActionResult and a fresh observation digest."
    )]
    pub async fn agent_step(
        &self,
        Parameters(params): Parameters<AgentStepParams>,
    ) -> Result<CallToolResult, McpError> {
        let defaults = crate::console_config::agent_defaults().options;
        let options = crate::agent::types::RunOptions {
            allow_destructive: params.allow_destructive,
            require_confirmation_for: params.require_confirmation_for,
            screenshot_format: defaults.screenshot_format,
            screenshot_max_bytes: defaults.screenshot_max_bytes,
            ..crate::agent::types::RunOptions::default()
        };
        let observation = crate::mcp::frame_capture::capture_observation(
            &self.state,
            80,
            options.screenshot_max_bytes,
        )
        .await
        .map_err(|e| McpError::internal_error(format!("capture: {}", e), None))?;
        let result = crate::agent::exec::execute(
            &self.state,
            &observation.display,
            &params.action,
            &options,
        )
        .await;
        let digest = if params.capture_observation {
            crate::mcp::frame_capture::capture_observation(
                &self.state,
                80,
                options.screenshot_max_bytes,
            )
            .await
            .ok()
            .map(|o| o.digest())
        } else {
            None
        };
        let body = serde_json::json!({
            "action": &params.action,
            "result": result,
            "observation": digest,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&body).unwrap(),
        )]))
    }

    #[tool(
        description = "Deterministically replay a list of agent Actions through the replay provider. Returns a RunReport."
    )]
    pub async fn agent_history_replay(
        &self,
        Parameters(params): Parameters<AgentHistoryReplayParams>,
    ) -> Result<CallToolResult, McpError> {
        if params.actions.is_none() && params.replay_path.is_none() {
            return Err(McpError::invalid_params(
                "either `actions` or `replay_path` must be provided".to_string(),
                None,
            ));
        }
        let start = AgentStartParams {
            task: params.task.unwrap_or_else(|| "replay".to_string()),
            provider: "replay".to_string(),
            model: None,
            budget: None,
            options: params.options,
            replay_actions: params.actions,
            replay_path: params.replay_path,
        };
        let report = self.start_agent_run(start, true).await?;
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&report).unwrap(),
        )]))
    }

    #[tool(description = "Report local configuration health for an agent provider.")]
    pub async fn provider_health(
        &self,
        Parameters(params): Parameters<ProviderHealthParams>,
    ) -> Result<CallToolResult, McpError> {
        if params.provider == "replay" {
            let info = serde_json::json!({
                "name": "replay",
                "configured": true,
                "internal": true,
                "default_endpoint": "",
                "default_model": "replay",
            });
            return Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&info).unwrap(),
            )]));
        }
        let provider = crate::agent::registry::provider_infos()
            .into_iter()
            .find(|p| p.name == params.provider)
            .ok_or_else(|| McpError::invalid_params("unknown provider".to_string(), None))?;
        let info = serde_json::json!({
            "name": provider.name,
            "display_name": provider.display_name,
            "provider_type": provider.provider_type,
            "configured": provider.configured,
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
