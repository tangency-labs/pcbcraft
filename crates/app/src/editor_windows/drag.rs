//! In-process pane dragging. No OS payload, IPC endpoint or second process can supply a view.

use egui::{Context, Pos2, Rect, Ui};

use super::ViewAction;
use crate::{Dialog, PcbApp, ViewId, ViewInput, editor_layout::HostId};

/// A gesture retains revision-checked authority from its originating pane until release.
#[derive(Debug)]
pub(super) struct PaneDrag {
    source: ViewInput,
    host: HostId,
    started_frame: u64,
    local: Pos2,
    source_pointer: Pos2,
    header_anchor: Pos2,
    desktop: Option<Pos2>,
    receiver: HostId,
    /// Explicit window-target row under the pointer; takes priority over geometric destinations.
    fallback: Option<DropDestination>,
    released: bool,
}

/// The operation shown by feedback is also the operation committed on release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DropDestination {
    NewWindow,
    Beside(ViewInput),
    Reorder(ViewInput),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-monitor pointer capture must compare physical positions even when destination
    /// scale differs and its monitor uses negative desktop coordinates.
    #[test]
    fn dock_geometry_handles_mixed_scales_and_negative_monitor_origins() {
        let source = HostGeometry { origin: Some(egui::pos2(100.0, 50.0)), scale: 1.5, content: Rect::NOTHING, panes: vec![] };
        let destination = HostGeometry {
            origin: Some(egui::pos2(-400.0, 20.0)),
            scale: 2.0,
            content: Rect::NOTHING,
            panes: vec![(ViewId(7), Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(210.0, 320.0)))],
        };
        let desktop = source.desktop(egui::pos2(-500.0, 150.0)).unwrap();
        assert_eq!(destination.target(desktop).map(|(id, _)| id), Some(ViewId(7)));
        assert!(destination.target(egui::pos2(50.0, 50.0)).is_none());
    }
}

/// Desktop coordinates are physical pixels so windows with different scales can exchange panes.
#[derive(Debug)]
pub(super) struct HostGeometry {
    origin: Option<Pos2>,
    scale: f32,
    content: Rect,
    panes: Vec<(ViewId, Rect)>,
}

impl HostGeometry {
    /// Convert client-local points only when the compositor exposes the native window position.
    fn desktop(&self, point: Pos2) -> Option<Pos2> {
        Some((self.origin? + point.to_vec2()) * self.scale)
    }

    /// Resolve a desktop point into one of this instance's currently rendered panes.
    fn target(&self, desktop: Pos2) -> Option<(ViewId, Rect)> {
        let local = desktop / self.scale - self.origin?.to_vec2();
        self.panes.iter().copied().find(|(_, rect)| rect.contains(local))
    }
}

impl PcbApp {
    /// Keep pane dragging's cursor after canvas tools and native edge controls render.
    pub(crate) fn editor_drag_cursor(&self, ctx: &Context) {
        if self.windows.drag.is_some() {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }
    }

    /// Start a native pane drag after egui's movement threshold, cancelling editing previews.
    /// A drag belongs exclusively to this application object and never accepts external payloads.
    pub(crate) fn begin_editor_drag(&mut self, ctx: &Context, id: ViewId) {
        if crate::IN_BROWSER || self.home || self.project_action_pending() || !matches!(self.dialog, Dialog::None) {
            return;
        }
        let Some(source) = self.views.view(id).and_then(|view| view.control_input(&self.session)) else { return };
        let Some(host) = self.views.owner(id) else { return };
        let Some(local) = ctx.input(|input| input.pointer.interact_pos()) else { return };
        self.views.cancel_gestures();
        self.cancel_library_gestures();
        egui::DragAndDrop::clear_payload(ctx);
        let desktop = self.windows.drag_geometry.get(&host).and_then(|geometry| geometry.desktop(local));
        self.windows.drag = Some(PaneDrag {
            source,
            host,
            started_frame: ctx.cumulative_frame_nr(),
            local,
            source_pointer: local,
            header_anchor: ctx.input(|input| input.pointer.press_origin()).unwrap_or(local),
            desktop,
            receiver: host,
            fallback: None,
            released: false,
        });
        self.repaint_editors(ctx);
    }

