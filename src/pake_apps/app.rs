use serde::{Serialize, Deserialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppType {
    WebApp,
    DesktopApp,
}

impl AppType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AppType::WebApp => "webapp",
            AppType::DesktopApp => "desktop",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "webapp" => Some(AppType::WebApp),
            "desktop" => Some(AppType::DesktopApp),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppMode {
    Native,
    Webview,
}

impl AppMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AppMode::Native => "native",
            AppMode::Webview => "webview",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "native" => Some(AppMode::Native),
            "webview" => Some(AppMode::Webview),
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
pub struct PakeApp {
    pub id: String,
    pub name: String,
    pub app_type: AppType,

    // WebApp fields
    pub url: Option<String>,
    pub mode: Option<AppMode>,
    pub show_nav: bool,

    // DesktopApp fields
    pub exec_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_vars: Option<HashMap<String, String>>,

    pub created_at: String,
}
