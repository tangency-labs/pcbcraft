//! Board selection and transform previews over resolved engine geometry; commands own edits.
use crate::{
    board_appearance::CopperFocus,
    canvas_camera::Camera,
    editor_view::{BoardState, ViewInput},
    preferences::Navigation,
};
use egui::{PointerButton, Pos2, Rect, Response, Sense, Ui, Vec2};
use pcbcraft_chrome::board;
use pcbcraft_engine::{
    board::{
        MAX_REFERENCE_OFFSET_NM, rotate as rotate_board,
        setup::{LayerPurpose, Side},
        view::{BoardView, CopperLayers, FootprintView, PadView},
    },
    schematic::{Point, rotate},
};
use std::collections::{BTreeMap, BTreeSet};

/// A local transform draft retains its originating input authority until completion or cancellation.
#[derive(Clone, Debug)]
pub(super) struct Draft {
    pub(super) source: ViewInput,
    pub(super) footprint: String,
    pub(super) position: Point,
    pub(super) rotation: u16,
    /// The side the footprint will be on; F flips it.
    pub(super) bottom: bool,
    origin: Point,
    anchor: Point,
    drag: bool,
}

/// A captured group of accepted assignments has preview geometry but no live placements.
/// The engine revalidates the same component set and revision when the canvas commits it.
#[derive(Clone, Debug)]
pub(super) struct Placement {
    pub(super) source: ViewInput,
    pub(super) components: Vec<String>,
    pub(super) candidate: std::sync::Arc<pcbcraft_engine::Project>,
    /// Await the originating native host's first focused frame without starting a gesture there.
    pub(super) handoff: bool,
    pub(super) position: Point,
    pub(super) rotation: u16,
}

/// A disposable marquee captures its view/revision and starting selection until release.
#[derive(Clone, Debug)]
pub(super) struct BoxDrag {
    pub(super) source: ViewInput,
    start: egui::Pos2,
    base: pcbcraft_engine::board::clipboard::BoardSelection,
    add: bool,
    toggle: bool,
    button: PointerButton,
}

/// A reference move owns input until its single release or placement click.
#[derive(Clone, Debug)]
pub(super) struct ReferenceDrag {
    pub(super) source: ViewInput,
    pub(super) footprint: String,
    pub(super) offset: Point,
    pub(super) appearance: Option<pcbcraft_engine::board::reference::Text>,
    original: Point,
    original_appearance: Option<pcbcraft_engine::board::reference::Text>,
    anchor: Point,
    rotation: u16,
    bottom: bool,
    dragging: bool,
    physical: bool,
}

impl ReferenceDrag {
    /// Capture local appearance and the pointer anchor independently of the footprint's physical pose.
    fn new(source: ViewInput, footprint: &FootprintView<'_>, anchor: Point, dragging: bool) -> Self {
        Self {
            source,
            footprint: footprint.id.clone(),
            offset: footprint.reference_offset,
            original: footprint.reference_offset,
            appearance: footprint.reference_text.clone(),
            original_appearance: footprint.reference_text.clone(),
            anchor,
            rotation: footprint.rotation,
            bottom: footprint.bottom,
            dragging,
            physical: footprint.physical_reference,
        }
    }

    /// Rotate generated print in board space, reversing the local turn for a mirrored bottom placement.
    /// Retain the change with an in-progress move so completing it remains one authored transaction.
    fn rotate_text(&mut self, degrees: u16) {
        if degrees != 0 && self.physical {
            let appearance = self.appearance.get_or_insert_with(Default::default);
            let local = if self.bottom { (360 - degrees) % 360 } else { degrees };
            appearance.rotation = Some((appearance.rotation.unwrap_or(0) + local) % 360);
        }
    }
}

/// Selection and authored intent are handled after the renderer releases its view/session borrows.
#[derive(Default)]
pub(super) struct Interaction {
    pub(super) clipboard: Option<crate::board_clipboard::Action>,
    pub(super) drawing_edit: Option<crate::board_drawing::Command>,
    pub(super) track_edit: Option<crate::board_drawing::Command>,
    pub(super) selection: Option<BTreeSet<String>>,
    pub(super) items: Option<pcbcraft_engine::board::clipboard::BoardSelection>,
    pub(super) edit: Option<Draft>,
    pub(super) reference_edit: Option<ReferenceDrag>,
    pub(super) reference_pick: Option<String>,
    pub(super) placement: Option<Placement>,
}

