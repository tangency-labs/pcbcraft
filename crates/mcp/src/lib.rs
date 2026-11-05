//! PCBCraft's MCP server.
//!
//! JSON-RPC 2.0 over stdio (newline-delimited), implementing the MCP lifecycle, tools and resources.
//! Tools are thin wrappers over engine commands, so an agent does what the app's menus do, through
//! the same code. Agents never touch project files.
//!
//! Adapted from SolidCraft's `crates/mcp` (see `ATTRIBUTION.md`).

mod backend;
#[cfg(not(target_arch = "wasm32"))]
pub mod live;
mod server;
mod tools;

pub use backend::{Backend, Headless};
pub use server::{PROTOCOL_VERSION, SUPPORTED_VERSIONS, Server};
pub use tools::{call_tool, tool_definitions};

#[cfg(test)]
mod tests;
