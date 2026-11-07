//! Independently addressed editor views over one session, placed in per-window layouts.

use std::collections::BTreeMap;

use egui::{Rect, Ui};
use pcbcraft_chrome::{editor, grid, shell, workspace};
use pcbcraft_engine::{Session, selection::Selection};

use crate::{
    Dialog, EditorView, PcbApp, Tool, ViewId, ViewInput,
    editor_layout::{HostId, Layout, ViewHost},
    editor_view::{DocumentTarget, Workspace},
    preferences::SelectionFollow,
    schematic_canvas,
};

/// The active field is a temporary render/edit slot, not a second project or global focus.
/// Binding exchanges a registry entry with that slot; each host retains its own focus/layout.
#[derive(Debug)]
pub(super) struct EditorViews {
    pub(super) active: EditorView,
    rest: BTreeMap<ViewId, EditorView>,
    pub(super) hosts: BTreeMap<HostId, ViewHost>,
    pub(super) host: HostId,
    pub(super) main: HostId,
    next_view: u64,
    next_host: u64,
    primary: ViewId,
    selection_source: Option<(ViewId, Workspace)>,
    pending_reveal: BTreeMap<ViewId, PendingReveal>,
}

/// A reveal addresses one live recipient and expires on input/revision or policy changes.
#[derive(Debug)]
struct PendingReveal {
    source: ViewInput,
    target: ViewInput,
    policy: SelectionFollow,
}

impl Default for EditorViews {
    fn default() -> Self {
        let mut root = ViewHost::new(ViewId(0));
        root.parked.push(ViewId(1));
        Self {
            active: EditorView::new(ViewId(0)),
            rest: BTreeMap::from([(ViewId(1), EditorView::new(ViewId(1)))]),
            hosts: BTreeMap::from([(HostId::ROOT, root)]),
            host: HostId::ROOT,
            main: HostId::ROOT,
            next_view: 2,
            next_host: 1,
            primary: ViewId(0),
            selection_source: None,
            pending_reveal: BTreeMap::new(),
        }
    }
}

impl EditorViews {
    /// Snapshot ordering always puts the host owning project controls first.
    pub(super) fn host_order(&self) -> Vec<HostId> {
        std::iter::once(self.main).chain(self.hosts.keys().copied().filter(|id| *id != self.main)).collect()
    }

