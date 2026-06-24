use super::types::{
    tool_id_for, CapabilityDiagnostic, CapabilitySkill, CapabilityTool, ConcurrencyPolicy,
    PermissionLevel, PermissionPolicy, SkillSource, ToolSource,
};
use crate::apps::app::ManagedApp;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(3);
const DESCRIBE_CACHE_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
pub struct CliManifest {
    #[serde(default)]
    pub commands: Vec<CliCommandSpec>,
    #[serde(default)]
    pub skills: Vec<CliSkillSpec>,
}

#[derive(Debug, Deserialize)]
pub struct CliCommandSpec {
    pub name: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub params: Vec<CliParamSpec>,
}

#[derive(Debug, Deserialize)]
pub struct CliParamSpec {
    pub name: String,
    #[serde(rename = "type", default)]
    pub param_type: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CliSkillSpec {
    pub name: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub steps: Vec<CliSkillStep>,
}

#[derive(Debug, Deserialize)]
pub struct CliSkillStep {
    #[serde(rename = "use")]
    pub use_command: String,
}

#[derive(Debug, Clone, Default)]
pub struct CliDescribeOutput {
    pub tools: Vec<CapabilityTool>,
    pub skills: Vec<CapabilitySkill>,
    pub diagnostics: Vec<CapabilityDiagnostic>,
}

static DESCRIBE_CACHE: OnceLock<Mutex<HashMap<String, (Instant, CliDescribeOutput)>>> =
    OnceLock::new();

pub async fn describe_cli_app(app: &ManagedApp) -> CliDescribeOutput {
    let Some(binary) = app
        .cli_binary_path
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return CliDescribeOutput::default();
    };
    let cache_key = describe_cache_key(app, binary);
    if let Some(cached) = cached_describe(&cache_key) {
        return cached;
    }

    let mut command = tokio::process::Command::new(binary);
    command.args(["describe", "--json"]);
    if let Some(env) = app.cli_env_vars.as_ref() {
        command.envs(env);
    }
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    let output = match tokio::time::timeout(DESCRIBE_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => {
            let output = diagnostic(app, "describe_spawn_failed", err.to_string());
            put_describe_cache(cache_key, output.clone());
            return output;
        }
        Err(_) => {
            let output = diagnostic(
                app,
                "describe_timeout",
                format!(
                    "describe --json exceeded {}ms",
                    DESCRIBE_TIMEOUT.as_millis()
                ),
            );
            put_describe_cache(cache_key, output.clone());
            return output;
        }
    };
    if !output.status.success() {
        let output = diagnostic(
            app,
            "describe_failed",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        );
        put_describe_cache(cache_key, output.clone());
        return output;
    }
    let manifest: CliManifest = match serde_json::from_slice(&output.stdout) {
        Ok(manifest) => manifest,
        Err(err) => {
            let output = diagnostic(app, "describe_invalid_json", err.to_string());
            put_describe_cache(cache_key, output.clone());
            return output;
        }
    };
    let output = from_manifest(app, binary, manifest);
    put_describe_cache(cache_key, output.clone());
    output
}

fn describe_cache_key(app: &ManagedApp, binary: &str) -> String {
    let mut env = app
        .cli_env_vars
        .as_ref()
        .map(|vars| vars.iter().collect::<Vec<_>>())
        .unwrap_or_default();
    env.sort_by(|a, b| a.0.cmp(b.0));
    let env_fingerprint = env
        .into_iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{}\n{}\n{}", app.id, binary, env_fingerprint)
}

fn cached_describe(cache_key: &str) -> Option<CliDescribeOutput> {
    let now = Instant::now();
    let mut cache = DESCRIBE_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap();
    cache.retain(|_, (created_at, _)| now.duration_since(*created_at) <= DESCRIBE_CACHE_TTL);
    cache.get(cache_key).map(|(_, output)| output.clone())
}

