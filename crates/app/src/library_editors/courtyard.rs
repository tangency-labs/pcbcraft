//! Courtyard outline generation dialog, vertex-handle canvas helpers and properties panel.
use super::*;
use egui::{Painter, Pos2, Rect, Stroke, vec2};
use pcbcraft_chrome::{Tokens, controls, properties, shell, units};
use pcbcraft_engine::{
    authoring::courtyard::{self, CourtyardSource},
    schematic::Point,
};

/// Screen-space radius of interactive courtyard vertex handles.
pub(super) const VERTEX_HANDLE_RADIUS: f32 = 4.5;
/// Pointer pick radius for grabbing or selecting a courtyard vertex.
pub(super) const VERTEX_PICK_RADIUS: f32 = 8.0;
/// Screen-space radius of edge midpoint handles used to insert a new vertex.
pub(super) const MIDPOINT_HANDLE_RADIUS: f32 = 3.0;
/// Pointer pick radius for clicking an edge midpoint handle.
pub(super) const MIDPOINT_PICK_RADIUS: f32 = 7.0;

/// IPC-7351 Density Level C / B / A courtyard clearance presets (Table 3-13).
const CLEARANCE_PRESETS: [(&str, &str, &str); 3] = [
    ("0.10", "0.10 mm", "Least density (IPC-7351 Level C: 0.10 mm)"),
    ("0.25", "0.25 mm", "Nominal density (IPC-7351 Level B: 0.25 mm)"),
    ("0.50", "0.50 mm", "Most density (IPC-7351 Level A: 0.50 mm)"),
];

/// Revision-bound draft for the Generate Courtyard modal dialog.
#[derive(Debug)]
pub(crate) struct CourtyardDraft {
    document: u64,
    view: crate::ViewId,
    revision: u64,
    source: String,
    clearance: String,
    stroke: String,
    replace_existing: bool,
    has_existing: bool,
    error: Option<String>,
}

impl CourtyardDraft {
    /// Capture the active footprint document, pane and revision so background changes cannot
    /// misdirect the generation transaction.
    pub(super) fn new(document: u64, view: crate::ViewId, doc: &Document) -> Self {
        let item = doc.session.snapshot();
        let has_existing = item.objects.values().any(courtyard::is_courtyard_primitive);
        Self {
            document,
            view,
            revision: doc.session.revision(),
            source: "pads_and_body".into(),
            clearance: format_mm(courtyard::DEFAULT_CLEARANCE_NM),
            stroke: format_mm(courtyard::DEFAULT_STROKE_NM),
            replace_existing: true,
            has_existing,
            error: None,
        }
    }

    /// Resolve the typed source, clearance and stroke fields into a validated authoring action.
    pub(super) fn parse_action(&self) -> Result<Action, String> {
        let source = match self.source.as_str() {
            "pads_and_body" => CourtyardSource::PadsAndBody,
            "pads_only" => CourtyardSource::PadsOnly,
            _ => return Err("Choose Pads and body or Pads only.".into()),
        };
        let clearance_nm = units::parse_length(&self.clearance, units::Unit::Mm).map_err(|e| format!("Clearance (mm): {e}"))?;
        if !(0..=courtyard::MAX_CLEARANCE_NM).contains(&clearance_nm) {
            return Err("Clearance (mm) must be between 0 and 10 mm.".into());
        }
        let stroke_nm = units::parse_length(&self.stroke, units::Unit::Mm).map_err(|e| format!("Stroke (mm): {e}"))?;
        courtyard::validate_stroke(stroke_nm)?;
        Ok(Action::GenerateCourtyard { clearance_nm, source, stroke_nm, replace_existing: self.replace_existing })
    }

    /// Compute a live human-readable size summary from the current form values without mutating
    /// the document.
    fn preview_summary(&self, item: &Item) -> Result<String, String> {
        let Action::GenerateCourtyard { clearance_nm, source, stroke_nm, .. } = self.parse_action()? else {
            return Err("Invalid courtyard settings.".into());
        };
        let primitive = courtyard::generate(item, clearance_nm, source, stroke_nm)?;
        let (points, _) = courtyard::courtyard_polygon(&primitive).ok_or("Generated courtyard is unavailable.")?;
        let m = courtyard::metrics(points).ok_or("Generated courtyard has no extents.")?;
        Ok(format!("Result: {} × {} mm · {} vertices", format_mm(m.width_nm), format_mm(m.height_nm), m.vertex_count))
    }
}

/// Format a nanometer length as millimeters with trailing zeros trimmed to at least 2 decimals
/// for compact display.
pub(super) fn format_mm(nm: i64) -> String {
    let text = format!("{:.3}", nm as f64 / 1_000_000.0);
    let trimmed = text.trim_end_matches('0');
    if let Some(dot) = trimmed.find('.') {
        let frac = trimmed.len().saturating_sub(dot + 1);
        if frac < 2 {
            return format!("{trimmed}{}", "0".repeat(2 - frac));
        }
    }
    trimmed.to_owned()
}

/// Find the closest vertex handle of `points` within [`VERTEX_PICK_RADIUS`] of `pointer`.
pub(super) fn pick_vertex(points: &[Point], camera: &crate::canvas_camera::Camera, area: Rect, pointer: Pos2) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (index, &point) in points.iter().enumerate() {
        let dist = pointer.distance(camera.screen(point, area));
        if dist <= VERTEX_PICK_RADIUS && best.is_none_or(|(_, prev)| dist < prev) {
            best = Some((index, dist));
        }
    }
    best.map(|(index, _)| index)
}

/// Find the closest edge midpoint handle of `points` within [`MIDPOINT_PICK_RADIUS`] of `pointer`.
pub(super) fn pick_edge_midpoint(points: &[Point], camera: &crate::canvas_camera::Camera, area: Rect, pointer: Pos2) -> Option<usize> {
    if points.len() < courtyard::MIN_VERTICES {
        return None;
    }
    let mut best: Option<(usize, f32)> = None;
    for edge in 0..points.len() {
        let Some(mid) = courtyard::edge_midpoint(points, edge) else { continue };
        let dist = pointer.distance(camera.screen(mid, area));
        if dist <= MIDPOINT_PICK_RADIUS && best.is_none_or(|(_, prev)| dist < prev) {
            best = Some((edge, dist));
        }
    }
    best.map(|(edge, _)| edge)
}

/// Paint interactive vertex handles and edge midpoint insertion markers on the selected
/// courtyard outline.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_handles(
    painter: &Painter,
    points: &[Point],
    selected_vertex: Option<usize>,
    hovered_vertex: Option<usize>,
    hovered_edge: Option<usize>,
    camera: &crate::canvas_camera::Camera,
    area: Rect,
    can_insert: bool,
) {
    let t = Tokens::get();
    if can_insert {
        for edge in 0..points.len() {
            let Some(mid) = courtyard::edge_midpoint(points, edge) else { continue };
            let screen = camera.screen(mid, area);
            let active = hovered_edge == Some(edge);
            let r = if active { MIDPOINT_HANDLE_RADIUS + 1.0 } else { MIDPOINT_HANDLE_RADIUS };
            let fill = if active { t.accent } else { t.board_canvas };
            let stroke = Stroke::new(1.0, if active { t.accent } else { t.icon_active_2 });
            painter.circle(screen, r, fill, stroke);
        }
    }
    for (index, &point) in points.iter().enumerate() {
        let screen = camera.screen(point, area);
        let active = selected_vertex == Some(index) || hovered_vertex == Some(index);
        let r = if active { VERTEX_HANDLE_RADIUS + 1.0 } else { VERTEX_HANDLE_RADIUS };
        let rect = Rect::from_center_size(screen, vec2(r * 2.0, r * 2.0));
        let fill = if active { t.accent } else { t.board_canvas };
        let stroke = Stroke::new(1.5, if active { t.accent } else { t.icon_active_2 });
        painter.rect(rect, 1.0, fill, stroke, egui::StrokeKind::Middle);
    }
}

/// Paint a live preview while drawing a new closed courtyard outline point-by-point.
pub(super) fn paint_draw_preview(painter: &Painter, points: &[Point], cursor: Point, camera: &crate::canvas_camera::Camera, area: Rect) {
    let t = Tokens::get();
    let stroke = Stroke::new(1.5, t.icon_active_2);
    let mut screen_pts: Vec<Pos2> = points.iter().map(|p| camera.screen(*p, area)).collect();
    let cursor_pos = camera.screen(cursor, area);
    if points.last() != Some(&cursor) {
        screen_pts.push(cursor_pos);
    }
    if screen_pts.len() >= 2 {
        painter.add(egui::Shape::line(screen_pts.clone(), stroke));
    }
    if screen_pts.len() >= 3
        && let (Some(&first), Some(&last)) = (screen_pts.first(), screen_pts.last())
    {
        painter.line_segment([last, first], Stroke::new(1.0, t.icon_active_2.gamma_multiply(0.55)));
    }
    for (index, &pos) in screen_pts.iter().enumerate() {
        let closing = index == 0 && points.len() >= courtyard::MIN_VERTICES && points.first() == Some(&cursor);
        let r = if closing { VERTEX_HANDLE_RADIUS + 1.5 } else { VERTEX_HANDLE_RADIUS - 0.5 };
        painter.circle(pos, r, if closing { t.accent } else { t.board_canvas }, stroke);
    }
}

impl Document {
    /// Return the first closed courtyard polygon primitive in this footprint, if any.
    pub(super) fn first_courtyard_id(&self) -> Option<u64> {
        let item = self.session.snapshot();
        item.objects
            .iter()
            .find(|(_, primitive)| courtyard::courtyard_polygon(primitive).is_some())
            .or_else(|| item.objects.iter().find(|(_, primitive)| courtyard::is_courtyard_primitive(primitive)))
            .map(|(id, _)| *id)
    }