/// Select footprints by click or directional marquee; preview one footprint transform at a time.
/// Empty-space primary drags and secondary drags box-select visible footprint bounds; Shift adds
/// and Ctrl/Command toggles against the captured selection. Invalid release cancels the box.
/// Transform drags retain their grab offset; M moves the origin
/// to the pointer. R rotates counterclockwise on screen, including in a mirrored view or a move.
/// Only completion on the canvas emits an edit; secondary click, disabled input or stale
/// authority discards the draft.
/// Footprint moves retain placement clicks over edges and win overlapping drag starts unless
/// an explicit drawing tool or an existing drawing edit already owns the gesture.
/// Physical pad color/visibility follow engine-resolved copper membership; the active copper
/// layer changes display priority for spanning lands without rebinding any geometry.
/// Selection filters suppress picking and transform shortcuts, without hiding rendered geometry.
/// Net highlighting shares derived connectivity keyed to the view's board/revision authority.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut Ui,
    project: BoardView<'_>,
    camera: &mut Camera,
    state: &mut BoardState,
    source: Option<ViewInput>,
    highlighted: &BTreeSet<String>,
    filters: crate::selection_filters::Filters,
    appearance: &crate::board_appearance::Appearance,
    owner_enabled: bool,
    navigation: Navigation,
    opacity: u8,
    grid_style: pcbcraft_chrome::grid::Style,
    label: &str,
    markers: &[(Point, bool)],
    unit: pcbcraft_chrome::units::Unit,
) -> Interaction {
    let idle = !crate::board_clipboard::busy(state);
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::CLICK | Sense::DRAG);
    let enabled = owner_enabled && !egui::Popup::is_any_open(ui.ctx());
    let route_menu_open = state.tracks.tool && response.context_menu_opened();
    if ui.input(|input| input.pointer.button_pressed(PointerButton::Secondary)) {
        state.context_menu_allowed = idle;
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Pane, true, label));
    camera.navigate_board(&response, rect, project, enabled, navigation);
    let placement_enabled = enabled;
    let enabled = enabled && state.placement.is_none();
    // The view-only ruler owns its clicks, so nothing else selects or edits while it is armed.
    let measuring = crate::board_measure::interact(ui, &response, camera, project, &mut state.measure, enabled && state.draft.is_none());
    let enabled = enabled && !measuring;
    let copper = project.setup().map(|setup| {
        setup
            .layers()
            .iter()
            .filter(|layer| layer.purpose == LayerPurpose::Copper && !state.hidden_layers.contains(&layer.id.to_string()))
            .min_by_key(|layer| state.active_layer.as_ref() != Some(&layer.id.to_string()))
    });
    let visible = copper.is_none_or(|layer| layer.is_some());
    let color = board::copper_color(copper.flatten().and_then(|layer| layer.side.map(|side| side == Side::Front)));
    let footprints: Vec<_> = project.footprints().collect();
    let hit_bounds: Vec<_> = footprints
        .iter()
        .map(|footprint| {
            let (pads, graphics) = drawing(footprint, camera, rect, project, state, visible, appearance);
            if pads.is_empty() && graphics.is_empty() {
                egui::Rect::NOTHING
            } else {
                board::footprint_bounds(&pads, &graphics, camera.screen(footprint.position, rect))
            }
        })
        .collect();
    let hit = |pointer: egui::Pos2| {
        footprints.iter().zip(&hit_bounds).rev().find_map(|(footprint, bounds)| (filters.footprints && bounds.contains(pointer)).then_some(footprint))
    };
    let lost_focus = ui.input(|i| i.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))));
    if !enabled || lost_focus || state.box_drag.as_ref().is_some_and(|drag| Some(drag.source) != source) {
        state.box_drag = None;
    }
    let hovered = response.hover_pos().filter(|_| visible).and_then(hit);
    let modifiers = ui.input(|i| i.modifiers);
    let drag_start = response.drag_started_by(PointerButton::Primary) && !modifiers.shift && !modifiers.ctrl && !modifiers.command;
    let reference_press = response.contains_pointer()
        && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary))
        && !modifiers.shift
        && !modifiers.ctrl
        && !modifiers.command;
    let reference_enabled = enabled
        && !lost_focus
        && filters.footprints
        && state.draft.is_none()
        && state.box_drag.is_none()
        && state.drawing.tool.is_none()
        && state.drawing.source().is_none()
        && !state.tracks.busy();
    let start = if reference_enabled && state.reference_drag.is_none() {
        let pointer = if reference_press || drag_start {
            ui.input(|i| i.pointer.press_origin())
        } else {
            response.hover_pos().filter(|_| state.move_requested)
        };
        pointer.and_then(|pointer| {
            let hovered = footprints
                .iter()
                .zip(&hit_bounds)
                .rev()
                .find(|(footprint, bounds)| {
                    reference_bounds(ui.painter(), camera, rect, project, state, footprint, **bounds, appearance).contains(pointer)
                })
                .map(|(footprint, _)| footprint);
            hovered
                .or_else(|| {
                    state
                        .move_requested
                        .then(|| footprints.iter().find(|footprint| state.reference.selected.as_ref() == Some(&footprint.id)))
                        .flatten()
                })
                .map(|footprint| (pointer, footprint))
        })
    } else {
        None
    };
    let reference_pick = start.map(|(_, footprint)| footprint.id.clone());
    let reference_rotation = camera.model_rotation(state.rotate_requested);
    let (mut reference_consumed, mut reference_edit) =
        move_reference(&response, camera, state, source.filter(|_| reference_enabled), start, reference_press || drag_start, reference_rotation);
    if reference_enabled
        && !reference_consumed
        && reference_rotation != 0
        && let Some(footprint) = footprints.iter().find(|footprint| state.reference.selected.as_ref() == Some(&footprint.id))
        && footprint.physical_reference
        && let Some(source) = source
    {
        let mut edit = ReferenceDrag::new(source, footprint, Point { x_nm: 0, y_nm: 0 }, false);
        edit.rotate_text(reference_rotation);
        reference_edit = Some(edit);
        reference_consumed = true;
    }
    if reference_consumed || state.reference.selected.is_some() {
        state.flip_requested = false;
        state.rotate_requested = 0;
    }
    let enabled = enabled && !reference_consumed;
    // A footprint gesture owns its completion and overlapping drag origin. Explicit drawing
    // tools and existing drawing edits keep their own input even when crossing a footprint.
    let footprint_drag = drag_start
        && visible
        && state.drawing.tool.is_none()
        && state.drawing.source().is_none()
        && !state.tracks.busy()
        && ui.input(|i| i.pointer.press_origin()).and_then(hit).is_some();
    // M moves selected copper when no footprint is selected; otherwise it stays a footprint move.
    if state.move_requested && state.selection.is_empty() && !state.tracks.selected.is_empty() {
        state.move_requested = false;
        state.tracks.move_requested = true;
    }
    // The route tool owns its clicks; without it, a plain click on a track selects that track
    // unless a footprint lies under the pointer, and a drag on copper moves it.
    let route = crate::board_tracks::interact(
        ui,
        &response,
        camera,
        project,
        &mut state.tracks,
        state.active_layer.as_deref(),
        &state.hidden_layers,
        source,
        (enabled || owner_enabled && route_menu_open)
            && state.draft.is_none()
            && state.box_drag.is_none()
            && !footprint_drag
            && state.drawing.tool.is_none()
            && state.drawing.source().is_none(),
        hovered.is_some(),
        crate::board_tracks::Kinds { tracks: filters.tracks, vias: filters.vias },
    );
    let edge = crate::board_drawing::interact(
        ui,
        &response,
        camera,
        project,
        &mut state.drawing,
        state.tracks.width_nm,
        state.active_layer.as_deref(),
        &state.hidden_layers,
        source,
        enabled && state.draft.is_none() && state.box_drag.is_none() && !footprint_drag && !route.consumed && !state.tracks.busy(),
        state.move_requested,
        filters.graphics,
    );
    let enabled = enabled && !edge.consumed && !route.consumed;
    let mut out = Interaction { drawing_edit: edge.command, track_edit: route.command, reference_edit, reference_pick, ..Interaction::default() };
    if (edge.consumed || route.consumed) && (!state.selection.is_empty() || state.reference.selected.is_some()) {
        out.selection = Some(BTreeSet::new());
    }
    if route.consumed {
        state.drawing.select(None);
    }
    // A via moved the route to another copper layer; follow it without cancelling the route.
    if let Some(layer) = route.switch_layer {
        state.active_layer = Some(layer);
    }
    if edge.consumed {
        state.tracks.selected.clear();
    }
    if !enabled || state.draft.as_ref().is_some_and(|draft| Some(draft.source) != source) {
        state.draft = None;
    }
    let moving = std::mem::take(&mut state.move_requested);
    let rotation = camera.model_rotation(std::mem::take(&mut state.rotate_requested));
    let rotating = rotation != 0;
    let flipping = std::mem::take(&mut state.flip_requested);
    let cancelled = enabled && ui.input(|i| i.pointer.button_pressed(PointerButton::Secondary));
    if cancelled {
        state.draft = None;
    }
    if enabled
        && !lost_focus
        && state.box_drag.is_none()
        && state.draft.is_none()
        && state.drawing.tool.is_none()
        && state.drawing.source().is_none()
        && !state.tracks.busy()
        && let Some(source) = source
        && let Some(start) = ui.input(|i| i.pointer.press_origin())
        && (response.drag_started_by(PointerButton::Secondary)
            || (response.drag_started_by(PointerButton::Primary)
                && hit(start).is_none()
                && !crate::board_tracks::copper_under(
                    start,
                    camera,
                    rect,
                    project,
                    &state.hidden_layers,
                    crate::board_tracks::Kinds { tracks: filters.tracks, vias: filters.vias },
                )))
    {
        state.box_drag = Some(BoxDrag {
            source,
            start,
            base: pcbcraft_engine::board::clipboard::BoardSelection {
                footprints: state.selection.clone(),
                drawings: state.drawing.selection(),
                tracks: state.tracks.selected.iter().filter(|id| !project.vias().iter().any(|v| v.id == **id)).copied().collect(),
                vias: state.tracks.selected.iter().filter(|id| project.vias().iter().any(|v| v.id == **id)).copied().collect(),
            },
            add: modifiers.shift,
            toggle: modifiers.ctrl || modifiers.command,
            button: if response.drag_started_by(PointerButton::Secondary) { PointerButton::Secondary } else { PointerButton::Primary },
        });
    }
    if enabled && !cancelled && !lost_focus && state.box_drag.is_none() {
        if state.draft.is_none()
            && visible
            && filters.footprints
            && let Some(source) = source
        {
            let picked = if drag_start {
                ui.input(|i| i.pointer.press_origin()).and_then(hit)
            } else if moving || rotating || flipping {
                hovered.or_else(|| (state.selection.len() == 1).then(|| footprints.iter().find(|f| state.selection.contains(&f.id))).flatten())
            } else {
                None
            };
            if let Some(footprint) = picked {
                state.drawing.select(None);
                out.selection = Some(BTreeSet::from([footprint.id.clone()]));
                let anchor = if drag_start {
                    ui.input(|i| i.pointer.press_origin()).map(|p| camera.unsnapped(p, rect)).unwrap_or(footprint.position)
                } else {
                    footprint.position
                };
                let draft = Draft {
                    source,
                    footprint: footprint.id.clone(),
                    position: footprint.position,
                    origin: footprint.position,
                    anchor,
                    rotation: footprint.rotation,
                    bottom: footprint.bottom,
                    drag: drag_start,
                };
                if moving || drag_start {
                    state.draft = Some(draft);
                } else if rotating {
                    out.edit = Some(Draft { rotation: (draft.rotation + rotation) % 360, ..draft });
                } else if flipping {
                    out.edit = Some(Draft { bottom: !draft.bottom, ..draft });
                }
            }
        }
        if let Some(draft) = &mut state.draft {
            if rotating {
                draft.rotation = (draft.rotation + rotation) % 360;
            }
            if flipping {
                draft.bottom = !draft.bottom;
            }
            if let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) {
                let pointer = camera.unsnapped(pointer, rect);
                let candidate = Point {
                    x_nm: draft.origin.x_nm.saturating_add(pointer.x_nm.saturating_sub(draft.anchor.x_nm)),
                    y_nm: draft.origin.y_nm.saturating_add(pointer.y_nm.saturating_sub(draft.anchor.y_nm)),
                };
                draft.position = camera.snap(camera.screen(candidate, rect), rect);
            }
            let completed = if draft.drag { response.drag_stopped_by(PointerButton::Primary) } else { response.clicked() && !moving };
            if completed {
                if response.contains_pointer() {
                    out.edit = state.draft.take();
                } else {
                    state.draft = None;
                }
            } else if draft.drag && !ui.input(|i| i.pointer.primary_down()) {
                state.draft = None;
            }
        } else if !moving && !rotating && !flipping && response.clicked() {
            let mut selection = if modifiers.shift || modifiers.ctrl || modifiers.command { state.selection.clone() } else { BTreeSet::new() };
            if let Some(footprint) = hovered {
                if (modifiers.ctrl || modifiers.command) && selection.contains(&footprint.id) {
                    selection.remove(&footprint.id);
                } else {
                    selection.insert(footprint.id.clone());
                }
            }
            out.selection = Some(selection);
        }
    }
    let p = ui.painter().with_clip_rect(rect);
    board::canvas(&p, rect, camera.screen(Point { x_nm: 0, y_nm: 0 }, rect), camera.snap_step(), opacity, grid_style);
    let ratsnest = state.ratsnest(project, source);
    let requested = std::mem::take(&mut state.net_highlight_requested);
    let connectivity =
        (requested || state.highlight_net.is_some() || state.tracks.route_contact().is_some()).then(|| state.copper_connectivity(project, source));
    if requested && let Some(connectivity) = &connectivity {
        state.highlight_net =
            response.hover_pos().and_then(|pointer| net_under(pointer, camera, rect, project, &state.hidden_layers, &connectivity.nets));
    }
    let focus = connectivity.as_deref().and_then(|data| CopperFocus::new(data, state.tracks.route_contact(), state.highlight_net.as_deref()));
    let context = board::context_painter(&p, focus.is_some());
    crate::board_drawing::paint(&context, rect, camera, project, &state.drawing, state.tracks.width_nm, &state.hidden_layers, appearance);
    // A transform committed in this frame is painted at its new pose. The engine applies it after
    // this pass, so painting the stored pose here would make the footprint spring back for a frame.
    // Tracks on its pads are painted where the move carries them.
    let shown = state.draft.clone().or_else(|| out.edit.clone());
    let shown = shown.as_ref();
    let carried = shown.map(|draft| project.carried_tracks(&draft.footprint, draft.position, draft.rotation, draft.bottom)).unwrap_or_default();
    crate::board_tracks::paint(
        ui,
        rect,
        camera,
        project,
        &state.tracks,
        state.active_layer.as_deref(),
        &state.hidden_layers,
        appearance,
        focus,
        &carried,
    );
    let selected_reference = out.reference_pick.clone().or_else(|| out.selection.is_none().then(|| state.reference.selected.clone()).flatten());
    if (!state.hidden_layers.contains(crate::editor_view::RATSNEST_KEY) || selected_reference.is_some())
        && let Some(ratsnest) = ratsnest
    {
        let draft = shown.and_then(|draft| footprints.iter().find(|footprint| footprint.id == draft.footprint).map(|footprint| (draft, footprint)));
        for line in &ratsnest.lines {
            let selected = selected_reference.as_ref().is_some_and(|id| line.from.footprint == *id || line.to.footprint == *id);
            if state.hidden_layers.contains(crate::editor_view::RATSNEST_KEY) && !selected {
                continue;
            }
            let at = |endpoint: &pcbcraft_engine::board::ratsnest::Endpoint| {
                let point = match draft {
                    Some((draft, footprint)) if endpoint.footprint == footprint.id => {
                        preview_point(endpoint.position, footprint.position, footprint.rotation, footprint.bottom, draft)
                    }
                    _ => endpoint.position,
                };
                camera.screen(point, rect)
            };
            if selected {
                board::selected_airwire(&p, at(&line.from), at(&line.to));
            } else {
                board::airwire(&context, at(&line.from), at(&line.to));
            }
        }
    }
    for mut footprint in footprints {
        if let Some(draft) = shown.filter(|d| d.footprint == footprint.id) {
            preview(&mut footprint, draft);
        }
        if let Some(draft) = state.reference_drag.as_ref().or(out.reference_edit.as_ref()).filter(|draft| draft.footprint == footprint.id) {
            footprint.reference_offset = draft.offset;
            footprint.reference_text = draft.appearance.clone();
            if footprint.physical_reference {
                let reference = project.reference_preview(&footprint, draft.offset);
                footprint.graphics.retain(|graphic| !graphic.reference);
                footprint.graphics.extend(reference);
            }
        }
        let center = camera.screen(footprint.position, rect);
        let (mut pads, graphics) = drawing(&footprint, camera, rect, project, state, visible, appearance);
        let bounds = board::footprint_bounds(&pads, &graphics, center);
        let shift = reference_shift(&footprint, camera, rect);
        let label_visible = if footprint.physical_reference {
            graphics.iter().any(|graphic| graphic.reference && rect.intersects(graphic.bounds()))
        } else {
            rect.intersects(board::footprint_reference_bounds(&p, bounds, footprint.reference, shift))
        };
        if (pads.is_empty() && graphics.is_empty()) || !(rect.intersects(bounds) || label_visible) {
            continue;
        }
        if let Some(focus) = focus {
            for pad in &mut pads {
                let emphasized = focus.contains_pad(&footprint.id, &pad.number, pad.net.as_deref());
                pad.color = Some(board::copper_emphasis(pad.color.unwrap_or(color), Some(emphasized)));
            }
        }
        let reference_bounds = reference_bounds(&p, camera, rect, project, state, &footprint, bounds, appearance);
        let selected = selected_reference.as_ref() == Some(&footprint.id);
        let reference_color = graphics
            .iter()
            .find(|graphic| graphic.reference)
            .map_or_else(|| pcbcraft_chrome::Tokens::get().board_ink, |graphic| graphic.color.unwrap_or_else(|| board::graphic_color(graphic.style)));
        if selected {
            board::reference_owner_link(&p, reference_bounds, center, reference_color);
        }
        board::footprint(
            &p,
            &pads,
            &graphics,
            if footprint.physical_reference { "" } else { footprint.reference },
            center,
            color,
            state.selection.contains(&footprint.id)
                || shown.is_some_and(|d| d.footprint == footprint.id)
                || selected_reference.as_ref() == Some(&footprint.id),
            highlighted.contains(&footprint.id),
            focus.is_some(),
            shift,
        );
        if selected || reference_enabled && response.hover_pos().is_some_and(|pointer| reference_bounds.contains(pointer)) {
            board::reference_outline(&p, reference_bounds, selected, reference_color);
        }
    }
    for (at, focused) in markers {
        let at = camera.screen(*at, rect);
        if rect.contains(at) {
            board::drc_marker(&p, at, *focused);
        }
    }
    if let Some(drag) = &state.box_drag
        && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
    {
        let window = egui::Rect::from_two_pos(drag.start, pointer);
        let crossing = pointer.x < drag.start.x;
        debug_assert_eq!(hit_bounds.len(), project.footprints().count());
        let mut hits = geometry_for_box(project, camera, rect, state, filters, window, crossing);
        hits.footprints = footprints_for_box(project, &hit_bounds, window, crossing, filters.footprints);
        pcbcraft_chrome::selection::board_marquee(&p, window, crossing, hits.len());
        if ui.input(|i| i.pointer.button_released(drag.button)) {
            if enabled && response.contains_pointer() {
                out.items = Some(combine_box(&drag.base, hits, drag.add, drag.toggle));
            }
            state.box_drag = None;
        } else if !ui.input(|i| i.pointer.button_down(drag.button)) {
            state.box_drag = None;
        }
    }
    if route.consumed {
        crate::board_tracks::paint_draft(&p, &response, camera, project, &state.tracks, state.active_layer.as_deref(), appearance);
    }
    crate::board_measure::paint(ui, rect, camera, project, &state.measure, unit);
    if project.footprints().next().is_none()
        && project.drawings().is_empty()
        && project.tracks().is_empty()
        && project.vias().is_empty()
        && state.drawing.tool.is_none()
        && !state.tracks.busy()
        && state.placement.is_none()
    {
        board::empty(&p, rect);
    }
    out.placement = place_group(ui, &response, project, camera, state, source, appearance, placement_enabled, visible, color, rotation);
    if source.is_some() && idle && state.context_menu_allowed && !crate::board_clipboard::busy(state) {
        response.context_menu(|ui| {
            use crate::board_clipboard::Action;
            use pcbcraft_chrome::menu::MenuItem;
            let selected = !state.selection.is_empty() || !state.tracks.selected.is_empty() || !state.drawing.selection().is_empty();
            out.clipboard = [
                ("Copy", "Ctrl+C", Action::Copy, selected),
                ("Cut", "Ctrl+X", Action::Cut, selected),
                ("Paste", "Ctrl+V", Action::Paste, true),
                ("Paste Options…", "Ctrl+Shift+V", Action::Options, true),
                ("Duplicate", "Ctrl+D", Action::Duplicate, selected),
                ("Delete", "Del", Action::Delete, selected),
                ("Select all", "Ctrl+A", Action::SelectAll, true),
            ]
            .into_iter()
            .find_map(|(label, key, action, available)| MenuItem::new(label).shortcut(key).enabled(available).show(ui).then_some(action));
        });
    }
    if state.draft.is_some() || state.placement.is_some() || state.reference_drag.is_some() {
        response.on_hover_cursor(egui::CursorIcon::Grabbing);
    }
    out
}

