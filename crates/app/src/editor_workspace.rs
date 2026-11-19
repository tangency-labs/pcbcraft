//! Independently addressed editor views over one session, placed in per-window layouts.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    ops::Deref,
    sync::Arc,
};

use egui::{Rect, Ui};
use pcbcraft_chrome::{editor, grid, shell, workspace};
use pcbcraft_engine::{
    Session, SheetTarget,
    board::view::{BoardTarget, BoardView},
    schematic::Point,
    selection::Selection,
};

use crate::{
    Dialog, EditorView, PcbApp, ViewId, ViewInput,
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
    report_probe: Option<ReportProbe>,
    pending_reveal: BTreeMap<ViewId, PendingReveal>,
}

/// A report projects stable addresses for one board and one live source revision. Neither
/// side is a local edit selection, and a report click never broadcasts a selection event.
#[derive(Debug)]
struct ReportProbe {
    source: ViewInput,
    board: BoardTarget,
    schematic: HashMap<SheetTarget, (Point, Arc<Selection>)>,
    first_sheet: Option<SheetTarget>,
    footprints: Arc<BTreeSet<String>>,
    board_anchor: Option<Point>,
    #[cfg(test)]
    symbols_visited: usize,
}

/// Release the view registry borrow before mutably borrowing the destination camera. Reports
/// share their already projected sets; ordinary selection following keeps its existing path.
pub(super) enum PaintHighlight<T> {
    Shared(Arc<T>),
    Owned(T),
}

impl<T> Deref for PaintHighlight<T> {
    type Target = T;

