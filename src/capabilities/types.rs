use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitySnapshot {
    pub apps: Vec<CapabilityApp>,
    pub tools: Vec<CapabilityTool>,
    pub diagnostics: Vec<CapabilityDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityApp {
    pub id: String,
    pub name: String,
    pub app_type: String,
    pub status: String,
    pub capabilities: Vec<String>,
    pub tool_count: usize,
    pub diagnostics: Vec<CapabilityDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityTool {
    pub id: String,
    pub app_id: String,
    pub name: String,
    pub summary: String,
    pub source: ToolSource,
    pub input_schema: Value,
    pub permission: PermissionLevel,
    pub permission_policy: PermissionPolicy,
    pub resources: Vec<String>,
    pub concurrency: ConcurrencyPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolSource {
    CliCommand {
        binary_path: String,
        command: String,
        env: HashMap<String, String>,
    },
    CliRaw {
        binary_path: String,
        env: HashMap<String, String>,
    },
    HttpApi {
        base_url: String,
        path: String,
    },
    DesktopAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityDiagnostic {
    pub app_id: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionLevel {
    Read,
    Write,
    Destructive,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionPolicy {
    Allow,
    Confirm,
    Deny,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConcurrencyPolicy {
    Shared,
    Exclusive,
    Queued,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallerContext {
    pub client_id: String,
    pub session_id: String,
    pub user: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallOutcome {
    pub ok: bool,
    pub call_id: String,
    pub tool_id: String,
    pub app_id: String,
    pub permission: PermissionLevel,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub requires_confirmation: bool,
    #[serde(default)]
    pub resources: Vec<String>,
}

pub fn slug_id(value: &str) -> String {
    let mut out = String::new();
    let mut last_underscore = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_underscore = false;
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    out.trim_matches('_').to_string()
}

pub fn tool_id_for(app_id: &str, name: &str) -> String {
    let app = slug_id(app_id);
    let name = slug_id(name);
    if app.is_empty() {
        name
    } else if name.is_empty() {
        app
    } else {
        format!("{}_{}", app, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_ids_are_stable_slugs() {
        assert_eq!(
            tool_id_for("twitter-cli", "Search Tweets"),
            "twitter_cli_search_tweets"
        );
        assert_eq!(
            tool_id_for("agent.browser", "snapshot"),
            "agent_browser_snapshot"
        );
    }
}
