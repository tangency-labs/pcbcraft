//! Coding agents and model endpoints for a Craft application's chat, recreated from SolidCraft's
//! agent chat until it is extracted as a shared library (see `ATTRIBUTION.md`). The host gives
//! the chat its HTTP client, keychain and live control endpoint; nothing here draws a window.
//!
//! - [`agents`]: the agents found and added, their settings and permission modes.
//! - [`discover`]: which ACP agents are installed, without signing in or starting a session.
//! - [`acp`] and [`acp_client`]: the Agent Client Protocol and its backend.
//! - [`api_client`]: OpenAI- and Anthropic-compatible endpoints, with the app running the tool loop.
//! - [`process`]: an agent as a child process in its own process group.
//! - [`backend`]: the seam between a chat panel and whatever answers it.
//! - [`markdown`]: a bounded Markdown reader for replies.

pub mod acp;
pub mod acp_client;
pub mod agents;
pub mod api_client;
pub mod backend;
pub mod discover;
mod i18n;
pub mod markdown;
pub mod process;

/// Most models kept from one listing.
const MODEL_LIST_CAP: usize = 2000;
/// Longest model id kept from a listing or sent to an agent.
pub const MODEL_ID_CAP: usize = 256;

/// Whether a model id can go to an agent: not empty, not too long, no control characters.
pub fn model_id_valid(id: &str) -> bool {
    !id.is_empty() && id.chars().count() <= MODEL_ID_CAP && !id.chars().any(char::is_control)
}

/// A tool call for a backend that runs its own tool loop in process, as the host's live control
/// requests take it: a method, its parameters and where the answer goes.
#[derive(Debug)]
pub struct ControlRequest {
    pub method: String,
    pub params: serde_json::Value,
    pub reply: std::sync::mpsc::Sender<Result<serde_json::Value, String>>,
}
