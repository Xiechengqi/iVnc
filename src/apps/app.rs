use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppType {
    BackgroundApp,
    DesktopApp,
    CliApp,
    RemoteApp,
}

impl AppType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AppType::BackgroundApp => "background",
            AppType::DesktopApp => "desktop",
            AppType::CliApp => "cli",
            AppType::RemoteApp => "remote",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "background" => Some(AppType::BackgroundApp),
            "desktop" => Some(AppType::DesktopApp),
            "cli" => Some(AppType::CliApp),
            "remote" => Some(AppType::RemoteApp),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppStatus {
    Running,
    /// Alive but its runtime is not ready yet.
    Starting,
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

    pub created_at: String,
    /// Persistent display position. Launch order is independent of this field.
    #[serde(default)]
    pub sort_order: i64,
}

#[cfg(test)]
impl ManagedApp {
    /// An application with only its identity set, for tests.
    pub fn for_test(id: &str, app_type: AppType) -> Self {
        Self {
            id: id.into(),
            name: "App".into(),
            app_type,
            autostart: false,
            url: None,
            launch_command: None,
            launch_env_vars: None,
            launch_cwd: None,
            launch_wait_timeout_secs: None,
            exec_command: (app_type == AppType::DesktopApp).then(|| "true".into()),
            env_vars: None,
            cli_binary_path: None,
            cli_env_vars: None,
            created_at: String::new(),
            sort_order: 0,
        }
    }
}