/// Preview a new group using engine-resolved geometry. Until a click commits, cancellation,
/// focus loss or stale input discards only presentation state, retaining assignments. Inert render
/// passes retain the handoff but cannot commit it; opening a modal cancels gestures at the host.
#[allow(clippy::too_many_arguments)]
fn place_group(
    ui: &mut Ui,
    response: &egui::Response,
    project: BoardView<'_>,
    camera: &Camera,
    state: &mut BoardState,
    source: Option<ViewInput>,
    appearance: &crate::board_appearance::Appearance,
    enabled: bool,
    visible: bool,
    color: egui::Color32,
    rotation: u16,
) -> Option<Placement> {
    let mut placement = state.placement.take()?;
    if Some(placement.source) != source {
        return None;
    }
    if placement.handoff && !ui.input(|input| input.focused) {
        state.placement = Some(placement);
        return None;
    }
    if ui.input(|i| {
        i.pointer.button_pressed(PointerButton::Secondary) || i.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false)))
    }) {
        return None;
    }
    if !enabled {
        state.placement = Some(placement);
        return None;
    }
    placement.handoff = false;
    placement.rotation = (placement.rotation + rotation) % 360;
    if let Some(pointer) = ui.input(|i| i.pointer.hover_pos()).filter(|p| response.rect.contains(*p)) {
        placement.position = camera.snap(pointer, response.rect);
    }
    let transform = |point: Point| {
        let (x, y) = rotate(point.x_nm, point.y_nm, (placement.rotation / 90) as u8);
        Point { x_nm: placement.position.x_nm + x, y_nm: placement.position.y_nm + y }
    };
    if let Some(board) = placement.candidate.board_view(project.target) {
        let components: BTreeSet<_> = placement.components.iter().map(String::as_str).collect();
        let ids: BTreeSet<_> = board.footprints().filter(|f| components.contains(f.component.as_str())).map(|f| f.id).collect();
        if !state.hidden_layers.contains(crate::editor_view::RATSNEST_KEY)
            && let Some(ratsnest) = board.ratsnest()
        {
            for line in ratsnest.lines.iter().filter(|line| ids.contains(&line.from.footprint) || ids.contains(&line.to.footprint)) {
                let point = |endpoint: &pcbcraft_engine::board::ratsnest::Endpoint| {
                    if ids.contains(&endpoint.footprint) { transform(endpoint.position) } else { endpoint.position }
                };
                pcbcraft_chrome::board::airwire(
                    ui.painter(),
                    camera.screen(point(&line.from), response.rect),
                    camera.screen(point(&line.to), response.rect),
                );
            }
        }
        for mut footprint in board.footprints().filter(|f| ids.contains(&f.id)) {
            for pad in &mut footprint.pads {
                pad.center = transform(pad.center);
                pad.copper = pad.copper.map(|p| p.map(transform));
            }
            for graphic in &mut footprint.graphics {
                graphic.points = graphic.points.iter().copied().map(transform).collect();
            }
            footprint.position = transform(footprint.position);
            footprint.rotation = placement.rotation;
            let (pads, graphics) = drawing(&footprint, camera, response.rect, board, state, visible, appearance);
            pcbcraft_chrome::board::footprint(
                ui.painter(),
                &pads,
                &graphics,
                if footprint.physical_reference { "" } else { footprint.reference },
                camera.screen(footprint.position, response.rect),
                color,
                true,
                false,
                false,
                reference_shift(&footprint, camera, response.rect),
            );
        }
    }
    if response.clicked() && response.contains_pointer() {
        return Some(placement);
    }
    state.placement = Some(placement);
    None
}