fn put_describe_cache(cache_key: String, output: CliDescribeOutput) {
    let mut cache = DESCRIBE_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap();
    cache.insert(cache_key, (Instant::now(), output));
}

pub fn from_manifest(app: &ManagedApp, binary: &str, manifest: CliManifest) -> CliDescribeOutput {
    let env = app.cli_env_vars.clone().unwrap_or_default();
    let tools = manifest
        .commands
        .into_iter()
        .filter(|cmd| !cmd.name.trim().is_empty())
        .map(|cmd| {
            let permission = classify_permission(&cmd.name, cmd.category.as_deref());
            CapabilityTool {
                id: tool_id_for(&app.id, &cmd.name),
                app_id: app.id.clone(),
                name: cmd.name.clone(),
                summary: cmd.summary.clone().unwrap_or_else(|| cmd.name.clone()),
                source: ToolSource::CliCommand {
                    binary_path: binary.to_string(),
                    command: cmd.name.clone(),
                    env: env.clone(),
                },
                input_schema: params_schema(&cmd.params),
                permission,
                permission_policy: default_policy(permission),
                resources: vec![format!("cli-app:{}", app.id)],
                concurrency: ConcurrencyPolicy::Exclusive,
            }
        })
        .collect();
    let skills = manifest
        .skills
        .into_iter()
        .filter(|skill| !skill.name.trim().is_empty())
        .map(|skill| CapabilitySkill {
            id: tool_id_for(&app.id, &skill.name),
            app_id: app.id.clone(),
            name: skill.name,
            summary: skill.summary.unwrap_or_default(),
            content: None,
            source: SkillSource::Manifest {
                steps: skill.steps.into_iter().map(|s| s.use_command).collect(),
            },
        })
        .collect();
    CliDescribeOutput {
        tools,
        skills,
        diagnostics: Vec::new(),
    }
}

fn diagnostic(app: &ManagedApp, code: &str, message: String) -> CliDescribeOutput {
    CliDescribeOutput {
        diagnostics: vec![CapabilityDiagnostic {
            app_id: app.id.clone(),
            code: code.to_string(),
            message,
        }],
        ..Default::default()
    }
}

pub fn classify_permission(command: &str, category: Option<&str>) -> PermissionLevel {
    match category.map(|v| v.to_ascii_lowercase()) {
        Some(c) if c == "read" => return PermissionLevel::Read,
        Some(c) if c == "write" => return classify_write_command(command),
        _ => {}
    }
    match command {
        "profile" | "timeline" | "trending" | "search" | "followers" | "followings"
        | "bookmarks" | "likes" | "notifications" | "article" | "download" | "tweet"
        | "replies" => PermissionLevel::Read,
        other => classify_write_command(other),
    }
}

fn classify_write_command(command: &str) -> PermissionLevel {
    match command {
        "delete" | "block" | "unblock" | "hide_reply" => PermissionLevel::Destructive,
        _ => PermissionLevel::Write,
    }
}

pub fn default_policy(permission: PermissionLevel) -> PermissionPolicy {
    match permission {
        PermissionLevel::Read => PermissionPolicy::Allow,
        PermissionLevel::Write => PermissionPolicy::Confirm,
        PermissionLevel::Destructive => PermissionPolicy::Deny,
    }
}

fn params_schema(params: &[CliParamSpec]) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for param in params {
        if param.name.trim().is_empty() {
            continue;
        }
        let typ = match param.param_type.as_deref().unwrap_or("string") {
            "integer" | "number" => "integer",
            "boolean" => "boolean",
            "array" => "array",
            _ => "string",
        };
        let mut prop = json!({ "type": typ });
        if let Some(desc) = param
            .description
            .as_deref()
            .filter(|v| !v.trim().is_empty())
        {
            prop["description"] = json!(desc);
        }
        properties.insert(param.name.clone(), prop);
        if param.required {
            required.push(Value::String(param.name.clone()));
        }
    }
    properties.insert(
        "cdp_port".to_string(),
        json!({
            "type": "string",
            "description": "Optional CDP port/session used by tools that need agent-browser."
        }),
    );
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": true
    })
}

