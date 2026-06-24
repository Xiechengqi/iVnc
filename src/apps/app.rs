use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppType {
    BackgroundApp,
    DesktopApp,
    CliApp,
}

impl AppType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AppType::BackgroundApp => "background",
            AppType::DesktopApp => "desktop",
            AppType::CliApp => "cli",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "background" => Some(AppType::BackgroundApp),
            "desktop" => Some(AppType::DesktopApp),
            "cli" => Some(AppType::CliApp),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppStatus {
    Running,
    Stopped,
    Crashed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedApp {
    pub id: String,
    pub name: String,
    pub app_type: AppType,
    pub autostart: bool,

    // BackgroundApp fields
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch_env_vars: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch_cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch_wait_timeout_secs: Option<u64>,

    // DesktopApp fields
    pub exec_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_vars: Option<HashMap<String, String>>,

    // CliApp fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_binary_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_env_vars: Option<HashMap<String, String>>,

    // Agent guidance for this app.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_paths: Option<Vec<String>>,

    pub created_at: String,
}
