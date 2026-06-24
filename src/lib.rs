//! iVnc - WebRTC streaming core
//!
//! A high-performance WebRTC streaming solution using smithay Wayland compositor and GStreamer.

#[cfg(feature = "agent")]
pub mod agent;
pub mod apps;
pub mod audio;
pub mod capabilities;
pub mod clipboard;
pub mod compositor;
pub mod config;
#[cfg(feature = "agent")]
pub mod console_config;
pub mod file_upload;
pub mod gstreamer;
pub mod input;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod proxy_panel;
pub mod runtime_settings;
pub mod system_clipboard;
pub mod terminal;
pub mod transport;
pub mod web;
pub mod webrtc;

// Re-exports
pub use config::{Config, HardwareEncoder, VideoCodec, WebRTCConfig};
pub use gstreamer::{PipelineConfig, VideoPipeline};
pub use input::{InputEvent, InputEventData};
pub use webrtc::{SessionManager, SignalingMessage};