    /// Capture visible and parked views without selection, input tokens or electrical data.
    pub(super) fn saved_state(&self, session: &Session) -> Option<crate::arrangements::Snapshot> {
        use crate::arrangements::{SavedHost, Snapshot};
        let mut ids = Vec::new();
        for host in self.host_order() {
            let host = self.hosts.get(&host)?;
            ids.extend(host.layout.views());
            ids.extend(&host.parked);
        }
        let indices: BTreeMap<_, _> = ids.iter().copied().enumerate().map(|(index, id)| (id, index)).collect();
        let views = ids.iter().map(|id| self.view(*id)?.saved_state(session)).collect::<Option<Vec<_>>>()?;
        let hosts = self
            .host_order()
            .into_iter()
            .map(|id| {
                let host = self.hosts.get(&id)?;
                Some(SavedHost {
                    layout: host.layout.saved_state(&indices)?,
                    focused: *indices.get(&host.focused)?,
                    parked: host.parked.iter().map(|id| indices.get(id).copied()).collect::<Option<Vec<_>>>()?,
                    collapsed: match &host.collapsed {
                        Some(layout) => Some(layout.saved_state(&indices)?),
                        None => None,
                    },
                    panels: Default::default(),
                    bounds: None,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Snapshot { views, hosts, primary: indices.get(&self.primary).copied().unwrap_or(0) })
    }

    /// Atomically replace presentation after validating the entire snapshot. IDs are never reused,
    /// so delayed input cannot acquire authority when an arrangement is revisited.
    pub(super) fn restore_state(&mut self, state: &crate::arrangements::Snapshot, session: &Session) -> Option<()> {
        if !state.valid() {
            return None;
        }
        let next_view = self.next_view.checked_add(state.views.len() as u64)?;
        let next_host = self.next_host.checked_add(state.hosts.len().saturating_sub(1) as u64)?;
        let ids: Vec<_> = (self.next_view..next_view).map(ViewId).collect();
        let mut views: BTreeMap<_, _> =
            ids.iter().copied().zip(&state.views).map(|(id, state)| (id, EditorView::from_state(id, state, session))).collect();
        let host_ids: Vec<_> = std::iter::once(HostId::ROOT).chain((self.next_host..next_host).map(HostId)).collect();
        let mut hosts = BTreeMap::new();
        for (id, saved) in host_ids.into_iter().zip(&state.hosts) {
            hosts.insert(
                id,
                ViewHost {
                    layout: Layout::from_state(&saved.layout, &ids)?,
                    focused: *ids.get(saved.focused)?,
                    parked: saved.parked.iter().map(|index| ids.get(*index).copied()).collect::<Option<Vec<_>>>()?,
                    collapsed: match &saved.collapsed {
                        Some(layout) => Some(Layout::from_state(layout, &ids)?),
                        None => None,
                    },
                    rects: Vec::new(),
                },
            );
        }
        let focused = hosts.get(&HostId::ROOT)?.focused;
        let active = views.remove(&focused)?;
        self.active = active;
        self.rest = views;
        self.hosts = hosts;
        self.host = HostId::ROOT;
        self.main = HostId::ROOT;
        self.primary = *ids.get(state.primary)?;
        self.next_view = next_view;
        self.next_host = next_host;
        self.selection_source = None;
        self.pending_reveal.clear();
        Some(())
    }

    /// The non-active view in the existing two-view test fixtures.
    #[cfg(test)]
    pub(super) fn other_mut(&mut self) -> &mut EditorView {
        self.rest.values_mut().next().unwrap()
    }

    /// Borrow a live or parked view without changing its host or focus.
    pub(super) fn view(&self, id: ViewId) -> Option<&EditorView> {
        if self.active.id() == id { Some(&self.active) } else { self.rest.get(&id) }
    }

    /// Mutably address a view; closed or unknown IDs cannot fall back to another view.
    pub(super) fn view_mut(&mut self, id: ViewId) -> Option<&mut EditorView> {
        if self.active.id() == id { Some(&mut self.active) } else { self.rest.get_mut(&id) }
    }

    /// Visit every view, including hidden panes, for shared-project invalidation and preferences.
    pub(super) fn all_mut(&mut self) -> impl Iterator<Item = &mut EditorView> {
        std::iter::once(&mut self.active).chain(self.rest.values_mut())
    }

    /// Bind only the render slot. Unknown IDs leave it unchanged and cannot authorize input.
    pub(super) fn bind(&mut self, id: ViewId) {
        if let Some(view) = self.rest.remove(&id) {
            let previous = std::mem::replace(&mut self.active, view);
            self.rest.insert(previous.id(), previous);
        }
    }

    /// Resolve placement independently of a view's workspace/document target.
    pub(super) fn owner(&self, id: ViewId) -> Option<HostId> {
        self.hosts.iter().find_map(|(host, layout)| layout.layout.views().contains(&id).then_some(*host))
    }

    /// The view holding the primary role for selection following, wherever it is hosted.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn primary(&self) -> ViewId {
        self.primary
    }

    /// Whether the current render host has a visible split.
    pub(super) fn is_split(&self) -> bool {
        self.hosts.get(&self.host).is_some_and(ViewHost::split)
    }

    /// Expose stable native hosts; opening another does not move any existing view.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn detached_hosts(&self) -> Vec<HostId> {
        self.hosts.keys().copied().filter(|id| *id != HostId::ROOT).collect()
    }

    /// Allocate a distinct input identity and copy only the source's target and navigation.
    fn duplicate(&mut self, source: ViewId) -> Option<ViewId> {
        let id = ViewId(self.next_view);
        let next = self.next_view.checked_add(1)?;
        let view = self.view_mut(source)?.fork(id);
        self.next_view = next;
        self.rest.insert(id, view);
        Some(id)
    }

    /// Create another view beside the addressed view, preserving the rest of its host layout.
    pub(super) fn open_beside(&mut self, source: ViewId) -> Option<ViewId> {
        let host = self.owner(source)?;
        let id = self.duplicate(source)?;
        if let Some(layout) = self.hosts.get_mut(&host) {
            layout.layout.insert(source, id);
            if let Some(collapsed) = &mut layout.collapsed {
                collapsed.insert(source, id);
            }
            layout.rects.clear();
        }
        self.suspend();
        Some(id)
    }

    /// Open another view in its own native host without moving or focusing existing views.
    pub(super) fn open_window(&mut self, source: ViewId) -> Option<HostId> {
        self.owner(source)?;
        let host = HostId(self.next_host);
        let next = self.next_host.checked_add(1)?;
        let id = self.duplicate(source)?;
        self.next_host = next;
        self.hosts.insert(host, ViewHost::new(id));
        self.suspend();
        Some(host)
    }

    /// Remove a placement and collapse only its branch. The caller retains or removes the view.
    fn unplace(&mut self, host: HostId, id: ViewId) {
        if let Some(mut layout) = self.hosts.remove(&host) {
            layout.collapsed = layout.collapsed.take().and_then(|tree| tree.remove(id));
            if let Some(remaining) = layout.layout.remove(id) {
                layout.layout = remaining;
                if layout.focused == id
                    && let Some(first) = layout.layout.visible_views().first()
                {
                    layout.focused = *first;
                }
                layout.rects.clear();
                self.hosts.insert(host, layout);
            } else {
                // Parked panes belong to this host and are discarded when its last leaf moves.
                for parked in layout.parked {
                    self.rest.remove(&parked);
                }
            }
        }
    }

    /// Move a pane and its cached documents to a new native host. A sole primary pane leaves
    /// a replacement behind; a sole detached pane already has the requested placement.
    pub(super) fn detach_view(&mut self, id: ViewId) -> Option<HostId> {
        let source = self.owner(id)?;
        let group = self.hosts.get(&source)?.layout.group(id)?.clone();
        let members = group.views();
        let sole = self.hosts.get(&source)?.layout.visible_views().len() == 1;
        if source != self.main && sole {
            return Some(source);
        }
        let host = HostId(self.next_host);
        let next = self.next_host.checked_add(1)?;
        if source == self.main && sole {
            // Leave one view behind without reopening the host's previously collapsed layout.
            let parked = self.hosts.get_mut(&source)?.parked.pop();
            let replacement = if let Some(parked) = parked {
                let camera = self.view_mut(id)?.camera.fork_navigation();
                if self.view(parked)?.workspace == self.view(id)?.workspace {
                    self.view_mut(parked)?.camera = camera;
                }
                parked
            } else {
                self.duplicate(id)?
            };
            if let Some(root) = self.hosts.get_mut(&source) {
                root.layout = Layout::View(replacement);
                root.focused = replacement;
                root.rects.clear();
                for member in &members {
                    root.collapsed = root.collapsed.take().and_then(|layout| layout.remove(*member));
                }
            }
        } else {
            for member in members {
                self.unplace(source, member);
            }
        }
        self.next_host = next;
        self.hosts.insert(host, ViewHost { layout: group, ..ViewHost::new(id) });
        self.suspend();
        self.bind_main();
        Some(host)
    }

    /// Dock a whole native pane, retaining its cached document navigation. Other panes and
    /// windows stay in place; an emptied source host and its parked views are pruned.
    pub(super) fn dock_view(&mut self, id: ViewId) {
        let Some(source) = self.owner(id).filter(|host| *host != self.main) else {
            return;
        };
        let Some(group) = self.hosts.get(&source).and_then(|host| host.layout.group(id)).cloned() else {
            return;
        };
        for member in group.views() {
            self.unplace(source, member);
        }
        if let Some(root) = self.hosts.get_mut(&self.main) {
            root.layout.insert_group(root.focused, &group);
            if let Some(collapsed) = &mut root.collapsed {
                collapsed.insert_group(root.focused, &group);
            }
            root.focused = id;
            root.rects.clear();
        }
        self.suspend();
        self.bind_main();
        self.prune_closed();
    }

    /// Restore the current main host's render slot, including after the original window closes.
    pub(super) fn bind_main(&mut self) {
        self.host = self.main;
        if let Some(root) = self.hosts.get(&self.main) {
            self.bind(root.focused);
        }
    }

    /// Dismiss any host while another survives, promoting a survivor without changing its layout.
    /// Removed IDs are never reused, so stale dialogs and delayed input cannot address a new view.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn close_host(&mut self, host: HostId) {
        if !self.hosts.contains_key(&host) || self.hosts.len() == 1 {
            return;
        }
        if host == self.main
            && let Some(next) = self.hosts.keys().copied().find(|id| *id != host)
        {
            self.main = next;
        }
        self.bind_main();
        if let Some(layout) = self.hosts.remove(&host) {
            for id in layout.layout.views().into_iter().chain(layout.parked) {
                self.rest.remove(&id);
            }
            self.prune_closed();
        }
    }

