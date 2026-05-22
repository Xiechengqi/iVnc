use super::types::{now_ms, Step};
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

pub fn default_trajectory_path(run_id: &str) -> PathBuf {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("ivnc")
        .join("trajectories");
    base.join(format!("{}.jsonl", sanitize_run_id(run_id)))
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