    fn deref(&self) -> &T {
        match self {
            Self::Shared(value) => value,
            Self::Owned(value) => value,
        }
    }
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
            report_probe: None,
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
        self.report_probe = None;
        self.pending_reveal.clear();
        Some(())
    }

    /// Reveals still waiting for their destination's next paint.
    #[cfg(test)]
    pub(super) fn pending_reveals(&self) -> usize {
        self.pending_reveal.len()
    }

    /// Every cached document in the pane holding a view, and the one that pane displays.
    #[cfg(test)]
    pub(super) fn pane(&self, id: ViewId) -> (Vec<ViewId>, ViewId) {
        let pane = self.hosts[&self.owner(id).unwrap()].layout.group(id).unwrap();
        (pane.views(), pane.visible_views()[0])
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

    /// Inline drafts own text input until accepted or cancelled, including in a native host.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn editing_text(&self) -> bool {
        self.active.schematic.text_edit.is_some() || self.rest.values().any(|view| view.schematic.text_edit.is_some())
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
    /// `replacement` retains the source's pending navigation while seeding a host left behind;
    /// ordinary copies supersede pending navigation so split views start at the displayed zoom.
    fn duplicate(&mut self, source: ViewId, replacement: bool) -> Option<ViewId> {
        let id = ViewId(self.next_view);
        let next = self.next_view.checked_add(1)?;
        let source = self.view_mut(source)?;
        let view = if replacement { source.fork_replacement(id) } else { source.fork(id) };
        self.next_view = next;
        self.rest.insert(id, view);
        Some(id)
    }

    /// Create another view beside the addressed view, preserving the rest of its host layout.
    pub(super) fn open_beside(&mut self, source: ViewId) -> Option<ViewId> {
        self.open_at(source, None)
    }

    /// Divide only the addressed pane, copying its current view into the requested half.
    /// Other panes and cached documents remain in place; no project data is duplicated.
    /// Without an explicit side, retain the existing adaptive beside placement.
    pub(super) fn open_at(&mut self, source: ViewId, side: Option<pcbcraft_chrome::editor::DockSide>) -> Option<ViewId> {
        self.open_into(source, source, side)
    }

    /// Open a copy of the source's displayed document inside the destination pane.
    /// Validate both addresses before copying; retain the original source placement and cameras.
    pub(super) fn open_into(&mut self, source: ViewId, target: ViewId, side: Option<pcbcraft_chrome::editor::DockSide>) -> Option<ViewId> {
        self.owner(source)?;
        let host = self.owner(target)?;
        let id = self.duplicate(source, false)?;
        if let Some(layout) = self.hosts.get_mut(&host) {
            layout.layout.insert_group_at(target, &Layout::View(id), side);
            if let Some(collapsed) = &mut layout.collapsed {
                collapsed.insert_group_at(target, &Layout::View(id), side);
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
        let id = self.duplicate(source, false)?;
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
    /// a replacement behind without consuming the moved view's pending navigation; a sole
    /// detached pane already has the requested placement. The pane's visible document retains
    /// focus even when a cached document ID addresses the move.
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
                let camera = self.view(id)?.camera.clone().fork_navigation();
                if self.view(parked)?.workspace == self.view(id)?.workspace {
                    self.view_mut(parked)?.camera = camera;
                }
                parked
            } else {
                self.duplicate(id, true)?
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
        let focused = group.visible_views().first().copied().unwrap_or(id);
        self.hosts.insert(host, ViewHost { layout: group, ..ViewHost::new(focused) });
        self.suspend();
        self.bind_main();
        Some(host)
    }

    /// Dock a whole native pane, retaining its cached document navigation. Other panes and
    /// windows stay in place; an emptied source host and its parked views are pruned.
    pub(super) fn dock_view(&mut self, id: ViewId) {
        if self.owner(id) == Some(self.main) {
            return;
        }
        if let Some(target) = self.hosts.get(&self.main).map(|host| host.focused) {
            self.move_beside(id, target);
        }
    }

    /// Move a pane and all cached documents beside a pane in another host. Validate both
    /// placements before removing anything; the sole main pane leaves a replacement behind.
    /// Runtime IDs, cameras (including deferred navigation), document state and the remaining
    /// split trees are retained. The replacement inherits no pending navigation from the moved view.
    /// Keep that replacement in the remembered split too, so reopening preserves focus and saving.
    pub(super) fn move_beside(&mut self, id: ViewId, target: ViewId) -> bool {
        self.move_to(id, target, None)
    }

    /// Move a whole pane to the requested edge, within a host or across native hosts.
    /// Validate distinct groups before removal; adaptive moves retain the cross-host policy.
    pub(super) fn move_to(&mut self, id: ViewId, target: ViewId, side: Option<pcbcraft_chrome::editor::DockSide>) -> bool {
        self.move_group(id, target, crate::editor_layout::DockPlacement::Pane(side))
    }

    /// Move an existing pane to an outer edge of the destination window's entire layout.
    pub(super) fn move_to_window(&mut self, id: ViewId, target: ViewId, side: pcbcraft_chrome::editor::DockSide) -> bool {
        self.move_group(id, target, crate::editor_layout::DockPlacement::Window(side))
    }

    /// Validate placement before unplacing the source, sharing focus, cached groups and host cleanup.
    fn move_group(&mut self, id: ViewId, target: ViewId, placement: crate::editor_layout::DockPlacement) -> bool {
        use crate::editor_layout::DockPlacement;
        let (Some(source), Some(destination)) = (self.owner(id), self.owner(target)) else { return false };
        if source == destination && matches!(placement, DockPlacement::Pane(None)) {
            return false;
        }
        let Some(group) = self.hosts.get(&source).and_then(|host| host.layout.group(id)).cloned() else { return false };
        if matches!(placement, DockPlacement::Pane(_)) && group.views().contains(&target) {
            return false;
        }
        if matches!(placement, DockPlacement::Window(_)) && source == destination && self.hosts[&source].layout.visible_views().len() == 1 {
            return false;
        }
        if source == self.main && self.hosts.get(&source).is_some_and(|host| host.layout.visible_views().len() == 1) {
            let Some(replacement) = self.duplicate(id, true) else { return false };
            if let Some(host) = self.hosts.get_mut(&source) {
                host.layout.insert(id, replacement);
                if let Some(collapsed) = &mut host.collapsed {
                    collapsed.insert(id, replacement);
                }
            }
        }
        let focused = group.visible_views().first().copied().unwrap_or(id);
        let members = group.views();
        for member in members {
            self.unplace(source, member);
        }
        if let Some(host) = self.hosts.get_mut(&destination) {
            placement.insert(&mut host.layout, target, &group);
            if let Some(collapsed) = &mut host.collapsed {
                placement.insert(collapsed, target, &group);
            }
            host.focused = focused;
            host.rects.clear();
        }
        self.suspend();
        self.bind_main();
        self.prune_closed();
        true
    }

    /// Reposition a pane within its current host without creating a window or a view. Cached
    /// documents travel together; layout ratios and independent presentation state stay intact.
    /// Reject unknown IDs, different hosts and two addresses in the same pane before changing it.
    pub(super) fn reorder_view(&mut self, id: ViewId, target: ViewId) -> bool {
        let Some(owner) = self.owner(id).filter(|host| self.owner(target) == Some(*host)) else { return false };
        let Some(host) = self.hosts.get_mut(&owner) else { return false };
        if !host.layout.reorder(id, target) {
            return false;
        }
        if let Some(collapsed) = &mut host.collapsed {
            collapsed.reorder(id, target);
        }
        host.focused = host.layout.group(id).and_then(|group| group.visible_views().first().copied()).unwrap_or(id);
        host.rects.clear();
        self.suspend();
        self.bind_main();
        true
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
        let live: BTreeSet<_> = std::iter::once(self.active.id()).chain(self.rest.keys().copied()).collect();
        if self.report_probe.as_ref().is_some_and(|probe| !live.contains(&probe.source.view())) {
            self.report_probe = None;
        }
        self.pending_reveal.retain(|_, pending| live.contains(&pending.source.view()) && live.contains(&pending.target.view()));
        if self.selection_source.is_some_and(|(id, _)| self.view(id).is_none()) {
            self.selection_source = None;
        }
        if self.view(self.primary).is_none() {
            self.primary = self.active.id();
        }
    }

    /// Seed every live and cached view with the remembered schematic and PCB spacings. Each
    /// view's active and parked workspace cameras take their own value, so switching workspaces
    /// does not make one editor inherit the other's spacing. Invalid values leave a camera's
    /// current spacing unchanged.
    pub(super) fn set_grid_defaults(&mut self, schematic_nm: i64, board_nm: i64) {
        for view in self.all_mut() {
            view.set_grid_defaults(schematic_nm, board_nm);
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
        self.report_probe = None;
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
        if self.report_probe.as_ref().is_some_and(|probe| !self.probe_is_current(probe, session)) {
            self.report_probe = None;
        }
    }

    /// Set a schematic selection, then broadcast correspondence without editing recipients.
    fn select(&mut self, mut selection: Selection, session: &Session, policy: SelectionFollow) {
        self.active.filters.retain(&mut selection, self.active.snapshot(session).as_deref());
        if self.active.workspace != Workspace::Schematic {
            return;
        }
        self.active.inspection.clear();
        self.active.schematic.selected_sheet_symbol = None;
        self.active.schematic.selection = selection;
        self.selection_changed(session, policy);
    }

    /// Select board placements explicitly; a highlight never calls this handler.
    pub(super) fn select_board(&mut self, selection: std::collections::BTreeSet<String>, session: &Session, policy: SelectionFollow) {
        if self.active.workspace != Workspace::Board {
            return;
        }
        self.active.inspection.clear();
        self.active.board.reference = Default::default();
        if !selection.is_empty() {
            self.active.board.drawing.cancel();
            self.active.board.zones.cancel();
            self.active.board.zones.select(None);
            self.active.board.drawing.select(None);
            self.active.board.tracks.cancel();
            self.active.board.tracks.selected.clear();
        }
        self.active.board.selection = if self.active.filters.footprints { selection } else { Default::default() };
        self.selection_changed(session, policy);
    }

    /// Select text independently of editable footprint geometry and project its owning component to other views.
    pub(super) fn select_board_reference(&mut self, footprint: String, session: &Session, policy: SelectionFollow) {
        if self.active.workspace != Workspace::Board || !self.active.filters.footprints {
            return;
        }
        self.active.inspection.clear();
        self.active.board.selection.clear();
        self.active.board.drawing.cancel();
        self.active.board.zones.cancel();
        self.active.board.zones.select(None);
        self.active.board.drawing.select(None);
        self.active.board.tracks.cancel();
        self.active.board.tracks.selected.clear();
        self.active.board.reference.select(footprint);
        self.active.board.properties_collapsed = false;
        self.selection_changed(session, policy);
    }

    /// Remember the source workspace as well as view, so switching one view can reveal its
    /// counterpart without turning the old selection into edit authority in the new workspace.
    fn selection_changed(&mut self, session: &Session, policy: SelectionFollow) {
        self.selection_source = Some((self.active.id(), self.active.workspace));
        self.report_probe = None;
        self.pending_reveal.clear();
        let Some(source) = self.active.input(session) else {
            return;
        };
        let from_primary = self.in_primary_pane(source.view());
        // Hidden cached documents never paint, so a queued reveal there could never be consumed.
        for id in self.hosts.values().flat_map(|host| host.layout.visible_views()) {
            let allowed = match policy {
                SelectionFollow::Off => false,
                SelectionFollow::Both => true,
                SelectionFollow::SecondaryFollowsPrimary => from_primary,
                SelectionFollow::PrimaryFollowsSecondary => self.in_primary_pane(id),
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

    /// The primary role belongs to the pane holding the primary view, whichever cached document
    /// it displays, so directed following continues after the pane changes sheet.
    fn in_primary_pane(&self, id: ViewId) -> bool {
        id == self.primary
            || self
                .owner(self.primary)
                .and_then(|host| self.hosts.get(&host)?.layout.group(self.primary))
                .is_some_and(|pane| pane.views().contains(&id))
    }

    /// Project an Update PCB row's part instances. The schematic side remains useful before
    /// placement, or the PCB side alone when a placed part is pending removal.
    pub(super) fn probe_update_parts(&mut self, source: ViewId, board: BoardTarget, parts: &BTreeSet<u64>, session: &Session) {
        let Some(view) = session.project().and_then(|project| project.board_view(board)) else { return };
        self.show_report_probe(source, board, parts, view.footprints_for_parts(parts), view, session);
    }

    /// Recognize only the DRC report's footprint and pad addresses, not track/via IDs, text or
    /// coordinates. Pads identify their owning footprint here; the finding marker locates the fault.
    fn probe_drc_finding(&mut self, source: ViewId, board: BoardTarget, index: usize, session: &Session) {
        let Some(view) = session.project().and_then(|project| project.board_view(board)) else { return };
        let Some(items) = self.view(source).and_then(|view| view.board.drc.finding_items(session, board, index)) else { return };
        let owners: BTreeSet<_> = items.iter().map(|item| item.split_once(':').map_or(item.as_str(), |(owner, _)| owner)).collect();
        let mut parts = BTreeSet::new();
        let mut footprints = BTreeSet::new();
        for owner in owners {
            if let Some(part) = view.footprint_part(owner) {
                parts.insert(part);
                footprints.insert(owner.to_owned());
            }
        }
        self.show_report_probe(source, board, &parts, footprints, view, session);
    }

    /// Resolve a report once. Work scales with the clicked row's addresses plus at most one
    /// pass over native symbols, not that pass multiplied by the number of open views.
    fn show_report_probe(
        &mut self,
        source: ViewId,
        board: BoardTarget,
        parts: &BTreeSet<u64>,
        footprints: BTreeSet<String>,
        project: BoardView<'_>,
        session: &Session,
    ) {
        let Some(input) = self.view(source).and_then(|view| view.input(session)) else { return };
        if self.owner(source).is_none() {
            return;
        }
        let mut first_sheet = None;
        let mut by_sheet: HashMap<_, (_, Selection)> = HashMap::new();
        #[cfg(test)]
        let mut symbols_visited = 0;
        if !parts.is_empty() {
            for (sheet, part, unit) in project.sheet_symbols() {
                #[cfg(test)]
                {
                    symbols_visited += 1;
                }
                if parts.contains(&part) {
                    first_sheet.get_or_insert(sheet);
                    by_sheet.entry(sheet).or_insert_with(|| (unit.position, Selection::default())).1.symbols.insert(unit.id());
                }
            }
        }
        let schematic = by_sheet.into_iter().map(|(sheet, (at, selection))| (sheet, (at, Arc::new(selection)))).collect();
        let board_anchor = footprints.first().and_then(|id| project.footprint_anchor(id));
        self.selection_source = None;
        self.pending_reveal.clear();
        self.report_probe = Some(ReportProbe {
            source: input,
            board,
            schematic,
            first_sheet,
            footprints: Arc::new(footprints),
            board_anchor,
            #[cfg(test)]
            symbols_visited,
        });
        let targets: Vec<_> = self.hosts.values().flat_map(|host| host.layout.visible_views()).collect();
        // An explicit report click reveals regardless of the automatic follow preference.
        for id in targets {
            if self.report_probe_for(id, session).is_some() {
                self.reveal_report(id, session, source);
            }
        }
    }

    /// Do not paint a captured report after an edit, reopen or change of source document.
    fn probe_is_current(&self, probe: &ReportProbe, session: &Session) -> bool {
        self.owner(probe.source.view()).is_some() && self.view(probe.source.view()).is_some_and(|view| view.accepts(probe.source, session))
    }

    /// A report applies only to a live project pane in its captured opening.
    fn report_probe_for(&self, id: ViewId, session: &Session) -> Option<&ReportProbe> {
        self.report_probe.as_ref().filter(|probe| {
            self.probe_is_current(probe, session) && self.owner(id).is_some() && self.view(id).and_then(|view| view.input(session)).is_some()
        })
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

    /// Resolve an ordinary edit selection and whether it may reveal across sheets.
    fn schematic_probe(&self, id: ViewId, session: &Session) -> Option<(bool, Selection)> {
        let (source, workspace) = self.probe_source(id, session)?;
        let selection = match workspace {
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
                .map_or_else(Selection::default, |board| board.selection_for_footprints(&source.board.probe_footprints())),
        };
        Some((workspace == Workspace::Board, selection))
    }

    /// Filter ordinary counterparts to the displayed sheet; share the prefiltered report set.
    pub(super) fn schematic_paint_highlight(&self, id: ViewId, session: &Session) -> PaintHighlight<Selection> {
        if let Some(probe) = self.report_probe_for(id, session) {
            if let Some(DocumentTarget::Sheet(sheet)) = self.view(id).and_then(EditorView::target)
                && let Some((_, selection)) = probe.schematic.get(&sheet)
            {
                return PaintHighlight::Shared(Arc::clone(selection));
            }
            return PaintHighlight::Owned(Selection::default());
        }
        PaintHighlight::Owned(self.selection_highlight(id, session))
    }

    fn selection_highlight(&self, id: ViewId, session: &Session) -> Selection {
        let Some((_, mut selection)) = self.schematic_probe(id, session) else {
            return Selection::default();
        };
        let Some(project) = self.view(id).and_then(|view| view.snapshot(session)) else {
            return Selection::default();
        };
        selection.retain(&project);
        selection
    }

    /// Share report counterparts for the captured board, retaining stable placement IDs.
    pub(super) fn board_paint_highlight(&self, id: ViewId, session: &Session) -> PaintHighlight<BTreeSet<String>> {
        if let Some(probe) = self.report_probe_for(id, session) {
            return if self.view(id).and_then(EditorView::target) == Some(DocumentTarget::Board(probe.board)) {
                PaintHighlight::Shared(Arc::clone(&probe.footprints))
            } else {
                PaintHighlight::Owned(BTreeSet::new())
            };
        }
        PaintHighlight::Owned(self.board_selection_highlight(id, session))
    }

    #[cfg(test)]
    pub(super) fn highlight_for(&self, id: ViewId, session: &Session) -> Selection {
        self.schematic_paint_highlight(id, session).clone()
    }

    #[cfg(test)]
    pub(super) fn board_highlight_for(&self, id: ViewId, session: &Session) -> BTreeSet<String> {
        self.board_paint_highlight(id, session).clone()
    }

    /// Project ordinary edit selection only, separately from the captured report highlights.
    fn board_selection_highlight(&self, id: ViewId, session: &Session) -> BTreeSet<String> {
        let Some((source, workspace)) = self.probe_source(id, session) else {
            return Default::default();
        };
        match workspace {
            Workspace::Schematic => session
                .project()
                .and_then(|p| p.board_view(self.view(id)?.board.target?))
                .map_or_else(Default::default, |board| board.footprints_for_selection(&source.schematic.selection)),
            Workspace::Board if self.view(id).is_some_and(|target| target.board.target == source.board.target) => source.board.probe_footprints(),
            Workspace::Board => Default::default(),
        }
    }

    /// Consume a reveal once, after the target host supplies geometry. Stale inputs, changed
    /// policies and closed hosts discard it, and each target uses its own coordinate system.
    fn reveal_for(&mut self, id: ViewId, session: &Session, policy: SelectionFollow) -> bool {
        let Some(pending) = self.pending_reveal.remove(&id) else {
            return false;
        };
        if pending.policy != policy
            || self.owner(id).is_none()
            || self.owner(pending.source.view()).is_none()
            || !self.view(pending.source.view()).is_some_and(|view| view.accepts(pending.source, session))
            || !self.view(id).is_some_and(|view| view.accepts(pending.target, session))
        {
            return false;
        }
        self.reveal_highlight(id, session, policy)
    }

    /// Reveal the counterpart without selecting it. PCB selections or report results may change
    /// the sheet a schematic pane displays. Report that change so the host paints the pane again.
    pub(super) fn reveal_highlight(&mut self, id: ViewId, session: &Session, policy: SelectionFollow) -> bool {
        if let Some(probe) = self.report_probe_for(id, session) {
            return self.reveal_report(id, session, probe.source.view());
        }
        match self.view(id).map(|view| view.workspace) {
            Some(Workspace::Schematic) => self.reveal_schematic(id, session, policy),
            Some(Workspace::Board) => {
                let highlight = self.board_selection_highlight(id, session);
                if let Some(view) = self.view_mut(id)
                    && let Some(project) = view.snapshot(session)
                    && let Some(target) = view.board.target.and_then(|target| project.board_view(target))
                {
                    view.camera.reveal_board(target, &highlight);
                }
                false
            }
            None => false,
        }
    }

    /// Keep a report match on the current sheet, or show the first matching sheet only in a peer
    /// pane, reusing its cached view of that sheet. Reveal one stable anchor per view without
    /// resolving graphics or changing zoom; a newly shown view centers once it has geometry.
    fn reveal_report(&mut self, id: ViewId, session: &Session, origin: ViewId) -> bool {
        let Some(probe) = self.report_probe.as_ref() else { return false };
        let (target, at) = match self.view(id).and_then(EditorView::target) {
            Some(DocumentTarget::Sheet(sheet)) => {
                if let Some(&(at, _)) = probe.schematic.get(&sheet) {
                    (None, at)
                } else if id != origin
                    && let Some(first) = probe.first_sheet
                    && let Some(&(at, _)) = probe.schematic.get(&first)
                {
                    (Some(first), at)
                } else {
                    return false;
                }
            }
            Some(DocumentTarget::Board(board)) if board == probe.board => {
                let Some(at) = probe.board_anchor else { return false };
                (None, at)
            }
            _ => return false,
        };
        let shown = match target {
            Some(sheet) => match self.show_sheet_in_pane(id, sheet, session) {
                Some(shown) => shown,
                None => return false,
            },
            None => id,
        };
        let Some(view) = self.view_mut(shown) else { return false };
        view.camera.reveal_report_anchor(at);
        target.is_some()
    }

    /// Keep a sheet that already contains a match; otherwise use the first matching sheet in
    /// project order. Match the resolved unit IDs, never a reference, sheet name or definition.
    /// A sheet change shows that sheet's document in the same pane and reveals there on its next
    /// paint, once the pane has supplied geometry; the matching sheet then settles in place.
    fn reveal_schematic(&mut self, id: ViewId, session: &Session, policy: SelectionFollow) -> bool {
        let Some((may_change_sheet, all)) = self.schematic_probe(id, session) else {
            return false;
        };
        let Some(current) = self.view(id).and_then(|view| view.snapshot(session)) else {
            return false;
        };
        let mut matches = all.clone();
        matches.retain(&current);
        if !matches.is_empty() {
            if let Some(view) = self.view_mut(id) {
                view.camera.reveal_selection(&current, &matches);
            }
            return false;
        }
        if !may_change_sheet || all.symbols.is_empty() {
            return false;
        }
        let Some(sheet) = current.first_sheet_for_symbols(&all) else {
            return false;
        };
        let Some(shown) = self.show_sheet_in_pane(id, sheet, session) else {
            return false;
        };
        if let Some((source, _)) = self.selection_source
            && let Some(source) = self.view(source).and_then(|view| view.input(session))
            && let Some(target) = self.view(shown).and_then(|view| view.input(session))
        {
            self.pending_reveal.insert(shown, PendingReveal { source, target, policy });
        }
        true
    }

    /// Display a sheet in the addressed view's pane without moving focus. Reuse the pane's cached
    /// view of that sheet, or open one from the addressed view's camera as Browser navigation
    /// does, so every cached document keeps its own navigation. The previous view stays cached.
    fn show_sheet_in_pane(&mut self, id: ViewId, sheet: pcbcraft_engine::SheetTarget, session: &Session) -> Option<ViewId> {
        let target = DocumentTarget::Sheet(sheet);
        let owner = self.owner(id)?;
        let cached = self
            .hosts
            .get(&owner)?
            .layout
            .group(id)?
            .views()
            .into_iter()
            .find(|view| self.view(*view).is_some_and(|view| view.target() == Some(target)));
        let shown = match cached {
            Some(shown) if shown == id => return None,
            Some(shown) => shown,
            None => {
                let shown = self.duplicate(id, false)?;
                if self.view_mut(shown)?.reveal_sheet(sheet, session).is_none() {
                    self.rest.remove(&shown);
                    return None;
                }
                let host = self.hosts.get_mut(&owner)?;
                host.layout.insert_tab(id, shown);
                if let Some(collapsed) = &mut host.collapsed {
                    collapsed.insert_tab(id, shown);
                }
                shown
            }
        };
        if let Some(view) = self.view_mut(id) {
            view.release_input();
        }
        let host = self.hosts.get_mut(&owner)?;
        host.layout.activate(shown);
        if let Some(collapsed) = &mut host.collapsed {
            collapsed.activate(shown);
        }
        if host.focused == id {
            host.focused = shown;
        }
        host.rects.clear();
        if self.active.id() == id {
            self.bind(shown);
        }
        Some(shown)
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
            let id = self.duplicate(source, false)?;
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
                    let library = self.active.library.clone();
                    if let Some(view) = self.view_mut(id) {
                        view.library = library;
                        if view.library.is_some() {
                            view.definition = None;
                        }
                    }
                    if let Some(document) = self.active.definition
                        && let Some(view) = self.view_mut(id)
                    {
                        view.definition = Some(document);
                    }
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
    /// Extend the top row's actual pane grids through the ribbon's exposed margins after camera
    /// navigation settles. Each segment uses its own view, palette and grid visibility settings.
    pub(super) fn paint_ribbon_background(&self, ribbon: Option<workspace::RibbonBackground>) {
        let Some(ribbon) = ribbon else { return };
        let t = pcbcraft_chrome::Tokens::get();
        let mut shapes = vec![egui::Shape::rect_filled(ribbon.area, 0, t.canvas)];
        if let Some(host) = self.views.hosts.get(&self.views.host) {
            let top = host.rects.iter().map(|(_, pane)| pane.top()).fold(f32::INFINITY, f32::min);
            for (id, pane) in host.rects.iter().filter(|(_, pane)| (pane.top() - top).abs() < 0.5) {
                let area = Rect::from_min_max(egui::pos2(pane.left(), ribbon.area.top()), egui::pos2(pane.right(), ribbon.area.bottom()))
                    .intersect(ribbon.area);
                // Frame texts stay within this pane's span, so one pane's text never covers its
                // neighbour's margin; at the window's edges the painter clips instead.
                let left = if (pane.left() - ribbon.area.left()).abs() < 0.5 { f32::NEG_INFINITY } else { pane.left() };
                let right = if (pane.right() - ribbon.area.right()).abs() < 0.5 { f32::INFINITY } else { pane.right() };
                let text_area = Rect::from_x_y_ranges(left..=right, f32::NEG_INFINITY..=f32::INFINITY);
                shapes.extend(self.view_background_shapes(ribbon.painter(), *id, area, text_area));
            }
        }
        ribbon.paint(shapes);
    }

    /// Paint exposed chrome margins with the addressed view's canvas palette and settled grid.
    /// A schematic view extends its sheet's paper backdrop. Missing/library views retain a plain
    /// canvas background; no input or model state changes. `painter` lays out frame texts, kept
    /// only within `text_area`.
    fn view_background_shapes(&self, painter: &egui::Painter, id: ViewId, area: Rect, text_area: Rect) -> Vec<egui::Shape> {
        let t = pcbcraft_chrome::Tokens::get();
        let plain = || vec![egui::Shape::rect_filled(area, 0, t.canvas)];
        let Some(view) = self.views.view(id) else { return plain() };
        if view.unavailable() || view.library.is_some() {
            return plain();
        }
        let (camera, fill, ink, opacity, style) = if let Some(definition) = view.definition {
            let Some(camera) = self.library_editors.view_camera(definition, id) else { return plain() };
            let fill = if self.library_kind(definition) == Some(pcbcraft_engine::authoring::Kind::Footprint) { t.board_canvas } else { t.canvas };
            (camera, fill, t.canvas_grid, pcbcraft_chrome::authoring::GRID_OPACITY, grid::Style::Dots)
        } else if view.workspace == Workspace::Board {
            (&view.camera, t.board_canvas, t.board_grid, self.preferences.grid_opacity(true), self.preferences.board_grid_style)
        } else if let Some(canvas) = view.camera.schematic_rect()
            && let Some(project) = view.snapshot(&self.session)
        {
            return crate::schematic_canvas::backdrop(painter, &project, &view.camera, canvas, area, text_area, self.preferences.schematic_display());
        } else {
            (&view.camera, t.canvas, t.canvas_grid, self.preferences.grid_opacity(false), grid::Style::Dots)
        };
        let mut shapes = vec![egui::Shape::rect_filled(area, 0, fill)];
        if let Some(canvas) = camera.schematic_rect()
            && let Some(color) = grid::color(ink, opacity)
        {
            let origin = camera.screen(pcbcraft_engine::schematic::Point { x_nm: 0, y_nm: 0 }, canvas);
            shapes.extend(grid::shapes(area, origin, camera.snap_step(), color, style));
        }
        shapes
    }

    /// Route explicit selection through the coordinator with the current follow policy.
    pub(super) fn select(&mut self, selection: Selection) {
        self.views.active.schematic.selected_leg = None;
        self.views.active.schematic.whole_wire_drag = false;
        self.views.select(selection, &self.session, self.preferences.selection_follow);
    }

    /// Select one revision-bound segment while projecting its wire identity to other views.
    pub(super) fn select_leg(&mut self, leg: schematic_canvas::LegTarget) {
        self.select(Selection::one_wire(leg.wire.clone()));
        if self.views.active.filters.wires {
            self.views.active.schematic.selected_leg = Some(leg);
        }
    }

    /// Change one view's snap spacing and remember it for its workspace. This is presentation
    /// only: the project, its revision, its unsaved state and undo history stay as they are,
    /// nothing moves, and other views keep their own spacing. An in-progress gesture is left
    /// alone; its next snapped point follows the new spacing. Out-of-range values are ignored.
    /// Schematic and PCB views update separate preferences, so changing one never replaces the
    /// other workspace's startup spacing.
    pub(super) fn set_view_grid(&mut self, id: ViewId, spacing_nm: i64) {
        let Some(workspace) = self.views.view(id).map(|view| view.workspace) else {
            return;
        };
        if let Some(view) = self.views.view_mut(id)
            && view.camera.set_snap_nm(spacing_nm)
        {
            match workspace {
                Workspace::Schematic => self.preferences.schematic_grid_nm = spacing_nm,
                Workspace::Board => self.preferences.board_grid_nm = spacing_nm,
            }
        }
    }

    /// Step the active view to the next finer (`finer`) or coarser grid preset, wrapping.
    pub(super) fn cycle_grid(&mut self, finer: bool) {
        let current = self.views.active.camera.snap_nm();
        let presets = self.views.active.workspace.grid_presets();
        if let Some(next) = grid::cycle(current, presets, finer) {
            self.set_view_grid(self.views.active.id(), next);
        }
    }

    /// Apply the presentation action from the View ribbon. A modal owns input until it closes.
    pub(super) fn split_editors(&mut self, ctx: &egui::Context, split: bool) {
        if (self.session.project().is_none() && !self.views.active.auxiliary()) || !matches!(self.dialog, Dialog::None) {
            return;
        }
        self.views.set_split(split);
        self.sync_library_bindings();
        egui::DragAndDrop::clear_payload(ctx);
        ctx.request_repaint();
    }

    /// Route focus without moving cameras or editing the document. Release text search and
    /// browser drag ownership when a pane takes over; dialog input remains with its owner.
    /// Focus preserves the addressed pane's project or definition binding.
    pub(super) fn activate_editor(&mut self, ctx: &egui::Context, id: ViewId) {
        if !matches!(self.dialog, Dialog::None) {
            return;
        }
        let changed = self.views.activate(id);
        if changed {
            self.cancel_library_gestures();
        }
        self.sync_library_focus();
        if !changed {
            return;
        }
        self.parts.release_focus(ctx);
        egui::DragAndDrop::clear_payload(ctx);
        ctx.request_repaint();
    }

    /// Resolve a new pointer press before consuming keyboard input. A held drag crossing the
    /// divider keeps its original owner; popups and modals do not transfer editor focus.
    pub(super) fn editor_pointer_focus(&mut self, ctx: &egui::Context) {
        if self.parts.is_modal() || !self.views.is_split() || self.home || !matches!(self.dialog, Dialog::None) || egui::Popup::is_any_open(ctx) {
            return;
        }
        let pressed = ctx.input(|input| input.pointer.any_pressed().then(|| input.pointer.interact_pos()).flatten());
        if let Some(pointer) = pressed
            && let Some((id, _)) = self.views.hosts.get(&self.views.host).and_then(|host| host.rects.iter().find(|(_, rect)| rect.contains(pointer)))
            && (self.views.view(*id).is_some_and(|view| view.auxiliary())
                || !ctx.layer_id_at(pointer).is_some_and(|layer| layer.order != egui::Order::Background))
        {
            self.activate_editor(ctx, *id);
        }
    }

    /// Render split and detached panes with one document header each. A single main canvas
    /// uses an overlaid breadcrumb without tab clearance, except for the boardless introduction
    /// that owns its context and scroll area. Workspace panels address the focused view;
    /// Mixed layouts confine project panels to the focused project pane; each definition pane
    /// owns its Browser and Properties inside its own rectangle.
    pub(super) fn editor_workspace(&mut self, ui: &mut Ui, workspace: Rect) {
        let has_library = self
            .views
            .hosts
            .get(&self.views.host)
            .is_some_and(|host| host.layout.views().iter().any(|id| self.views.view(*id).is_some_and(|view| view.auxiliary())));
        if self.session.project().is_none() && !has_library {
            self.canvas(ui);
            return;
        }
        let project_focus = !self.views.active.auxiliary();
        let board = project_focus && self.views.active.workspace == Workspace::Board;
        let area = ui.available_rect_before_wrap();
        let panes = if let Some(host) = self.views.hosts.get_mut(&self.views.host) {
            let panes = host.layout.panes(ui);
            host.rects = panes.clone();
            panes
        } else {
            Vec::new()
        };
        let headers = panes.len() > 1 || (!crate::IN_BROWSER && self.views.host != self.views.main);
        self.editor_drag_targets(ui, &panes);
        let panel_rects: Vec<_> = panes.iter().filter(|(id, _)| !has_library || *id == self.views.active.id()).map(|(_, rect)| *rect).collect();
        let area = editor::panel_area(area, &panel_rects, headers, ui.spacing().item_spacing.y);
        let board_panel = board.then(|| self.board_panels(ui, area));
        let mut pick = if board || !project_focus { None } else { self.editor_panels(ui, area) };
        let mut slot_panels = self.views.active.panels.clone();
        if self.net_inspector.open {
            slot_panels[crate::workspace_panels::PROPERTIES] =
                pcbcraft_chrome::workspace::panels::Panel::new(pcbcraft_chrome::workspace::panels::Position::Right);
        }
        let slots = crate::workspace_panels::areas(
            area,
            self.browser.rect(),
            &slot_panels,
            [false, self.properties_open || self.net_inspector.open, self.views.active.filters_open],
            true,
        );
        let properties_area = slots[crate::workspace_panels::PROPERTIES];
        let filters_area = board_panel.as_ref().map_or(slots[crate::workspace_panels::FILTERS], |(_, slot)| *slot);
        #[cfg(not(target_arch = "wasm32"))]
        let assistant = if self.views.host != self.views.main { Rect::NOTHING } else { self.assistant.covered_rect(workspace) };
        #[cfg(target_arch = "wasm32")]
        let assistant = {
            let _ = workspace;
            Rect::NOTHING
        };
        let mut covered = board_panel.map_or_else(
            || if project_focus { vec![self.browser.rect(), self.parts.rect()] } else { vec![Rect::NOTHING; 2] },
            |(panels, _)| panels.to_vec(),
        );
        covered.push(assistant);
        let compact = self.preferences.panel_style == pcbcraft_chrome::workspace::PanelStyle::Compact;
        let mut properties_rect = Rect::NOTHING;
        if project_focus && !board {
            let inspector_area = properties_area;
            self.net_inspector.show(ui, self.views.active.snapshot(&self.session), inspector_area);
            properties_rect = if self.net_inspector.open { self.net_inspector.rect } else { self.selection_properties(ui, area, properties_area) };
            covered.push(properties_rect);
        }
        // Compact columns close up beneath the Properties or Net Inspector card drawn above.
        let filters_area = if board {
            filters_area
        } else {
            use crate::workspace_panels::{FILTERS, PROPERTIES, follow, on_side};
            follow(filters_area, [on_side(&slot_panels, PROPERTIES, slot_panels[FILTERS].position(), properties_rect)], compact)
        };
        // Filters are drawn before DRC so a Compact DRC card can start below a right-docked
        // Selection filters card, which stacks directly under Properties.
        let filters_rect = if project_focus && self.views.active.filters_open {
            let rect = self.selection_filters(ui, area, filters_area);
            covered.push(rect);
            rect
        } else {
            Rect::NOTHING
        };
        if board && let Some((panels, _)) = board_panel {
            use crate::workspace_panels::{FILTERS, follow, on_side};
            use pcbcraft_chrome::workspace::panels::Position;
            // The DRC card floats on the right below Properties, leaving Layers and Properties clear.
            let mut drc_area = area;
            // `board_panels` returns the Browser, Layers and Properties cards in that order.
            drc_area.min.y = drc_area.min.y.max(panels[2].bottom() + if compact { 0. } else { 8. });
            // In Compact it also follows a right-docked Selection filters card instead of overlapping it.
            let drc_area = follow(drc_area, [on_side(&self.views.active.panels, FILTERS, Position::Right, filters_rect)], compact);
            let revision = self.session.revision();
            match self.views.active.board.drc.show(ui, drc_area, revision, self.preferences.units) {
                crate::board_drc::Action::Focus { at, board, index } => {
                    self.views.probe_drc_finding(self.views.active.id(), board, index, &self.session);
                    self.views.active.camera.center_on(at);
                    self.repaint_editors(ui.ctx());
                }
                crate::board_drc::Action::Rerun => self.run_drc(),
                crate::board_drc::Action::None => {}
            }
            covered.push(self.views.active.board.drc.rect);
        }
        for (id, rect) in panes {
            let tab_ids = self.views.hosts.get(&self.views.host).and_then(|host| host.layout.group(id)).map(Layout::views).unwrap_or_default();
            let pane_id = tab_ids.first().copied().unwrap_or(id);
            let path = self.view_title(id);
            let header = headers.then(|| {
                let area = Rect::from_min_max(rect.min, egui::pos2(rect.right(), (rect.top() + editor::header_height()).min(rect.bottom())));
                let painter = ui.painter_at(area);
                let slot = painter.add(egui::Shape::Noop);
                (area, painter, slot)
            });
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
                            PaneAction::OpenBeside => Some(ViewAction::OpenBeside),
                            PaneAction::Split(side) => Some(ViewAction::OpenAt(side)),
                            PaneAction::OpenWindow => Some(ViewAction::OpenWindow),
                            PaneAction::Detach => Some(ViewAction::Detach),
                            PaneAction::Dock => Some(ViewAction::Dock),
                            PaneAction::Close => Some(ViewAction::Close),
                            PaneAction::Drag => {
                                self.begin_editor_drag(ui.ctx(), id);
                                None
                            }
                        };
                        if let Some(action) = action {
                            self.queue_view_action(ui.ctx(), id, action);
                        }
                    }
                }
                // Workspace painters may fill their maximum rectangle. Keep the shared
                // pane header outside their clip, including Library and definition panes.
                ui.set_clip_rect(ui.clip_rect().intersect(ui.available_rect_before_wrap()));
                if self.views.view(id).is_some_and(|view| view.library.is_some()) {
                    self.library_workspace_pane(ui, id);
                    return;
                }
                if self.views.view(id).is_some_and(|view| view.definition.is_some()) {
                    self.library_pane(ui, id);
                    return;
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
                if let Some(view) = self.views.view_mut(id) {
                    view.canvas_cards = covered.iter().map(|card| card.intersect(canvas)).filter(|card| card.is_positive()).collect();
                }
                pcbcraft_chrome::automation::region(ui, format!("canvas:{}", id.0), uncovered, matches!(self.dialog, Dialog::None));
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
                    if matches!(self.dialog, Dialog::None) && self.views.reveal_for(id, &self.session, self.preferences.selection_follow) {
                        ui.ctx().request_repaint();
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
                    if matches!(self.dialog, Dialog::None) && self.views.reveal_for(id, &self.session, self.preferences.selection_follow) {
                        ui.ctx().request_repaint();
                    }
                    let highlight = self.views.schematic_paint_highlight(id, &self.session);
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
                                    &mut view.schematic.overlap_cache,
                                    None,
                                    &view.schematic.selection,
                                    view.schematic.selected_leg.as_ref(),
                                    view.schematic.whole_wire_drag,
                                    view.schematic.selected_sheet_symbol,
                                    &highlight,
                                    &view.inspection,
                                    view.filters,
                                    false,
                                    false,
                                    self.preferences.navigation,
                                    self.preferences.schematic_display(),
                                    self.preferences.auto_heal_wires,
                                    self.parts.repeats(),
                                    self.preferences.units,
                                    &view.schematic.wire_draft,
                                    &label,
                                    None,
                                );
                            }
                        });
                    });
                }
                // The boardless introduction supplies its own context and occupies the canvas;
                // an overlaid breadcrumb would collide with its scrollable content in short panes.
                let empty_board = self.views.active.workspace == Workspace::Board && self.views.active.board.target.is_none();
                if !headers
                    && !empty_board
                    && let Some(path) = self.views.active.sheet_path(&self.session)
                {
                    let count = if self.views.active.workspace == Workspace::Board {
                        format!(
                            "{} footprints",
                            self.session
                                .project()
                                .and_then(|project| project.board_view(self.views.active.board.target?))
                                .map_or(0, |board| board.footprint_ids().len())
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
            if let Some((area, painter, slot)) = header {
                let text_area = Rect::from_x_y_ranges(area.x_range(), f32::NEG_INFINITY..=f32::INFINITY);
                painter.set(slot, egui::Shape::Vec(self.view_background_shapes(&painter, id, area, text_area)));
            }
        }
        self.sync_library_focus();
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
            self.views.active.filters,
            &self.views.active.inspection,
            &self.views.active.board.hidden_layers,
        );
        // Panels belong to individual hosts; apply personal criteria to each bound chooser.
        self.parts.set_filter(self.library_preferences.filter.clone());
        if let Some(pick) = self.parts.show(ui, area, self.browser.rect(), source, self.session.libraries(), self.session.project()) {
            match pick {
                crate::parts_browser::Pick::Filters => {
                    self.dialog = crate::Dialog::Preferences(crate::preferences::PreferencesWindow::at_libraries(self.preferences));
                }
                crate::parts_browser::Pick::Place(placement) => self.begin_template(placement),
                crate::parts_browser::Pick::Unit { definition, component, unit, value } => {
                    self.views.active.schematic.tool = Some(crate::schematic_canvas::Tool::PlaceSymbol {
                        definition,
                        component: Some(component),
                        part: None,
                        unit,
                        value,
                        revision: self.session.revision(),
                        turns: 0,
                    });
                }
                crate::parts_browser::Pick::Libraries => self.open_library_workspace(ui.ctx()),
                crate::parts_browser::Pick::Stale => self.message = Some("View or project changed; reopen the Parts Catalog to continue.".into()),
            }
        }
        pick.map(|pick| (source, pick))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A large connection-style row spanning 768 of 1,024 parts must visit at most one
    /// symbol per live unit on activation, with no report-sized copies on repeat paint.
    #[test]
    fn large_report_probe_is_single_pass_and_reuses_paint_sets() {
        const UNITS: usize = 1024;
        const ENDPOINT_PARTS: usize = 768;
        const PAINTS: usize = 32;
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board":true})).unwrap();
        for i in 0..UNITS {
            let x = (i % 32) as i64 * 5_080_000;
            let y = (i / 32) as i64 * 5_080_000;
            session.apply(pcbcraft_engine::PLACE_RESISTOR, &json!({"x_nm":x,"y_nm":y})).unwrap();
            if i < ENDPOINT_PARTS {
                let symbol = session.project().unwrap().symbols().last().unwrap().id();
                session
                    .apply(
                        pcbcraft_engine::PLACE_FOOTPRINT,
                        &json!({"symbol":symbol,"definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":x,"y_nm":y}),
                    )
                    .unwrap();
            }
        }
        let project = session.project().unwrap();
        let board = project.board_target().unwrap();
        let part_numbers: BTreeSet<_> = project.board_view(board).unwrap().sheet_symbols().take(ENDPOINT_PARTS).map(|(_, part, _)| part).collect();
        assert_eq!(project.all_symbols().count(), UNITS);
        assert_eq!(part_numbers.len(), ENDPOINT_PARTS);

        let mut views = EditorViews::default();
        views.bind_opened_project(&session);
        let schematic = views.active.id();
        let peer = views.open_beside(schematic).unwrap();
        let pcb = views.open_beside(peer).unwrap();
        views.switch_workspace(pcb, Workspace::Board, &session);
        views.probe_update_parts(schematic, board, &part_numbers, &session);
        let report = views.report_probe.as_ref().unwrap();
        assert_eq!(report.symbols_visited, UNITS);
        let (_, symbol_set) = &report.schematic[&project.sheet_target()];
        assert_eq!(symbol_set.symbols.len(), ENDPOINT_PARTS);
        assert_eq!(report.footprints.len(), ENDPOINT_PARTS);
        for _ in 0..PAINTS {
            for pane in [schematic, peer] {
                let PaintHighlight::Shared(highlight) = views.schematic_paint_highlight(pane, &session) else { panic!("must reuse report symbols") };
                assert!(Arc::ptr_eq(&highlight, symbol_set));
            }
            let PaintHighlight::Shared(highlight) = views.board_paint_highlight(pcb, &session) else { panic!("must reuse report placements") };
            assert!(Arc::ptr_eq(&highlight, &report.footprints));
        }
    }
}
