use super::exec;
use super::provider::{BrainProvider, ProviderError, ProviderSession};
use super::types::{
    now_ms, Action, ActionResult, FinishReason, History, RunOptions, RunReport, Step,
};
use crate::mcp::frame_capture;
use crate::web::SharedState;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug)]
pub enum RunError {
    Provider(ProviderError),
    Interrupted(RunReport),
    BudgetExceeded(RunReport),
    MaxStepsReached(RunReport),
    Capture(String),
}

pub async fn run_agent(
    state: Arc<SharedState>,
    provider: Arc<dyn BrainProvider>,
    task: String,
    options: RunOptions,
    run_id: String,
) -> Result<RunReport, RunError> {
    state.reset_agent_stop();
    state.set_agent_exclusive(true, "agent_start");
    let started_ms = now_ms();
    let mut history = History::new(task.clone(), options.budget.clone());
    let mut session = ProviderSession::new(run_id.clone());
    let cancel = state.agent_cancel_token();
    let provider_name = provider.name();
    let trajectory_path = options
        .record_trajectory
        .then(|| super::trajectory::default_trajectory_path(&run_id));
    let mut consecutive_out_of_bounds = 0u8;
    if let Err(err) = provider.reset(&task).await {
        state.set_agent_exclusive(false, "provider_error");
        return Err(RunError::Provider(err));
    }

    let mut report = build_report(
        &run_id,
        &task,
        &history,
        started_ms,
        FinishReason::Running,
        trajectory_path.clone(),
    );
    state.agent_runs.insert(report.clone());

    for _ in 0..options.budget.max_steps {
        if state.agent_stop_requested() {
            report = build_report(
                &run_id,
                &task,
                &history,
                started_ms,
                FinishReason::Interrupted,
                trajectory_path.clone(),
            );
            state.agent_runs.update(report.clone());
            state.set_agent_exclusive(false, "interrupted");
            return Err(RunError::Interrupted(report));
        }

        let mut observation = match frame_capture::capture_observation(
            &state,
            80,
            options.screenshot_max_bytes,
        )
        .await
        {
            Ok(observation) => observation,
            Err(err) => {
                report = build_report(
                    &run_id,
                    &task,
                    &history,
                    started_ms,
                    FinishReason::ProviderError,
                    trajectory_path.clone(),
                );
                report.pending_question = Some(err.clone());
                state.agent_runs.update(report);
                state.set_agent_exclusive(false, "capture_error");
                return Err(RunError::Capture(err));
            }
        };
        history.record_observation();
        if options.record_frames_to_disk {
            if let Err(err) =
                persist_observation_frame(&mut observation, &run_id, history.observations_seen)
                    .await
            {
                report = build_report(
                    &run_id,
                    &task,
                    &history,
                    started_ms,
                    FinishReason::ProviderError,
                    trajectory_path.clone(),
                );
                report.pending_question = Some(err.clone());
                state.agent_runs.update(report);
                state.set_agent_exclusive(false, "frame_persist_error");
                return Err(RunError::Capture(err));
            }
        }
        let digest = observation.digest();

        let turn = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                report = build_report(&run_id, &task, &history, started_ms, FinishReason::Interrupted, trajectory_path.clone());
                state.agent_runs.update(report.clone());
                state.set_agent_exclusive(false, "interrupted");
                return Err(RunError::Interrupted(report));
            }
            res = provider.next_action(&task, &observation, &history, &mut session) => match res {
                Ok(t) => t,
                Err(ProviderError::RateLimited { retry_after_ms }) => {
                    tokio::time::sleep(Duration::from_millis(retry_after_ms)).await;
                    continue;
                }
                Err(e) => {
                    report = build_report(&run_id, &task, &history, started_ms, FinishReason::ProviderError, trajectory_path.clone());
                    state.agent_runs.update(report.clone());
                    state.set_agent_exclusive(false, "provider_error");
                    return Err(RunError::Provider(e));
                }
            }
        };

        if let Some(response_id) = turn.provider_response_id.clone() {
            session.previous_response_id = Some(response_id);
        }
        if !turn.pending_safety_checks.is_empty() {
            report = build_report(
                &run_id,
                &task,
                &history,
                started_ms,
                FinishReason::Safety,
                trajectory_path.clone(),
            );
            report.pending_safety_checks = turn.pending_safety_checks;
            state.agent_runs.update(report.clone());
            state.set_agent_exclusive(false, "safety");
            return Ok(report);
        }

        for action in turn.actions.into_iter().take(options.max_actions_per_step) {
            let before = now_ms();
            if let Action::Done { success, .. } = action.clone() {
                let step = Step {
                    observation: digest.clone(),
                    action,
                    result: ActionResult::Ok,
                    elapsed_ms: 0,
                    provider_usage: turn.usage.clone(),
                };
                push_step(
                    &mut history,
                    step,
                    &state,
                    provider_name,
                    trajectory_path.as_ref(),
                )
                .await;
                report = build_report(
                    &run_id,
                    &task,
                    &history,
                    started_ms,
                    FinishReason::Done { success },
                    trajectory_path.clone(),
                );
                state.agent_runs.update(report.clone());
                state.set_agent_exclusive(false, "done");
                return Ok(report);
            }
            if let Action::Ask { question } = action.clone() {
                let step = Step {
                    observation: digest.clone(),
                    action,
                    result: ActionResult::Ok,
                    elapsed_ms: 0,
                    provider_usage: turn.usage.clone(),
                };
                push_step(
                    &mut history,
                    step,
                    &state,
                    provider_name,
                    trajectory_path.as_ref(),
                )
                .await;
                report = build_report(
                    &run_id,
                    &task,
                    &history,
                    started_ms,
                    FinishReason::Ask,
                    trajectory_path.clone(),
                );
                report.pending_question = Some(question);
                state.agent_runs.update(report.clone());
                state.set_agent_exclusive(false, "ask");
                return Ok(report);
            }
            let result = if options.dry_run {
                ActionResult::Ok
            } else {
                exec::execute(&state, &observation.display, &action, &options).await
            };
            consecutive_out_of_bounds = if matches!(result, ActionResult::OutOfBounds { .. }) {
                consecutive_out_of_bounds.saturating_add(1)
            } else {
                0
            };
            let force_observe = matches!(action, Action::Screenshot);
            let needs_settle = action_needs_settle(&action);
            let step = Step {
                observation: digest.clone(),
                action,
                result,
                elapsed_ms: now_ms().saturating_sub(before),
                provider_usage: turn.usage.clone(),
            };
            push_step(
                &mut history,
                step,
                &state,
                provider_name,
                trajectory_path.as_ref(),
            )
            .await;
            report = build_report(
                &run_id,
                &task,
                &history,
                started_ms,
                FinishReason::Running,
                trajectory_path.clone(),
            );
            state.agent_runs.update(report.clone());

            if consecutive_out_of_bounds >= 3 {
                report = build_report(
                    &run_id,
                    &task,
                    &history,
                    started_ms,
                    FinishReason::BudgetExceeded,
                    trajectory_path.clone(),
                );
                report.pending_question =
                    Some("aborted after 3 consecutive out-of-bounds actions".to_string());
                state.agent_runs.update(report.clone());
                state.set_agent_exclusive(false, "out_of_bounds");
                return Err(RunError::BudgetExceeded(report));
            }
            if force_observe {
                break;
            }
            if needs_settle {
                tokio::time::sleep(Duration::from_millis(options.action_settle_ms)).await;
            }
        }

        if history.over_budget(started_ms) {
            report = build_report(
                &run_id,
                &task,
                &history,
                started_ms,
                FinishReason::BudgetExceeded,
                trajectory_path.clone(),
            );
            state.agent_runs.update(report.clone());
            state.set_agent_exclusive(false, "budget_exceeded");
            return Err(RunError::BudgetExceeded(report));
        }
    }

    report = build_report(
        &run_id,
        &task,
        &history,
        started_ms,
        FinishReason::MaxStepsReached,
        trajectory_path.clone(),
    );
    state.agent_runs.update(report.clone());
    state.set_agent_exclusive(false, "max_steps_reached");
    Err(RunError::MaxStepsReached(report))
}

