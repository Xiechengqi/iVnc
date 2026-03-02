use serde::{Serialize, Deserialize};

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
    pub url: String,
    pub mode: AppMode,
    pub dark_mode: bool,
    pub autostart: bool,
    pub show_nav: bool,
    pub created_at: String,
}
