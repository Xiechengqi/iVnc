use super::call_log::{append, now_ms, preview, redact, CapabilityCallRecord};
use super::locks;
use super::registry::build_snapshot;
use super::types::{CallerContext, CapabilityTool, PermissionPolicy, ToolCallOutcome, ToolSource};
use crate::apps::api::AppsState;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

const DEFAULT_TIMEOUT_MS: u64 = 60_000;
const MAX_TIMEOUT_MS: u64 = 300_000;
const OUTPUT_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct CapabilityCallRequest {
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub caller: Option<CallerContext>,
}

pub async fn call_tool(
    apps: Arc<AppsState>,
    tool_id: &str,
    request: CapabilityCallRequest,
    fallback_caller: CallerContext,
) -> ToolCallOutcome {
    let caller = request.caller.unwrap_or(fallback_caller);
    let call_id = format!("cap_{}", uuid::Uuid::new_v4());
    let started = Instant::now();
    let snapshot = build_snapshot(&apps).await;
    let Some(tool) = snapshot.tools.into_iter().find(|tool| tool.id == tool_id) else {
        return ToolCallOutcome {
            ok: false,
            call_id,
            tool_id: tool_id.to_string(),
            app_id: String::new(),
            permission: super::types::PermissionLevel::Read,
            duration_ms: 0,
            exit_code: None,
            stdout: None,
            stderr: None,
            error: Some("unknown tool".to_string()),
            requires_confirmation: false,
            resources: Vec::new(),
        };
    };

    let permission_error = match tool.permission_policy {
        PermissionPolicy::Allow => None,
        PermissionPolicy::Confirm if request.confirm => None,
        PermissionPolicy::Confirm => Some(("tool requires confirmation", true)),
        PermissionPolicy::Deny => Some(("permission denied", false)),
    };
    if let Some((message, requires_confirmation)) = permission_error {
        let outcome = ToolCallOutcome {
            ok: false,
            call_id,
            tool_id: tool.id.clone(),
            app_id: tool.app_id.clone(),
            permission: tool.permission,
            duration_ms: started.elapsed().as_millis() as u64,
            exit_code: None,
            stdout: None,
            stderr: None,
            error: Some(message.to_string()),
            requires_confirmation,
            resources: tool.resources.clone(),
        };
        write_log(&caller, &tool, &request.arguments, &outcome).await;
        return outcome;
    }

    let holder = format!("{}:{}", caller.client_id, caller.session_id);
    let _guard = match locks::acquire(&tool.resources, &holder) {
        Ok(guard) => guard,
        Err(busy) => {
            let outcome = ToolCallOutcome {
                ok: false,
                call_id,
                tool_id: tool.id.clone(),
                app_id: tool.app_id.clone(),
                permission: tool.permission,
                duration_ms: started.elapsed().as_millis() as u64,
                exit_code: None,
                stdout: None,
                stderr: None,
                error: Some(format!(
                    "resource_busy: {} held by {}; retry_after_ms={}",
                    busy.resource, busy.holder, busy.retry_after_ms
                )),
                requires_confirmation: false,
                resources: tool.resources.clone(),
            };
            write_log(&caller, &tool, &request.arguments, &outcome).await;
            return outcome;
        }
    };

    let mut outcome = execute_tool(&tool, request.arguments.clone(), call_id, started).await;
    outcome.resources = tool.resources.clone();
    write_log(&caller, &tool, &request.arguments, &outcome).await;
    outcome
}

async fn execute_tool(
    tool: &CapabilityTool,
    arguments: Value,
    call_id: String,
    started: Instant,
) -> ToolCallOutcome {
    match &tool.source {
        ToolSource::CliCommand {
            binary_path,
            command,
            env,
        } => {
            execute_cli_command(binary_path, command, env, arguments, tool, call_id, started).await
        }
        ToolSource::CliRaw { binary_path, env } => {
            execute_cli_raw(binary_path, env, arguments, tool, call_id, started).await
        }
        _ => ToolCallOutcome {
            ok: false,
            call_id,
            tool_id: tool.id.clone(),
            app_id: tool.app_id.clone(),
            permission: tool.permission,
            duration_ms: started.elapsed().as_millis() as u64,
            exit_code: None,
            stdout: None,
            stderr: None,
            error: Some("tool source is not implemented".to_string()),
            requires_confirmation: false,
            resources: Vec::new(),
        },
    }
}

