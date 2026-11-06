//! Schematic view state and input addresses. A view borrows the project session; it never owns
//! a document copy, file lifecycle or undo history.

use pcbcraft_engine::{Session, SheetTarget, selection::Selection};

use crate::schematic_canvas::{Camera, Draft, Tool};

#[cfg(test)]
mod tests;

/// Runtime identity assigned by the host, independent of a sheet or native window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) struct ViewId(pub(super) u64);

/// Address of input captured in a view at a particular document revision. Delayed actions must
/// validate this address before reading selection, changing a preview or dispatching an edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ViewInput {
    view: ViewId,
    target: SheetTarget,
    input_epoch: u64,
    pub(super) revision: u64,
}

/// A persistent sheet address plus the runtime file adoption that bound it to the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Binding {
    target: SheetTarget,
    history_root_revision: u64,
}

impl Binding {
    /// Resolve the current sheet, distinguishing a reopened file from the preceding history.
    fn current(session: &Session) -> Option<Self> {
        Some(Self { target: session.project()?.sheet_target(), history_root_revision: session.history_root_revision() })
    }
}

/// The camera, selection and unfinished interaction for one schematic view. Panels and project
/// services remain with the application host in this slice; none of these fields are persisted.
#[derive(Debug)]
pub(super) struct SchematicView {
    id: ViewId,
    binding: Option<Binding>,
    input_epoch: u64,
    pub(super) camera: Camera,
    pub(super) selection: Selection,
    pub(super) tool: Option<Tool>,
    pub(super) wire_draft: Draft,
    pub(super) rotate_requested: bool,
}

impl Default for SchematicView {
    fn default() -> Self {
        Self::new(ViewId::default())
    }
}

impl SchematicView {
    /// Construct an unbound view with a host-assigned identity and default navigation.
    pub(super) fn new(id: ViewId) -> Self {
        Self {
            id,
            binding: None,
            input_epoch: 0,
            camera: Camera::default(),
            selection: Selection::default(),
            tool: None,
            wire_draft: Draft::default(),
            rotate_requested: false,
        }
    }

    /// Identify this view for input routing and egui widget scopes, regardless of its host.
    pub(super) fn id(&self) -> ViewId {
        self.id
    }

    /// Resolve the displayed sheet path from the current binding, never from the runtime view
    /// number. The engine currently has one sheet, Main; hierarchy will extend this resolution.
    /// Renaming the project updates the label without changing the view or sheet identity.
    pub(super) fn sheet_path(&self, session: &Session) -> Option<String> {
        self.input(session)?;
        Some(format!("{} / Main", session.project()?.name))
    }

    /// Scope egui interactions to this view's current opening. A held pointer or context menu
    /// from an earlier project must not attach itself to a replacement with the same object IDs.
    pub(super) fn widget_id(&self) -> egui::Id {
        egui::Id::new((self.id, self.binding))
    }

    /// Capture an input address only while this view is bound to the session's current sheet.
    pub(super) fn input(&self, session: &Session) -> Option<ViewInput> {
        let binding = self.binding.filter(|binding| Some(*binding) == Binding::current(session))?;
        Some(ViewInput { view: self.id, target: binding.target, input_epoch: self.input_epoch, revision: session.revision() })
    }

    /// Reject input from another view, another sheet, a reopened file, an earlier focus interval
    /// or an outdated revision.
    pub(super) fn accepts(&self, input: ViewInput, session: &Session) -> bool {
        self.input(session) == Some(input)
    }

    /// Cancel the local gesture, including queued keyboard input and wire corners. This never
    /// changes document state or selection and is safe when a dialog or Home takes over input.
    pub(super) fn cancel_gesture(&mut self) {
        self.tool = None;
        self.wire_draft = Draft::default();
        self.rotate_requested = false;
    }

    /// Relinquish input when focus or hosting changes. Old events stay invalid even after this
    /// view becomes active again without a document edit. Navigation and selection survive.
    pub(super) fn release_input(&mut self) {
        self.cancel_gesture();
        self.input_epoch += 1;
    }

    /// Rebind after project replacement or reopening, preserving the view's runtime identity.
    /// Ordinary edits and undo retain the camera and surviving selection, but cancel old drafts.
    /// Return whether the sheet or history root changed. Synchronization never edits the session.
    pub(super) fn sync(&mut self, session: &Session) -> bool {
        let binding = Binding::current(session);
        let changed = self.binding != binding;
        if changed {
            *self = Self::new(self.id);
            self.binding = binding;
        }
        if self.tool.as_ref().is_some_and(|tool| tool.revision() != session.revision()) {
            self.cancel_gesture();
        }
        if !matches!(self.tool, Some(Tool::Wire { .. })) {
            self.wire_draft = Draft::default();
        }
        if let Some(project) = session.project() {
            self.selection.retain(project);
        } else {
            self.selection = Selection::default();
        }
        changed
    }
}
