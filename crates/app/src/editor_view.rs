//! Document-addressed view state. Hosts own placement; the session owns authored data.

use crate::{
    canvas_camera::Camera,
    schematic_canvas::{Draft, LegTarget, Tool},
};
use pcbcraft_engine::{Session, SheetTarget, board::view::BoardTarget, selection::Selection};

#[cfg(test)]
mod tests;

/// Runtime view identity, independent of its document or native window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct ViewId(pub(super) u64);

/// Built-in document presentations, not a plugin package API. Plugin tools/providers use
/// host contracts without acquiring ownership of the documents they inspect or edit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum Workspace {
    #[default]
    Schematic,
    Board,
}

impl Workspace {
    /// Resolve a presentation name without using it as a persistent identity.
    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Schematic => "Schematic",
            Self::Board => "PCB layout",
        }
    }
}

/// Captured input expires across focus, workspace, project-opening and revision changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ViewInput {
    view: ViewId,
    target: DocumentTarget,
    input_epoch: u64,
    pub(super) revision: u64,
}

impl ViewInput {
    /// Address the view independently of where the host currently paints it.
    pub(super) fn view(self) -> ViewId {
        self.view
    }
}

/// Document identity is distinct from the local view and display name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(super) enum DocumentTarget {
    Sheet(SheetTarget),
    Board(BoardTarget),
}

impl DocumentTarget {
    /// Resolve only an explicitly addressed document in this project; names are not identities.
    pub(super) fn available(self, session: &Session) -> bool {
        session.project().is_some_and(|project| match self {
            Self::Sheet(target) => project.for_sheet(target).is_some(),
            Self::Board(target) => project.board_view(target).is_some(),
        })
    }
}

/// Serializable presentation for an explicitly addressed document, excluding edit authority.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ViewState {
    target: DocumentTarget,
    title: String,
    camera: crate::canvas_camera::CameraState,
    parked_camera: crate::canvas_camera::CameraState,
    board: Option<BoardTarget>,
    #[serde(default)]
    schematic: Option<SheetTarget>,
    hidden_layers: std::collections::BTreeSet<String>,
    active_layer: Option<String>,
    layers_collapsed: bool,
    browser_collapsed: bool,
    properties_collapsed: bool,
}

