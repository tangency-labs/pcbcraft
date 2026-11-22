//! Runtime authority for command dispatch. Project snapshots do not carry permission to edit.

use crate::{
    CANCEL_BOARD_SETUP_PREVIEW, CANCEL_UPDATE, CHECK_DESIGN, CommandError, EXPORT_FABRICATION, PREVIEW_ASSEMBLY_PROFILE, PREVIEW_BOARD_SETUP,
    PREVIEW_UPDATE, Session, annotation, board, clipboard, clipboard_text, command, library::sources, net_classes, netlist, nets, no_connect,
    parameters, part_groups, sheets,
};

/// Command access set by the host when a session is created.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionScope {
    /// Inspect documents and run non-authoring commands, including session-local previews.
    ReadOnly,
    /// Run the normal command set, subject to each command's validation.
    #[default]
    Edit,
}

impl SessionScope {
    /// Stable spelling used in session discovery and inspection.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Edit => "edit",
        }
    }
}

impl Session {
    /// Start an empty session with a fixed command scope. Trusted hosts may load a snapshot;
    /// protocol requests cannot change the scope or use host lifecycle methods.
    pub fn with_scope(scope: SessionScope) -> Self {
        Self { scope, ..Self::default() }
    }

    /// The access scope this session was created with, independent of its open project.
    pub const fn scope(&self) -> SessionScope {
        self.scope
    }

    /// Allow reviewed read commands; new commands require explicit review before they are
    /// admitted to read-only sessions. Refuse before interpreting user-supplied edit parameters.
    pub(crate) fn authorize_command(&self, id: &str) -> Result<(), CommandError> {
        if self.scope == SessionScope::Edit || is_read_command(id) {
            return Ok(());
        }
        let known = command(id).ok_or_else(|| CommandError::Unknown(id.to_owned()))?;
        Err(CommandError::OutOfScope { command: known.id })
    }
}

/// These commands return data or manage only ephemeral review state. In particular, Copy
/// returns data without setting a system clipboard. Export/publication preparation does
/// not write files, netlist comparison only reads the text it is given, and annotation
/// preview caches no proposal. Lifecycle lookup and inspection only read exact pinned or
/// loaded content. Loading/removing library sources, lifecycle adoption and Cut are not reads.
fn is_read_command(id: &str) -> bool {
    matches!(
        id,
        no_connect::CHECK
            | clipboard_text::LABELS
            | clipboard_text::OUTLINE
            | clipboard::COPY
            | board::clipboard::COPY
            | sheets::page::LIST_TEMPLATES
            | parameters::LIST
            | parameters::RESOLVE
            | annotation::PREVIEW
            | netlist::EXPORT_NETLIST
            | netlist::COMPARE_NETLIST
            | netlist::EXPORT_BOARD_NETLIST
            | PREVIEW_UPDATE
            | CANCEL_UPDATE
            | PREVIEW_BOARD_SETUP
            | CANCEL_BOARD_SETUP_PREVIEW
            | PREVIEW_ASSEMBLY_PROFILE
            | board::class_rules::ROUTE_RULES
            | board::route::PLAN_ROUTE
            | CHECK_DESIGN
            | EXPORT_FABRICATION
            | net_classes::INSPECT
            | net_classes::RESOLVE
            | nets::INSPECT_CLASSES
            | part_groups::INSPECT
            | part_groups::INSPECT_GROUPS
            | sources::LIST
            | sources::SEARCH
            | sources::AVAILABLE_REVISIONS
            | sources::WHERE_USED
            | sources::RESOLVE_PART
            | sources::PARSE_QUANTITY
            | sources::PREVIEW_PIN_REMAP
            | sources::PREVIEW_FOOTPRINT_REVISION
            | sources::PREPARE_PART_PUBLICATION
            | sources::PREPARE_PUBLICATION
            | sources::lifecycle::LOOKUP
            | sources::lifecycle::INSPECT
    )
}

#[cfg(test)]
mod tests;
