//! Application capability discovery and invocation.
//!
//! This layer keeps iVNC as a capability hub: managed apps expose CLI
//! commands as tools, and the web console and HTTP API discover and call
//! them through one audited gateway.

pub mod call_log;
pub mod cli_describe;
pub mod executor;
pub mod locks;
pub mod registry;
pub mod types;

pub use executor::{call_tool, CapabilityCallRequest};
pub use registry::build_snapshot;
pub use types::*;
