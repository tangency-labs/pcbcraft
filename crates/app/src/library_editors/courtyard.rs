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
        if !(1..=2_000_000).contains(&stroke_nm) {
            return Err("Stroke (mm) must be between 0.001 and 2 mm.".into());
        }
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
        Ok(format!("Result: {} × {} mm ({} vertices)", format_mm(m.width_nm), format_mm(m.height_nm), m.vertex_count))
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
        if let Some(first_id) = courtyard_ids.first().copied() {
            let count = courtyard_ids.len();
            let outline_word = if count == 1 { "outline" } else { "outlines" };
            let shape_word = if count == 1 { "shape" } else { "shapes" };
            let summary = match item.objects.get(&first_id).and_then(courtyard::courtyard_polygon) {
                Some((points, _)) => match courtyard::metrics(points) {
                    Some(m) => {
                        format!("{count} {outline_word} · {} × {} mm ({} vertices)", format_mm(m.width_nm), format_mm(m.height_nm), m.vertex_count)
                    }
                    None => format!("{count} courtyard {outline_word}"),
                },
                None => format!("{count} courtyard {shape_word}"),
            };
            ui.label(shell::dim(summary));
        } else {
            ui.label(shell::dim("No courtyard outline"));
        }
        let mut show = self.show_courtyard;
        if controls::checkbox(ui, &mut show, "Show courtyard layer").changed() {
            self.set_show_courtyard(show);
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
        let active_vertex = self.selected_vertex.filter(|&i| i < count).unwrap_or(0);
        self.selected_vertex = Some(active_vertex);

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
        let mut show = self.show_courtyard;
        if controls::checkbox(ui, &mut show, "Show courtyard layer").changed() {
            self.set_show_courtyard(show);
        }
        ui.add_space(Tokens::get().space_4);

        let mut chosen = active_vertex.to_string();
        let options: Vec<(String, String)> = (0..count).map(|i| (i.to_string(), format!("Vertex {}", i + 1))).collect();
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
        if !pending
            && let Ok(parsed) = chosen.parse::<usize>()
            && parsed < count
        {
            self.selected_vertex = Some(parsed);
        }
        let vertex_index = self.selected_vertex.unwrap_or(0).min(count.saturating_sub(1));

        let apply_payload = if let Some(draft) = self.draft.as_mut() {
            if expanded {
                properties::section(ui, "All vertices");
                for i in 0..count {
                    if draft.show_courtyard_vertex_row(ui, i) {
                        enter_apply = true;
                    }
                }
            } else if draft.show_courtyard_vertex(ui, vertex_index) {
                enter_apply = true;
            }
            let dirty = draft.dirty();
            let mut apply_clicked = false;
            ui.horizontal(|ui| {
                add_vertex = controls::enabled_button(ui, !dirty && count < courtyard::MAX_VERTICES, "Add vertex")
                    .on_hover_text("Insert a vertex at the exact midpoint of the next edge")
                    .clicked();
                remove_vertex = controls::enabled_button(ui, !dirty && count > courtyard::MIN_VERTICES, "Remove vertex")
                    .on_hover_text("Remove the selected vertex (Backspace)")
                    .clicked();
            });
            ui.horizontal(|ui| {
                apply_clicked = controls::enabled_button(ui, dirty, "Apply").clicked() || (enter_apply && dirty);
                reset_clicked = controls::button(ui, "Reset").clicked();
                regenerate = controls::enabled_button(ui, !dirty, "Regenerate…").clicked();
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
        } else if add_vertex {
            let result = courtyard::edge_midpoint(points, vertex_index)
                .ok_or_else(|| "Unable to compute edge midpoint.".to_owned())
                .and_then(|mid| courtyard::with_inserted_vertex(primitive, vertex_index, mid))
                .and_then(|candidate| self.execute(self.session.revision(), Action::Replace { object: object_id, primitive: candidate }));
            match result {
                Ok(()) => self.selected_vertex = Some(vertex_index + 1),
                Err(error) => self.error = Some(error),
            }
        } else if remove_vertex {
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
                apply |= ui.add(controls::action("Generate", true)).clicked();
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
                draft.error = Some(error);
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
    use egui_kittest::{Harness, kittest::Queryable};

    fn click_at(h: &mut Harness<'_, PcbApp>, pos: Pos2) {
        h.hover_at(pos);
        h.run();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run();
        }
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

    /// Generate Courtyard dialog rejects invalid clearance, clears stale errors when edited,
    /// generates the default 0.25 mm outline, and inserts an exact midpoint without changing area.
    #[test]
    fn generate_courtyard_dialog_creates_editable_outline_and_protects_invalid_input() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let id = h.state().library_editors.active.unwrap();
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

        // Properties panel now displays the Courtyard outline editor with vertex controls.
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

        // Re-enable via Properties checkbox (clear selection first so Footprint summary is shown).
        h.state_mut().library_editors.documents.get_mut(&id).unwrap().select_many(Default::default()).unwrap();
        h.run();
        crate::tests::click(&mut h, "Show courtyard layer");
        assert!(h.state().library_editors.documents[&id].show_courtyard);
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), rev);

        // Positive control with the layer visible: the exact same marquee and Cmd+A select cy_id.
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

    /// Pending property edits refuse switching vertices or activating C (without turning the
    /// courtyard layer on), while Apply commits atomically and supports Undo.
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

        h.get_by_label("Vertex X (mm)").click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Vertex X (mm)").type_text("-1.5");
        h.run();
        assert!(h.state().library_editors.documents[&id].pending());

        // Clicking vertex 1 on the canvas while edits are pending refuses to switch vertex.
        let area = h.get_by_label("Library canvas").rect();
        let v1_screen = {
            let doc = &h.state().library_editors.documents[&id];
            let snap = doc.session.snapshot();
            let (pts, _) = courtyard::courtyard_polygon(&snap.objects[&selected]).unwrap();
            doc.camera.screen(pts[1], area)
        };
        click_at(&mut h, v1_screen);
        assert_eq!(h.state().library_editors.documents[&id].selected_vertex, Some(0));

        // Pressing C while edits are pending refuses to start the tool and keeps show_courtyard unchanged.
        h.state_mut().library_editors.documents.get_mut(&id).unwrap().show_courtyard = false;
        h.hover_at(area.center());
        h.run();
        h.key_press(egui::Key::C);
        h.run();
        assert!(!h.state().library_editors.documents[&id].show_courtyard);
        assert!(matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Select));
        h.state_mut().library_editors.documents.get_mut(&id).unwrap().show_courtyard = true;

        crate::tests::click(&mut h, "Apply");
        let snap = h.state().library_editors.documents[&id].session.snapshot();
        let (pts, _) = courtyard::courtyard_polygon(&snap.objects[&selected]).unwrap();
        assert_eq!(pts[0].x_nm, -1_500_000);

        crate::tests::click(&mut h, "Undo Library Edit");
        let undone = h.state().library_editors.documents[&id].session.snapshot();
        let (pts, _) = courtyard::courtyard_polygon(&undone.objects[&selected]).unwrap();
        assert_eq!(pts[0].x_nm, -1_050_000);
    }

    /// Drive the footprint canvas directly for C draw (Escape + Enter), vertex dragging with grid
    /// snap and one-step undo, midpoint click insertion, and Backspace vertex removal.
    #[test]
    fn courtyard_canvas_draw_drag_midpoint_and_backspace_interactions() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let id = h.state().library_editors.active.unwrap();
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
                doc.camera.screen(Point { x_nm: -2_000_000, y_nm: -1_000_000 }, area),
                doc.camera.screen(Point { x_nm: 2_000_000, y_nm: -1_000_000 }, area),
                doc.camera.screen(Point { x_nm: 0, y_nm: 2_000_000 }, area),
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

        // Click edge midpoint handle: inserts exact midpoint without changing area.
        click_at(&mut h, mid_screen);
        let after_mid = h.state().library_editors.documents[&id].session.snapshot();
        let (mid_pts, _) = courtyard::courtyard_polygon(&after_mid.objects[&cy_id]).unwrap();
        assert_eq!(mid_pts.len(), 5);
        assert_eq!(courtyard::metrics(mid_pts).unwrap().area_nm2, before_area);

        // Press Backspace over canvas: removes the selected 5th vertex back to 4.
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

    /// Generate Courtyard dialog refuses to apply when the document revision changed underneath it.
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

        crate::tests::click(&mut h, "Generate");
        assert!(matches!(h.state().dialog, Dialog::LibraryCourtyard(..)));
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), rev_after_bg);
    }
}