    /// Close a whole pane, including its cached documents, except the last primary pane.
    /// Other placements and navigation survive; closing never reveals an older document.
    pub(super) fn close_view(&mut self, id: ViewId) {
        let Some(host) = self.owner(id) else {
            return;
        };
        if host == self.main && self.hosts.get(&host).is_some_and(|host| host.layout.visible_views().len() == 1) {
            return;
        }
        let Some(members) = self.hosts.get(&host).and_then(|host| host.layout.group(id)).map(Layout::views) else {
            return;
        };
        for member in &members {
            self.unplace(host, *member);
        }
        self.bind_main();
        for member in members {
            self.rest.remove(&member);
        }
        self.prune_closed();
    }

    /// Remove selection/navigation references to closed views and choose a surviving primary
    /// role if necessary. Focus changes never change this role.
    fn prune_closed(&mut self) {
        let live: std::collections::BTreeSet<_> = std::iter::once(self.active.id()).chain(self.rest.keys().copied()).collect();
        self.pending_reveal.retain(|_, pending| live.contains(&pending.source.view()) && live.contains(&pending.target.view()));
        if self.selection_source.is_some_and(|(id, _)| self.view(id).is_none()) {
            self.selection_source = None;
        }
        if self.view(self.primary).is_none() {
            self.primary = self.active.id();
        }
    }