/// Select only visible footprint bounds, sharing the exact geometry used for click picking.
/// A window requires full containment; crossing requires overlap. Empty hidden bounds never hit.
/// `bounds` must contain one entry per `project.footprints()` item in the same iteration order.
fn footprints_for_box(project: BoardView<'_>, bounds: &[egui::Rect], window: egui::Rect, crossing: bool, enabled: bool) -> BTreeSet<String> {
    project
        .footprints()
        .zip(bounds)
        .filter_map(|(footprint, bounds)| {
            (enabled && bounds.is_positive() && if crossing { window.intersects(*bounds) } else { window.contains_rect(*bounds) })
                .then_some(footprint.id)
        })
        .collect()
}

/// Whether a pad has visible copper; unconfigured bindings remain visible, while legacy markers
/// follow the board's overall copper visibility just as they do in normal footprint rendering.
fn pad_visible(pad: &PadView, view: BoardView<'_>, hidden: &BTreeSet<String>) -> bool {
    match &pad.copper_layers {
        Some(CopperLayers::Resolved { layers, .. }) => layers.iter().any(|layer| !hidden.contains(&layer.to_string())),
        Some(CopperLayers::Unconfigured { .. }) => true,
        None => view
            .setup()
            .is_none_or(|setup| setup.layers().iter().any(|layer| layer.purpose == LayerPurpose::Copper && !hidden.contains(&layer.id.to_string()))),
    }
}

