//! Wayland compositor module based on smithay
//!
//! Provides an embedded headless Wayland compositor using smithay's
//! Pixman software renderer for zero-copy frame capture.

pub mod grabs;
pub mod handlers;
pub mod headless;
pub mod state;

pub use headless::HeadlessBackend;
pub use state::Compositor;
