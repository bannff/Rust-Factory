#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(clippy::missing_errors_doc)]

//! Minimal provider-neutral Sandbox capability.
//!
//! The core exposes start, execute, status, and stop. It claims no persistence,
//! replay, retries, recovery, or exactly-once effects. Optional Docker and MCP
//! adapters are feature-gated; process and transport lifecycle remain external.

#[cfg(feature = "docker")]
pub mod docker;
#[cfg(feature = "mcp")]
pub mod mcp;
#[cfg(feature = "observability")]
pub mod observability;

mod error;
mod model;
mod port;
mod service;
mod validation;

pub use error::SandboxError;
pub use model::*;
pub use port::*;
pub use service::{DenySandbox, DropEvents, SandboxService};
