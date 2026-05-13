//! Application management module.
//!
//! Manages web-to-desktop applications with native Chrome windows.

pub mod api;
pub mod app;
pub mod datadir;
pub mod desktop_entry;
pub mod native;
pub mod process;
pub mod service_process;
pub mod state_recovery;
pub mod store;
