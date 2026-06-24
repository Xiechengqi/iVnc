use super::types::{
    now_ms, BudgetExceeded, ObservationDigest, ProviderUsage, ToolCallRecord, ToolResultRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    RunStarted {
        run_id: String,
        task: String,
        provider: String,
        dry_run: bool,
    },
    ObservationCaptured {
        digest: ObservationDigest,
        observations_seen: u32,
    },
    ContextPrepared {
        has_turn_hints: bool,
        dynamic_context_bytes: usize,
    },
    ProviderRequestStarted {
        provider: String,
    },
    ProviderResponseReceived {
        provider: String,
        action_count: usize,
        usage: Option<ProviderUsage>,
        provider_response_id: Option<String>,
    },
    ProviderError {
        provider: String,
        message: String,
    },
    ToolExecutionStarted {
        step: usize,
        tool_call: ToolCallRecord,
    },
    ToolExecutionFinished {
        step: usize,
        tool_call: ToolCallRecord,
        tool_result: ToolResultRecord,
        elapsed_ms: u64,
    },
    BudgetExceeded {
        budget: BudgetExceeded,
    },
    RunFinished {
        finish_reason: serde_json::Value,
        success: Option<bool>,
        output: Option<String>,
    },
}

#[derive(Debug, Serialize)]
struct EventLine<'a> {
    seq: u64,
    ts_ms: u64,
    run_id: &'a str,
    #[serde(flatten)]
    event: &'a AgentEvent,
}

pub fn event_path_for_trajectory(trajectory_path: Option<&Path>, run_id: &str) -> PathBuf {
    trajectory_path
        .map(|path| path.with_extension("events.jsonl"))
        .unwrap_or_else(|| super::trajectory::default_event_path(run_id))
}

pub async fn append_event(
    path: &Path,
    run_id: &str,
    seq: u64,
    event: &AgentEvent,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let line = EventLine {
        seq,
        ts_ms: now_ms(),
        run_id,
        event,
    };
    let mut line = serde_json::to_string(&line).map_err(|e| e.to_string())?;
    line.push('\n');
    tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|e| e.to_string())?
        .write_all(line.as_bytes())
        .await
        .map_err(|e| e.to_string())
}

pub async fn read_events(path: &Path) -> Result<Vec<serde_json::Value>, String> {
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect())
}

pub fn finish_reason_value(reason: &super::types::FinishReason) -> serde_json::Value {
    serde_json::to_value(reason).unwrap_or_else(|_| json!({ "kind": "unknown" }))
}