/// The net under a screen point: a visible pad's net, else the cached net of a track or via there.
fn net_under(
    pointer: egui::Pos2,
    camera: &Camera,
    rect: egui::Rect,
    view: BoardView<'_>,
    hidden: &BTreeSet<String>,
    nets: &BTreeMap<uuid::Uuid, String>,
) -> Option<String> {
    view.footprints()
        .flat_map(|footprint| footprint.pads)
        .filter(|pad| pad_visible(pad, view, hidden))
        .find(|pad| pad.copper.is_some_and(|land| egui::Rect::from_points(&land.map(|corner| camera.screen(corner, rect))).contains(pointer)))
        .and_then(|pad| pad.net)
        .or_else(|| {
            crate::board_tracks::copper_at(pointer, camera, rect, view, hidden, crate::board_tracks::Kinds::ALL).and_then(|id| nets.get(&id).cloned())
        })
}

/// Resolve the visible reference hit area, excluding pad, body and fixed silkscreen geometry.
#[allow(clippy::too_many_arguments)]
fn reference_bounds(
    painter: &egui::Painter,
    camera: &Camera,
    rect: Rect,
    project: BoardView<'_>,
    state: &BoardState,
    footprint: &FootprintView<'_>,
    bounds: Rect,
    appearance: &crate::board_appearance::Appearance,
) -> Rect {
    if !footprint.physical_reference {
        return if bounds.is_finite() {
            board::footprint_reference_bounds(painter, bounds, footprint.reference, reference_shift(footprint, camera, rect))
        } else {
            Rect::NOTHING
        };
    }
    let side = if footprint.bottom { Side::Back } else { Side::Front };
    if appearance.technical(project, LayerPurpose::Silkscreen, side, state.active_layer.as_deref(), &state.hidden_layers).is_none() {
        return Rect::NOTHING;
    }
    footprint.graphics.iter().filter(|graphic| graphic.reference).fold(Rect::NOTHING, |bounds, graphic| {
        let extent = graphic.points.iter().fold(Rect::NOTHING, |extent, point| extent.union(Rect::from_pos(camera.screen(*point, rect))));
        let half_stroke = graphic.stroke_nm.unwrap_or_default() as f32 / pcbcraft_engine::schematic::GRID_NM as f32 * camera.unit_step() / 2.0;
        bounds.union(extent.expand(half_stroke + 2.0))
    })
}

