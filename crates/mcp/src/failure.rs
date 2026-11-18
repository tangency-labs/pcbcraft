//! A failed tool call or live request: a published code, a message for people and details.

use pcbcraft_engine::{CommandError, error_codes::ErrorCode};
use serde_json::{Value, json};

/// Why a call failed. The code is one of [`ErrorCode::ALL`], or a newer one passed through from
/// an editor that publishes more; clients branch on it, never on the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    code: String,
    message: String,
    details: Value,
}

impl Failure {
    /// A failure with no details.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code: code.as_str().to_owned(), message: message.into(), details: json!({}) }
    }

    /// Attach an object of machine-readable facts, such as the current revision of a stale edit.
    pub fn with_details(mut self, details: serde_json::Map<String, Value>) -> Self {
        self.details = Value::Object(details);
        self
    }

    /// Read a live editor's error reply. A reply without a well-formed code, from an editor that
    /// predates codes, reads as unavailable. Missing or non-object details read as an empty object.
    pub fn from_reply(reply: &Value, message: &str) -> Self {
        let code = reply["code"]
            .as_str()
            .filter(|code| !code.is_empty() && code.split('_').all(|word| !word.is_empty() && word.bytes().all(|byte| byte.is_ascii_lowercase())));
        let details = reply.get("details").filter(|details| details.is_object()).cloned().unwrap_or_else(|| json!({}));
        Self { code: code.unwrap_or(ErrorCode::Unavailable.as_str()).to_owned(), message: message.to_owned(), details }
    }

    /// The stable code, snake_case.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// The message for people, which may be reworded between releases.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Machine-readable facts, always an object.
    pub fn details(&self) -> &Value {
        &self.details
    }

    /// The failure as replies carry it: `{"code", "message", "details"}`.
    pub fn to_json(&self) -> Value {
        json!({"code": self.code, "message": self.message, "details": self.details})
    }
}

impl std::fmt::Display for Failure {
    /// The message alone, as tool results and replies carried it before codes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Failure {}

/// An engine refusal keeps its code, message and details.
impl From<CommandError> for Failure {
    fn from(error: CommandError) -> Self {
        Self { code: error.code().as_str().to_owned(), message: error.to_string(), details: error.details() }
    }
}

/// A plain refusal reads as unavailable: the request was understood but cannot run now.
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::new(ErrorCode::Unavailable, message)
    }
}

/// A plain refusal reads as unavailable, as for [`String`].
impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Self::new(ErrorCode::Unavailable, message)
    }
}
