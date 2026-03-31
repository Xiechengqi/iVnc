//! Pake application management module
//!
//! Manages web-to-desktop applications with native (Chrome --app) and webview modes.

pub mod api;
pub mod app;
pub mod datadir;
pub mod desktop_entry;
pub mod native;
pub mod process;
pub mod state_recovery;
pub mod store;
pub mod webview_manager_process;

// Re-export the process-based WebViewManager as the default
pub use webview_manager_process::WebViewManager;