/// Project a local reference displacement into the editor-only label's screen coordinates.
fn reference_shift(footprint: &FootprintView<'_>, camera: &Camera, rect: Rect) -> Vec2 {
    let local = footprint.reference_offset;
    let (x_nm, y_nm) = repose(local.x_nm, local.y_nm, (0, false), (footprint.rotation, footprint.bottom));
    let origin = pcbcraft_engine::board::DEFAULT_REFERENCE_OFFSET;
    camera.screen(Point { x_nm, y_nm }, rect) - camera.screen(origin, rect)
}

/// Capture a visible reference on mouse-down or M, keeping the grab point stable under mirroring
/// and rotation. A mouse move commits only after a real drag; plain clicks discard the draft.
/// Completion requires a canvas release or click with the original edit authority.
fn move_reference(
    response: &Response,
    camera: &Camera,
    state: &mut BoardState,
    source: Option<ViewInput>,
    start: Option<(Pos2, &FootprintView<'_>)>,
    dragging: bool,
    rotation: u16,
) -> (bool, Option<ReferenceDrag>) {
    let rect = response.rect;
    let mut draft = if let Some(draft) = state.reference_drag.take() {
        if Some(draft.source) != source || response.ctx.input(|input| input.pointer.button_pressed(PointerButton::Secondary)) {
            return (true, None);
        }
        draft
    } else if let (Some(source), Some((pointer, footprint))) = (source, start) {
        state.move_requested = false;
        let mut draft = ReferenceDrag::new(source, footprint, camera.unsnapped(pointer, rect), dragging);
        if footprint.physical_reference {
            draft.rotate_text(rotation);
        }
        state.reference_drag = Some(draft);
        return (true, None);
    } else {
        return (false, None);
    };
    draft.rotate_text(rotation);
    let complete = if draft.dragging { response.drag_stopped_by(PointerButton::Primary) } else { response.clicked() };
    let pointer = if complete || draft.dragging { response.interact_pointer_pos().or_else(|| response.hover_pos()) } else { response.hover_pos() };
    if let Some(pointer) = pointer.filter(|pointer| rect.contains(*pointer)) {
        let at = camera.unsnapped(pointer, rect);
        let (x, y) = repose(at.x_nm - draft.anchor.x_nm, at.y_nm - draft.anchor.y_nm, (draft.rotation, draft.bottom), (0, false));
        let limit = MAX_REFERENCE_OFFSET_NM;
        draft.offset = Point { x_nm: (draft.original.x_nm + x).clamp(-limit, limit), y_nm: (draft.original.y_nm + y).clamp(-limit, limit) };
    }
    if complete {
        return (
            true,
            (response.contains_pointer() && (draft.offset != draft.original || draft.appearance != draft.original_appearance)).then_some(draft),
        );
    }
    if !draft.dragging || response.ctx.input(|input| input.pointer.primary_down()) {
        state.reference_drag = Some(draft);
    }
    (true, None)
}

/// Transform resolved pad offsets only for painting; pad identities and authored snapshots stay intact.
fn preview(footprint: &mut FootprintView<'_>, draft: &Draft) {
    let transform = |point| preview_point(point, footprint.position, footprint.rotation, footprint.bottom, draft);
    for pad in &mut footprint.pads {
        pad.center = transform(pad.center);
        pad.copper = pad.copper.map(|points| points.map(transform));
    }
    for graphic in &mut footprint.graphics {
        graphic.points = graphic.points.iter().copied().map(transform).collect();
        if draft.bottom != footprint.bottom {
            graphic.side = if draft.bottom { Side::Back } else { Side::Front };
        }
    }
    footprint.position = draft.position;
    footprint.rotation = draft.rotation;
    footprint.bottom = draft.bottom;
}

/// Match the footprint's disposable move/rotation/flip preview without changing pad identity.
/// Unchanged orientation keeps resolved offsets exact; pose changes use whole-degree geometry.
fn preview_point(point: Point, origin: Point, rotation: u16, bottom: bool, draft: &Draft) -> Point {
    let (x, y) = repose(point.x_nm - origin.x_nm, point.y_nm - origin.y_nm, (rotation, bottom), (draft.rotation, draft.bottom));
    Point { x_nm: draft.position.x_nm.saturating_add(x), y_nm: draft.position.y_nm.saturating_add(y) }
}

/// Move a resolved offset between validated whole-degree poses (rotation, bottom). Equal poses
/// preserve exact offsets. Same-side changes rotate by the angle difference; a side change
/// reflects X then rotates by the angle sum, applying the engine's rounding only once.
fn repose(x: i64, y: i64, from: (u16, bool), to: (u16, bool)) -> (i64, i64) {
    if from == to {
        return (x, y);
    }
    let mirrored = from.1 != to.1;
    let degrees = if mirrored { (from.0 + to.0) % 360 } else { (to.0 + 360 - from.0) % 360 };
    rotate_board(if mirrored { -x } else { x }, y, degrees)
}

/// Project physical geometry without deriving dimensions or copper membership from UI state.
/// Engine memberships drive pad color/visibility; technical graphics retain their layer-role visibility.
fn drawing(
    footprint: &FootprintView<'_>,
    camera: &Camera,
    rect: egui::Rect,
    project: BoardView<'_>,
    state: &BoardState,
    legacy_visible: bool,
    appearance: &crate::board_appearance::Appearance,
) -> (Vec<board::PadDrawing>, Vec<board::GraphicDrawing>) {
    let pads = footprint
        .pads
        .iter()
        .filter_map(|pad| {
            let color = pad
                .copper_layers
                .as_ref()
                .and_then(|membership| appearance.copper(project, membership, state.active_layer.as_deref(), &state.hidden_layers));
            if (pad.copper.is_some() && color.is_none()) || (pad.copper.is_none() && !legacy_visible) {
                return None;
            }
            Some(board::PadDrawing {
                number: pad.number.clone(),
                center: camera.screen(pad.center, rect),
                copper: pad.copper.map(|points| points.map(|p| camera.screen(p, rect))),
                color,
                contour: match pad.shape {
                    pcbcraft_engine::board::definition::PadShape::Rectangle => board::PadContour::Rectangle,
                    pcbcraft_engine::board::definition::PadShape::RoundedRectangle => board::PadContour::RoundedRectangle {
                        radius: (camera.screen(Point { x_nm: pad.center.x_nm + pad.corner_radius_nm, y_nm: pad.center.y_nm }, rect)
                            - camera.screen(pad.center, rect))
                        .length(),
                    },
                    pcbcraft_engine::board::definition::PadShape::Circle => board::PadContour::Circle,
                    pcbcraft_engine::board::definition::PadShape::Oval => board::PadContour::Oval,
                },
                drill_radius: pad.drill_nm.map(|diameter| {
                    (camera.screen(Point { x_nm: pad.center.x_nm + diameter, y_nm: pad.center.y_nm }, rect) - camera.screen(pad.center, rect))
                        .length()
                        / 2.
                }),
                through_hole: pad.through_hole,
                net: pad.net.clone(),
            })
        })
        .collect();
    let graphics = footprint
        .graphics
        .iter()
        .filter_map(|graphic| {
            use pcbcraft_engine::board::definition::GraphicPurpose;
            let (purpose, style) = match graphic.purpose {
                GraphicPurpose::Fabrication => (LayerPurpose::Fabrication, board::GraphicStyle::Fabrication),
                GraphicPurpose::Silkscreen => (LayerPurpose::Silkscreen, board::GraphicStyle::Silkscreen),
                GraphicPurpose::Courtyard => (LayerPurpose::Courtyard, board::GraphicStyle::Courtyard),
            };
            appearance.technical(project, purpose, graphic.side, state.active_layer.as_deref(), &state.hidden_layers).map(|color| {
                board::GraphicDrawing {
                    points: graphic.points.iter().map(|p| camera.screen(*p, rect)).collect(),
                    style,
                    color: Some(color),
                    closed: graphic.closed,
                    reference: graphic.reference,
                    stroke_width: graphic.stroke_nm.map(|width| {
                        (camera.screen(Point { x_nm: width, y_nm: 0 }, rect) - camera.screen(Point { x_nm: 0, y_nm: 0 }, rect)).length()
                    }),
                }
            })
        })
        .collect();
    (pads, graphics)
}

/// Clip an actual segment against a rectangle; an empty corner of a diagonal's bounds is not a hit.
fn segment_in_box(a: egui::Pos2, b: egui::Pos2, window: egui::Rect) -> bool {
    let (mut enter, mut leave) = (0_f32, 1_f32);
    for (start, end, min, max) in [(a.x, b.x, window.left(), window.right()), (a.y, b.y, window.top(), window.bottom())] {
        let delta = end - start;
        if delta == 0. {
            if start < min || start > max {
                return false;
            }
        } else {
            let (x, y) = ((min - start) / delta, (max - start) / delta);
            enter = enter.max(x.min(y));
            leave = leave.min(x.max(y));
            if enter > leave {
                return false;
            }
        }
    }
    true
}

/// Copper and graphic marquee hits follow physical strokes and via circles, respecting hidden layers
/// and category filters. A containment box requires the complete object; crossing needs one contact.
#[allow(clippy::too_many_arguments)]
fn geometry_for_box(
    project: BoardView<'_>,
    camera: &Camera,
    rect: egui::Rect,
    state: &BoardState,
    filters: crate::selection_filters::Filters,
    window: egui::Rect,
    crossing: bool,
) -> pcbcraft_engine::board::clipboard::BoardSelection {
    use pcbcraft_engine::board::clipboard::BoardSelection;
    let radius = |nm| (camera.screen(Point { x_nm: nm, y_nm: 0 }, rect) - camera.screen(Point { x_nm: 0, y_nm: 0 }, rect)).length();
    let stroke = |points: Vec<egui::Pos2>, closed: bool, width: f32| {
        if crossing {
            let expanded = window.expand(width / 2.);
            let count = points.len().saturating_sub(usize::from(!closed));
            points.iter().copied().zip(points.iter().copied().cycle().skip(1)).take(count).any(|(a, b)| segment_in_box(a, b, expanded))
        } else {
            let inset = window.shrink(width / 2.);
            inset.is_positive() && points.iter().all(|p| inset.contains(*p))
        }
    };
    let drawings = project
        .drawings()
        .into_iter()
        .filter(|d| filters.graphics && !state.hidden_layers.contains(&d.layer.to_string()))
        .filter(|d| stroke(d.points.iter().map(|p| camera.screen(*p, rect)).collect(), d.closed, d.stroke_nm.map_or(0., radius)))
        .map(|d| d.id)
        .collect();
    let tracks = project
        .tracks()
        .iter()
        .filter(|t| filters.tracks && !state.hidden_layers.contains(&t.layer.to_string()))
        .filter(|t| stroke(vec![camera.screen(t.start, rect), camera.screen(t.end, rect)], false, radius(t.width_nm)))
        .map(|t| t.id)
        .collect();
    let visible_copper = project.layers().iter().any(|l| l.purpose == LayerPurpose::Copper && !state.hidden_layers.contains(&l.id.to_string()));
    let vias = project
        .vias()
        .iter()
        .filter(|_| filters.vias && visible_copper)
        .filter(|v| {
            let center = camera.screen(v.at, rect);
            let radius = radius(v.diameter_nm / 2);
            if crossing {
                window.clamp(center).distance(center) <= radius
            } else {
                window.contains_rect(egui::Rect::from_center_size(center, egui::Vec2::splat(radius * 2.)))
            }
        })
        .map(|v| v.id)
        .collect();
    BoardSelection { drawings, tracks, vias, ..Default::default() }
}

/// Select every visible supported physical object through the same geometry/category policy as
/// a marquee, without requiring a schematic camera or changing the project.
pub(super) fn all_visible(
    project: BoardView<'_>,
    state: &BoardState,
    filters: crate::selection_filters::Filters,
    appearance: &crate::board_appearance::Appearance,
) -> pcbcraft_engine::board::clipboard::BoardSelection {
    let camera = Camera::default();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100., 100.));
    let visible = project
        .setup()
        .is_none_or(|s| s.layers().iter().any(|l| l.purpose == LayerPurpose::Copper && !state.hidden_layers.contains(&l.id.to_string())));
    let mut selected = geometry_for_box(project, &camera, rect, state, filters, egui::Rect::EVERYTHING, false);
    if filters.footprints {
        selected.footprints = project
            .footprints()
            .filter(|f| {
                let (pads, graphics) = drawing(f, &camera, rect, project, state, visible, appearance);
                !pads.is_empty() || !graphics.is_empty()
            })
            .map(|f| f.id)
            .collect();
    }
    selected
}

