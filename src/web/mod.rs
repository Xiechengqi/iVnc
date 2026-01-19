//! HTTP server for health checks and metrics
//!
//! Provides a lightweight HTTP server for monitoring.

pub mod shared;
pub use shared::SharedState;

pub mod embedded_assets;

pub mod http_server;
#[cfg(feature = "webrtc-streaming")]
pub use http_server::run_http_server_with_webrtc;
#[cfg(not(feature = "webrtc-streaming"))]
pub use http_server::run_http_server;