    /// Set startup grid spacing for all views; invalid spacing leaves every camera unchanged.
    pub(super) fn set_grid_everywhere(&mut self, spacing_nm: i64) {
        for view in self.all_mut() {
            view.camera.set_snap_nm(spacing_nm);
        }
    }

    /// Bind all live and cached views after successful file adoption, preserving host placement
    /// and workspace choices. This explicit project replacement precedes saved-layout restoration;
    /// ordinary synchronization must still retain genuinely missing document placeholders.
    pub(super) fn bind_opened_project(&mut self, session: &Session) {
        for view in self.all_mut() {
            view.bind_opened_project(session);
        }
        self.selection_source = None;
        self.pending_reveal.clear();
    }

    /// Synchronize every visible and parked view with the authoritative project opening.
    pub(super) fn sync(&mut self, session: &Session) {
        let mut changed = false;
        for view in self.all_mut() {
            changed |= view.sync(session);
        }
        if changed {
            self.selection_source = None;
            self.pending_reveal.clear();
        }
    }

    /// Set a schematic selection, then broadcast correspondence without editing recipients.
    fn select(&mut self, selection: Selection, session: &Session, policy: SelectionFollow) {
        if self.active.workspace != Workspace::Schematic {
            return;
        }
        self.active.schematic.selected_sheet_symbol = None;
        self.active.schematic.selection = selection;
        self.selection_changed(session, policy);
    }

    /// Select board placements explicitly; a highlight never calls this handler.
    pub(super) fn select_board(&mut self, selection: std::collections::BTreeSet<String>, session: &Session, policy: SelectionFollow) {
        if self.active.workspace != Workspace::Board {
            return;
        }
        if !selection.is_empty() {
            self.active.board.drawing.cancel();
            self.active.board.drawing.selected = None;
        }
        self.active.board.selection = selection;
        self.selection_changed(session, policy);
    }

    /// Remember the source workspace as well as view, so switching one view can reveal its
    /// counterpart without turning the old selection into edit authority in the new workspace.
    fn selection_changed(&mut self, session: &Session, policy: SelectionFollow) {
        self.selection_source = Some((self.active.id(), self.active.workspace));
        self.pending_reveal.clear();
        let Some(source) = self.active.input(session) else {
            return;
        };
        for id in self.hosts.values().flat_map(|host| host.layout.views()) {
            let allowed = match policy {
                SelectionFollow::Off => false,
                SelectionFollow::Both => true,
                SelectionFollow::SecondaryFollowsPrimary => source.view() == self.primary,
                SelectionFollow::PrimaryFollowsSecondary => id == self.primary,
            };
            if id != source.view()
                && allowed
                && let Some(view) = self.view(id).filter(|view| self.active.shares_project(view, session))
                && let Some(target) = view.input(session)
            {
                self.pending_reveal.insert(id, PendingReveal { source, target, policy });
            }
        }
    }

    /// Resolve a live source and destination in this opening. A source workspace may be parked
    /// within its view after switching, but closed views or replaced projects cannot contribute.
    fn probe_source(&self, id: ViewId, session: &Session) -> Option<(&EditorView, Workspace)> {
        let (source_id, workspace) = self.selection_source?;
        let source = self.view(source_id)?;
        let target = self.view(id)?;
        if (source_id == id && target.workspace == workspace)
            || self.owner(source_id).is_none()
            || self.owner(id).is_none()
            || !source.shares_project(target, session)
        {
            return None;
        }
        Some((source, workspace))
    }

    /// Project through component identity when crossing workspaces; literal wire selections
    /// remain local to schematic views. The result never changes edit selection or focus.
    pub(super) fn highlight_for(&self, id: ViewId, session: &Session) -> Selection {
        let Some((source, workspace)) = self.probe_source(id, session) else {
            return Selection::default();
        };
        let mut selection = match workspace {
            Workspace::Schematic => {
                if source.workspace == Workspace::Schematic && !self.view(id).is_some_and(|target| source.shares_sheet(target, session)) {
                    Selection::default()
                } else {
                    source.schematic.selection.clone()
                }
            }
            Workspace::Board => session
                .project()
                .and_then(|p| p.board_view(source.board.target?))
                .map_or_else(Selection::default, |board| board.selection_for_footprints(&source.board.selection)),
        };
        if let Some(project) = self.view(id).and_then(|view| view.snapshot(session)) {
            selection.retain(&project);
        } else {
            selection = Selection::default();
        }
        selection
    }