/// Modifier boxes combine each category independently against the original mixed selection.
fn combine_box(
    base: &pcbcraft_engine::board::clipboard::BoardSelection,
    hits: pcbcraft_engine::board::clipboard::BoardSelection,
    add: bool,
    toggle: bool,
) -> pcbcraft_engine::board::clipboard::BoardSelection {
    /// Combine one ordered category with toggle taking precedence over additive selection.
    fn combine<T: Ord + Clone>(base: &BTreeSet<T>, hits: BTreeSet<T>, add: bool, toggle: bool) -> BTreeSet<T> {
        let mut selection = if add || toggle { base.clone() } else { BTreeSet::new() };
        for id in hits {
            if !toggle || !selection.remove(&id) {
                selection.insert(id);
            }
        }
        selection
    }
    pcbcraft_engine::board::clipboard::BoardSelection {
        footprints: combine(&base.footprints, hits.footprints, add, toggle),
        drawings: combine(&base.drawings, hits.drawings, add, toggle),
        tracks: combine(&base.tracks, hits.tracks, add, toggle),
        vias: combine(&base.vias, hits.vias, add, toggle),
    }
}

#[cfg(test)]
mod clipboard_geometry_tests {
    use super::*;
    /// A diagonal crossing is a stroke hit only where the segment actually enters the window.
    #[test]
    fn diagonal_corner_and_exact_contact_are_distinct() {
        let a = egui::pos2(0., 0.);
        let b = egui::pos2(10., 10.);
        assert!(!segment_in_box(a, b, egui::Rect::from_min_max(egui::pos2(0., 8.), egui::pos2(2., 10.))));
        assert!(segment_in_box(a, b, egui::Rect::from_min_max(egui::pos2(4., 4.), egui::pos2(6., 6.))));
        assert!(segment_in_box(a, b, egui::Rect::from_min_max(egui::pos2(10., 10.), egui::pos2(12., 12.))));
        assert!(!segment_in_box(egui::pos2(0., 0.), egui::pos2(0., 10.), egui::Rect::from_min_max(egui::pos2(1., 0.), egui::pos2(2., 10.))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Crossing includes the visible edge of a physical graphic, while enclosure requires
    /// its full width rather than only its centerline.
    #[test]
    fn graphic_marquee_accounts_for_physical_stroke() {
        let mut h = crate::tests::harness();
        h.state_mut().run("project.new", &serde_json::json!({"create_board":true}));
        h.run();
        let view = h.state().session().project().unwrap().boards().next().unwrap();
        let layer = view.layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).unwrap().id;
        let revision = h.state().session().revision();
        h.state_mut().run(pcbcraft_engine::ADD_DRAWING, &serde_json::json!({"layer":layer,"points":[{"x_nm":0,"y_nm":0},{"x_nm":10_000_000,"y_nm":0}],"closed":false,"stroke_nm":2_540_000,"expected_revision":revision}));
        let view = h.state().session().project().unwrap().boards().next().unwrap();
        let camera = &h.state().views.active.camera;
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000., 700.));
        let a = camera.screen(Point { x_nm: 0, y_nm: 0 }, rect);
        let b = camera.screen(Point { x_nm: 10_000_000, y_nm: 0 }, rect);
        let half = 2_540_000. * camera.unit_step() / pcbcraft_engine::schematic::GRID_NM as f32 / 2.;
        let state = BoardState::default();
        let filters = crate::selection_filters::Filters::default();
        let crossing = egui::Rect::from_min_max(egui::pos2(a.x + 1., a.y + half - 1.), egui::pos2(b.x - 1., a.y + half + 1.));
        assert_eq!(geometry_for_box(view, camera, rect, &state, filters, crossing, true).drawings.len(), 1);
        let narrow = egui::Rect::from_min_max(a - egui::vec2(half + 1., half - 1.), b + egui::vec2(half + 1., half - 1.));
        assert!(geometry_for_box(view, camera, rect, &state, filters, narrow, false).drawings.is_empty());
        assert_eq!(geometry_for_box(view, camera, rect, &state, filters, narrow.expand(2.), false).drawings.len(), 1);
    }