impl ViewState {
    /// Bound layer data and validate transforms before replacing live views.
    pub(super) fn valid(&self) -> bool {
        self.title.len() <= 1024
            && self.camera.valid()
            && self.parked_camera.valid()
            && self.hidden_layers.len() <= 128
            && self.hidden_layers.iter().chain(self.active_layer.iter()).all(|id| id.len() <= 128)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Binding {
    target: DocumentTarget,
    history_root_revision: u64,
}

impl Binding {
    /// Resolve a target in the current opening, including a board without setup or placements.
    fn current(session: &Session, workspace: Workspace, board: Option<BoardTarget>, sheet: Option<SheetTarget>) -> Option<Self> {
        let project = session.project()?;
        let target = match workspace {
            Workspace::Schematic => DocumentTarget::Sheet(project.for_sheet(sheet.unwrap_or_else(|| project.sheet_target()))?.sheet_target()),
            Workspace::Board => DocumentTarget::Board(project.board_view(board?)?.target),
        };
        Some(Self { target, history_root_revision: session.history_root_revision() })
    }
}

/// Schematic interaction state is separate from board selection and input handling.
#[derive(Debug, Default)]
pub(super) struct SchematicState {
    pub(super) properties: crate::properties::State,
    pub(super) selection: Selection,
    pub(super) selected_sheet_symbol: Option<u64>,
    pub(super) selected_leg: Option<LegTarget>,
    pub(super) whole_wire_drag: bool,
    pub(super) tool: Option<Tool>,
    pub(super) wire_draft: Draft,
    pub(super) rotate_requested: bool,
    pub(super) move_requested: bool,
}

/// Board interaction state. Layer visibility and selection are local, never authored edits.
/// New views hide fabrication/courtyard layers once setup exists; restores preserve user choices.
#[derive(Debug, Default)]
pub(super) struct BoardState {
    pub(super) drawing: crate::board_drawing::State,
    pub(super) silkscreen_draft: Option<crate::board_panels::SilkscreenDraft>,
    pub(super) draft: Option<crate::board_canvas::Draft>,
    pub(super) move_requested: bool,
    pub(super) rotate_requested: u16,
    pub(super) selection: std::collections::BTreeSet<String>,
    pub(super) hidden_layers: std::collections::BTreeSet<String>,
    pub(super) layer_defaults_applied: bool,
    pub(super) active_layer: Option<String>,
    pub(super) layers_collapsed: bool,
    pub(super) browser_collapsed: bool,
    pub(super) properties_collapsed: bool,
    pub(super) target: Option<BoardTarget>,
}

impl BoardState {
    /// Apply view-local layer choices through the same handler for Layers and live control.
    /// Changing the active layer or hiding it cancels disposable drawing gestures.
    pub(super) fn set_layer(&mut self, id: &str, active: bool, visible: Option<bool>) {
        if active && self.active_layer.as_deref() != Some(id) {
            self.drawing.cancel();
            self.active_layer = Some(id.to_owned());
        }
        if let Some(visible) = visible {
            if visible {
                self.hidden_layers.remove(id);
            } else {
                self.drawing.cancel();
                self.hidden_layers.insert(id.to_owned());
            }
        }
    }
}

/// One view retains independent navigation and selection for each built-in workspace.
/// Neither its current target nor switching workspaces changes history or saved bytes.
#[derive(Debug)]
pub(super) struct EditorView {
    id: ViewId,
    binding: Option<Binding>,
    input_epoch: u64,
    pub(super) workspace: Workspace,
    pub(super) camera: Camera,
    parked_camera: Camera,
    pub(super) schematic: SchematicState,
    pub(super) board: BoardState,
    unavailable: Option<ViewState>,
    /// Explicit sheet address survives workspace switches; immutable projections are cached per revision.
    sheet: Option<SheetTarget>,
    snapshot_root: Option<std::sync::Arc<pcbcraft_engine::Project>>,
    sheet_snapshot: Option<std::sync::Arc<pcbcraft_engine::Project>>,
}

impl Default for EditorView {
    fn default() -> Self {
        Self::new(ViewId::default())
    }
}

impl EditorView {
    /// Return the resolved document identity, excluding unavailable placeholders.
    pub(super) fn target(&self) -> Option<DocumentTarget> {
        self.binding.map(|binding| binding.target)
    }

    /// Start a successfully opened project's default document in this workspace. Runtime identity
    /// and snap spacing survive, while old document addresses, navigation and edit authority expire.
    /// Saved arrangements are restored separately afterward and still validate explicit targets.
    pub(super) fn bind_opened_project(&mut self, session: &Session) {
        let workspace = self.workspace;
        let snap_nm = self.camera.snap_nm();
        *self = Self::new(self.id);
        self.workspace = workspace;
        self.camera.set_snap_nm(snap_nm);
        self.sync(session);
    }

    /// Explicitly retarget a view after the host validates the document's existence.
    pub(super) fn open_document(&mut self, target: DocumentTarget, session: &Session) {
        match target {
            DocumentTarget::Sheet(sheet) => {
                self.switch(Workspace::Schematic, session);
                self.sheet = Some(sheet);
                self.sync(session);
            }
            DocumentTarget::Board(board) => {
                self.switch(Workspace::Board, session);
                self.board.target = Some(board);
                self.sync(session);
            }
        }
    }

    /// Read this view's sheet from the current authoritative revision. A cached immutable
    /// projection avoids rebuilding geometry caches on every frame; stale caches never authorize input.
    pub(super) fn snapshot(&self, session: &Session) -> Option<std::sync::Arc<pcbcraft_engine::Project>> {
        let root = session.snapshot()?;
        let target = self.sheet.unwrap_or_else(|| root.sheet_target());
        if self.snapshot_root.as_ref().is_some_and(|old| std::sync::Arc::ptr_eq(old, &root))
            && self.sheet_snapshot.as_ref().is_some_and(|p| p.sheet_target() == target)
        {
            return self.sheet_snapshot.clone();
        }
        root.for_sheet(target).map(std::sync::Arc::new).or_else(|| (self.workspace == Workspace::Board).then_some(root))
    }

    /// Capture presentation only; unresolved saved targets remain intact instead of being rebound.
    pub(super) fn saved_state(&self, session: &Session) -> Option<ViewState> {
        if let Some(state) = &self.unavailable {
            return Some(state.clone());
        }
        Some(self.presentation_state(self.binding?.target, self.sheet_path(session)?))
    }

    /// Capture navigation without resolving a target, including an explicit board that just
    /// became unavailable. Selection, gestures and input authority never enter the record.
    fn presentation_state(&self, target: DocumentTarget, title: String) -> ViewState {
        ViewState {
            target,
            title,
            camera: self.camera.saved_state(),
            parked_camera: self.parked_camera.saved_state(),
            board: self.board.target,
            schematic: self.sheet,
            hidden_layers: self.board.hidden_layers.clone(),
            active_layer: self.board.active_layer.clone(),
            layers_collapsed: self.board.layers_collapsed,
            browser_collapsed: self.board.browser_collapsed,
            properties_collapsed: self.board.properties_collapsed,
        }
    }

    /// Resolve a saved target with a fresh runtime identity and no selection or gesture.
    /// Missing targets retain their snapshot and cannot authorize editing another document.
    pub(super) fn from_state(id: ViewId, state: &ViewState, session: &Session) -> Self {
        let mut view = Self::new(id);
        view.workspace = match state.target {
            DocumentTarget::Sheet(_) => Workspace::Schematic,
            DocumentTarget::Board(_) => Workspace::Board,
        };
        view.board.target = state.board;
        view.sheet = state.schematic;
        if let DocumentTarget::Sheet(sheet) = state.target {
            view.sheet = Some(sheet);
        }
        view.sync(session);
        if view.binding.is_none_or(|binding| binding.target != state.target) {
            view.binding = None;
            view.unavailable = Some(state.clone());
        }
        view.camera = Camera::from_state(&state.camera);
        view.parked_camera = Camera::from_state(&state.parked_camera);
        view.board.hidden_layers = state.hidden_layers.clone();
        view.board.layer_defaults_applied = true;
        view.board.active_layer = state.active_layer.clone();
        view.board.layers_collapsed = state.layers_collapsed;
        view.board.browser_collapsed = state.browser_collapsed;
        view.board.properties_collapsed = state.properties_collapsed;
        view
    }

    /// Unavailable saved documents occupy a closeable view but never become an editing target.
    pub(super) fn unavailable(&self) -> bool {
        self.unavailable.is_some()
    }

    /// Create an unbound schematic presentation with fresh navigation and interaction state.
    pub(super) fn new(id: ViewId) -> Self {
        Self {
            id,
            binding: None,
            input_epoch: 0,
            workspace: Workspace::Schematic,
            camera: Camera::default(),
            parked_camera: Camera::default(),
            schematic: SchematicState::default(),
            board: BoardState::default(),
            unavailable: None,
            sheet: None,
            snapshot_root: None,
            sheet_snapshot: None,
        }
    }

    /// Duplicate the displayed target/navigation and layer visibility, without copying selection
    /// or unfinished gestures. The two workspace cameras remain independent.
    pub(super) fn fork(&mut self, id: ViewId) -> Self {
        Self {
            binding: self.binding,
            sheet: self.sheet,
            snapshot_root: self.snapshot_root.clone(),
            sheet_snapshot: self.sheet_snapshot.clone(),
            workspace: self.workspace,
            camera: self.camera.fork_navigation(),
            parked_camera: self.parked_camera.fork_navigation(),
            unavailable: self.unavailable.clone(),
            board: BoardState {
                target: self.board.target,
                layers_collapsed: self.board.layers_collapsed,
                browser_collapsed: self.board.browser_collapsed,
                properties_collapsed: self.board.properties_collapsed,
                hidden_layers: self.board.hidden_layers.clone(),
                layer_defaults_applied: self.board.layer_defaults_applied,
                active_layer: self.board.active_layer.clone(),
                ..BoardState::default()
            },
            ..Self::new(id)
        }
    }

    /// Switch only this view and restore that workspace's navigation. Old input remains invalid
    /// even when switching back without any document edit.
    pub(super) fn switch(&mut self, workspace: Workspace, session: &Session) {
        if self.unavailable.is_some() {
            *self = Self::new(self.id);
            self.sync(session);
        }
        if self.workspace == workspace || session.project().is_none() {
            return;
        }
        self.release_input();
        std::mem::swap(&mut self.camera, &mut self.parked_camera);
        self.workspace = workspace;
        self.binding = Binding::current(session, workspace, self.board.target, self.sheet);
    }

    /// Identify the input owner independently of the currently focused host.
    pub(super) fn id(&self) -> ViewId {
        self.id
    }

    /// Resolve the document breadcrumb; a runtime view number is never a sheet name.
    pub(super) fn sheet_path(&self, session: &Session) -> Option<String> {
        if let Some(state) = &self.unavailable {
            return Some(format!("{} (unavailable)", state.title));
        }
        self.input(session)?;
        let name = match self.workspace {
            Workspace::Schematic => self.sheet_snapshot.as_ref()?.sheet_name(),
            Workspace::Board => session.project()?.board_view(self.board.target?)?.name(),
        };
        Some(format!("{} › {name}", session.project()?.name))
    }

    /// Both targets must resolve in this session's current opening to exchange correspondence.
    pub(super) fn shares_project(&self, other: &Self, session: &Session) -> bool {
        self.input(session).is_some() && other.input(session).is_some()
    }

    /// Two schematic views of the same sheet share literal schematic selection addresses.
    pub(super) fn shares_sheet(&self, other: &Self, session: &Session) -> bool {
        self.workspace == Workspace::Schematic && self.binding == other.binding && self.shares_project(other, session)
    }

    /// Scope widgets to their document; captured actions separately validate the input epoch.
    pub(super) fn widget_id(&self) -> egui::Id {
        egui::Id::new((self.id, self.binding))
    }

    /// Capture input only while this view is bound to its current document opening.
    pub(super) fn input(&self, session: &Session) -> Option<ViewInput> {
        let binding = self.binding.filter(|binding| Some(*binding) == Binding::current(session, self.workspace, self.board.target, self.sheet))?;
        Some(ViewInput { view: self.id, target: binding.target, input_epoch: self.input_epoch, revision: session.revision() })
    }

    /// Permit placement/close controls for an unavailable view without granting edit authority.
    pub(super) fn control_input(&self, session: &Session) -> Option<ViewInput> {
        self.input(session).or_else(|| {
            self.unavailable.as_ref().map(|state| ViewInput {
                view: self.id,
                target: state.target,
                input_epoch: self.input_epoch,
                revision: session.revision(),
            })
        })
    }

    /// Refuse another view's, old workspace's or stale revision's action.
    pub(super) fn accepts(&self, input: ViewInput, session: &Session) -> bool {
        self.input(session) == Some(input)
    }

    /// Drop unfinished gestures without altering selection or the document.
    pub(super) fn cancel_gesture(&mut self) {
        self.board.drawing.cancel();
        self.board.draft = None;
        self.board.move_requested = false;
        self.board.rotate_requested = 0;
        self.schematic.tool = None;
        self.schematic.wire_draft = Draft::default();
        self.schematic.rotate_requested = false;
        self.schematic.move_requested = false;
    }

    /// Invalidate captured input on focus or host transitions, retaining both cameras.
    pub(super) fn release_input(&mut self) {
        self.cancel_gesture();
        self.input_epoch += 1;
    }

    /// Reopening resets local state while retaining workspace choice and grid. Ordinary edits
    /// prune removed selections/layers. A stale segment clears its whole selection so Delete
    /// cannot widen into a wire deletion; synchronization never mutates the session. Explicit
    /// sheet/board targets that no longer resolve become unavailable, closeable placeholders;
    /// undo restores their address and navigation. Only omitted targets use compatibility defaults.
    pub(super) fn sync(&mut self, session: &Session) -> bool {
        if let Some(state) = &self.unavailable {
            if state.target.available(session) {
                *self = Self::from_state(self.id, &state.clone(), session);
                return true;
            }
            return false;
        }
        let root = session.snapshot();
        if self.snapshot_root.as_ref().zip(root.as_ref()).is_some_and(|(old, new)| !old.same_document(new)) {
            self.sheet = None;
        }
        if self.sheet.is_none() {
            self.sheet = root.as_ref().map(|p| p.sheet_target());
        }
        if self.workspace == Workspace::Schematic
            && self.sheet.is_some()
            && root.as_ref().is_some_and(|p| self.sheet.and_then(|target| p.for_sheet(target)).is_none())
        {
            let target = self.sheet.map(DocumentTarget::Sheet);
            if let Some(target) = target {
                let title = self.sheet_snapshot.as_ref().map_or("Sheet", |p| p.sheet_name()).to_owned();
                self.unavailable = Some(self.presentation_state(target, title));
                self.binding = None;
                self.release_input();
                self.schematic.selection = Selection::default();
                self.sheet_snapshot = None;
            }
            return true;
        }
        let board_target = self.board.target.or_else(|| session.project().map(|p| p.board_target()));
        if self.workspace == Workspace::Board
            && let Some(target) = board_target
            && session.project().is_some_and(|project| project.board_view(target).is_none())
        {
            self.unavailable = Some(self.presentation_state(DocumentTarget::Board(target), Workspace::Board.title().into()));
            self.release_input();
            self.binding = None;
            self.schematic = SchematicState::default();
            self.board.selection.clear();
            return true;
        }
        let binding = Binding::current(session, self.workspace, board_target, self.sheet);
        let changed = self.binding != binding;
        if changed {
            let snap_nm = self.camera.snap_nm();
            let workspace = self.workspace;
            let sheet = self.sheet;
            *self = Self::new(self.id);
            self.sheet = sheet;
            self.workspace = workspace;
            self.camera.set_snap_nm(snap_nm);
            self.binding = binding;
        }
        self.board.target = board_target;
        if !self.snapshot_root.as_ref().zip(root.as_ref()).is_some_and(|(old, new)| std::sync::Arc::ptr_eq(old, new))
            || self.sheet_snapshot.as_ref().map(|p| p.sheet_target()) != self.sheet
        {
            self.sheet_snapshot = root.as_ref().and_then(|p| {
                let target = self.sheet?;
                if p.sheet_target() == target { Some(p.clone()) } else { p.for_sheet(target).map(std::sync::Arc::new) }
            });
            self.snapshot_root = root;
        }
        if self.board.draft.as_ref().is_some_and(|draft| !self.accepts(draft.source, session)) {
            self.cancel_gesture();
        }
        if self.board.drawing.source().is_some_and(|source| !self.accepts(source, session)) {
            self.cancel_gesture();
        }
        if self.schematic.tool.as_ref().is_some_and(|tool| tool.revision() != session.revision()) {
            self.cancel_gesture();
        }
        if self.schematic.selected_leg.as_ref().is_some_and(|leg| leg.revision != session.revision()) {
            self.schematic.selected_leg = None;
            self.schematic.selection = Selection::default();
        }
        if !matches!(self.schematic.tool, Some(Tool::Wire { .. })) {
            self.schematic.wire_draft = Draft::default();
        }
        if let Some(project) = session.project() {
            if let Some(sheet) = &self.sheet_snapshot {
                self.schematic.selection.retain(sheet);
                self.schematic.selected_sheet_symbol =
                    self.schematic.selected_sheet_symbol.filter(|id| sheet.sheet_symbols().iter().any(|s| s.id == *id));
            } else {
                self.schematic.selection = Selection::default();
                self.schematic.selected_sheet_symbol = None;
            }
            let placed: std::collections::BTreeSet<_> = self
                .board
                .target
                .and_then(|target| project.board_view(target))
                .into_iter()
                .flat_map(|board| board.footprints())
                .map(|footprint| footprint.id)
                .collect();
            self.board.selection.retain(|id| placed.contains(id));
            let setup = self.board.target.and_then(|target| project.board_view(target)).and_then(|board| board.setup());
            let layers: std::collections::BTreeSet<_> = self
                .board
                .target
                .and_then(|target| project.board_view(target))
                .into_iter()
                .flat_map(|board| board.layers())
                .map(|layer| layer.id.to_string())
                .collect();
            if !self.board.layer_defaults_applied
                && let Some(setup) = setup
            {
                use pcbcraft_engine::board::setup::LayerPurpose;
                self.board.hidden_layers.extend(
                    setup
                        .layers()
                        .iter()
                        .filter(|layer| matches!(layer.purpose, LayerPurpose::Fabrication | LayerPurpose::Courtyard))
                        .map(|layer| layer.id.to_string()),
                );
                self.board.layer_defaults_applied = true;
            }
            self.board.hidden_layers.retain(|id| layers.contains(id));
            if self.board.active_layer.as_ref().is_none_or(|id| !layers.contains(id)) {
                self.board.active_layer = setup.and_then(|setup| {
                    setup.stackup().strata.iter().find_map(|stratum| {
                        if let pcbcraft_engine::board::setup::Stratum::Copper { layer, .. } = stratum { Some(layer.to_string()) } else { None }
                    })
                });
            }
        }
        changed
    }
}