    /// Resolve board counterparts using the shared engine adapter, preserving stable placement IDs.
    pub(super) fn board_highlight_for(&self, id: ViewId, session: &Session) -> std::collections::BTreeSet<String> {
        let Some((source, workspace)) = self.probe_source(id, session) else {
            return Default::default();
        };
        match workspace {
            Workspace::Schematic => session
                .project()
                .and_then(|p| p.board_view(self.view(id)?.board.target?))
                .map_or_else(Default::default, |board| board.footprints_for_selection(&source.schematic.selection)),
            Workspace::Board if self.view(id).is_some_and(|target| target.board.target == source.board.target) => source.board.selection.clone(),
            Workspace::Board => Default::default(),
        }
    }

    /// Consume a reveal once, after the target host supplies geometry. Stale inputs, changed
    /// policies and closed hosts discard it, and each target uses its own coordinate system.
    fn reveal_for(&mut self, id: ViewId, session: &Session, policy: SelectionFollow) {
        let Some(pending) = self.pending_reveal.remove(&id) else {
            return;
        };
        if pending.policy != policy
            || self.owner(id).is_none()
            || self.owner(pending.source.view()).is_none()
            || !self.view(pending.source.view()).is_some_and(|view| view.accepts(pending.source, session))
            || !self.view(id).is_some_and(|view| view.accepts(pending.target, session))
        {
            return;
        }
        self.reveal_highlight(id, session);
    }

    /// Explicit Reveal and optional following use the same correspondence, without selection.
    pub(super) fn reveal_highlight(&mut self, id: ViewId, session: &Session) {
        let schematic = self.highlight_for(id, session);
        let board = self.board_highlight_for(id, session);
        if let Some(view) = self.view_mut(id)
            && let Some(project) = view.snapshot(session)
        {
            match view.workspace {
                Workspace::Schematic => view.camera.reveal_selection(&project, &schematic),
                Workspace::Board => {
                    if let Some(target) = view.board.target.and_then(|target| project.board_view(target)) {
                        view.camera.reveal_board(target, &board);
                    }
                }
            }
        }
    }

    /// Change one target without moving its host or rewriting another view's camera.
    pub(super) fn switch_workspace(&mut self, id: ViewId, workspace: Workspace, session: &Session) {
        self.pending_reveal.clear();
        if let Some(view) = self.view_mut(id) {
            view.switch(workspace, session);
        }
    }

    /// Switch the originating pane's document, retaining prior navigation privately. Views in
    /// other panes stay independent; switching adds no tab strip or named arrangement.
    pub(super) fn open_document(&mut self, source: ViewId, target: DocumentTarget, session: &Session) -> Option<ViewId> {
        if !target.available(session) {
            return None;
        }
        let owner = self.owner(source)?;
        let tabs = self.hosts.get(&owner)?.layout.group(source)?.views();
        let existing = tabs.into_iter().find(|id| self.view(*id).is_some_and(|view| view.target() == Some(target)));
        let id = if let Some(id) = existing {
            id
        } else if self.view(source)?.unavailable() {
            self.view_mut(source)?.open_document(target, session);
            source
        } else {
            let id = self.duplicate(source)?;
            self.view_mut(id)?.open_document(target, session);
            let host = self.hosts.get_mut(&owner)?;
            host.layout.insert_tab(source, id);
            if let Some(collapsed) = &mut host.collapsed {
                collapsed.insert_tab(source, id);
            }
            id
        };
        if let Some(host) = self.hosts.get_mut(&owner) {
            host.layout.activate(id);
            if let Some(collapsed) = &mut host.collapsed {
                collapsed.activate(id);
            }
            host.focused = id;
            host.rects.clear();
        }
        if id != source {
            self.suspend();
        }
        self.bind_main();
        Some(id)
    }

    /// Cancel every local preview when a modal or Home takes input ownership.
    pub(super) fn cancel_gestures(&mut self) {
        self.pending_reveal.clear();
        for view in self.all_mut() {
            view.cancel_gesture();
        }
    }

    /// Invalidate captured input across focus, layout and project transitions without editing.
    pub(super) fn suspend(&mut self) {
        self.pending_reveal.clear();
        for view in self.all_mut() {
            view.release_input();
        }
    }

    /// Change only the current host's focused pane. Other hosts keep their focus and panels.
    fn activate(&mut self, id: ViewId) -> bool {
        if id == self.active.id() || self.owner(id) != Some(self.host) {
            return false;
        }
        self.suspend();
        if let Some(host) = self.hosts.get_mut(&self.host) {
            host.focused = id;
            host.layout.activate(id);
            if let Some(collapsed) = &mut host.collapsed {
                collapsed.activate(id);
            }
        }
        self.bind(id);
        true
    }