    /// Return the selected object's ID and courtyard polygon vertices when a single closed
    /// courtyard outline is selected.
    pub(super) fn selected_courtyard<'a>(&self, item: &'a Item) -> Option<(u64, &'a [Point], i64)> {
        if self.selection.len() != 1 {
            return None;
        }
        let id = self.selected?;
        let primitive = item.objects.get(&id)?;
        let (points, stroke_nm) = courtyard::courtyard_polygon(primitive)?;
        Some((id, points, stroke_nm))
    }

    /// Toggle courtyard layer visibility without mutating the document. Hiding the layer
    /// deselects any selected courtyard objects (refusing if buffered edits are pending) and
    /// cancels any active courtyard draw or vertex-drag tool.
    pub(super) fn set_show_courtyard(&mut self, show: bool) {
        if show == self.show_courtyard {
            return;
        }
        if !show {
            let item = self.session.snapshot();
            let has_selected_courtyard = self.selection.iter().any(|id| item.objects.get(id).is_some_and(courtyard::is_courtyard_primitive));
            if has_selected_courtyard {
                let remaining =
                    self.selection.iter().copied().filter(|id| !item.objects.get(id).is_some_and(courtyard::is_courtyard_primitive)).collect();
                if let Err(error) = self.select_many(remaining) {
                    self.error = Some(error);
                    return;
                }
            }
            if matches!(
                self.tool,
                canvas::Tool::Courtyard { .. } | canvas::Tool::DragCourtyardVertex { .. } | canvas::Tool::Draw { kind: canvas::Place::Courtyard, .. }
            ) {
                self.tool = canvas::Tool::Select;
            }
        }
        self.show_courtyard = show;
    }

    /// Render the compact Courtyard summary section inside Footprint Definition properties
    /// (when no individual primitive is selected).
    pub(super) fn courtyard_summary_section(&mut self, ui: &mut Ui) {
        let item = self.session.snapshot();
        let courtyard_ids: Vec<u64> = item.objects.iter().filter(|(_, p)| courtyard::is_courtyard_primitive(p)).map(|(id, _)| *id).collect();
        properties::section(ui, "Courtyard");
        if let Some(first_id) = self.first_courtyard_id() {
            let count = courtyard_ids.len();
            let outline_word = if count == 1 { "outline" } else { "outlines" };
            let shape_word = if count == 1 { "shape" } else { "shapes" };
            let summary = match item.objects.get(&first_id).and_then(courtyard::courtyard_polygon) {
                Some((points, _)) => match courtyard::metrics(points) {
                    // With several outlines the size and vertices are those of the first one,
                    // the outline "Select courtyard" selects, and the label says so.
                    Some(m) => {
                        let size = format!("{} × {} mm · {} vertices", format_mm(m.width_nm), format_mm(m.height_nm), m.vertex_count);
                        if count == 1 { format!("1 outline · {size}") } else { format!("{count} outlines · first of {count}: {size}") }
                    }
                    None => format!("{count} courtyard {outline_word}"),
                },
                None => format!("{count} courtyard {shape_word}"),
            };
            ui.label(shell::dim(summary));
        } else {
            ui.label(shell::dim("No courtyard outline"));
        }
        ui.horizontal(|ui| {
            if controls::enabled_button(ui, !self.pending(), "Generate courtyard…").clicked() {
                self.courtyard_requested = true;
            }
            if let Some(first_id) = self.first_courtyard_id()
                && controls::enabled_button(ui, !self.pending(), "Select courtyard").clicked()
            {
                self.set_show_courtyard(true);
                let _ = self.select(Some(first_id));
            }
        });
    }

    /// Render the dedicated Courtyard outline properties inspector when a closed courtyard
    /// polygon is selected. Returns `true` when it rendered the panel.
    pub(super) fn courtyard_outline_inspector(&mut self, ui: &mut Ui, object_id: u64, expanded: bool) -> bool {
        let snapshot = self.session.snapshot();
        let Some(primitive) = snapshot.objects.get(&object_id) else {
            return false;
        };
        let Some((points, _)) = courtyard::courtyard_polygon(primitive) else {
            return false;
        };
        let count = points.len();
        let active_vertex = self.selected_vertex.filter(|&i| i < count);
        self.selected_vertex = active_vertex;

        if self.draft.as_ref().is_some_and(|draft| draft.revision != self.session.revision()) {
            if self.pending() {
                self.error = Some("The library document changed; reset these fields before applying new edits.".into());
            } else {
                self.draft = None;
                self.error = Some("The library document changed; property fields were refreshed.".into());
            }
        }
        properties::identity(ui, "Courtyard outline", &snapshot.name);
        if let Some(error) = &self.error {
            ui.label(shell::error(error));
        }
        if let Some(m) = courtyard::metrics(points) {
            ui.label(shell::dim(format!("{} × {} mm · {} vertices", format_mm(m.width_nm), format_mm(m.height_nm), m.vertex_count)));
            if !m.is_simple {
                ui.label(shell::error("Outline intersects itself; move a vertex to fix."));
            }
        }
        ui.add_space(Tokens::get().space_4);

        let mut chosen = active_vertex.map(|i| i.to_string()).unwrap_or_default();
        let mut options: Vec<(String, String)> = Vec::with_capacity(count + 1);
        options.push((String::new(), "No vertex selected".into()));
        options.extend((0..count).map(|i| (i.to_string(), format!("Vertex {}", i + 1))));
        let option_refs: Vec<(&str, String)> = options.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        let _ = self.draft.get_or_insert_with(|| form::Draft::new(&snapshot, Some(object_id), self.session.revision()));

        let mut enter_apply = false;
        let mut reset_clicked = false;
        let mut add_vertex = false;
        let mut remove_vertex = false;
        let mut regenerate = false;

        if let Some(draft) = self.draft.as_mut() {
            enter_apply |= draft.show_courtyard_stroke(ui);
        }

        let pending = self.pending();
        ui.add_enabled_ui(!pending, |ui| {
            properties::choice(ui, "Vertex", egui::Id::unique(("courtyard_vertex", object_id)), &mut chosen, &option_refs);
        });
        if !pending {
            if chosen.is_empty() {
                self.selected_vertex = None;
            } else if let Ok(parsed) = chosen.parse::<usize>()
                && parsed < count
            {
                self.selected_vertex = Some(parsed);
            }
        }
        let selected_vertex = self.selected_vertex.filter(|&i| i < count);

        let apply_payload = if let Some(draft) = self.draft.as_mut() {
            if expanded {
                properties::section(ui, "All vertices");
                for i in 0..count {
                    if draft.show_courtyard_vertex_row(ui, i) {
                        enter_apply = true;
                    }
                }
            } else if let Some(vertex_index) = selected_vertex {
                if draft.show_courtyard_vertex(ui, vertex_index) {
                    enter_apply = true;
                }
            } else {
                ui.label(shell::dim("Click a vertex handle or choose one above to edit coordinates."));
            }
            let dirty = draft.dirty();
            let mut apply_clicked = false;
            ui.horizontal(|ui| {
                add_vertex = controls::enabled_button(ui, !dirty && selected_vertex.is_some() && count < courtyard::MAX_VERTICES, "Add vertex")
                    .on_hover_text("Insert a vertex at the midpoint of the next edge, on the nanometre grid")
                    .clicked();
                remove_vertex = controls::enabled_button(ui, !dirty && selected_vertex.is_some() && count > courtyard::MIN_VERTICES, "Remove vertex")
                    .on_hover_text("Remove the selected vertex (Backspace)")
                    .clicked();
            });
            ui.horizontal(|ui| {
                regenerate = controls::enabled_button(ui, !dirty, "Regenerate…").clicked();
            });
            ui.horizontal(|ui| {
                apply_clicked = controls::enabled_button(ui, dirty, "Apply").clicked() || (enter_apply && dirty);
                reset_clicked = controls::button(ui, "Reset").clicked();
            });
            apply_clicked.then(|| (draft.revision, draft.action()))
        } else {
            None
        };

        if let Some((revision, action_result)) = apply_payload {
            let result = action_result.and_then(|action| self.session.execute(revision, action));
            match result {
                Ok(_) => {
                    self.draft = None;
                    self.tool = canvas::Tool::Select;
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        } else if reset_clicked {
            self.draft = None;
            self.error = None;
        } else if add_vertex && let Some(vertex_index) = selected_vertex {
            let result = courtyard::edge_midpoint(points, vertex_index)
                .ok_or_else(|| "Unable to compute edge midpoint.".to_owned())
                .and_then(|mid| courtyard::with_inserted_vertex(primitive, vertex_index, mid))
                .and_then(|candidate| self.execute(self.session.revision(), Action::Replace { object: object_id, primitive: candidate }));
            match result {
                Ok(()) => self.selected_vertex = Some(vertex_index + 1),
                Err(error) => self.error = Some(error),
            }
        } else if remove_vertex && let Some(vertex_index) = selected_vertex {
            match courtyard::with_removed_vertex(primitive, vertex_index)
                .and_then(|candidate| self.execute(self.session.revision(), Action::Replace { object: object_id, primitive: candidate }))
            {
                Ok(()) => {
                    let next_len = count.saturating_sub(1);
                    self.selected_vertex = (next_len > 0).then(|| vertex_index.min(next_len - 1));
                }
                Err(error) => self.error = Some(error),
            }
        } else if regenerate {
            self.courtyard_requested = true;
        }
        true
    }
}

impl PcbApp {
    /// Generate a closed courtyard polygon from the footprint's pads and optional body graphics.
    /// Applies to the captured definition/view/revision; Cancel or Escape leaves the document untouched.
    pub(crate) fn library_courtyard_dialog(&mut self, ctx: &Context, mut draft: Box<CourtyardDraft>) {
        let mut apply = false;
        let mut cancel = false;
        let dismissed = shell::dialog(ctx, "library_courtyard", "Generate Courtyard", |ui| {
            let prev_source = draft.source.clone();
            properties::choice(
                ui,
                "Source",
                egui::Id::unique("courtyard_source"),
                &mut draft.source,
                &[("pads_and_body", "Pads and body".into()), ("pads_only", "Pads only".into())],
            );
            if draft.source != prev_source {
                draft.error = None;
            }
            let clearance_resp = properties::text(ui, "Clearance (mm)", &mut draft.clearance);
            if clearance_resp.changed() {
                draft.error = None;
            }
            if clearance_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                apply = true;
            }
            ui.horizontal(|ui| {
                // Indent the presets under the Clearance value: 72 px is the shared properties
                // label column width used by `properties::field`/`choice`.
                ui.add_space(72.0 + ui.spacing().item_spacing.x);
                for (value, label, tip) in CLEARANCE_PRESETS {
                    if controls::small_button(ui, label).on_hover_text(tip).clicked() {
                        draft.clearance = value.into();
                        draft.error = None;
                    }
                }
            });
            let stroke_resp = properties::text(ui, "Stroke (mm)", &mut draft.stroke);
            if stroke_resp.changed() {
                draft.error = None;
            }
            if stroke_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                apply = true;
            }
            if draft.has_existing {
                let prev_replace = draft.replace_existing;
                controls::checkbox(ui, &mut draft.replace_existing, "Replace existing courtyard");
                if draft.replace_existing != prev_replace {
                    draft.error = None;
                }
            }
            let preview = self.library_editors.documents.get(&draft.document).map(|doc| draft.preview_summary(&doc.session.snapshot()));
            match (&draft.error, &preview) {
                (Some(error), _) => {
                    ui.label(shell::error(error));
                }
                (None, Some(Ok(summary))) => {
                    ui.label(shell::dim(summary));
                }
                (None, Some(Err(message))) => {
                    ui.label(shell::error(message));
                }
                (None, None) => {}
            }
            ui.horizontal(|ui| {
                apply |= controls::button(ui, "Generate").clicked();
                cancel = controls::button(ui, "Cancel").clicked();
            });
        });
        if apply {
            let result = draft.parse_action().and_then(|action| {
                if self.bind_library_view(draft.view) != Some(draft.document) {
                    return Err("The source definition pane is no longer open.".into());
                }
                self.library_editors.edit(draft.document, draft.revision, action)
            });
            if let Err(error) = result {
                if let Some(doc) = self.library_editors.documents.get(&draft.document)
                    && doc.session.revision() != draft.revision
                {
                    draft.revision = doc.session.revision();
                    draft.has_existing = doc.session.snapshot().objects.values().any(courtyard::is_courtyard_primitive);
                    draft.error = Some("The footprint changed; check the preview and press Generate again.".into());
                } else {
                    draft.error = Some(error);
                }
                self.dialog = Dialog::LibraryCourtyard(draft);
            } else {
                if let Some(doc) = self.library_editors.documents.get_mut(&draft.document) {
                    doc.show_courtyard = true;
                }
                self.repaint_editors(ctx);
            }
        } else if !(cancel || dismissed) {
            self.dialog = Dialog::LibraryCourtyard(draft);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };

    fn click_at(h: &mut Harness<'_, PcbApp>, pos: Pos2) {
        h.hover_at(pos);
        h.run();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run();
        }
    }

    /// Send two complete primary clicks within the double-click interval.
    fn double_click_at(h: &mut Harness<'_, PcbApp>, pos: Pos2) {
        h.hover_at(pos);
        h.run();
        for _ in 0..2 {
            for pressed in [true, false] {
                h.input_mut().time = Some(h.ctx.input(|i| i.time) + 0.03);
                h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                h.run_steps(1);
            }
        }
        h.input_mut().time = None;
        h.run();
    }

    /// Find the sole enabled node with `label` (distinguishing an active modal field from a
    /// disabled background inspector field of the same name).
    fn enabled_by_label<'a>(h: &'a Harness<'_, PcbApp>, label: &'a str) -> egui_kittest::Node<'a> {
        let mut nodes = h.query_all_by_label(label).filter(|node| !node.accesskit_node().is_disabled());
        let node = nodes.next().unwrap_or_else(|| panic!("no enabled node labelled {label:?}"));
        assert!(nodes.next().is_none(), "expected one enabled node labelled {label:?}");
        node
    }

    fn drag_at(h: &mut Harness<'_, PcbApp>, from: Pos2, to: Pos2) {
        h.hover_at(from);
        h.run();
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run();
        h.hover_at(to);
        h.run();
        h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run();
    }

    /// Zoom the definition view out by `factor` about its centre, as the user would with the
    /// wheel. Fit fills the usable canvas with a margin of only 28 px, so without this a
    /// courtyard generated after the first frame, a hand-drawn outline or a point outside an edge
    /// falls off the canvas or under a panel, where the pointer never reaches the canvas.
    fn zoom_out(h: &mut Harness<'_, PcbApp>, id: u64, factor: f32) {
        if let Some(doc) = h.state_mut().library_editors.documents.get_mut(&id) {
            let zoom = doc.camera.zoom();
            doc.camera.set_zoom(zoom / factor);
        }
        h.run();
    }

    /// A footprint editor whose default pads carry a generated 0.25 mm courtyard, selected with
    /// no vertex chosen, zoomed out to half the fitted size so points outside the outline stay on
    /// the canvas. Returns the harness, the document ID and the courtyard object ID.
    fn harness_with_generated_courtyard() -> (Harness<'static, PcbApp>, u64, u64) {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let id = app.library_editors.active.unwrap();
        app.library_editors
            .edit(
                id,
                0,
                Action::GenerateCourtyard {
                    clearance_nm: courtyard::DEFAULT_CLEARANCE_NM,
                    source: CourtyardSource::PadsAndBody,
                    stroke_nm: courtyard::DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        zoom_out(&mut h, id, 2.0);
        let cy_id = h.state().library_editors.documents[&id].selected.unwrap();
        (h, id, cy_id)
    }

    /// Type `value` into the labelled property field, replacing its text, without committing it.
    fn type_into(h: &mut Harness<'_, PcbApp>, label: &str, value: &str) {
        h.get_by_label(label).click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label(label).type_text(value);
        h.run();
    }

    /// Screen point 40 px outside the middle of the courtyard's edge `edge`, away from the
    /// outline's centre, where no vertex, midpoint, pad or body graphic can be picked.
    fn empty_point_outside_edge(h: &Harness<'_, PcbApp>, id: u64, cy_id: u64, edge: usize) -> Pos2 {
        let area = h.get_by_label("Library canvas").rect();
        let doc = &h.state().library_editors.documents[&id];
        let snap = doc.session.snapshot();
        let (pts, _) = courtyard::courtyard_polygon(&snap.objects[&cy_id]).unwrap();
        let m = courtyard::metrics(pts).unwrap();
        let centre = doc.camera.screen(Point { x_nm: (m.min.x_nm + m.max.x_nm) / 2, y_nm: (m.min.y_nm + m.max.y_nm) / 2 }, area);
        let mid = doc.camera.screen(courtyard::edge_midpoint(pts, edge).unwrap(), area);
        mid + (mid - centre).normalized() * 40.0
    }

    /// Revision, Undo/Redo availability and the whole item of one library document, captured
    /// before a refused action and compared after it to prove the refusal changed nothing.
    #[derive(Debug, PartialEq)]
    struct DocState {
        revision: u64,
        can_undo: bool,
        can_redo: bool,
        item: std::sync::Arc<Item>,
    }

    fn doc_state(app: &PcbApp, id: u64) -> DocState {
        let session = &app.library_editors.documents[&id].session;
        DocState { revision: session.revision(), can_undo: session.can_undo(), can_redo: session.can_redo(), item: session.snapshot() }
    }

    /// Whether the button labelled `label` is disabled in the accessibility tree.
    fn button_disabled(h: &Harness<'_, PcbApp>, label: &str) -> bool {
        h.get_by_role_and_label(egui::Role::Button, label).accesskit_node().is_disabled()
    }

    fn select_vertex_on_canvas(h: &mut Harness<'_, PcbApp>, id: u64, object: u64, vertex: usize) {
        let area = h.get_by_label("Library canvas").rect();
        let pos = {
            let doc = &h.state().library_editors.documents[&id];
            let snap = doc.session.snapshot();
            let (pts, _) = courtyard::courtyard_polygon(&snap.objects[&object]).unwrap();
            doc.camera.screen(pts[vertex], area)
        };
        click_at(h, pos);
        assert_eq!(h.state().library_editors.documents[&id].selected_vertex, Some(vertex));
    }

    /// Generate Courtyard dialog rejects invalid clearance, clears stale errors when edited,
    /// generates the default 0.25 mm outline, requires explicit vertex selection, and inserts an
    /// exact midpoint without changing area.
    #[test]
    fn generate_courtyard_dialog_creates_editable_outline_and_protects_invalid_input() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let id = h.state().library_editors.active.unwrap();
        // The first frame fits the pads; the 0.25 mm courtyard generated below is larger.
        zoom_out(&mut h, id, 2.0);
        let before_rev = h.state().library_editors.documents[&id].session.revision();

        crate::tests::click(&mut h, "Generate courtyard…");
        assert!(matches!(h.state().dialog, Dialog::LibraryCourtyard(..)));

        // Reject invalid clearance without mutating the document.
        h.get_by_label("Clearance (mm)").click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Clearance (mm)").type_text("-1");
        h.run();
        crate::tests::click(&mut h, "Generate");
        assert!(matches!(h.state().dialog, Dialog::LibraryCourtyard(..)));
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), before_rev);

        // Editing the field clears the stale error and restores the live size preview.
        h.get_by_label("Clearance (mm)").click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Clearance (mm)").type_text("0.25");
        h.run();
        if let Dialog::LibraryCourtyard(draft) = &h.state().dialog {
            assert!(draft.error.is_none());
        } else {
            panic!("expected courtyard dialog to remain open");
        }

        crate::tests::click(&mut h, "Generate");
        assert!(matches!(h.state().dialog, Dialog::None));

        let snap = h.state().library_editors.documents[&id].session.snapshot();
        let selected = h.state().library_editors.documents[&id].selected.unwrap();
        let (pts, stroke) = courtyard::courtyard_polygon(&snap.objects[&selected]).unwrap();
        assert_eq!(stroke, courtyard::DEFAULT_STROKE_NM);
        let before = courtyard::metrics(pts).unwrap();
        assert_eq!(before.width_nm, 2_100_000);
        assert_eq!(before.height_nm, 1_300_000);

        // No vertex is implicitly selected until the user clicks one or chooses it.
        assert_eq!(h.state().library_editors.documents[&id].selected_vertex, None);
        assert!(h.query_by_label("Vertex X (mm)").is_none());

        // Selecting vertex 0 on the canvas reveals vertex coordinate fields and actions.
        select_vertex_on_canvas(&mut h, id, selected, 0);
        assert!(h.query_by_label("Vertex X (mm)").is_some());
        assert!(h.query_by_label("Vertex Y (mm)").is_some());
        assert!(h.query_by_label("Add vertex").is_some());
        assert!(h.query_by_label("Remove vertex").is_some());

        // Add vertex inserts the exact midpoint on the 0.25 mm outline without changing its area.
        crate::tests::click(&mut h, "Add vertex");
        let after_add = h.state().library_editors.documents[&id].session.snapshot();
        let (added_pts, _) = courtyard::courtyard_polygon(&after_add.objects[&selected]).unwrap();
        let after = courtyard::metrics(added_pts).unwrap();
        assert_eq!(after.vertex_count, 5);
        assert_eq!(after.area_nm2, before.area_nm2);
        assert_eq!(after.width_nm, before.width_nm);
        assert_eq!(after.height_nm, before.height_nm);

        crate::tests::click(&mut h, "Remove vertex");
        let after_remove = h.state().library_editors.documents[&id].session.snapshot();
        assert_eq!(courtyard::courtyard_polygon(&after_remove.objects[&selected]).unwrap().0.len(), 4);
    }

    /// Hiding the courtyard layer clears courtyard selection without mutating revision and
    /// excludes hidden courtyard outlines from marquee box select and Select All.
    #[test]
    fn courtyard_layer_toggle_clears_selection_and_excludes_hidden_courtyard_from_marquee_and_select_all() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let id = app.library_editors.active.unwrap();
        app.library_editors
            .edit(
                id,
                0,
                Action::GenerateCourtyard {
                    clearance_nm: courtyard::DEFAULT_CLEARANCE_NM,
                    source: CourtyardSource::PadsAndBody,
                    stroke_nm: courtyard::DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let cy_id = h.state().library_editors.documents[&id].selected.unwrap();
        let rev = h.state().library_editors.documents[&id].session.revision();

        // Hide via the View ribbon button ("Courtyard layer"): clears selection, preserves revision.
        crate::tests::click(&mut h, "View");
        crate::tests::click(&mut h, "Courtyard layer");
        assert!(!h.state().library_editors.documents[&id].show_courtyard);
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), rev);
        assert!(!h.state().library_editors.documents[&id].selection.contains(&cy_id));

        // Compute a marquee box that strictly encloses the entire courtyard outline and pads.
        let area = h.get_by_label("Library canvas").rect();
        let box_rect = {
            let doc = &h.state().library_editors.documents[&id];
            let snap = doc.session.snapshot();
            let (pts, _) = courtyard::courtyard_polygon(&snap.objects[&cy_id]).unwrap();
            pts.iter().fold(Rect::NOTHING, |r, p| r.union(Rect::from_pos(doc.camera.screen(*p, area)))).expand(20.0)
        };

        // Marquee box select over the outline selects the pads (positive control) and excludes hidden courtyard.
        drag_at(&mut h, box_rect.left_top(), box_rect.right_bottom());
        let marquee_hidden = &h.state().library_editors.documents[&id].selection;
        assert!(marquee_hidden.contains(&1) && marquee_hidden.contains(&2));
        assert!(!marquee_hidden.contains(&cy_id));

        // Select All (Cmd+A) over the canvas selects the pads and excludes hidden courtyard.
        h.state_mut().library_editors.documents.get_mut(&id).unwrap().select_many(Default::default()).unwrap();
        h.run();
        h.hover_at(area.center());
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run();
        let all_hidden = &h.state().library_editors.documents[&id].selection;
        assert!(all_hidden.contains(&1) && all_hidden.contains(&2));
        assert!(!all_hidden.contains(&cy_id));

        // Re-enable via the View ribbon button ("Courtyard layer").
        crate::tests::click(&mut h, "Courtyard layer");
        assert!(h.state().library_editors.documents[&id].show_courtyard);
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), rev);

        // Positive control with the layer visible: the exact same marquee and Cmd+A select cy_id.
        h.state_mut().library_editors.documents.get_mut(&id).unwrap().select_many(Default::default()).unwrap();
        h.run();
        drag_at(&mut h, box_rect.left_top(), box_rect.right_bottom());
        let marquee_visible = &h.state().library_editors.documents[&id].selection;
        assert!(marquee_visible.contains(&1) && marquee_visible.contains(&2) && marquee_visible.contains(&cy_id));

        h.state_mut().library_editors.documents.get_mut(&id).unwrap().select_many(Default::default()).unwrap();
        h.run();
        h.hover_at(area.center());
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run();
        let all_visible = &h.state().library_editors.documents[&id].selection;
        assert!(all_visible.contains(&1) && all_visible.contains(&2) && all_visible.contains(&cy_id));
    }

    /// Pending property edits refuse switching vertices or activating C with explicit error
    /// messages (verified with Reset positive controls), while Apply commits atomically with Undo.
    #[test]
    fn courtyard_vertex_coordinate_edit_validates_supports_undo_and_refuses_while_pending() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let id = app.library_editors.active.unwrap();
        app.library_editors
            .edit(
                id,
                0,
                Action::GenerateCourtyard {
                    clearance_nm: courtyard::DEFAULT_CLEARANCE_NM,
                    source: CourtyardSource::PadsAndBody,
                    stroke_nm: courtyard::DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let selected = h.state().library_editors.documents[&id].selected.unwrap();
        select_vertex_on_canvas(&mut h, id, selected, 0);

        h.get_by_label("Vertex X (mm)").click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Vertex X (mm)").type_text("-1.5");
        h.run();
        assert!(h.state().library_editors.documents[&id].pending());

        // Clicking vertex 1 on the canvas while edits are pending refuses with an explicit error.
        let area = h.get_by_label("Library canvas").rect();
        let v1_screen = {
            let doc = &h.state().library_editors.documents[&id];
            let snap = doc.session.snapshot();
            let (pts, _) = courtyard::courtyard_polygon(&snap.objects[&selected]).unwrap();
            doc.camera.screen(pts[1], area)
        };
        let before = doc_state(h.state(), id);
        click_at(&mut h, v1_screen);
        assert_eq!(h.state().library_editors.documents[&id].selected_vertex, Some(0));
        assert_eq!(h.state().library_editors.documents[&id].error.as_deref(), Some("Apply or reset the property fields before switching vertices."));
        assert_eq!(doc_state(h.state(), id), before);

        // Pressing C while edits are pending refuses with an explicit error and keeps show_courtyard false.
        h.state_mut().library_editors.documents.get_mut(&id).unwrap().show_courtyard = false;
        h.hover_at(area.center());
        h.run();
        h.key_press(egui::Key::C);
        h.run();
        assert_eq!(h.state().library_editors.documents[&id].error.as_deref(), Some("Apply or reset the property fields before changing tools."));
        assert!(!h.state().library_editors.documents[&id].show_courtyard);
        assert!(matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Select));
        assert_eq!(doc_state(h.state(), id), before);
        h.state_mut().library_editors.documents.get_mut(&id).unwrap().show_courtyard = true;

        // Positive controls after Reset: the same click selects vertex 1 and the same C arms Courtyard.
        crate::tests::click(&mut h, "Reset");
        assert!(!h.state().library_editors.documents[&id].pending());
        click_at(&mut h, v1_screen);
        assert_eq!(h.state().library_editors.documents[&id].selected_vertex, Some(1));
        h.state_mut().library_editors.documents.get_mut(&id).unwrap().show_courtyard = false;
        h.hover_at(area.center());
        h.run();
        h.key_press(egui::Key::C);
        h.run();
        assert!(matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Draw { kind: canvas::Place::Courtyard, .. }));
        assert!(h.state().library_editors.documents[&id].show_courtyard);
        h.key_press(egui::Key::Escape);
        h.run();

        // Apply commits vertex 0 edit atomically and Undo restores the original coordinate.
        select_vertex_on_canvas(&mut h, id, selected, 0);
        h.get_by_label("Vertex X (mm)").click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Vertex X (mm)").type_text("-1.5");
        h.run();
        crate::tests::click(&mut h, "Apply");
        let snap = h.state().library_editors.documents[&id].session.snapshot();
        let (pts, _) = courtyard::courtyard_polygon(&snap.objects[&selected]).unwrap();
        assert_eq!(pts[0].x_nm, -1_500_000);

        crate::tests::click(&mut h, "Undo Library Edit");
        let undone = h.state().library_editors.documents[&id].session.snapshot();
        let (pts, _) = courtyard::courtyard_polygon(&undone.objects[&selected]).unwrap();
        assert_eq!(pts[0].x_nm, -1_050_000);
    }

    /// Drive the footprint canvas directly for C draw (Escape, Enter + Undo, first-vertex close,
    /// double-click close), vertex dragging with grid snap and one-step undo, midpoint click
    /// insertion, and Backspace vertex removal.
    #[test]
    fn courtyard_canvas_draw_drag_midpoint_and_backspace_interactions() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let id = h.state().library_editors.active.unwrap();
        // The first frame fits the pads; the triangle and the generated outline reach beyond them.
        zoom_out(&mut h, id, 4.0);
        let area = h.get_by_label("Library canvas").rect();

        // Press C to arm Courtyard draw tool, then Escape cancels cleanly.
        h.hover_at(area.center());
        h.run();
        h.key_press(egui::Key::C);
        h.run();
        assert!(matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Draw { kind: canvas::Place::Courtyard, .. }));
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Select));

        // Draw a 3-vertex courtyard triangle using C + 3 clicks + Enter, then undo it.
        let (p0, p1, p2) = {
            let doc = &h.state().library_editors.documents[&id];
            (
                doc.camera.screen(Point { x_nm: -1_000_000, y_nm: -500_000 }, area),
                doc.camera.screen(Point { x_nm: 1_000_000, y_nm: -500_000 }, area),
                doc.camera.screen(Point { x_nm: 0, y_nm: 1_000_000 }, area),
            )
        };
        h.hover_at(area.center());
        h.run();
        h.key_press(egui::Key::C);
        h.run();
        click_at(&mut h, p0);
        click_at(&mut h, p1);
        click_at(&mut h, p2);
        h.key_press(egui::Key::Enter);
        h.run();
        let drawn_id = h.state().library_editors.documents[&id].selected.unwrap();
        let drawn_snap = h.state().library_editors.documents[&id].session.snapshot();
        assert_eq!(courtyard::courtyard_polygon(&drawn_snap.objects[&drawn_id]).unwrap().0.len(), 3);
        crate::tests::click(&mut h, "Undo Library Edit");
        assert!(!h.state().library_editors.documents[&id].session.snapshot().objects.contains_key(&drawn_id));

        // Close a drawn courtyard by clicking back on the first vertex.
        h.hover_at(area.center());
        h.run();
        h.key_press(egui::Key::C);
        h.run();
        click_at(&mut h, p0);
        click_at(&mut h, p1);
        click_at(&mut h, p2);
        click_at(&mut h, p0);
        let first_close_id = h.state().library_editors.documents[&id].selected.unwrap();
        assert_eq!(
            courtyard::courtyard_polygon(&h.state().library_editors.documents[&id].session.snapshot().objects[&first_close_id]).unwrap().0.len(),
            3
        );
        crate::tests::click(&mut h, "Undo Library Edit");

        // Close a drawn courtyard by double-clicking the final vertex.
        h.hover_at(area.center());
        h.run();
        h.key_press(egui::Key::C);
        h.run();
        click_at(&mut h, p0);
        click_at(&mut h, p1);
        double_click_at(&mut h, p2);
        let dbl_close_id = h.state().library_editors.documents[&id].selected.unwrap();
        assert_eq!(
            courtyard::courtyard_polygon(&h.state().library_editors.documents[&id].session.snapshot().objects[&dbl_close_id]).unwrap().0.len(),
            3
        );

        // Generate default 0.25 mm courtyard replacing the drawn one.
        let rev = h.state().library_editors.documents[&id].session.revision();
        h.state_mut()
            .library_editors
            .edit(
                id,
                rev,
                Action::GenerateCourtyard {
                    clearance_nm: courtyard::DEFAULT_CLEARANCE_NM,
                    source: CourtyardSource::PadsAndBody,
                    stroke_nm: courtyard::DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        h.run();
        let cy_id = h.state().library_editors.documents[&id].selected.unwrap();
        let (before_area, mid_screen, v0_screen, drag_to_screen, orig_v0) = {
            let doc = &h.state().library_editors.documents[&id];
            let snap = doc.session.snapshot();
            let (pts, _) = courtyard::courtyard_polygon(&snap.objects[&cy_id]).unwrap();
            let area_nm2 = courtyard::metrics(pts).unwrap().area_nm2;
            let mid = courtyard::edge_midpoint(pts, 1).unwrap();
            (
                area_nm2,
                doc.camera.screen(mid, area),
                doc.camera.screen(pts[0], area),
                doc.camera.screen(Point { x_nm: -1_520_000, y_nm: -980_000 }, area),
                pts[0],
            )
        };

        // Click edge midpoint handle: inserts exact midpoint without changing area and selects it.
        click_at(&mut h, mid_screen);
        let after_mid = h.state().library_editors.documents[&id].session.snapshot();
        let (mid_pts, _) = courtyard::courtyard_polygon(&after_mid.objects[&cy_id]).unwrap();
        assert_eq!(mid_pts.len(), 5);
        assert_eq!(courtyard::metrics(mid_pts).unwrap().area_nm2, before_area);
        assert_eq!(h.state().library_editors.documents[&id].selected_vertex, Some(2));

        // Press Backspace over canvas: removes the explicitly selected 5th vertex back to 4.
        h.hover_at(area.center());
        h.run();
        h.key_press(egui::Key::Backspace);
        h.run();
        let after_bs = h.state().library_editors.documents[&id].session.snapshot();
        assert_eq!(courtyard::courtyard_polygon(&after_bs.objects[&cy_id]).unwrap().0.len(), 4);

        // Drag vertex 0 on the canvas: snaps to grid, commits in 1 edit, and Undo restores orig_v0.
        drag_at(&mut h, v0_screen, drag_to_screen);
        let after_drag = h.state().library_editors.documents[&id].session.snapshot();
        let (drag_pts, _) = courtyard::courtyard_polygon(&after_drag.objects[&cy_id]).unwrap();
        let snap_nm = h.state().library_editors.documents[&id].camera.snap_nm();
        assert_ne!(drag_pts[0], orig_v0);
        assert_eq!(drag_pts[0].x_nm % snap_nm, 0);
        assert_eq!(drag_pts[0].y_nm % snap_nm, 0);

        crate::tests::click(&mut h, "Undo Library Edit");
        let after_undo = h.state().library_editors.documents[&id].session.snapshot();
        assert_eq!(courtyard::courtyard_polygon(&after_undo.objects[&cy_id]).unwrap().0[0], orig_v0);
    }

    /// Generate Courtyard dialog refuses a stale revision, rebases to the current document
    /// revision with guidance, and succeeds on the second Generate click.
    #[test]
    fn courtyard_dialog_refuses_stale_revision() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let id = h.state().library_editors.active.unwrap();

        crate::tests::click(&mut h, "Generate courtyard…");
        assert!(matches!(h.state().dialog, Dialog::LibraryCourtyard(..)));

        // Advance document revision underneath the open dialog.
        h.state_mut()
            .library_editors
            .edit(
                id,
                0,
                Action::GenerateCourtyard {
                    clearance_nm: 100_000,
                    source: CourtyardSource::PadsOnly,
                    stroke_nm: courtyard::DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        let rev_after_bg = h.state().library_editors.documents[&id].session.revision();
        let before = doc_state(h.state(), id);

        crate::tests::click(&mut h, "Generate");
        assert_eq!(doc_state(h.state(), id), before);
        if let Dialog::LibraryCourtyard(draft) = &h.state().dialog {
            assert_eq!(draft.revision, rev_after_bg);
            assert!(draft.has_existing);
            assert_eq!(draft.error.as_deref(), Some("The footprint changed; check the preview and press Generate again."));
        } else {
            panic!("expected courtyard dialog to remain open after stale revision");
        }

        // Pressing Generate again now applies cleanly against the rebased revision.
        crate::tests::click(&mut h, "Generate");
        assert!(matches!(h.state().dialog, Dialog::None));
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), rev_after_bg + 1);
    }

    /// Cmd+C copy chord (even when Cmd is released within the same input batch) and Shift+C must
    /// not arm the Courtyard draw tool or turn on the courtyard layer.
    #[test]
    fn courtyard_c_shortcut_ignores_copy_chord_and_shift_c() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let id = h.state().library_editors.active.unwrap();
        h.state_mut().library_editors.documents.get_mut(&id).unwrap().show_courtyard = false;
        let area = h.get_by_label("Library canvas").rect();
        h.hover_at(area.center());
        h.run();

        // Simulate Cmd+C where Command is released in the same input batch: the C press still
        // carries Command, while no modifier is held by the time the batch is read.
        h.input_mut().events.push(egui::Event::Key {
            key: egui::Key::C,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        h.input_mut().events.push(egui::Event::Key {
            key: egui::Key::C,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.run();
        assert!(matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Select));
        assert!(!h.state().library_editors.documents[&id].show_courtyard);

        // Shift+C must also not arm Courtyard.
        h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::C);
        h.run();
        assert!(matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Select));
        assert!(!h.state().library_editors.documents[&id].show_courtyard);
    }

    /// Backspace on a freshly selected courtyard (with no vertex explicitly selected) deletes the
    /// object rather than silently dropping vertex 0, and Cmd+Backspace never removes a vertex.
    #[test]
    fn courtyard_backspace_without_selected_vertex_deletes_object_and_ignores_modifiers() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let id = app.library_editors.active.unwrap();
        app.library_editors
            .edit(
                id,
                0,
                Action::GenerateCourtyard {
                    clearance_nm: courtyard::DEFAULT_CLEARANCE_NM,
                    source: CourtyardSource::PadsAndBody,
                    stroke_nm: courtyard::DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let cy_id = h.state().library_editors.documents[&id].selected.unwrap();
        assert_eq!(h.state().library_editors.documents[&id].selected_vertex, None);

        let area = h.get_by_label("Library canvas").rect();
        h.hover_at(area.center());
        h.run();
        h.key_press(egui::Key::Backspace);
        h.run();
        assert!(!h.state().library_editors.documents[&id].session.snapshot().objects.contains_key(&cy_id));

        // Undo restores the intact 4-vertex courtyard outline.
        crate::tests::click(&mut h, "Undo Library Edit");
        let snap = h.state().library_editors.documents[&id].session.snapshot();
        assert_eq!(courtyard::courtyard_polygon(&snap.objects[&cy_id]).unwrap().0.len(), 4);

        // With vertex 0 explicitly selected, Cmd+Backspace must not remove the vertex.
        h.state_mut().library_editors.documents.get_mut(&id).unwrap().select(Some(cy_id)).unwrap();
        h.run();
        select_vertex_on_canvas(&mut h, id, cy_id, 0);
        h.hover_at(area.center());
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Backspace);
        h.run();
        let after_cmd_bs = h.state().library_editors.documents[&id].session.snapshot();
        assert_eq!(courtyard::courtyard_polygon(&after_cmd_bs.objects[&cy_id]).unwrap().0.len(), 4);
    }

    /// The live `app.library_edit` command generates a courtyard through the same revision-checked
    /// action as the dialog, with defaults for everything but the clearance. Stale revisions,
    /// symbol documents and unknown fields fail before any mutation; the project is untouched.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn live_library_edit_generates_courtyard_with_defaults_and_revision_check() {
        let mut app = PcbApp::default();
        let ctx = Context::default();
        let project_revision = app.session.revision();
        app.run_window_command(&ctx, "app.library_new", &json!({"kind":"footprint"})).unwrap();
        let id = app.library_editors.active.unwrap();
        let generate = |revision: u64, extra: Option<(&str, Value)>| {
            let mut action = json!({"operation":"generate_courtyard","clearance_nm":250_000});
            if let (Some((key, value)), Some(map)) = (extra, action.as_object_mut()) {
                map.insert(key.into(), value);
            }
            json!({"document":id,"expected_revision":revision,"action":action})
        };
        let before = doc_state(&app, id);
        assert!(app.run_window_command(&ctx, "app.library_edit", &generate(1, None)).is_err());
        assert_eq!(doc_state(&app, id), before);
        assert!(app.run_window_command(&ctx, "app.library_edit", &generate(0, Some(("margin_nm", json!(1))))).is_err());
        assert_eq!(doc_state(&app, id), before);
        // An out-of-range stroke is refused with the same message as the dialog and inspector.
        let wide = app.run_window_command(&ctx, "app.library_edit", &generate(0, Some(("stroke_nm", json!(5_000_000)))));
        assert!(wide.unwrap_err().contains("Stroke (mm) must be between 0.001 and 2 mm."));
        assert_eq!(doc_state(&app, id), before);
        app.run_window_command(&ctx, "app.library_edit", &generate(0, None)).unwrap();
        let item = app.library_editors.documents[&id].session.snapshot();
        let outlines: Vec<_> = item.objects.values().filter_map(courtyard::courtyard_polygon).collect();
        assert_eq!(outlines.len(), 1);
        let (pts, stroke) = outlines[0];
        assert_eq!(stroke, courtyard::DEFAULT_STROKE_NM);
        let m = courtyard::metrics(pts).unwrap();
        assert_eq!((m.width_nm, m.height_nm, m.vertex_count), (2_100_000, 1_300_000, 4));
        // Generating again replaces the outline by default instead of stacking a second one.
        app.run_window_command(&ctx, "app.library_edit", &generate(1, None)).unwrap();
        let item = app.library_editors.documents[&id].session.snapshot();
        assert_eq!(item.objects.values().filter(|p| courtyard::is_courtyard_primitive(p)).count(), 1);
        assert_eq!(app.session.revision(), project_revision);

        app.run_window_command(&ctx, "app.library_new", &json!({"kind":"symbol"})).unwrap();
        let symbol = app.library_editors.active.unwrap();
        let symbol_before = doc_state(&app, symbol);
        let refused = app.run_window_command(
            &ctx,
            "app.library_edit",
            &json!({"document":symbol,"expected_revision":0,"action":{"operation":"generate_courtyard","clearance_nm":250_000}}),
        );
        assert!(refused.is_err());
        assert_eq!(doc_state(&app, symbol), symbol_before);
    }

    /// Hiding the courtyard layer while the selected outline has pending property edits is
    /// refused: the layer stays visible, the selection, vertex and edits are kept and the
    /// document is untouched. After Reset the same button hides the layer and deselects it.
    #[test]
    fn courtyard_layer_hide_refused_while_pending_keeps_selection_and_document() {
        let (mut h, id, cy_id) = harness_with_generated_courtyard();
        select_vertex_on_canvas(&mut h, id, cy_id, 1);
        type_into(&mut h, "Vertex X (mm)", "2");
        assert!(h.state().library_editors.documents[&id].pending());
        let before = doc_state(h.state(), id);

        crate::tests::click(&mut h, "View");
        crate::tests::click(&mut h, "Courtyard layer");
        let doc = &h.state().library_editors.documents[&id];
        assert_eq!(doc.error.as_deref(), Some("Apply or reset the property fields before changing selection."));
        assert!(doc.show_courtyard);
        assert!(doc.selection.contains(&cy_id));
        assert_eq!(doc.selected_vertex, Some(1));
        assert!(doc.pending());
        assert_eq!(doc_state(h.state(), id), before);

        // Positive control: after Reset the same button hides the layer and clears the selection.
        crate::tests::click(&mut h, "Reset");
        crate::tests::click(&mut h, "Courtyard layer");
        let doc = &h.state().library_editors.documents[&id];
        assert!(!doc.show_courtyard);
        assert!(!doc.selection.contains(&cy_id));
        assert_eq!(doc_state(h.state(), id), before);
    }

    /// With a vertex selected and property edits pending, Backspace refuses with an explicit
    /// error and keeps the whole outline (it must not fall through to deleting the object).
    /// After Reset the same Backspace removes that vertex, and at three vertices it refuses.
    #[test]
    fn courtyard_backspace_with_selected_vertex_and_pending_edits_keeps_outline() {
        let (mut h, id, cy_id) = harness_with_generated_courtyard();
        select_vertex_on_canvas(&mut h, id, cy_id, 2);
        type_into(&mut h, "Vertex X (mm)", "2");
        assert!(h.state().library_editors.documents[&id].pending());
        let rev = h.state().library_editors.documents[&id].session.revision();
        let before = doc_state(h.state(), id);

        // Clicking empty canvas takes focus from the field; the selection change is refused.
        let empty = empty_point_outside_edge(&h, id, cy_id, 2);
        click_at(&mut h, empty);
        assert!(!h.get_by_label("Vertex X (mm)").is_focused());
        assert_eq!(h.state().library_editors.documents[&id].error.as_deref(), Some("Apply or reset the property fields before changing selection."));
        assert!(h.state().library_editors.documents[&id].selection.contains(&cy_id));
        assert_eq!(h.state().library_editors.documents[&id].selected_vertex, Some(2));
        assert_eq!(doc_state(h.state(), id), before);

        h.key_press(egui::Key::Backspace);
        h.run();
        assert_eq!(doc_state(h.state(), id), before);
        let doc = &h.state().library_editors.documents[&id];
        assert_eq!(doc.error.as_deref(), Some("Apply or reset the property fields before removing a vertex."));
        assert_eq!(doc.session.revision(), rev);
        assert!(doc.pending());
        let snap = doc.session.snapshot();
        assert_eq!(courtyard::courtyard_polygon(&snap.objects[&cy_id]).unwrap().0.len(), 4);
        assert!(h.query_by_label("Apply or reset the property fields before removing a vertex.").is_some());

        // Positive control: after Reset the same key removes vertex 3 (index 2) only.
        crate::tests::click(&mut h, "Reset");
        assert!(!h.state().library_editors.documents[&id].pending());
        h.hover_at(empty);
        h.run();
        h.key_press(egui::Key::Backspace);
        h.run();
        let doc = &h.state().library_editors.documents[&id];
        assert_eq!(doc.session.revision(), rev + 1);
        assert_eq!(courtyard::courtyard_polygon(&doc.session.snapshot().objects[&cy_id]).unwrap().0.len(), 3);
        assert_eq!(doc.selected_vertex, Some(2));

        // At three vertices Backspace refuses and keeps the triangle.
        let triangle = doc_state(h.state(), id);
        h.key_press(egui::Key::Backspace);
        h.run();
        assert_eq!(doc_state(h.state(), id), triangle);
        let doc = &h.state().library_editors.documents[&id];
        assert_eq!(doc.error.as_deref(), Some("A courtyard outline needs at least three vertices."));
        assert_eq!(doc.session.revision(), rev + 1);
        assert_eq!(courtyard::courtyard_polygon(&doc.session.snapshot().objects[&cy_id]).unwrap().0.len(), 3);
    }

    /// Pressing Escape during a vertex drag cancels it cleanly so releasing over a midpoint handle
    /// does not insert a vertex or mutate the document.
    #[test]
    fn courtyard_escape_mid_drag_ignores_release_over_midpoint() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let id = app.library_editors.active.unwrap();
        app.library_editors
            .edit(
                id,
                0,
                Action::GenerateCourtyard {
                    clearance_nm: courtyard::DEFAULT_CLEARANCE_NM,
                    source: CourtyardSource::PadsAndBody,
                    stroke_nm: courtyard::DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let cy_id = h.state().library_editors.documents[&id].selected.unwrap();
        let rev_before = h.state().library_editors.documents[&id].session.revision();
        let area = h.get_by_label("Library canvas").rect();
        let (v0_screen, mid_screen) = {
            let doc = &h.state().library_editors.documents[&id];
            let snap = doc.session.snapshot();
            let (pts, _) = courtyard::courtyard_polygon(&snap.objects[&cy_id]).unwrap();
            (doc.camera.screen(pts[0], area), doc.camera.screen(courtyard::edge_midpoint(pts, 1).unwrap(), area))
        };

        h.hover_at(v0_screen);
        h.run();
        h.event(egui::Event::PointerButton { pos: v0_screen, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run();
        h.hover_at(mid_screen);
        h.run();
        assert!(matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::DragCourtyardVertex { .. }));
        h.key_press(egui::Key::Escape);
        h.run();
        h.event(egui::Event::PointerButton {
            pos: mid_screen,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.run();

        assert!(matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Select));
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), rev_before);
        let snap = h.state().library_editors.documents[&id].session.snapshot();
        assert_eq!(courtyard::courtyard_polygon(&snap.objects[&cy_id]).unwrap().0.len(), 4);
    }

    /// Enter submits Generate Courtyard from Clearance and Stroke fields, and the inspector
    /// enforces the 0.001..=2 mm stroke range and commits valid Stroke edits on Enter.
    #[test]
    fn courtyard_dialog_enter_and_inspector_stroke_validation() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let id = h.state().library_editors.active.unwrap();

        // Enter in Clearance (mm) submits the dialog.
        crate::tests::click(&mut h, "Generate courtyard…");
        h.get_by_label("Clearance (mm)").click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Clearance (mm)").type_text("0.5");
        h.key_press(egui::Key::Enter);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::None));

        // Enter in Stroke (mm) submits the dialog.
        crate::tests::click(&mut h, "Regenerate…");
        enabled_by_label(&h, "Stroke (mm)").click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        enabled_by_label(&h, "Stroke (mm)").type_text("0.15");
        h.key_press(egui::Key::Enter);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::None));
        let cy_id = h.state().library_editors.documents[&id].selected.unwrap();
        assert_eq!(courtyard::courtyard_polygon(&h.state().library_editors.documents[&id].session.snapshot().objects[&cy_id]).unwrap().1, 150_000);

        // Inspector Stroke rejects out-of-range values with the exact range message.
        h.get_by_label("Stroke (mm)").click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Stroke (mm)").type_text("5");
        h.run();
        let before = doc_state(h.state(), id);
        crate::tests::click(&mut h, "Apply");
        assert_eq!(h.state().library_editors.documents[&id].error.as_deref(), Some("Stroke (mm) must be between 0.001 and 2 mm."));
        assert_eq!(doc_state(h.state(), id), before);

        // Valid inspector Stroke commits on Enter.
        h.get_by_label("Stroke (mm)").click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Stroke (mm)").type_text("0.2");
        h.key_press(egui::Key::Enter);
        h.run();
        assert_eq!(courtyard::courtyard_polygon(&h.state().library_editors.documents[&id].session.snapshot().objects[&cy_id]).unwrap().1, 200_000);
    }

    /// Snapshot state: courtyard draw tool active on a footprint with no courtyard yet.
    #[test]
    fn courtyard_shot_tool_active_and_empty() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let id = h.state().library_editors.active.unwrap();
        let area = h.get_by_label("Library canvas").rect();
        h.hover_at(area.center());
        h.run();
        h.key_press(egui::Key::C);
        h.run();
        assert!(matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Draw { kind: canvas::Place::Courtyard, start: None, .. }));
        assert!(h.state().library_editors.documents[&id].first_courtyard_id().is_none());
    }

    /// Snapshot state: courtyard outline actively being drawn with two placed vertices and a live preview segment.
    #[test]
    fn courtyard_shot_outline_being_drawn() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let id = h.state().library_editors.active.unwrap();
        // The first frame fits the pads; the outline being drawn reaches beyond them.
        zoom_out(&mut h, id, 4.0);
        let area = h.get_by_label("Library canvas").rect();
        let (p0, p1, p2) = {
            let doc = &h.state().library_editors.documents[&id];
            (
                doc.camera.screen(Point { x_nm: -1_500_000, y_nm: -900_000 }, area),
                doc.camera.screen(Point { x_nm: 1_500_000, y_nm: -900_000 }, area),
                doc.camera.screen(Point { x_nm: 1_500_000, y_nm: 900_000 }, area),
            )
        };
        h.hover_at(area.center());
        h.run();
        h.key_press(egui::Key::C);
        h.run();
        click_at(&mut h, p0);
        click_at(&mut h, p1);
        h.hover_at(p2);
        h.run();
        assert!(matches!(&h.state().library_editors.documents[&id].tool, canvas::Tool::Courtyard { points, .. } if points.len() == 2));
    }

    /// Snapshot state: generated courtyard outline with vertex 0 explicitly selected and vertex properties panel open.
    #[test]
    fn courtyard_shot_vertex_selected_with_panel() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let id = app.library_editors.active.unwrap();
        app.library_editors
            .edit(
                id,
                0,
                Action::GenerateCourtyard {
                    clearance_nm: courtyard::DEFAULT_CLEARANCE_NM,
                    source: CourtyardSource::PadsAndBody,
                    stroke_nm: courtyard::DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let cy_id = h.state().library_editors.documents[&id].selected.unwrap();
        select_vertex_on_canvas(&mut h, id, cy_id, 0);
        assert!(h.query_by_label("Vertex X (mm)").is_some());
        assert!(h.query_by_label("Vertex Y (mm)").is_some());
    }

    /// Snapshot state: Generate Courtyard dialog open with an invalid clearance validation error.
    #[test]
    fn courtyard_shot_validation_error() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        crate::tests::click(&mut h, "Generate courtyard…");
        h.get_by_label("Clearance (mm)").click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Clearance (mm)").type_text("-1");
        h.run();
        crate::tests::click(&mut h, "Generate");
        assert!(matches!(&h.state().dialog, Dialog::LibraryCourtyard(draft) if draft.error.is_some()));
    }

    /// Snapshot state: footprint with a generated courtyard whose courtyard layer is hidden.
    #[test]
    fn courtyard_shot_layer_hidden() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let id = app.library_editors.active.unwrap();
        app.library_editors
            .edit(
                id,
                0,
                Action::GenerateCourtyard {
                    clearance_nm: courtyard::DEFAULT_CLEARANCE_NM,
                    source: CourtyardSource::PadsAndBody,
                    stroke_nm: courtyard::DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        crate::tests::click(&mut h, "View");
        crate::tests::click(&mut h, "Courtyard layer");
        assert!(!h.state().library_editors.documents[&id].show_courtyard);
        assert!(h.state().library_editors.documents[&id].selected.is_none());
    }

    /// Snapshot state: Generate Courtyard dialog open with the stale-revision rebase error.
    #[test]
    fn courtyard_shot_stale_revision_dialog() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let id = h.state().library_editors.active.unwrap();
        crate::tests::click(&mut h, "Generate courtyard…");
        h.state_mut()
            .library_editors
            .edit(
                id,
                0,
                Action::GenerateCourtyard {
                    clearance_nm: 100_000,
                    source: CourtyardSource::PadsOnly,
                    stroke_nm: courtyard::DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        crate::tests::click(&mut h, "Generate");
        assert!(matches!(&h.state().dialog, Dialog::LibraryCourtyard(draft) if draft.error.is_some()));
    }

    /// Snapshot state: keyboard focus inside the Vertex X (mm) input of the Courtyard outline panel.
    #[test]
    fn courtyard_shot_keyboard_focus_in_vertex_panel() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let id = app.library_editors.active.unwrap();
        app.library_editors
            .edit(
                id,
                0,
                Action::GenerateCourtyard {
                    clearance_nm: courtyard::DEFAULT_CLEARANCE_NM,
                    source: CourtyardSource::PadsAndBody,
                    stroke_nm: courtyard::DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let cy_id = h.state().library_editors.documents[&id].selected.unwrap();
        select_vertex_on_canvas(&mut h, id, cy_id, 0);
        h.get_by_label("Vertex X (mm)").click();
        h.run();
        assert!(h.get_by_label("Vertex X (mm)").is_focused());
        assert_eq!(h.state().library_editors.documents[&id].selected_vertex, Some(0));
    }

    /// Snapshot state: Generate Courtyard dialog open with default settings and live size preview.
    #[test]
    fn courtyard_shot_dialog_default_preview() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        crate::tests::click(&mut h, "Generate courtyard…");
        assert!(matches!(&h.state().dialog, Dialog::LibraryCourtyard(draft) if draft.error.is_none() && !draft.has_existing));
        assert!(h.query_by_label("Result: 2.10 × 1.30 mm · 4 vertices").is_some());
        assert!(h.query_by_label("Replace existing courtyard").is_none());
    }

    /// Snapshot state: Generate Courtyard dialog open when an existing courtyard is present,
    /// showing the "Replace existing courtyard" checkbox.
    #[test]
    fn courtyard_shot_dialog_replace_existing() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let id = app.library_editors.active.unwrap();
        app.library_editors
            .edit(
                id,
                0,
                Action::GenerateCourtyard {
                    clearance_nm: courtyard::DEFAULT_CLEARANCE_NM,
                    source: CourtyardSource::PadsAndBody,
                    stroke_nm: courtyard::DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        crate::tests::click(&mut h, "Regenerate…");
        assert!(matches!(&h.state().dialog, Dialog::LibraryCourtyard(draft) if draft.has_existing && draft.error.is_none()));
        assert!(h.query_by_label("Replace existing courtyard").is_some());
    }

    /// Snapshot state: keyboard focus on Clearance (mm) inside the Generate Courtyard dialog.
    #[test]
    fn courtyard_shot_dialog_keyboard_focus() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        crate::tests::click(&mut h, "Generate courtyard…");
        h.get_by_label("Clearance (mm)").click();
        h.run();
        assert!(h.get_by_label("Clearance (mm)").is_focused());
    }

    /// Snapshot state: populated Courtyard summary in Footprint Definition properties with
    /// "Select courtyard" action visible.
    #[test]
    fn courtyard_shot_summary_populated() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let id = app.library_editors.active.unwrap();
        app.library_editors
            .edit(
                id,
                0,
                Action::GenerateCourtyard {
                    clearance_nm: courtyard::DEFAULT_CLEARANCE_NM,
                    source: CourtyardSource::PadsAndBody,
                    stroke_nm: courtyard::DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        h.state_mut().library_editors.documents.get_mut(&id).unwrap().select_many(Default::default()).unwrap();
        h.run();
        assert!(h.state().library_editors.documents[&id].selected.is_none());
        assert!(h.query_by_label("Select courtyard").is_some());
        assert!(h.query_by_label("1 outline · 2.10 × 1.30 mm · 4 vertices").is_some());
    }

    /// With two outlines (generated without replacing), the summary labels its size as the
    /// first outline's, and Select courtyard selects that first outline.
    #[test]
    fn courtyard_summary_with_two_outlines_names_the_first() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let id = app.library_editors.active.unwrap();
        for (revision, clearance_nm) in [(0, courtyard::DEFAULT_CLEARANCE_NM), (1, 500_000)] {
            app.library_editors
                .edit(
                    id,
                    revision,
                    Action::GenerateCourtyard {
                        clearance_nm,
                        source: CourtyardSource::PadsAndBody,
                        stroke_nm: courtyard::DEFAULT_STROKE_NM,
                        replace_existing: false,
                    },
                )
                .unwrap();
        }
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        h.state_mut().library_editors.documents.get_mut(&id).unwrap().select_many(Default::default()).unwrap();
        h.run();
        assert!(h.query_by_label("2 outlines · first of 2: 2.10 × 1.30 mm · 4 vertices").is_some());
        let first = h.state().library_editors.documents[&id].first_courtyard_id().unwrap();
        crate::tests::click(&mut h, "Select courtyard");
        assert_eq!(h.state().library_editors.documents[&id].selected, Some(first));
    }

    /// Snapshot state: Courtyard outline inspector displaying a self-intersection validation error
    /// after clicking Apply.
    #[test]
    fn courtyard_shot_inspector_self_intersection_error() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let id = app.library_editors.active.unwrap();
        app.library_editors
            .edit(
                id,
                0,
                Action::GenerateCourtyard {
                    clearance_nm: courtyard::DEFAULT_CLEARANCE_NM,
                    source: CourtyardSource::PadsAndBody,
                    stroke_nm: courtyard::DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let cy_id = h.state().library_editors.documents[&id].selected.unwrap();
        select_vertex_on_canvas(&mut h, id, cy_id, 0);
        h.get_by_label("Vertex X (mm)").click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Vertex X (mm)").type_text("0.5");
        h.run();
        h.get_by_label("Vertex Y (mm)").click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Vertex Y (mm)").type_text("1.5");
        h.run();
        crate::tests::click(&mut h, "Apply");
        assert_eq!(h.state().library_editors.documents[&id].error.as_deref(), Some("A courtyard outline cannot intersect itself."));
    }

    /// Snapshot state: Courtyard outline inspector with pending edits (Apply enabled, Vertex
    /// choice and Add/Remove vertex buttons disabled).
    #[test]
    fn courtyard_shot_inspector_pending_edits() {
        let (mut h, id, cy_id) = harness_with_generated_courtyard();
        select_vertex_on_canvas(&mut h, id, cy_id, 0);
        // Positive control: with no pending edits the vertex actions and Vertex choice are live.
        assert!(button_disabled(&h, "Apply"));
        assert!(!button_disabled(&h, "Add vertex"));
        assert!(!button_disabled(&h, "Remove vertex"));
        assert!(!button_disabled(&h, "Regenerate…"));
        assert!(h.query_all_by_label("Vertex").all(|node| !node.accesskit_node().is_disabled()));

        type_into(&mut h, "Vertex X (mm)", "-1.5");
        assert!(h.state().library_editors.documents[&id].pending());
        assert!(!button_disabled(&h, "Apply"));
        assert!(button_disabled(&h, "Add vertex"));
        assert!(button_disabled(&h, "Remove vertex"));
        assert!(button_disabled(&h, "Regenerate…"));
        let vertex_choice: Vec<_> = h.query_all_by_label("Vertex").collect();
        assert!(!vertex_choice.is_empty());
        assert!(vertex_choice.iter().all(|node| node.accesskit_node().is_disabled()));
    }

    /// Snapshot state: Courtyard outline inspector after Backspace was refused because a vertex
    /// is selected while coordinate edits are pending; the outline keeps all four vertices.
    #[test]
    fn courtyard_shot_backspace_refused_while_pending() {
        let (mut h, id, cy_id) = harness_with_generated_courtyard();
        select_vertex_on_canvas(&mut h, id, cy_id, 2);
        type_into(&mut h, "Vertex X (mm)", "2");
        let empty = empty_point_outside_edge(&h, id, cy_id, 2);
        click_at(&mut h, empty);
        h.key_press(egui::Key::Backspace);
        h.run();
        let doc = &h.state().library_editors.documents[&id];
        assert!(doc.pending());
        assert_eq!(courtyard::courtyard_polygon(&doc.session.snapshot().objects[&cy_id]).unwrap().0.len(), 4);
        assert!(h.query_by_label("Apply or reset the property fields before removing a vertex.").is_some());
    }

    /// Snapshot state: the pointer hovers the midpoint handle of edge 2 on the selected outline,
    /// which is drawn enlarged in the accent colour; nothing is inserted until a click.
    #[test]
    fn courtyard_shot_hover_midpoint() {
        let (mut h, id, cy_id) = harness_with_generated_courtyard();
        let rev = h.state().library_editors.documents[&id].session.revision();
        let area = h.get_by_label("Library canvas").rect();
        let mid_screen = {
            let doc = &h.state().library_editors.documents[&id];
            let snap = doc.session.snapshot();
            let (pts, _) = courtyard::courtyard_polygon(&snap.objects[&cy_id]).unwrap();
            doc.camera.screen(courtyard::edge_midpoint(pts, 1).unwrap(), area)
        };
        h.hover_at(mid_screen);
        h.run();
        let doc = &h.state().library_editors.documents[&id];
        let snap = doc.session.snapshot();
        let (pts, _) = courtyard::courtyard_polygon(&snap.objects[&cy_id]).unwrap();
        assert_eq!(pick_edge_midpoint(pts, &doc.camera, area, mid_screen), Some(1));
        assert_eq!(pick_vertex(pts, &doc.camera, area, mid_screen), None);
        assert!(!doc.pending() && pts.len() < courtyard::MAX_VERTICES);
        assert!(matches!(doc.tool, canvas::Tool::Select));
        assert_eq!(doc.selection.len(), 1);
        assert_eq!(doc.selected, Some(cy_id));
        assert_eq!(doc.session.revision(), rev);
        assert_eq!(pts.len(), 4);
    }

    /// Snapshot state: expanded Library Properties dialog showing "All vertices" rows.
    #[test]
    fn courtyard_shot_expanded_all_vertices() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let id = app.library_editors.active.unwrap();
        app.library_editors
            .edit(
                id,
                0,
                Action::GenerateCourtyard {
                    clearance_nm: courtyard::DEFAULT_CLEARANCE_NM,
                    source: CourtyardSource::PadsAndBody,
                    stroke_nm: courtyard::DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let area = h.get_by_label("Library canvas").rect();
        h.hover_at(area.center());
        h.run();
        h.key_press(egui::Key::E);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::LibraryProperties(..)));
        assert!(h.query_by_label("Vertex 1 X (mm)").is_some());
    }

    /// Snapshot state: canvas mid-drag of vertex 0 into a self-intersecting position showing the
    /// red invalid-drag warning ring and no outline preview.
    #[test]
    fn courtyard_shot_invalid_vertex_drag() {
        let (mut h, id, cy_id) = harness_with_generated_courtyard();
        let area = h.get_by_label("Library canvas").rect();
        // Vertex 1 (index 0) is the lower-left corner; just above the opposite edge is invalid.
        let invalid = Point { x_nm: 500_000, y_nm: 900_000 };
        let (v0_screen, invalid_screen) = {
            let doc = &h.state().library_editors.documents[&id];
            let snap = doc.session.snapshot();
            let (pts, _) = courtyard::courtyard_polygon(&snap.objects[&cy_id]).unwrap();
            (doc.camera.screen(pts[0], area), doc.camera.screen(invalid, area))
        };
        assert!(area.contains(invalid_screen));
        h.hover_at(v0_screen);
        h.run();
        h.event(egui::Event::PointerButton { pos: v0_screen, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run();
        h.hover_at(invalid_screen);
        h.run();
        let doc = &h.state().library_editors.documents[&id];
        let canvas::Tool::DragCourtyardVertex { vertex, original, .. } = &doc.tool else {
            panic!("expected an active courtyard vertex drag");
        };
        assert_eq!(*vertex, 0);
        // The snapped target is the point the canvas previews. It crosses the edge from vertex 3
        // to vertex 4, so the move is invalid and the canvas paints the warning ring instead.
        let target = doc.camera.snap(invalid_screen, area);
        assert_eq!(target, invalid);
        assert_eq!(courtyard::with_moved_vertex(original, 0, target).unwrap_err(), "A courtyard outline cannot intersect itself.");
        // Positive control: an outward move of the same vertex is valid and would be previewed.
        assert!(courtyard::with_moved_vertex(original, 0, Point { x_nm: -1_500_000, y_nm: -1_000_000 }).is_ok());
        assert_eq!(doc.session.snapshot().objects.get(&cy_id), Some(original));
    }

    /// Pasting a copy of the selected courtyard, as Duplicate does, selects the copy with no
    /// vertex chosen, so Backspace cannot remove a vertex the person never picked on it. An
    /// edit that keeps the same outline selected keeps its chosen vertex.
    #[test]
    fn courtyard_vertex_selection_clears_when_an_edit_selects_another_object() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let id = app.library_editors.active.unwrap();
        app.library_editors
            .edit(
                id,
                0,
                Action::GenerateCourtyard {
                    clearance_nm: courtyard::DEFAULT_CLEARANCE_NM,
                    source: CourtyardSource::PadsAndBody,
                    stroke_nm: courtyard::DEFAULT_STROKE_NM,
                    replace_existing: true,
                },
            )
            .unwrap();
        let doc = app.library_editors.documents.get_mut(&id).unwrap();
        let cy_id = doc.selected.unwrap();
        let snapshot = doc.session.snapshot();
        let original = &snapshot.objects[&cy_id];
        let (points, _) = courtyard::courtyard_polygon(original).unwrap();
        assert!(points.len() > 2);

        // Positive control: an edit of the same outline keeps the chosen vertex.
        doc.selected_vertex = Some(2);
        let candidate = courtyard::with_inserted_vertex(original, 0, courtyard::edge_midpoint(points, 0).unwrap()).unwrap();
        let revision = doc.session.revision();
        doc.execute(revision, Action::Replace { object: cy_id, primitive: candidate }).unwrap();
        assert_eq!(doc.selected, Some(cy_id));
        assert_eq!(doc.selected_vertex, Some(2));

        let copy = doc.session.snapshot().objects[&cy_id].clone();
        let revision = doc.session.revision();
        doc.execute(revision, Action::Paste { primitives: vec![copy], offset: Point { x_nm: 1_000_000, y_nm: 0 } }).unwrap();
        let pasted = doc.selected.unwrap();
        assert_ne!(pasted, cy_id);
        assert!(courtyard::courtyard_polygon(&doc.session.snapshot().objects[&pasted]).is_some());
        assert_eq!(doc.selected_vertex, None);
    }
}
