//! PCBCraft's MCP server.
//!
//! JSON-RPC 2.0 over stdio (newline-delimited), implementing the MCP lifecycle, tools and resources.
//! Tools are thin wrappers over engine commands, so an agent does what the app's menus do, through
//! the same code. A live editor adds window commands, run by the window's own handlers. Agents
//! never touch project files.
//!
//! Adapted from SolidCraft's `crates/mcp` (see `ATTRIBUTION.md`).

mod backend;
#[cfg(not(target_arch = "wasm32"))]
pub mod live;
mod server;
mod tools;

pub use backend::{Backend, Headless, WINDOW_NAMESPACES, is_window_command};
pub use server::{PROTOCOL_VERSION, SUPPORTED_VERSIONS, Server};
pub use tools::{call_tool, tool_definitions};

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod net_registry_tests;
#[cfg(test)]
mod tests;