    /// Toggle a split within this host only. Collapsing parks its other views; reopening seeds
    /// the restored panes from the active camera, retaining their separate edit selections.
    fn set_split(&mut self, split: bool) {
        if split == self.is_split() {
            return;
        }
        let source = self.active.id();
        if split {
            let collapsed = self.hosts.get_mut(&self.host).and_then(|host| host.collapsed.take());
            if let Some(layout) = collapsed.filter(|layout| matches!(layout, Layout::Split { .. })) {
                for id in layout.views().into_iter().filter(|id| *id != source) {
                    let camera = self.active.camera.fork_navigation();
                    let workspace = self.active.workspace;
                    if let Some(view) = self.view_mut(id).filter(|view| view.workspace == workspace) {
                        view.camera = camera;
                    }
                }
                if let Some(host) = self.hosts.get_mut(&self.host) {
                    host.layout = layout;
                    host.parked.clear();
                }
            } else {
                let parked = self.hosts.get_mut(&self.host).and_then(|host| host.parked.pop());
                if let Some(id) = parked {
                    let camera = self.active.camera.fork_navigation();
                    let workspace = self.active.workspace;
                    if let Some(view) = self.view_mut(id).filter(|view| view.workspace == workspace) {
                        view.camera = camera;
                    }
                    if let Some(host) = self.hosts.get_mut(&self.host) {
                        host.layout.insert(source, id);
                    }
                } else {
                    self.open_beside(source);
                }
            }
        } else if let Some(host) = self.hosts.get_mut(&self.host) {
            let group = host.layout.group(source).cloned().unwrap_or(Layout::View(source));
            let retained = group.views();
            let layout = std::mem::replace(&mut host.layout, group);
            let remembered = host.collapsed.take().unwrap_or(layout);
            host.parked = remembered.views().into_iter().filter(|id| !retained.contains(id)).collect();
            host.collapsed = Some(remembered);
        }
        if let Some(host) = self.hosts.get_mut(&self.host) {
            host.focused = source;
            host.rects.clear();
        }
        self.suspend();
    }
}

impl PcbApp {
    /// Route explicit selection through the coordinator with the current follow policy.
    pub(super) fn select(&mut self, selection: Selection) {
        self.views.active.schematic.selected_leg = None;
        self.views.active.schematic.whole_wire_drag = false;
        self.views.select(selection, &self.session, self.preferences.selection_follow);
    }

    /// Select one revision-bound segment while projecting its wire identity to other views.
    pub(super) fn select_leg(&mut self, leg: schematic_canvas::LegTarget) {
        self.select(Selection::one_wire(leg.wire.clone()));
        self.views.active.schematic.selected_leg = Some(leg);
    }

    /// Change one view's snap spacing and remember it for the next start. This is presentation
    /// only: the project, its revision, its unsaved state and undo history stay as they are,
    /// nothing moves, and other views keep their own spacing. An in-progress gesture is left
    /// alone; its next snapped point follows the new spacing. Out-of-range values are ignored.
    pub(super) fn set_view_grid(&mut self, id: ViewId, spacing_nm: i64) {
        if let Some(view) = self.views.view_mut(id)
            && view.camera.set_snap_nm(spacing_nm)
        {
            self.preferences.schematic_grid_nm = spacing_nm;
        }
    }

    /// Step the active view to the next finer (`finer`) or coarser grid preset, wrapping.
    pub(super) fn cycle_grid(&mut self, finer: bool) {
        let current = self.views.active.camera.snap_nm();
        if let Some(next) = grid::cycle(current, &grid::SCHEMATIC_SPACING_PRESETS_NM, finer) {
            self.set_view_grid(self.views.active.id(), next);
        }
    }

    /// Apply the presentation action from the View ribbon. A modal owns input until it closes.
    pub(super) fn split_editors(&mut self, ctx: &egui::Context, split: bool) {
        if self.session.project().is_none() || !matches!(self.dialog, Dialog::None) {
            return;
        }
        self.views.set_split(split);
        egui::DragAndDrop::clear_payload(ctx);
        ctx.request_repaint();
    }

    /// Route focus without moving cameras or editing the document. Release text search and
    /// browser drag ownership when a pane takes over; dialog input remains with its owner.
    pub(super) fn activate_editor(&mut self, ctx: &egui::Context, id: ViewId) {
        if !matches!(self.dialog, Dialog::None) || !self.views.activate(id) {
            return;
        }
        self.parts.release_focus(ctx);
        egui::DragAndDrop::clear_payload(ctx);
        ctx.request_repaint();
    }

