//! HTTP server for health checks and metrics
//!
//! Provides a lightweight HTTP server for monitoring.

mod access;
pub mod auth;
pub mod shared;
pub use shared::SharedState;

pub mod embedded_assets;

mod github_update;
pub mod http_server;
mod proxy_cookies;
pub use http_server::run_http_server_with_webrtc;