pub fn raw_cli_tool(app: &ManagedApp) -> Option<CapabilityTool> {
    let binary = app.cli_binary_path.as_deref()?.trim();
    if binary.is_empty() {
        return None;
    }
    Some(CapabilityTool {
        id: tool_id_for(&app.id, "run"),
        app_id: app.id.clone(),
        name: "run".to_string(),
        summary: format!("Run CLI app {}", app.name),
        source: ToolSource::CliRaw {
            binary_path: binary.to_string(),
            env: app.cli_env_vars.clone().unwrap_or_default(),
        },
        input_schema: json!({
            "type": "object",
            "properties": {
                "args": { "type": "array", "items": { "type": "string" } },
                "timeout_ms": { "type": "integer" }
            },
            "required": ["args"],
            "additionalProperties": false
        }),
        permission: PermissionLevel::Write,
        permission_policy: PermissionPolicy::Confirm,
        resources: vec![format!("cli-app:{}", app.id)],
        concurrency: ConcurrencyPolicy::Exclusive,
    })
}

#[allow(dead_code)]
fn _env_keys(env: &HashMap<String, String>) -> Vec<String> {
    let mut keys = env.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::app::{AppType, ManagedApp};

    fn app() -> ManagedApp {
        ManagedApp {
            id: "twitter-cli".to_string(),
            name: "Twitter CLI".to_string(),
            app_type: AppType::CliApp,
            autostart: false,
            url: None,
            launch_command: None,
            launch_env_vars: None,
            launch_cwd: None,
            launch_wait_timeout_secs: None,
            exec_command: None,
            env_vars: None,
            cli_binary_path: Some("/tmp/twitter-cli".to_string()),
            cli_env_vars: None,
            skill_paths: None,
            created_at: "now".to_string(),
        }
    }

    #[test]
    fn manifest_commands_become_tools_with_permissions() {
        let manifest = CliManifest {
            commands: vec![
                CliCommandSpec {
                    name: "search".to_string(),
                    category: Some("read".to_string()),
                    summary: Some("Search tweets".to_string()),
                    params: vec![CliParamSpec {
                        name: "query".to_string(),
                        param_type: Some("string".to_string()),
                        required: true,
                        description: None,
                    }],
                },
                CliCommandSpec {
                    name: "post".to_string(),
                    category: Some("write".to_string()),
                    summary: None,
                    params: Vec::new(),
                },
                CliCommandSpec {
                    name: "delete".to_string(),
                    category: Some("write".to_string()),
                    summary: None,
                    params: Vec::new(),
                },
            ],
            skills: vec![CliSkillSpec {
                name: "monitor_keyword".to_string(),
                summary: Some("Monitor keyword".to_string()),
                steps: vec![CliSkillStep {
                    use_command: "search".to_string(),
                }],
            }],
        };
        let described = from_manifest(&app(), "/tmp/twitter-cli", manifest);
        assert_eq!(described.tools.len(), 3);
        assert_eq!(described.tools[0].id, "twitter_cli_search");
        assert_eq!(described.tools[0].permission, PermissionLevel::Read);
        assert_eq!(
            described.tools[1].permission_policy,
            PermissionPolicy::Confirm
        );
        assert_eq!(described.tools[2].permission_policy, PermissionPolicy::Deny);
        assert_eq!(described.skills[0].id, "twitter_cli_monitor_keyword");
    }

    #[test]
    fn input_schema_preserves_required_params() {
        let schema = params_schema(&[CliParamSpec {
            name: "query".to_string(),
            param_type: Some("string".to_string()),
            required: true,
            description: Some("Search query".to_string()),
        }]);
        assert_eq!(schema["required"][0], "query");
        assert_eq!(schema["properties"]["cdp_port"]["type"], "string");
    }
}