async fn execute_cli_command(
    binary: &str,
    command_name: &str,
    env: &std::collections::HashMap<String, String>,
    arguments: Value,
    tool: &CapabilityTool,
    call_id: String,
    started: Instant,
) -> ToolCallOutcome {
    let mut params = arguments.as_object().cloned().unwrap_or_default();
    let timeout_ms = params
        .remove("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .clamp(100, MAX_TIMEOUT_MS);
    let cdp_port = params
        .remove("cdp_port")
        .and_then(|v| {
            v.as_str()
                .map(ToOwned::to_owned)
                .or_else(|| v.as_u64().map(|n| n.to_string()))
        })
        .filter(|v| !v.trim().is_empty());
    let params_json = Value::Object(params).to_string();
    let mut command = tokio::process::Command::new(binary);
    command.args(["execute", command_name, "--params", &params_json]);
    if let Some(cdp_port) = cdp_port.as_deref() {
        command.args(["--cdp-port", cdp_port]);
    }
    execute_command(command, env, timeout_ms, tool, call_id, started).await
}

async fn execute_cli_raw(
    binary: &str,
    env: &std::collections::HashMap<String, String>,
    arguments: Value,
    tool: &CapabilityTool,
    call_id: String,
    started: Instant,
) -> ToolCallOutcome {
    let args = arguments
        .get("args")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let timeout_ms = arguments
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .clamp(100, MAX_TIMEOUT_MS);
    let mut command = tokio::process::Command::new(binary);
    command.args(args);
    execute_command(command, env, timeout_ms, tool, call_id, started).await
}

async fn execute_command(
    mut command: tokio::process::Command,
    env: &std::collections::HashMap<String, String>,
    timeout_ms: u64,
    tool: &CapabilityTool,
    call_id: String,
    started: Instant,
) -> ToolCallOutcome {
    let isolated = crate::paths::merge_cli_isolation_env(&tool.app_id, env)
        .unwrap_or_else(|_| env.clone());
    command.envs(&isolated);
    for key in crate::paths::APP_ENV_REMOVE {
        command.env_remove(key);
    }
    if let Ok(val) = std::env::var("XDG_RUNTIME_DIR") {
        command.env("XDG_RUNTIME_DIR", val);
    }
    if let Ok(val) = std::env::var("WAYLAND_DISPLAY") {
        command.env("WAYLAND_DISPLAY", val);
    }
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    let output =
        match tokio::time::timeout(Duration::from_millis(timeout_ms), command.output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(err)) => {
                return error_outcome(tool, call_id, started, format!("spawn_failed: {}", err));
            }
            Err(_) => {
                return error_outcome(
                    tool,
                    call_id,
                    started,
                    format!("timeout: exceeded {}ms", timeout_ms),
                );
            }
        };
    let stdout = truncate_output(&String::from_utf8_lossy(&output.stdout));
    let stderr = truncate_output(&String::from_utf8_lossy(&output.stderr));
    ToolCallOutcome {
        ok: output.status.success(),
        call_id,
        tool_id: tool.id.clone(),
        app_id: tool.app_id.clone(),
        permission: tool.permission,
        duration_ms: started.elapsed().as_millis() as u64,
        exit_code: output.status.code(),
        stdout: Some(stdout),
        stderr: Some(stderr),
        error: None,
        requires_confirmation: false,
        resources: Vec::new(),
    }
}

fn error_outcome(
    tool: &CapabilityTool,
    call_id: String,
    started: Instant,
    message: String,
) -> ToolCallOutcome {
    ToolCallOutcome {
        ok: false,
        call_id,
        tool_id: tool.id.clone(),
        app_id: tool.app_id.clone(),
        permission: tool.permission,
        duration_ms: started.elapsed().as_millis() as u64,
        exit_code: None,
        stdout: None,
        stderr: None,
        error: Some(message),
        requires_confirmation: false,
        resources: Vec::new(),
    }
}

async fn write_log(
    caller: &CallerContext,
    tool: &CapabilityTool,
    arguments: &Value,
    outcome: &ToolCallOutcome,
) {
    let record = CapabilityCallRecord {
        call_id: outcome.call_id.clone(),
        ts_ms: now_ms(),
        caller: caller.clone(),
        app_id: tool.app_id.clone(),
        tool_id: tool.id.clone(),
        permission: tool.permission,
        resources: tool.resources.clone(),
        arguments_redacted: redact(arguments),
        ok: outcome.ok,
        exit_code: outcome.exit_code,
        stdout_preview: outcome.stdout.as_deref().map(preview),
        stderr_preview: outcome.stderr.as_deref().map(preview),
        duration_ms: outcome.duration_ms,
        error: outcome.error.clone(),
    };
    let _ = append(&record).await;
}

fn truncate_output(text: &str) -> String {
    text.chars().take(OUTPUT_LIMIT).collect()
}

#[allow(dead_code)]
async fn write_stdin(mut stdin: tokio::process::ChildStdin, value: &str) {
    let _ = stdin.write_all(value.as_bytes()).await;
}