    /// Resolve a new pointer press before consuming keyboard input. A held drag crossing the
    /// divider keeps its original owner; popups and modals do not transfer editor focus.
    pub(super) fn editor_pointer_focus(&mut self, ctx: &egui::Context) {
        if !self.views.is_split() || self.home || !matches!(self.dialog, Dialog::None) || egui::Popup::is_any_open(ctx) {
            return;
        }
        let pressed = ctx.input(|input| input.pointer.any_pressed().then(|| input.pointer.interact_pos()).flatten());
        if let Some(pointer) = pressed
            && !ctx.layer_id_at(pointer).is_some_and(|layer| layer.order != egui::Order::Background)
            && let Some((id, _)) = self.views.hosts.get(&self.views.host).and_then(|host| host.rects.iter().find(|(_, rect)| rect.contains(pointer)))
        {
            self.activate_editor(ctx, *id);
        }
    }

    /// Render split panes with one document header each, or a single canvas with an
    /// overlaid breadcrumb and no tab clearance. Workspace panels address the focused view;
    /// their geometry changes with the layout, never with focus alone.
    pub(super) fn editor_workspace(&mut self, ui: &mut Ui, workspace: Rect) {
        if self.session.project().is_none() {
            self.canvas(ui);
            return;
        }
        let board = self.views.active.workspace == Workspace::Board;
        if !board {
            #[cfg(not(target_arch = "wasm32"))]
            let clearance = if self.views.host != self.views.main { 0.0 } else { self.assistant.rail_clearance() };
            #[cfg(target_arch = "wasm32")]
            let clearance = 0.0;
            self.net_inspector.show(ui, self.views.active.snapshot(&self.session), clearance);
        }
        let area = ui.available_rect_before_wrap();
        let panes = if let Some(host) = self.views.hosts.get_mut(&self.views.host) {
            let panes = host.layout.panes(ui);
            host.rects = panes.clone();
            panes
        } else {
            Vec::new()
        };
        let headers = panes.len() > 1;
        let area = editor::panel_area(area, &panes.iter().map(|(_, rect)| *rect).collect::<Vec<_>>(), headers, ui.spacing().item_spacing.y);
        let board_panel = board.then(|| self.board_panels(ui, area));
        let mut pick = if board { None } else { self.editor_panels(ui, area) };
        #[cfg(not(target_arch = "wasm32"))]
        let assistant = if self.views.host != self.views.main { Rect::NOTHING } else { self.assistant.covered_rect(workspace) };
        #[cfg(target_arch = "wasm32")]
        let assistant = {
            let _ = workspace;
            Rect::NOTHING
        };
        let mut covered = board_panel.map_or_else(|| vec![self.browser.rect(), self.parts.rect()], |panels| panels.to_vec());
        covered.push(assistant);
        if !board {
            covered.push(self.selection_properties(ui, area));
        }
        for (id, rect) in panes {
            let tab_ids = self.views.hosts.get(&self.views.host).and_then(|host| host.layout.group(id)).map(Layout::views).unwrap_or_default();
            let pane_id = tab_ids.first().copied().unwrap_or(id);
            let path = self.views.view(id).and_then(|view| view.sheet_path(&self.session)).unwrap_or_default();
            ui.scope_builder(egui::UiBuilder::new().id_salt(("editor_pane", pane_id)).max_rect(rect), |ui| {
                ui.set_clip_rect(rect);
                let active = self.views.active.id() == id;
                if headers {
                    let (clicked, action) = editor::pane_header(ui, &path, active, !crate::IN_BROWSER, self.views.host != self.views.main);
                    if clicked {
                        self.activate_editor(ui.ctx(), id);
                    }
                    if let Some(action) = action {
                        use crate::editor_windows::ViewAction;
                        use editor::PaneAction;
                        let action = match action {
                            PaneAction::OpenBeside => ViewAction::OpenBeside,
                            PaneAction::OpenWindow => ViewAction::OpenWindow,
                            PaneAction::Detach => ViewAction::Detach,
                            PaneAction::Dock => ViewAction::Dock,
                            PaneAction::Close => ViewAction::Close,
                        };
                        self.queue_view_action(ui.ctx(), id, action);
                    }
                }
                if self.views.view(id).is_some_and(EditorView::unavailable) {
                    shell::canvas(ui, |ui| {
                        ui.centered_and_justified(|ui| {
                            ui.label("This document is unavailable. Choose a workspace to replace this view.");
                        });
                    });
                    return;
                }
                let canvas = ui.available_rect_before_wrap();
                let uncovered = workspace::catalog::uncovered(canvas, &covered);
                if self.views.view(id).is_some_and(|view| view.workspace == Workspace::Board) {
                    if let Some(view) = self.views.view_mut(id)
                        && view.camera.set_uncovered(canvas, uncovered)
                    {
                        ui.ctx().request_repaint();
                    }
                    if matches!(self.dialog, Dialog::None) {
                        self.views.reveal_for(id, &self.session, self.preferences.selection_follow);
                    }
                    self.board_pane(ui, id);
                } else if self.views.active.id() == id {
                    if self.views.active.camera.set_uncovered(canvas, uncovered) {
                        ui.ctx().request_repaint();
                    }
                    if matches!(self.dialog, Dialog::None) {
                        self.views.reveal_for(id, &self.session, self.preferences.selection_follow);
                    }
                    if let Some((source, pick)) = pick.take()
                        && self.views.active.accepts(source, &self.session)
                    {
                        self.queue_view_action(ui.ctx(), source.view(), crate::editor_windows::ViewAction::Browse(pick));
                    }
                    self.canvas(ui);
                } else {
                    if let Some(view) = self.views.view_mut(id)
                        && view.camera.set_uncovered(canvas, uncovered)
                    {
                        ui.ctx().request_repaint();
                    }
                    // A context-menu action in the earlier pane can open a modal this frame.
                    if matches!(self.dialog, Dialog::None) {
                        self.views.reveal_for(id, &self.session, self.preferences.selection_follow);
                    }
                    let highlight = self.views.highlight_for(id, &self.session);
                    let label = format!("Schematic canvas {}", id.0 + 1);
                    let Some(view) = self.views.view_mut(id) else {
                        return;
                    };
                    ui.push_id(view.widget_id(), |ui| {
                        shell::canvas(ui, |ui| {
                            if let Some(project) = view.snapshot(&self.session) {
                                // The readout follows the pointer in either pane, focused or not.
                                if let Some(point) = super::pointer_on_canvas(ui, &view.camera) {
                                    self.pointer_next = Some(point);
                                }
                                schematic_canvas::show(
                                    ui,
                                    &project,
                                    self.session.revision(),
                                    &mut view.camera,
                                    None,
                                    &view.schematic.selection,
                                    view.schematic.selected_leg.as_ref(),
                                    view.schematic.whole_wire_drag,
                                    view.schematic.selected_sheet_symbol,
                                    &highlight,
                                    false,
                                    self.preferences.navigation,
                                    self.preferences.grid_opacity_percent,
                                    self.preferences.auto_heal_wires,
                                    &view.schematic.wire_draft,
                                    &label,
                                );
                            }
                        });
                    });
                }
                if !headers && let Some(path) = self.views.active.sheet_path(&self.session) {
                    let count = if self.views.active.workspace == Workspace::Board {
                        format!(
                            "{} footprints",
                            self.session
                                .project()
                                .and_then(|project| project.board_view(self.views.active.board.target?))
                                .map_or(0, |board| board.footprints().count())
                        )
                    } else {
                        self.session.project().map_or_else(String::new, |project| {
                            let index = project
                                .sheets()
                                .position(|sheet| Some(DocumentTarget::Sheet(sheet.target)) == self.views.active.target())
                                .unwrap_or(0);
                            format!("{} / {}", index + 1, project.sheets().len())
                        })
                    };
                    if self.views.active.workspace == Workspace::Board {
                        workspace::canvas_heading(
                            ui.painter(),
                            canvas,
                            covered[0],
                            covered[2],
                            &path,
                            &count,
                            [pcbcraft_chrome::Tokens::get().board_ink; 2],
                        );
                    } else {
                        workspace::sheet_heading(ui.painter(), canvas, covered[0], &path, &count);
                    }
                }
            });
        }
        if !self.views.pending_reveal.is_empty() {
            // A selection in the later-painted pane must wake the earlier target host.
            self.repaint_editors(ui.ctx());
        }
    }

    /// Draw shared floating panels inside the host's header-free area. Return a Browser pick
    /// with its originating view address so centering can use that pane's actual uncovered canvas.
    fn editor_panels(&mut self, ui: &mut Ui, area: Rect) -> Option<(crate::ViewInput, crate::project_browser::Action)> {
        let source = self.views.active.input(&self.session)?;
        let browser_area = self.parts.browser_area(area).intersect(area);
        let browser_area =
            if self.properties_open && !self.parts.open { pcbcraft_chrome::properties::browser_area(browser_area) } else { browser_area };
        let pick = self.browser.show(
            ui,
            browser_area,
            &self.session,
            self.views.active.target()?,
            &self.views.active.schematic.selection,
            &self.views.active.board.selection,
        );
        let placing = match &self.views.active.schematic.tool {
            Some(Tool::PlaceSymbol { definition, .. }) => Some(definition.clone()),
            _ => None,
        };
        if let Some(definition) = self.parts.show(ui, area, self.browser.rect(), placing.as_ref(), source, self.session.libraries()) {
            self.begin_placement(definition);
        }
        pick.map(|pick| (source, pick))
    }
}