fn action_needs_settle(action: &Action) -> bool {
    !matches!(action, Action::Wait { .. } | Action::Screenshot)
}

fn build_report(
    run_id: &str,
    task: &str,
    history: &History,
    started_ms: u64,
    finish_reason: FinishReason,
    trajectory_path: Option<PathBuf>,
) -> RunReport {
    let tokens_in = history
        .steps
        .iter()
        .filter_map(|s| s.provider_usage.as_ref().map(|u| u.input_tokens))
        .sum();
    let tokens_out = history
        .steps
        .iter()
        .filter_map(|s| s.provider_usage.as_ref().map(|u| u.output_tokens))
        .sum();
    let success = match finish_reason {
        FinishReason::Done { success } => Some(success),
        _ => None,
    };
    RunReport {
        run_id: run_id.to_string(),
        task: task.to_string(),
        success,
        finish_reason,
        steps_taken: history.steps.len(),
        tokens_in,
        tokens_out,
        wall_ms: now_ms().saturating_sub(started_ms),
        trajectory_path,
        pending_question: None,
        pending_safety_checks: Vec::new(),
        last_action: history.steps.last().map(|s| s.action.clone()),
        last_result: history.steps.last().map(|s| s.result.clone()),
    }
}

async fn push_step(
    history: &mut History,
    step: Step,
    state: &Arc<SharedState>,
    provider_name: &str,
    trajectory_path: Option<&PathBuf>,
) {
    let step_index = history.steps.len();
    broadcast_action(state, step_index, &step);
    if let Some(path) = trajectory_path {
        if let Err(err) =
            super::trajectory::append_step(path, step_index, provider_name, &step).await
        {
            log::warn!("failed to append agent trajectory: {}", err);
        }
    }
    history.push(step);
}

fn broadcast_action(state: &Arc<SharedState>, seq: usize, step: &Step) {
    let payload = serde_json::json!({
        "seq": seq,
        "ts": now_ms(),
        "action": &step.action,
        "result": &step.result,
        "elapsed_ms": step.elapsed_ms,
    });
    state.send_text(format!("mcp_action,{}", payload));
}

async fn persist_observation_frame(
    observation: &mut super::types::Observation,
    run_id: &str,
    observation_index: u32,
) -> Result<(), String> {
    let frame_dir = super::trajectory::default_trajectory_path(run_id)
        .with_extension("")
        .with_file_name(format!("{}_frames", run_id));
    tokio::fs::create_dir_all(&frame_dir)
        .await
        .map_err(|e| e.to_string())?;
    match &mut observation.frame {
        super::types::ObservationFrame::JpegBytes {
            bytes, frame_path, ..
        } => {
            let path = frame_dir.join(format!("obs_{:04}.jpg", observation_index));
            tokio::fs::write(&path, bytes)
                .await
                .map_err(|e| e.to_string())?;
            *frame_path = Some(path);
        }
        super::types::ObservationFrame::PngBytes {
            bytes, frame_path, ..
        } => {
            let path = frame_dir.join(format!("obs_{:04}.png", observation_index));
            tokio::fs::write(&path, bytes)
                .await
                .map_err(|e| e.to_string())?;
            *frame_path = Some(path);
        }
        super::types::ObservationFrame::RawXrgb { .. } => {}
    }
    Ok(())
}
