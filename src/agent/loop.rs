use super::exec;
use super::provider::{BrainProvider, ProviderError, ProviderSession};
use super::types::{
    now_ms, Action, ActionResult, FinishReason, History, RunOptions, RunReport, RunSource, Step,
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
    let source = state
        .agent_runs
        .get(&run_id)
        .map(|r| r.source)
        .unwrap_or_default();
    let mut consecutive_out_of_bounds = 0u8;
    let mut substantive = false;
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
        source.clone(),
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
                source.clone(),
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
                    source.clone(),
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
                    source.clone(),
                );
                report.pending_question = Some(err.clone());
                state.agent_runs.update(report);
                state.set_agent_exclusive(false, "frame_persist_error");
                return Err(RunError::Capture(err));
            }
        }
        let digest = observation.digest();

        let turn_hints = compute_turn_hints(&history, &digest.sha256, options.budget.max_steps);
        let turn_task = augment_task(&task, turn_hints.as_deref());

        let turn = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                report = build_report(&run_id, &task, &history, started_ms, FinishReason::Interrupted, trajectory_path.clone(), source.clone());
                state.agent_runs.update(report.clone());
                state.set_agent_exclusive(false, "interrupted");
                return Err(RunError::Interrupted(report));
            }
            res = provider.next_action(&turn_task, &observation, &history, &mut session) => match res {
                Ok(t) => t,
                Err(ProviderError::RateLimited { retry_after_ms }) => {
                    tokio::time::sleep(Duration::from_millis(retry_after_ms)).await;
                    continue;
                }
                Err(e) => {
                    report = build_report(&run_id, &task, &history, started_ms, FinishReason::ProviderError, trajectory_path.clone(), source.clone());
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
                source.clone(),
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
                    source.clone(),
                );
                if success
                    && !substantive
                    && report
                        .output
                        .as_deref()
                        .map_or(true, |o| o.trim().is_empty())
                {
                    report.warnings.push(
                        "reported success=true without taking any substantive action or providing an output deliverable — result is likely unverified".to_string(),
                    );
                }
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
                    source.clone(),
                );
                report.pending_question = Some(question);
                state.agent_runs.update(report.clone());
                state.set_agent_exclusive(false, "ask");
                return Ok(report);
            }
            let loop_blocked = !options.dry_run
                && is_loop_repeat(&action, &history, &digest.sha256);
            let result = if loop_blocked {
                ActionResult::ExecutorError {
                    message: "blocked_loop: duplicate of previous action that did not change the screen — pick a different approach (different coordinates, different key, scroll, or call done)".to_string(),
                }
            } else if options.dry_run {
                ActionResult::Ok
            } else {
                exec::execute(&state, &observation.display, &action, &options).await
            };
            let destructive_blocked = matches!(
                &result,
                ActionResult::ExecutorError { message } if message.starts_with("destructive_blocked")
            );
            consecutive_out_of_bounds = if matches!(result, ActionResult::OutOfBounds { .. }) {
                consecutive_out_of_bounds.saturating_add(1)
            } else {
                0
            };
            let force_observe = matches!(action, Action::Screenshot);
            let needs_settle = action_needs_settle(&action);
            if matches!(result, ActionResult::Ok) && is_substantive_action(&action) {
                substantive = true;
            }
            let blocked_message = if destructive_blocked {
                if let ActionResult::ExecutorError { message } = &result {
                    Some(message.clone())
                } else {
                    None
                }
            } else {
                None
            };
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
                source.clone(),
            );
            state.agent_runs.update(report.clone());

            if let Some(message) = blocked_message {
                report = build_report(
                    &run_id,
                    &task,
                    &history,
                    started_ms,
                    FinishReason::Ask,
                    trajectory_path.clone(),
                    source.clone(),
                );
                report.pending_question = Some(format!(
                    "Destructive action blocked ({}). Re-run with allow_destructive=true or remove the kind from require_confirmation_for to proceed.",
                    message
                ));
                state.agent_runs.update(report.clone());
                state.set_agent_exclusive(false, "destructive_blocked");
                return Ok(report);
            }

            if consecutive_out_of_bounds >= 3 {
                report = build_report(
                    &run_id,
                    &task,
                    &history,
                    started_ms,
                    FinishReason::BudgetExceeded,
                    trajectory_path.clone(),
                    source.clone(),
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
            if needs_settle && !loop_blocked {
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
                source.clone(),
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
        source.clone(),
    );
    state.agent_runs.update(report.clone());
    state.set_agent_exclusive(false, "max_steps_reached");
    Err(RunError::MaxStepsReached(report))
}

fn action_needs_settle(action: &Action) -> bool {
    !matches!(action, Action::Wait { .. } | Action::Screenshot)
}

/// Whether an action represents substantive work that can change the environment
/// (as opposed to passive observation/cursor moves). Used to detect runs that
/// claim success without having actually done anything.
fn is_substantive_action(action: &Action) -> bool {
    matches!(
        action,
        Action::MouseClick { .. }
            | Action::MouseDown { .. }
            | Action::MouseUp { .. }
            | Action::MouseDrag { .. }
            | Action::Scroll { .. }
            | Action::Zoom { .. }
            | Action::TypeText { .. }
            | Action::KeyChord { .. }
            | Action::KeyHold { .. }
            | Action::ClipboardWrite { .. }
            | Action::WindowFocus { .. }
            | Action::WindowClose { .. }
            | Action::LaunchApp { .. }
    )
}

fn build_report(
    run_id: &str,
    task: &str,
    history: &History,
    started_ms: u64,
    finish_reason: FinishReason,
    trajectory_path: Option<PathBuf>,
    source: RunSource,
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
    let output = history.steps.last().and_then(|s| match &s.action {
        Action::Done { output, .. } if !output.trim().is_empty() => Some(output.clone()),
        _ => None,
    });
    RunReport {
        run_id: run_id.to_string(),
        task: task.to_string(),
        success,
        finish_reason,
        steps_taken: history.steps.len(),
        tokens_in,
        tokens_out,
        wall_ms: now_ms().saturating_sub(started_ms),
        started_at_ms: started_ms,
        trajectory_path,
        pending_question: None,
        pending_safety_checks: Vec::new(),
        last_action: history.steps.last().map(|s| s.action.clone()),
        last_result: history.steps.last().map(|s| s.result.clone()),
        output,
        warnings: Vec::new(),
        source,
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

fn canonical_action(a: &Action) -> String {
    fn r5(v: i32) -> i32 {
        (v / 5) * 5
    }
    match a {
        Action::MouseClick {
            x,
            y,
            button,
            click_count,
            ..
        } => format!(
            "click:{}:{}:{:?}:{}",
            r5(*x),
            r5(*y),
            button,
            click_count
        ),
        Action::MouseMove { x, y, .. } => format!("move:{}:{}", r5(*x), r5(*y)),
        Action::MouseDown { x, y, button, .. } => {
            format!("mousedown:{}:{}:{:?}", r5(*x), r5(*y), button)
        }
        Action::MouseUp { x, y, button, .. } => {
            format!("mouseup:{}:{}:{:?}", r5(*x), r5(*y), button)
        }
        Action::Scroll { x, y, dx, dy, .. } => format!(
            "scroll:{:?}:{:?}:{}:{}",
            x.map(r5),
            y.map(r5),
            r5(*dx),
            r5(*dy)
        ),
        Action::TypeText { text, press_enter } => format!("type:{}:{}", text, press_enter),
        Action::KeyChord { combo } => format!("key:{}", combo.to_ascii_lowercase()),
        Action::KeyHold { key, ms } => format!("hold:{}:{}", key.to_ascii_lowercase(), ms),
        Action::WindowFocus { id } => format!("winfocus:{}", id),
        Action::WindowClose { id } => format!("winclose:{}", id),
        Action::Zoom { level_delta, .. } => format!("zoom:{}", level_delta),
        Action::ClipboardWrite { text } => format!("clipwrite:{}", text),
        Action::ClipboardRead => "clipread".to_string(),
        Action::MouseDrag { path, button, .. } => {
            format!("drag:{}:{:?}:{:?}", path.len(), button, path.first())
        }
        Action::LaunchApp { app, url } => {
            format!("launch:{}:{}", app.to_ascii_lowercase(), url.as_deref().unwrap_or(""))
        }
        Action::Wait { .. } | Action::Screenshot | Action::Done { .. } | Action::Ask { .. } => {
            String::new()
        }
    }
}

fn is_loop_repeat(action: &Action, history: &History, current_sha: &str) -> bool {
    if matches!(action, Action::Wait { .. } | Action::Screenshot) {
        return false;
    }
    let Some(last) = history.steps.last() else {
        return false;
    };
    if last.observation.sha256 != current_sha {
        return false;
    }
    let cur = canonical_action(action);
    if cur.is_empty() {
        return false;
    }
    cur == canonical_action(&last.action)
}

fn brief_action(a: &Action) -> String {
    match a {
        Action::MouseClick { x, y, .. } => format!("click({},{})", x, y),
        Action::MouseMove { x, y, .. } => format!("move({},{})", x, y),
        Action::Scroll { dx, dy, .. } => format!("scroll(dx={},dy={})", dx, dy),
        Action::TypeText { text, .. } => {
            let snippet: String = text.chars().take(40).collect();
            format!("type({:?})", snippet)
        }
        Action::KeyChord { combo } => format!("key({})", combo),
        Action::KeyHold { key, ms } => format!("hold({},{}ms)", key, ms),
        Action::Wait { ms } => format!("wait({}ms)", ms),
        Action::Screenshot => "screenshot".to_string(),
        Action::WindowFocus { id } => format!("window_focus({})", id),
        Action::WindowClose { id } => format!("window_close({})", id),
        Action::MouseDrag { path, .. } => format!("drag({} points)", path.len()),
        Action::Zoom { level_delta, .. } => format!("zoom({})", level_delta),
        other => format!("{:?}", other),
    }
}

fn compute_turn_hints(history: &History, current_sha: &str, max_steps: u32) -> Option<String> {
    let mut hints: Vec<String> = Vec::new();

    if history.steps.len() >= 2 {
        let prev = &history.steps[history.steps.len() - 1].observation.sha256;
        let prev2 = &history.steps[history.steps.len() - 2].observation.sha256;
        if prev == current_sha && prev2 == current_sha {
            let brief = brief_action(&history.steps.last().unwrap().action);
            hints.push(format!(
                "WARNING: the last 2 actions did NOT change the screen (sha256 identical). \
                 Your last action ({}) is having no effect — switch strategy: try different \
                 coordinates, a different key, scroll, or call done(success=false, \
                 reason='stuck: ...') if you cannot recover.",
                brief
            ));
        }
    }

    let steps_used = history.steps.len() as u32;
    let remaining = max_steps.saturating_sub(steps_used);
    let progress = if max_steps == 0 {
        0.0
    } else {
        steps_used as f32 / max_steps as f32
    };
    if remaining <= 1 {
        hints.push(
            "FINAL STEP: this is your last allowed step. Return \
             done(success=<your best judgment>, reason='<concise summary of what you \
             found or why you failed>') NOW. Any other action will be wasted."
                .to_string(),
        );
    } else if progress > 0.6 {
        hints.push(format!(
            "Budget status: {}/{} steps used, {} remaining. Strongly prefer done over \
             more exploration if the current screenshot already lets you answer the task.",
            steps_used, max_steps, remaining
        ));
    }

    if hints.is_empty() {
        None
    } else {
        Some(hints.join("\n"))
    }
}

fn augment_task(original: &str, hints: Option<&str>) -> String {
    match hints {
        Some(h) if !h.is_empty() => format!("{}\n\n[Turn hints]\n{}", original, h),
        _ => original.to_string(),
    }
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