    /// Observe each native input stream without depending on viewport rendering order.
    /// Escape, modal/project transitions and stale source authority cancel without moving views.
    /// Raw release survives native focus/hover resets; a later press abandons lost capture,
    /// while repeated passes over the gesture's initial frame retain coalesced press/motion.
    pub(crate) fn observe_editor_drag(&mut self, ctx: &Context) {
        let host = self.views.host;
        let geometry = self.windows.drag_geometry.entry(host).or_insert_with(|| HostGeometry {
            origin: None,
            scale: ctx.pixels_per_point(),
            content: Rect::NOTHING,
            panes: Vec::new(),
        });
        geometry.origin = ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.min));
        geometry.scale = ctx.pixels_per_point();
        geometry.content = ctx.content_rect();
        let Some(drag) = &self.windows.drag else { return };
        let fresh_press = (host != drag.host || ctx.cumulative_frame_nr() != drag.started_frame)
            && ctx.input(|input| {
                input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: true, .. }))
            });
        if self.home
            || self.project_action_pending()
            || !matches!(self.dialog, Dialog::None)
            || ctx.input(|input| input.key_pressed(egui::Key::Escape))
            || fresh_press
            || self.views.owner(drag.source.view()) != Some(drag.host)
            || !self.views.view(drag.source.view()).is_some_and(|view| view.control_input(&self.session) == Some(drag.source))
        {
            self.windows.drag = None;
            self.repaint_editors(ctx);
            return;
        }
        let motion = ctx.input(|input| {
            let released = input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: false, .. }));
            input.events.iter().rev().find_map(|event| match event {
                egui::Event::PointerMoved(pos) | egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, .. } => {
                    Some((*pos, released))
                }
                _ => None,
            })
        });
        if let Some((local, released)) = motion
            && let Some(drag) = &mut self.windows.drag
        {
            drag.local = local;
            if host == drag.host {
                drag.source_pointer = local;
            }
            drag.desktop = self.windows.drag_geometry.get(&host).and_then(|geometry| geometry.desktop(local));
            drag.receiver = host;
            drag.fallback = None;
            drag.released |= released;
            self.repaint_editors(ctx);
        }
        if self.windows.drag.is_some() {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }
    }

    /// Whether detaching creates a new host, rather than addressing an already detached sole pane.
    fn drag_can_detach(&self, drag: &PaneDrag) -> bool {
        drag.host == self.views.main || self.views.hosts.get(&drag.host).is_some_and(|host| host.layout.visible_views().len() > 1)
    }

    /// Resolve the currently indicated operation. Local targets take priority; source-window
    /// coordinates cannot silently target a different window underneath it. Native captured
    /// input outside the source can address exposed panes in other windows of this instance.
    fn drag_destination(&self, drag: &PaneDrag) -> Option<DropDestination> {
        if let Some(destination) = drag.fallback {
            return Some(destination);
        }
        let inside_source =
            drag.receiver == drag.host && self.windows.drag_geometry.get(&drag.host).is_some_and(|geometry| geometry.content.contains(drag.local));
        if inside_source {
            let source_group = self.views.hosts.get(&drag.host)?.layout.group(drag.source.view())?.views();
            let target = self
                .windows
                .drag_geometry
                .get(&drag.host)?
                .panes
                .iter()
                .find(|(id, rect)| !source_group.contains(id) && rect.contains(drag.local))
                .map(|(id, _)| *id)?;
            return self.views.view(target)?.control_input(&self.session).map(DropDestination::Reorder);
        }
        if !inside_source {
            let target = if let Some(desktop) = drag.desktop {
                self.windows
                    .drag_geometry
                    .iter()
                    .filter(|(host, _)| **host != drag.host)
                    .find_map(|(_, geometry)| geometry.target(desktop).map(|(id, _)| id))
            } else if drag.receiver != drag.host {
                self.windows
                    .drag_geometry
                    .get(&drag.receiver)
                    .and_then(|geometry| geometry.panes.iter().find(|(_, rect)| rect.contains(drag.local)).map(|(id, _)| *id))
            } else {
                None
            };
            if let Some(target) = target.and_then(|id| self.views.view(id)).and_then(|view| view.control_input(&self.session)) {
                return Some(DropDestination::Beside(target));
            }
            if drag.receiver == drag.host && self.drag_can_detach(drag) {
                return Some(DropDestination::NewWindow);
            }
        }
        None
    }

    /// Cache each host's panes, show discoverable destinations in the source and animate the
    /// actual dock region. All platforms get the same local targets; geometry also enables
    /// direct drops over exposed panes. Selected rows describe explicit window operations.
    pub(crate) fn editor_drag_targets(&mut self, ui: &Ui, panes: &[(ViewId, Rect)]) {
        let host = self.views.host;
        if let Some(geometry) = self.windows.drag_geometry.get_mut(&host) {
            geometry.panes = panes.to_vec();
        }
        if let Some(drag) = &self.windows.drag
            && host == drag.host
        {
            let targets: Vec<_> =
                self.views.hosts.iter().filter(|(id, _)| **id != host).map(|(id, host)| (id.0, self.view_title(host.focused))).collect();
            let pane = panes.iter().find(|(id, _)| *id == drag.source.view()).map_or(ui.available_rect_before_wrap(), |(_, rect)| *rect);
            let pointer = (drag.receiver == host).then_some(drag.source_pointer);
            let hovered = pcbcraft_chrome::editor::window_drop_targets(
                ui,
                pane,
                drag.header_anchor,
                &targets,
                self.views.main.0,
                pcbcraft_chrome::editor::WindowDropState {
                    pointer,
                    can_detach: self.drag_can_detach(drag),
                    detaching: matches!(self.drag_destination(drag), Some(DropDestination::NewWindow)),
                },
            );
            let target = match hovered {
                Some(pcbcraft_chrome::editor::WindowDrop::NewWindow) => Some(DropDestination::NewWindow),
                Some(pcbcraft_chrome::editor::WindowDrop::Host(id)) => self
                    .views
                    .hosts
                    .get(&HostId(id))
                    .and_then(|host| self.views.view(host.focused))
                    .and_then(|view| view.control_input(&self.session))
                    .map(DropDestination::Beside),
                None => None,
            };
            if let Some(drag) = &mut self.windows.drag {
                let changed = drag.fallback != target;
                drag.fallback = target;
                if changed {
                    self.repaint_editors(ui.ctx());
                }
            }
        }
        let destination = self.windows.drag.as_ref().and_then(|drag| self.drag_destination(drag));
        for (id, rect) in panes {
            let active = matches!(destination, Some(DropDestination::Beside(target)) if target.view() == *id);
            pcbcraft_chrome::editor::dock_preview(ui, ui.scope_id().with(("dock_destination", id)), *rect, active, self.windows.drag.is_some());
            let reposition = matches!(destination, Some(DropDestination::Reorder(target)) if target.view() == *id);
            pcbcraft_chrome::editor::reorder_preview(
                ui,
                ui.scope_id().with(("reorder_destination", id)),
                *rect,
                reposition,
                self.windows.drag.is_some(),
            );
        }
        if let Some(drag) = &self.windows.drag
            && drag.host == host
        {
            let source = panes.iter().find(|(id, _)| *id == drag.source.view()).map_or(Rect::NOTHING, |(_, rect)| *rect);
            pcbcraft_chrome::editor::pane_drag_source(ui, source, true);
        } else {
            pcbcraft_chrome::editor::pane_drag_source(ui, Rect::NOTHING, false);
        }
    }

    /// Commit only after every host has restored its render slots. A release in another pane
    /// moves the original pane; a release outside native bounds detaches it. Drops elsewhere in
    /// the source window do nothing. This queue uses the same handlers as other view controls.
    pub(crate) fn finish_editor_drag(&mut self, ctx: &Context) {
        self.windows.drag_geometry.retain(|host, _| self.views.hosts.contains_key(host));
        self.windows.spawn_positions.retain(|host, _| self.views.hosts.contains_key(host));
        if !self.windows.drag.as_ref().is_some_and(|drag| drag.released) {
            return;
        }
        let destination = self.windows.drag.as_ref().and_then(|drag| self.drag_destination(drag));
        let Some(drag) = self.windows.drag.take() else { return };
        let action = match destination {
            Some(DropDestination::Beside(target)) => Some(ViewAction::MoveBeside(target)),
            Some(DropDestination::Reorder(target)) => Some(ViewAction::Reorder(target)),
            Some(DropDestination::NewWindow) => {
                let position = drag.desktop.and_then(|desktop| self.windows.drag_geometry.get(&drag.host).map(|geometry| desktop / geometry.scale));
                Some(ViewAction::DetachAt(position))
            }
            None => None,
        };
        if let Some(action) = action {
            self.windows.pending.push((drag.source, action));
        }
        self.repaint_editors(ctx);
    }
}
