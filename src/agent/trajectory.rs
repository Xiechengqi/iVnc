use super::types::{now_ms, RunReport, Step};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Serialize)]
struct TrajectoryLine<'a> {
    step: usize,
    ts_ms: u64,
    provider: &'a str,
    #[serde(flatten)]
    data: &'a Step,
}

fn trajectories_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("ivnc")
        .join("trajectories")
}

pub fn default_trajectory_path(run_id: &str) -> PathBuf {
    trajectories_dir().join(format!("{}.jsonl", sanitize_run_id(run_id)))
}

pub fn default_event_path(run_id: &str) -> PathBuf {
    trajectories_dir().join(format!("{}.events.jsonl", sanitize_run_id(run_id)))
}

/// Sidecar path holding the serialized `RunReport` for a run, so the run list
/// survives process restarts.
pub fn report_path(run_id: &str) -> PathBuf {
    trajectories_dir().join(format!("{}.report.json", sanitize_run_id(run_id)))
}

/// Best-effort synchronous write of a run report sidecar. Errors are ignored —
/// losing a sidecar only degrades the run list, never correctness.
pub fn write_report_sync(report: &RunReport) {
    let path = report_path(&report.run_id);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(report) {
        let _ = std::fs::write(&path, json);
    }
}

/// Load every persisted run report from disk (best-effort).
pub fn load_all_reports() -> Vec<RunReport> {
    let dir = trajectories_dir();
    let mut reports = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return reports,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(".report.json"))
            .unwrap_or(false)
        {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(report) = serde_json::from_str::<RunReport>(&content) {
                    reports.push(report);
                }
            }
        }
    }
    reports
}

pub async fn append_step(
    path: &Path,
    step_index: usize,
    provider: &str,
    step: &Step,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let line = TrajectoryLine {
        step: step_index,
        ts_ms: now_ms(),
        provider,
        data: step,
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

fn sanitize_run_id(run_id: &str) -> String {
    run_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