    /// Whole-degree source poses move without changing their resolved offsets; side changes and
    /// rotations agree with canonical engine geometry within its nanometer rounding precision.
    #[test]
    fn repose_preserves_whole_degree_poses() {
        let local = (3_345_678, 1_234_567);
        for from_angle in [0, 17, 45, 127, 359] {
            for from_bottom in [false, true] {
                let point = rotate_board(if from_bottom { -local.0 } else { local.0 }, local.1, from_angle);
                let from = (from_angle, from_bottom);
                assert_eq!(repose(point.0, point.1, from, from), point, "unchanged pose {from:?}");
                for to_angle in [from_angle, (from_angle + 90) % 360, (from_angle + 37) % 360] {
                    for to_bottom in [false, true] {
                        let to = (to_angle, to_bottom);
                        let expected = rotate_board(if to_bottom { -local.0 } else { local.0 }, local.1, to_angle);
                        let actual = repose(point.0, point.1, from, to);
                        assert!(
                            (actual.0 - expected.0).abs() <= 1 && (actual.1 - expected.1).abs() <= 1,
                            "{from:?} -> {to:?}: {actual:?} vs {expected:?}"
                        );
                    }
                }
                assert_eq!(repose(point.0, point.1, from, ((from_angle + 90) % 360, from_bottom)), rotate(point.0, point.1, 1));
            }
        }
    }

    /// A side change mirrors local X about the origin; same-side moves match one combined turn.
    #[test]
    fn repose_mirrors_on_flip_and_composes_turns() {
        assert_eq!(repose(3, 1, (0, false), (0, true)), (-3, 1));
        assert_eq!(repose(3, 1, (0, false), (90, false)), rotate(3, 1, 1));
        assert_eq!(repose(3, 1, (90, false), (180, false)), rotate(3, 1, 1));
        let there = repose(3, 1, (90, false), (90, true));
        assert_eq!(repose(there.0, there.1, (90, true), (90, false)), (3, 1), "flipping back restores the offset");
    }
}
