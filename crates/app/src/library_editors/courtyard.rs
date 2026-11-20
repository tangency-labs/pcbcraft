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

    /// Render the compact Courtyard summary section inside Footprint Definition properties
    /// (when no individual primitive is selected).
    pub(super) fn courtyard_summary_section(&mut self, ui: &mut Ui) {
        let item = self.session.snapshot();
        let courtyard_ids: Vec<u64> = item.objects.iter().filter(|(_, p)| courtyard::is_courtyard_primitive(p)).map(|(id, _)| *id).collect();
        properties::section(ui, "Courtyard");
        if let Some(first_id) = courtyard_ids.first().copied() {
            let summary = match item.objects.get(&first_id).and_then(courtyard::courtyard_polygon) {
                Some((points, _)) => match courtyard::metrics(points) {
                    Some(m) => format!(
                        "{} outline · {} × {} mm ({} vertices)",
                        courtyard_ids.len(),
                        format_mm(m.width_nm),
                        format_mm(m.height_nm),
                        m.vertex_count
                    ),
                    None => format!("{} courtyard outline", courtyard_ids.len()),
                },
                None => format!("{} courtyard shape", courtyard_ids.len()),
            };
            ui.label(shell::dim(summary));
        } else {
            ui.label(shell::dim("No courtyard outline"));
        }
        controls::checkbox(ui, &mut self.show_courtyard, "Show courtyard layer");
        ui.horizontal(|ui| {
            if controls::enabled_button(ui, !self.pending(), "Generate courtyard…").clicked() {
                self.courtyard_requested = true;
            }
            if let Some(first_id) = self.first_courtyard_id()
                && controls::enabled_button(ui, !self.pending(), "Select courtyard").clicked()
            {
                self.show_courtyard = true;
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
        controls::checkbox(ui, &mut self.show_courtyard, "Show courtyard layer");
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

        properties::section(ui, "Vertex");
        properties::choice(ui, "Vertex", egui::Id::unique(("courtyard_vertex", object_id)), &mut chosen, &option_refs);
        if let Ok(parsed) = chosen.parse::<usize>()
            && parsed < count
        {
            self.selected_vertex = Some(parsed);
        }
        let vertex_index = self.selected_vertex.unwrap_or(0).min(count.saturating_sub(1));

        let apply_payload = if let Some(draft) = self.draft.as_mut() {
            if draft.show_courtyard_vertex(ui, vertex_index) {
                enter_apply = true;
            }
            if expanded && count > 1 {
                properties::section(ui, "All vertices");
                for i in 0..count {
                    if i == vertex_index {
                        continue;
                    }
                    if draft.show_courtyard_vertex_row(ui, i) {
                        enter_apply = true;
                    }
                }
            }
            let dirty = draft.dirty();
            let mut apply_clicked = false;
            ui.horizontal(|ui| {
                add_vertex = controls::enabled_button(ui, !dirty && count < courtyard::MAX_VERTICES, "Add vertex")
                    .on_hover_text("Insert a vertex at the midpoint of the next edge")
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
            let result =
                courtyard::edge_midpoint(points, vertex_index).ok_or_else(|| "Unable to compute edge midpoint.".to_owned()).and_then(|mid| {
                    let snapped = Point { x_nm: snap_to_grid(mid.x_nm, self.camera.snap_nm()), y_nm: snap_to_grid(mid.y_nm, self.camera.snap_nm()) };
                    let candidate = courtyard::with_inserted_vertex(primitive, vertex_index, snapped)
                        .or_else(|_| courtyard::with_inserted_vertex(primitive, vertex_index, mid))?;
                    self.execute(self.session.revision(), Action::Replace { object: object_id, primitive: candidate })
                });
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

/// Snap a single nanometer coordinate to the nearest multiple of `grid_nm` when positive.
pub(super) fn snap_to_grid(value_nm: i64, grid_nm: i64) -> i64 {
    if grid_nm <= 0 {
        return value_nm;
    }
    ((value_nm as f64 / grid_nm as f64).round() as i64).saturating_mul(grid_nm)
}

impl PcbApp {
    /// Generate a closed courtyard polygon from the footprint's pads and optional body graphics.
    /// Applies to the captured definition/view/revision; Cancel or Escape leaves the document untouched.
    pub(crate) fn library_courtyard_dialog(&mut self, ctx: &Context, mut draft: Box<CourtyardDraft>) {
        let mut apply = false;
        let mut cancel = false;
        let preview = self.library_editors.documents.get(&draft.document).map(|doc| draft.preview_summary(&doc.session.snapshot()));
        let dismissed = shell::dialog(ctx, "library_courtyard", "Generate Courtyard", |ui| {
            properties::choice(
                ui,
                "Source",
                egui::Id::unique("courtyard_source"),
                &mut draft.source,
                &[("pads_and_body", "Pads and body".into()), ("pads_only", "Pads only".into())],
            );
            let clearance_resp = properties::text(ui, "Clearance (mm)", &mut draft.clearance);
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
            if stroke_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                apply = true;
            }
            if draft.has_existing {
                controls::checkbox(ui, &mut draft.replace_existing, "Replace existing courtyard");
            }
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

    /// Generate Courtyard dialog parses clearance presets, rejects invalid clearance, generates
    /// a closed courtyard polygon in one undo step, and switches Properties to the courtyard panel.
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

        // Pick IPC-7351 Most density preset (0.50 mm) and generate.
        crate::tests::click(&mut h, "0.50 mm");
        crate::tests::click(&mut h, "Generate");
        assert!(matches!(h.state().dialog, Dialog::None));

        let snap = h.state().library_editors.documents[&id].session.snapshot();
        let selected = h.state().library_editors.documents[&id].selected.unwrap();
        let (pts, stroke) = courtyard::courtyard_polygon(&snap.objects[&selected]).unwrap();
        assert_eq!(stroke, courtyard::DEFAULT_STROKE_NM);
        assert_eq!(courtyard::metrics(pts).unwrap().width_nm, 2_600_000);
        assert_eq!(courtyard::metrics(pts).unwrap().height_nm, 1_800_000);

        // Properties panel now displays the Courtyard outline editor with vertex controls.
        assert!(h.query_by_label("Vertex X (mm)").is_some());
        assert!(h.query_by_label("Vertex Y (mm)").is_some());
        assert!(h.query_by_label("Add vertex").is_some());
        assert!(h.query_by_label("Remove vertex").is_some());

        // Add vertex inserts a 5th vertex in one undo step; Remove vertex restores 4 vertices.
        crate::tests::click(&mut h, "Add vertex");
        let after_add = h.state().library_editors.documents[&id].session.snapshot();
        assert_eq!(courtyard::courtyard_polygon(&after_add.objects[&selected]).unwrap().0.len(), 5);

        crate::tests::click(&mut h, "Remove vertex");
        let after_remove = h.state().library_editors.documents[&id].session.snapshot();
        assert_eq!(courtyard::courtyard_polygon(&after_remove.objects[&selected]).unwrap().0.len(), 4);
    }

    /// Courtyard layer visibility toggle hides and restores the courtyard layer without editing
    /// the footprint document.
    #[test]
    fn courtyard_layer_toggle_does_not_mutate_document_revision() {
        let mut app = PcbApp::default();
        app.new_library_item(Kind::Footprint);
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let id = h.state().library_editors.active.unwrap();
        assert!(h.state().library_editors.documents[&id].show_courtyard);
        let rev = h.state().library_editors.documents[&id].session.revision();

        crate::tests::click(&mut h, "Show courtyard layer");
        assert!(!h.state().library_editors.documents[&id].show_courtyard);
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), rev);

        crate::tests::click(&mut h, "Show courtyard layer");
        assert!(h.state().library_editors.documents[&id].show_courtyard);
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), rev);
    }

    /// Editing a vertex coordinate in the Courtyard Properties panel validates atomically and
    /// supports undo.
    #[test]
    fn courtyard_vertex_coordinate_edit_validates_and_supports_undo() {
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
        crate::tests::click(&mut h, "Apply");

        let snap = h.state().library_editors.documents[&id].session.snapshot();
        let (pts, _) = courtyard::courtyard_polygon(&snap.objects[&selected]).unwrap();
        assert_eq!(pts[0].x_nm, -1_500_000);

        crate::tests::click(&mut h, "Undo Library Edit");
        let undone = h.state().library_editors.documents[&id].session.snapshot();
        let (pts, _) = courtyard::courtyard_polygon(&undone.objects[&selected]).unwrap();
        assert_eq!(pts[0].x_nm, -1_050_000);
    }
}
