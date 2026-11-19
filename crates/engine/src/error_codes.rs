//! Stable codes for every refusal and failure a command can report.
//!
//! A failure carries a snake_case code from [`ErrorCode::ALL`] (or, over MCP, a newer editor's
//! code passed through) beside its message. The code is
//! the contract a client branches on; the message is for people and may be reworded. The list
//! is published in `docs/mcp.md` and locked in `docs/schemas/error-codes-v1.json`: codes are only
//! ever appended, never renamed, removed or reused. A code that stops being produced stays in the
//! list.

/// One published failure code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorCode {
    /// No command has the requested id.
    UnknownCommand,
    /// The parameters do not fit the command's described schema, or are refused by its own checks.
    InvalidParams,
    /// The command exists but cannot run in the current state.
    Unavailable,
    /// A revision-qualified request or review token does not match the current revision.
    StaleRevision,
    /// An id names nothing in the current project. Some commands report an id that is not of
    /// the expected form the same way, others as [`ErrorCode::InvalidParams`].
    NotFound,
    /// The edit would exceed a fixed limit.
    CapExceeded,
    /// The command acts on an open editor window and there is none.
    NeedsLiveEditor,
    /// The person declined the request in the editor.
    Denied,
    /// The request was canceled or expired before it ran to completion; nothing changed.
    Cancelled,
    /// The editor is occupied with another request of the same kind, or has no room for another.
    Busy,
    /// No usable reply came back from the editor, so the command may have run.
    NoReply,
    /// Connecting or sending the request failed. A failed send may still have delivered a
    /// complete request, so inspect the project before retrying an edit.
    ConnectionFailed,
    /// The request itself is malformed: an unknown method or a missing command id.
    InvalidRequest,
    /// The call states a version of the command other than the one this build runs.
    VersionMismatch,
    /// The session's access scope does not allow the command.
    OutOfScope,
}

impl ErrorCode {
    /// Every published code, in publication order. Codes are only appended.
    pub const ALL: &[Self] = &[
        Self::UnknownCommand,
        Self::InvalidParams,
        Self::Unavailable,
        Self::StaleRevision,
        Self::NotFound,
        Self::CapExceeded,
        Self::NeedsLiveEditor,
        Self::Denied,
        Self::Cancelled,
        Self::Busy,
        Self::NoReply,
        Self::ConnectionFailed,
        Self::InvalidRequest,
        Self::VersionMismatch,
        Self::OutOfScope,
    ];

    /// The code as clients see it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownCommand => "unknown_command",
            Self::InvalidParams => "invalid_params",
            Self::Unavailable => "unavailable",
            Self::StaleRevision => "stale_revision",
            Self::NotFound => "not_found",
            Self::CapExceeded => "cap_exceeded",
            Self::NeedsLiveEditor => "needs_live_editor",
            Self::Denied => "denied",
            Self::Cancelled => "cancelled",
            Self::Busy => "busy",
            Self::NoReply => "no_reply",
            Self::ConnectionFailed => "connection_failed",
            Self::InvalidRequest => "invalid_request",
            Self::VersionMismatch => "version_mismatch",
            Self::OutOfScope => "out_of_scope",
        }
    }

    /// The published code spelled `code`, if there is one.
    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|known| known.as_str() == code)
    }
}

impl std::fmt::Display for ErrorCode {
    /// The code as clients see it, the same as [`ErrorCode::as_str`].
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests;
