//! Transport-agnostic agent-run launcher.
//!
//! Both the MCP server (`agent_start` / `agent_run` tools) and the web console
//! (`POST /api/console/agent-start`) start agent runs through
//! [`launch_agent_run`]. This keeps the single-active-run guard, option merge,
//! provider construction, initial-report bookkeeping and the spawned
//! `run_agent` error mapping in one place.

use crate::agent::types::{Action, Budget, FinishReason, RunOptions, RunReport, RunSource};
use crate::web::SharedState;
use std::sync::Arc;

/// A request to launch an agent run, independent of transport. Replay paths are
/// resolved to `replay_actions` by the caller (the MCP layer keeps its own
/// error type for file loading).
pub struct LaunchRequest {
    pub task: String,
    pub provider: String,
    pub model: Option<String>,
    pub budget: Option<Budget>,
    pub options: Option<RunOptions>,
    pub replay_actions: Option<Vec<Action>>,
    /// Optional provenance label. Defaults to `RunSource::Manual` when `None`.
    pub source: Option<RunSource>,
}

/// Failure modes shared across transports. Each transport maps these to its own
/// error representation (MCP error codes, HTTP status codes).
pub enum LaunchError {
    /// A run is already active; carries the active run id.
    AlreadyActive(String),
    /// The named provider is not available in this build; carries the name.
    ProviderUnavailable(String),
    /// The request is malformed (e.g. empty task).
    InvalidRequest(String),
    /// An internal error occurred (e.g. the run task panicked).
    Internal(String),
}

/// Launch an agent run. When `wait` is false the initial `Running` report is
/// returned immediately and the run proceeds in the background; when true the
/// call awaits the spawned task and returns the final report.
pub async fn launch_agent_run(
    state: Arc<SharedState>,
    req: LaunchRequest,
    wait: bool,
) -> Result<RunReport, LaunchError> {
    if req.task.trim().is_empty() {
        return Err(LaunchError::InvalidRequest("task must not be empty".into()));
    }

    let mut options = crate::console_config::agent_defaults().options;
    if let Some(explicit) = req.options {
        options = explicit;
    }
    if let Some(budget) = req.budget {
        options.budget = budget;
    }

    let provider =
        crate::agent::registry::build_provider(&req.provider, req.model, req.replay_actions)
            .ok_or_else(|| LaunchError::ProviderUnavailable(req.provider.clone()))?;

    let run_id = format!("run_{}", uuid::Uuid::new_v4());
    let trajectory_path = options
        .record_trajectory
        .then(|| crate::agent::trajectory::default_trajectory_path(&run_id));
    let event_path = Some(crate::agent::events::event_path_for_trajectory(
        trajectory_path.as_deref(),
        &run_id,
    ));
    let initial = RunReport {
        run_id: run_id.clone(),
        task: req.task.clone(),
        success: None,
        finish_reason: FinishReason::Running,
        steps_taken: 0,
        tokens_in: 0,
        tokens_out: 0,
        wall_ms: 0,
        started_at_ms: crate::agent::types::now_ms(),
        trajectory_path,
        event_path,
        budget_exceeded: None,
        pending_question: None,
        pending_safety_checks: Vec::new(),
        last_action: None,
        last_result: None,
        output: None,
        warnings: Vec::new(),
        source: req.source.unwrap_or_default(),
    };

    // Atomic single-active guard: rejects if another run is already Running.
    state
        .agent_runs
        .try_begin(initial.clone())
        .map_err(LaunchError::AlreadyActive)?;

    let state_for_task = state.clone();
    let task = req.task;
    let run_id_for_task = run_id.clone();
    let options_for_task = options;
    let handle = tokio::spawn(async move {
        let result = crate::agent::run_agent(
            state_for_task.clone(),
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
                let mut report = state_for_task
                    .agent_runs
                    .get(&run_id_for_task)
                    .unwrap_or_else(|| RunReport {
                        run_id: run_id_for_task.clone(),
                        task: String::new(),
                        success: None,
                        finish_reason: FinishReason::ProviderError,
                        steps_taken: 0,
                        tokens_in: 0,
                        tokens_out: 0,
                        wall_ms: 0,
                        started_at_ms: crate::agent::types::now_ms(),
                        trajectory_path: None,
                        event_path: Some(crate::agent::trajectory::default_event_path(
                            &run_id_for_task,
                        )),
                        budget_exceeded: None,
                        pending_question: None,
                        pending_safety_checks: Vec::new(),
                        last_action: None,
                        last_result: None,
                        output: None,
                        warnings: Vec::new(),
                        source: RunSource::default(),
                    });
                report.finish_reason = FinishReason::ProviderError;
                report.pending_question = Some(err.to_string());
                state_for_task.agent_runs.update(report.clone());
                report
            }
            Err(crate::agent::r#loop::RunError::Capture(err)) => {
                let mut report = state_for_task
                    .agent_runs
                    .get(&run_id_for_task)
                    .unwrap_or_else(|| RunReport {
                        run_id: run_id_for_task.clone(),
                        task: String::new(),
                        success: None,
                        finish_reason: FinishReason::ProviderError,
                        steps_taken: 0,
                        tokens_in: 0,
                        tokens_out: 0,
                        wall_ms: 0,
                        started_at_ms: crate::agent::types::now_ms(),
                        trajectory_path: None,
                        event_path: Some(crate::agent::trajectory::default_event_path(
                            &run_id_for_task,
                        )),
                        budget_exceeded: None,
                        pending_question: None,
                        pending_safety_checks: Vec::new(),
                        last_action: None,
                        last_result: None,
                        output: None,
                        warnings: Vec::new(),
                        source: RunSource::default(),
                    });
                report.finish_reason = FinishReason::ProviderError;
                report.pending_question = Some(err);
                state_for_task.agent_runs.update(report.clone());
                report
            }
        }
    });

    if wait {
        handle
            .await
            .map_err(|e| LaunchError::Internal(e.to_string()))
    } else {
        Ok(initial)
    }
}
