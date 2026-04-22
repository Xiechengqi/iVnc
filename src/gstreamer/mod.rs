//! GStreamer integration for video encoding
//!
//! This module provides GStreamer-based video pipeline using appsrc
//! for encoding compositor frames to H.264/VP8/VP9 for WebRTC streaming.

pub mod encoder;
pub mod pipeline;

pub use pipeline::{PipelineConfig, VideoPipeline};

use std::error::Error;
use std::fmt;

/// GStreamer-related errors
#[derive(Debug)]
pub enum GstError {
    /// GStreamer initialization failed
    InitFailed(String),
    /// Pipeline creation failed
    PipelineFailed(String),
    /// Encoder not available
    EncoderNotFound(String),
    /// Element linking failed
    LinkFailed(String),
    /// State change failed
    StateChangeFailed(String),
}

impl fmt::Display for GstError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GstError::InitFailed(msg) => write!(f, "GStreamer init failed: {}", msg),
            GstError::PipelineFailed(msg) => write!(f, "Pipeline creation failed: {}", msg),
            GstError::EncoderNotFound(msg) => write!(f, "Encoder not found: {}", msg),
            GstError::LinkFailed(msg) => write!(f, "Element linking failed: {}", msg),
            GstError::StateChangeFailed(msg) => write!(f, "State change failed: {}", msg),
        }
    }
}

impl Error for GstError {}
