use super::types::{CallerContext, PermissionLevel};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

const PREVIEW_CHARS: usize = 2048;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityCallRecord {
    pub call_id: String,
    pub ts_ms: u64,
    pub caller: CallerContext,
    pub app_id: String,
    pub tool_id: String,
    pub permission: PermissionLevel,
    pub resources: Vec<String>,
    pub arguments_redacted: Value,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr_preview: Option<String>,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn call_log_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/root/.local/share"))
        .join("ivnc")
        .join("capability_calls.jsonl")
}

pub async fn append(record: &CapabilityCallRecord) -> Result<(), String> {
    let path = call_log_path();
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let mut line = serde_json::to_string(record).map_err(|e| e.to_string())?;
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

pub async fn read_recent(limit: usize) -> Vec<CapabilityCallRecord> {
    let Ok(text) = tokio::fs::read_to_string(call_log_path()).await else {
        return Vec::new();
    };
    let mut records = text
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<CapabilityCallRecord>(line).ok())
        .take(limit)
        .collect::<Vec<_>>();
    records.reverse();
    records
}

pub fn redact(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let redacted = if is_secret_key(k) {
                        Value::String("[redacted]".to_string())
                    } else {
                        redact(v)
                    };
                    (k.clone(), redacted)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
        _ => value.clone(),
    }
}

pub fn preview(text: &str) -> String {
    text.chars().take(PREVIEW_CHARS).collect()
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "password",
        "token",
        "secret",
        "api_key",
        "authorization",
        "cookie",
        "bearer",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_nested_secret_arguments() {
        let value = json!({
            "query": "openai",
            "auth": { "api_key": "sk-test", "cookie": "abc" },
            "items": [{ "token": "secret" }]
        });
        let redacted = redact(&value);
        assert_eq!(redacted["query"], "openai");
        assert_eq!(redacted["auth"]["api_key"], "[redacted]");
        assert_eq!(redacted["auth"]["cookie"], "[redacted]");
        assert_eq!(redacted["items"][0]["token"], "[redacted]");
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
