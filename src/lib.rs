//! Cubase MCP integration daemon.
//!
//! The crate deliberately separates MCP, the logical bridge protocol, and the
//! physical bridge transport. This keeps Cubase-specific integration details
//! out of the MCP-facing layer.

pub mod bridge;
pub mod config;
pub mod installer;
pub mod mcp;
pub mod protocol;
pub mod service;
pub(crate) mod tools;
