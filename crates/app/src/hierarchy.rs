//! Hierarchy presentation submits the same revision-checked commands as MCP.

use crate::{
    Dialog, PcbApp, ViewInput,
    editor_view::DocumentTarget,
    schematic_canvas::{Camera, Click, Tool},
};
use egui::{Align2, FontId, Key, Painter, Pos2, Rect, Response, Stroke, Ui, Vec2};
use pcbcraft_chrome::{MenuItem, Tokens, shell, theme};
use pcbcraft_engine::{
    Project, SheetTarget,
    hierarchy::{self as engine, Direction},
    schematic::{GRID_NM, Point},
};
use serde_json::json;

/// Canvas objects that own hierarchy commands, independently of component selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Sheet,
    Label,
}

/// Actions captured by the canvas and validated by its owning view before execution.
#[derive(Clone, Copy, Debug)]
pub(super) enum Action {
    Enter,
    Edit,
    Move,
    Delete,
}

/// A corner keeps the opposite edges fixed during a sheet resize.
#[derive(Clone, Copy, Debug)]
pub(super) struct Corner {
    left: bool,
    top: bool,
}

/// One complete authored rectangle, submitted atomically after a valid gesture ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Geometry {
    pub at: Point,
    pub size: Point,
}

/// View-local pickup state retains the original grab point and immutable document geometry.
#[derive(Clone, Debug)]
pub(super) struct Drag {
    pub block: engine::SheetSymbol,
    start: Point,
    size: Point,
    pub corner: Option<Corner>,
}

impl Drag {
    /// Snap pointer travel while retaining the initial offset. Resizing clamps at interface
    /// clearance instead of flipping the rectangle through its opposite edges.
    pub fn geometry(&self, project: &Project, camera: &Camera, rect: Rect, pointer: Pos2) -> Geometry {
        let point = camera.snap(pointer, rect);
        let Some(corner) = self.corner else {
            return Geometry {
                at: Point { x_nm: self.block.at.x_nm + point.x_nm - self.start.x_nm, y_nm: self.block.at.y_nm + point.y_nm - self.start.y_nm },
                size: self.size,
            };
        };
        let min = project.sheet_symbol_min_size(&self.block);
        let right = self.block.at.x_nm + self.size.x_nm;
        let bottom = self.block.at.y_nm + self.size.y_nm;
        let x = if corner.left { point.x_nm.min(right - min.x_nm) } else { self.block.at.x_nm };
        let y = if corner.top { point.y_nm.min(bottom - min.y_nm) } else { self.block.at.y_nm };
        let right = if corner.left { right } else { point.x_nm.max(x + min.x_nm) };
        let bottom = if corner.top { bottom } else { point.y_nm.max(y + min.y_nm) };
        Geometry { at: Point { x_nm: x, y_nm: y }, size: Point { x_nm: right - x, y_nm: bottom - y } }
    }

    /// Use the engine's validated geometry for preview, retaining electrical endpoint identity.
    pub fn preview(&self, project: &Project, geometry: Geometry) -> Option<Project> {
        let address = format!("sheet-symbol-{}", self.block.id);
        if self.corner.is_some() {
            project.resized_sheet_symbol(&address, geometry.at, geometry.size).ok()
        } else {
            project.moved_hierarchy(&address, geometry.at).ok()
        }
    }
}

/// A modal interface edit retains its originating document/revision across native hosts.
#[derive(Debug)]
pub(super) struct Edit {
    source: ViewInput,
    sheet: SheetTarget,
    kind: Kind,
    id: Option<u64>,
    at: Point,
    name: String,
    direction: Direction,
    child: Option<SheetTarget>,
    error: Option<String>,
    focused: bool,
}

/// Present directions as UI labels while commands retain their stable enum spellings.
fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Input => "Input",
        Direction::Output => "Output",
        Direction::Bidirectional => "Bidirectional",
        Direction::TriState => "Tri-state",
        Direction::Passive => "Passive",
    }
}

/// Paint native hierarchy geometry with shared chrome colors, metrics and typography.
pub(super) fn paint(
    painter: &Painter,
    rect: Rect,
    camera: &Camera,
    project: &Project,
    tool: Option<&Tool>,
    pointer: Option<Pos2>,
    selected: Option<u64>,
) {
    let tokens = Tokens::get();
    let moving = match tool {
        Some(Tool::Hierarchy { kind, id, offset, .. }) => Some((*kind, *id, pointer.map(|p| camera.snap(p + *offset, rect)))),
        _ => None,
    };
    for block in project.sheet_symbols() {
        let at = moving.filter(|(kind, id, _)| *kind == Kind::Sheet && *id == Some(block.id)).and_then(|(_, _, at)| at).unwrap_or(block.at);
        let size = project.sheet_symbol_size(block);
        let bounds = Rect::from_min_max(camera.screen(at, rect), camera.screen(Point { x_nm: at.x_nm + size.x_nm, y_nm: at.y_nm + size.y_nm }, rect));
        let color = if selected == Some(block.id) { tokens.accent } else { tokens.text };
        let stroke = Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, color);
        painter.rect_stroke(bounds, 0, stroke, egui::StrokeKind::Middle);
        paint_bounded_text(
            painter,
            bounds.left_top() - egui::vec2(0., tokens.space_4),
            project.child_name(block),
            FontId::proportional(tokens.font_button),
            color,
            bounds.width(),
            Align2::LEFT_BOTTOM,
        );
        for pin in project.sheet_pins(block) {
            let point = Point { x_nm: pin.at.x_nm + at.x_nm - block.at.x_nm, y_nm: pin.at.y_nm + at.y_nm - block.at.y_nm };
            let pos = camera.screen(point, rect);
            let right = pin.label.direction.right();
            let dx = if right { -tokens.space_8 } else { tokens.space_8 };
            painter.line_segment([pos, pos + egui::vec2(dx, 0.)], stroke);
            paint_bounded_text(
                painter,
                pos + egui::vec2(dx * 1.5, 0.),
                &pin.label.name,
                FontId::proportional(tokens.font_small),
                tokens.text,
                (bounds.width() / 2. - tokens.space_16).max(0.),
                if right { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER },
            );
        }
        if (selected == Some(block.id) || pointer.is_some_and(|p| bounds.expand(theme::SCHEMATIC_HANDLE_HIT_RADIUS).contains(p)))
            && matches!(tool, None | Some(Tool::SheetGesture { .. }))
        {
            for (pos, _) in corners(bounds) {
                painter.rect(
                    Rect::from_center_size(pos, Vec2::splat(2. * theme::SCHEMATIC_HANDLE_HALF_SIZE)),
                    0,
                    tokens.panel,
                    Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, tokens.accent),
                    egui::StrokeKind::Middle,
                );
            }
        }
    }
    for label in project.hierarchical_labels() {
        let at = moving.filter(|(kind, id, _)| *kind == Kind::Label && *id == Some(label.id)).and_then(|(_, _, at)| at).unwrap_or(label.at);
        paint_label(painter, camera.screen(at, rect), &label.name, label.direction, tokens.schematic_wire);
    }
    if let Some((kind, None, Some(at))) = moving {
        let pos = camera.screen(at, rect);
        if kind == Kind::Sheet {
            let far = camera.screen(Point { x_nm: at.x_nm + 16 * GRID_NM, y_nm: at.y_nm + 10 * GRID_NM }, rect);
            painter.rect_stroke(
                Rect::from_two_pos(pos, far),
                0,
                Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, tokens.accent),
                egui::StrokeKind::Middle,
            );
        } else {
            paint_label(painter, pos, "Label", Direction::Input, tokens.accent);
        }
    }
}

/// Keep pin names on their own half of the symbol, with ellipses at small zoom levels.
#[allow(clippy::too_many_arguments)]
fn paint_bounded_text(painter: &Painter, at: Pos2, text: &str, font: FontId, color: egui::Color32, width: f32, align: Align2) {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap.max_width = width;
    job.wrap.max_rows = 1;
    let galley = painter.layout_job(job);
    painter.galley(align.anchor_size(at, galley.size()).min, galley, color);
}

/// Derive the same screen rectangle for outlines, selection, title hit tests and handles.
fn block_rect(project: &Project, block: &engine::SheetSymbol, camera: &Camera, rect: Rect) -> Rect {
    let size = project.sheet_symbol_size(block);
    Rect::from_min_max(camera.screen(block.at, rect), camera.screen(Point { x_nm: block.at.x_nm + size.x_nm, y_nm: block.at.y_nm + size.y_nm }, rect))
}

/// Include the external sheet name in body selection and movement, without moving the outline.
fn body_rect(bounds: Rect) -> Rect {
    let t = Tokens::get();
    Rect::from_min_max(bounds.min - egui::vec2(0., t.font_button + t.space_8), bounds.max)
}

/// All four corners use the same geometry for drawing and pointer hit testing.
fn corners(rect: Rect) -> [(Pos2, Corner); 4] {
    [
        (rect.left_top(), Corner { left: true, top: true }),
        (rect.right_top(), Corner { left: false, top: true }),
        (rect.left_bottom(), Corner { left: true, top: false }),
        (rect.right_bottom(), Corner { left: false, top: false }),
    ]
}

/// The interface anchor is the pointed end; direction also appears in the properties dialog.
pub(super) fn paint_label(painter: &Painter, pos: Pos2, name: &str, direction: Direction, color: egui::Color32) {
    let t = Tokens::get();
    let sign = if direction.right() { 1. } else { -1. };
    let back = pos + egui::vec2(sign * t.space_8, 0.);
    painter.add(egui::Shape::line(
        vec![back - egui::vec2(0., t.space_4), pos, back + egui::vec2(0., t.space_4)],
        Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, color),
    ));
    painter.text(
        back + egui::vec2(sign * t.space_4, 0.),
        if sign < 0. { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER },
        name,
        FontId::proportional(t.font_button),
        color,
    );
}

/// Resolve an editor-local hierarchy hover for clipboard keys, using current view geometry and
/// filters. The caller separately checks canvas input ownership and explicit selection priority.
pub(super) fn clipboard_hover(project: &Project, camera: &Camera, ctx: &egui::Context, filters: crate::selection_filters::Filters) -> Option<String> {
    let rect = camera.schematic_rect()?;
    let pointer = ctx.input(|i| i.pointer.hover_pos())?;
    if !rect.contains(pointer) {
        return None;
    }
    hit(project, camera, rect, pointer, filters).map(|(kind, id, _)| format!("{}-{id}", if kind == Kind::Sheet { "sheet-symbol" } else { "label" }))
}

/// Hit label text before block interiors; pin anchors retain wire-target priority on plain clicks.
fn hit(project: &Project, camera: &Camera, rect: Rect, pointer: Pos2, filters: crate::selection_filters::Filters) -> Option<(Kind, u64, Point)> {
    let t = Tokens::get();
    for label in project.hierarchical_labels().iter().rev().filter(|_| filters.labels) {
        let pos = camera.screen(label.at, rect);
        let width = (label.name.chars().count() as f32 * t.font_button).max(t.space_24);
        let offset = if label.direction.right() { 0. } else { -width };
        if Rect::from_min_size(pos + egui::vec2(offset, -t.space_8), egui::vec2(width, t.space_16)).contains(pointer) {
            return Some((Kind::Label, label.id, label.at));
        }
    }
    for block in project.sheet_symbols().iter().rev().filter(|_| filters.sheets) {
        if body_rect(block_rect(project, block, camera, rect)).contains(pointer) {
            return Some((Kind::Sheet, block.id, block.at));
        }
    }
    None
}

/// Let a hovered hierarchy object handle its edit/delete shortcuts before an older component
/// selection. The canvas repeats the hit test against current layout before issuing an action.
pub(super) fn hovered(project: &Project, camera: &Camera, pointer: Pos2, filters: crate::selection_filters::Filters) -> bool {
    camera.schematic_rect().is_some_and(|rect| rect.contains(pointer) && hit(project, camera, rect, pointer, filters).is_some())
}

/// Consume hierarchy placement, body/corner drags, shortcuts and menus before component selection.
/// A nested result distinguishes an owned menu with no action from unrelated canvas input.
#[allow(clippy::too_many_arguments)]
pub(super) fn input(
    ui: &mut Ui,
    response: &Response,
    project: &Project,
    camera: &Camera,
    rect: Rect,
    tool: Option<&Tool>,
    enabled: bool,
    selected: Option<u64>,
    filters: crate::selection_filters::Filters,
) -> Option<Option<Click>> {
    if let Some(Tool::SheetGesture { drag, .. }) = tool {
        let lost = ui.input(|i| i.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))));
        if lost || response.secondary_clicked() {
            return Some(Some(Click::FinishSheetGesture(None)));
        }
        if ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary)) {
            let geometry = ui
                .input(|i| i.pointer.hover_pos())
                .filter(|p| enabled && rect.contains(*p))
                .map(|p| drag.geometry(project, camera, rect, p))
                .filter(|g| drag.preview(project, *g).is_some());
            return Some(Some(Click::FinishSheetGesture(geometry)));
        }
        return Some(None);
    }
    if let Some(Tool::Hierarchy { offset, .. }) = tool {
        if !enabled {
            return Some(None);
        }
        let point = response.hover_pos().map(|p| camera.snap(p + *offset, rect));
        response.clone().on_hover_cursor(egui::CursorIcon::Crosshair);
        return Some(
            point
                .filter(|p| {
                    (-pcbcraft_engine::schematic::MAX_CENTER_NM..=pcbcraft_engine::schematic::MAX_CENTER_NM).contains(&p.x_nm)
                        && (-pcbcraft_engine::schematic::MAX_CENTER_NM..=pcbcraft_engine::schematic::MAX_CENTER_NM).contains(&p.y_nm)
                })
                .filter(|_| response.clicked())
                .map(Click::HierarchyPlace),
        );
    }
    if tool.is_some() {
        return None;
    }
    if enabled
        && filters.sheets
        && response.drag_started_by(egui::PointerButton::Primary)
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
    {
        for block in project.sheet_symbols().iter().rev() {
            let bounds = block_rect(project, block, camera, rect);
            let corner = corners(bounds)
                .into_iter()
                .find(|(pos, _)| pos.distance_sq(origin) <= theme::SCHEMATIC_HANDLE_HIT_RADIUS.powi(2))
                .map(|(_, corner)| corner);
            let pin = project.sheet_pins(block).any(|pin| camera.screen(pin.at, rect).distance_sq(origin) <= theme::SCHEMATIC_PIN_HIT_RADIUS.powi(2));
            if corner.is_some() || (body_rect(bounds).contains(origin) && !pin) {
                let drag = Drag { block: block.clone(), start: camera.snap(origin, rect), size: project.sheet_symbol_size(block), corner };
                return Some(Some(Click::GrabSheetGesture(Box::new(drag))));
            }
        }
    }
    if enabled && let Some(pointer) = response.hover_pos() {
        for block in project.sheet_symbols() {
            for pin in project.sheet_pins(block) {
                if camera.screen(pin.at, rect).distance_sq(pointer) <= theme::SCHEMATIC_PIN_HIT_RADIUS.powi(2) {
                    response.clone().on_hover_text(format!("{} · {}", pin.label.name, direction_name(pin.label.direction)));
                }
            }
        }
    }
    let menu_id = response.id.with("hierarchy_target");
    if response.secondary_clicked() {
        let target = response.interact_pointer_pos().and_then(|p| hit(project, camera, rect, p, filters));
        ui.data_mut(|data| match target {
            Some(value) => {
                data.insert_temp(menu_id, value);
            }
            None => data.remove::<(Kind, u64, Point)>(menu_id),
        });
    }
    if (enabled || response.context_menu_opened())
        && let Some((kind, id, _)) = ui.data(|data| data.get_temp::<(Kind, u64, Point)>(menu_id))
        && (if kind == Kind::Sheet { filters.sheets } else { filters.labels })
        && let Some(menu) = pcbcraft_chrome::menu::context(response, |ui| {
            let rows = if kind == Kind::Sheet {
                vec![
                    ("Enter Sheet", "", Action::Enter),
                    ("Rename Sheet…", "E", Action::Edit),
                    ("Move", "M", Action::Move),
                    ("Remove Sheet Symbol", "Del", Action::Delete),
                ]
            } else {
                vec![("Properties…", "E", Action::Edit), ("Move", "M", Action::Move), ("Delete Hierarchical Label", "Del", Action::Delete)]
            };
            if let Some(action) = rows
                .into_iter()
                .find_map(|(name, key, action)| MenuItem::new(name).shortcut(key).show(ui).then_some(Click::HierarchyAction(kind, id, action)))
            {
                return Some(action);
            }
            pcbcraft_chrome::menu::separator(ui);
            [
                ("Copy", "Ctrl+C", crate::schematic_canvas::ToolAction::Copy),
                ("Cut", "Ctrl+X", crate::schematic_canvas::ToolAction::Cut),
                ("Duplicate", "Ctrl+D", crate::schematic_canvas::ToolAction::Duplicate),
            ]
            .into_iter()
            .find_map(|(label, key, action)| {
                MenuItem::new(label).shortcut(key).show(ui).then(|| Click::HierarchyClipboard(kind, id, action, ui.ctx().clone()))
            })
        })
    {
        return Some(menu.inner);
    }
    if !enabled || ui.ctx().egui_wants_keyboard_input() {
        return None;
    }
    if let Some(pointer) = response.hover_pos().filter(|_| filters.sheets) {
        let corner = project
            .sheet_symbols()
            .iter()
            .rev()
            .flat_map(|block| corners(block_rect(project, block, camera, rect)))
            .find(|(pos, _)| pos.distance_sq(pointer) <= theme::SCHEMATIC_HANDLE_HIT_RADIUS.powi(2));
        if let Some((_, corner)) = corner {
            ui.ctx().set_cursor_icon(if corner.left == corner.top { egui::CursorIcon::ResizeNwSe } else { egui::CursorIcon::ResizeNeSw });
        }
    }
    let hovered = response.hover_pos().and_then(|p| hit(project, camera, rect, p, filters));
    let (kind, id, _) = hovered.or_else(|| {
        selected.filter(|_| filters.sheets).and_then(|id| project.sheet_symbols().iter().find(|s| s.id == id).map(|s| (Kind::Sheet, id, s.at)))
    })?;
    let action = if hovered.is_some() && kind == Kind::Sheet && response.double_clicked() {
        Some(Action::Enter)
    } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::E)) {
        Some(Action::Edit)
    } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::M)) {
        Some(Action::Move)
    } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Delete) || i.consume_key(egui::Modifiers::NONE, Key::Backspace)) {
        Some(Action::Delete)
    } else {
        None
    };
    if let Some(action) = action {
        return Some(Some(Click::HierarchyAction(kind, id, action)));
    }
    if hovered.is_some()
        && kind == Kind::Sheet
        && response.clicked()
        && let Some(pointer) = response.hover_pos()
        && project.sheet_symbols().iter().find(|b| b.id == id).is_some_and(|block| {
            !project.sheet_pins(block).any(|pin| camera.screen(pin.at, rect).distance_sq(pointer) <= theme::SCHEMATIC_PIN_HIT_RADIUS.powi(2))
        })
    {
        return Some(Some(Click::SelectSheetSymbol(id)));
    }
    None
}

impl PcbApp {
    /// Begin a revision-bound placement only for an available schematic document.
    pub(super) fn begin_hierarchy(&mut self, kind: Kind) {
        if self.views.active.snapshot(&self.session).is_none() {
            return;
        }
        self.views.active.schematic.tool = Some(Tool::Hierarchy { kind, id: None, offset: Vec2::ZERO, revision: self.session.revision() });
    }

    /// Navigate the originating pane; each view keeps its own sheet and camera.
    pub(super) fn leave_sheet(&mut self) {
        let Some(project) = self.views.active.snapshot(&self.session) else {
            return;
        };
        if let Some(parent) = project.parent_sheet(project.sheet_target()) {
            self.views.open_document(self.views.active.id(), DocumentTarget::Sheet(parent), &self.session);
        }
    }

    /// Handle a captured object action after the canvas owner has rejected stale input.
    pub(super) fn hierarchy_action(&mut self, source: ViewInput, kind: Kind, id: u64, action: Action) {
        let Some(project) = self.views.active.snapshot(&self.session) else {
            return;
        };
        let at = match kind {
            Kind::Sheet => project.sheet_symbols().iter().find(|s| s.id == id).map(|s| s.at),
            Kind::Label => project.hierarchical_labels().iter().find(|l| l.id == id).map(|l| l.at),
        };
        let Some(at) = at else {
            return;
        };
        match action {
            Action::Enter => {
                if let Some(child) = project.child_sheet(id) {
                    self.views.open_document(source.view(), DocumentTarget::Sheet(child), &self.session);
                }
            }
            Action::Edit => {
                if kind == Kind::Sheet {
                    if let Some(child) = project.child_sheet(id) {
                        self.begin_sheet_edit(source.view(), pcbcraft_engine::sheets::RENAME_SHEET, Some(child));
                    }
                } else {
                    self.begin_hierarchy_edit(source, kind, Some(id), at);
                }
            }
            Action::Move => {
                self.views.active.schematic.tool = Some(Tool::Hierarchy { kind, id: Some(id), offset: Vec2::ZERO, revision: source.revision });
            }
            Action::Delete => {
                self.run(
                    if kind == Kind::Sheet { engine::REMOVE_SHEET } else { engine::DELETE_LABEL },
                    &json!({"id":format!("{}-{id}",if kind==Kind::Sheet{"sheet-symbol"}else{"label"}),"expected_revision":source.revision}),
                );
            }
        }
    }

    /// Move uses explicit object identity; creation opens a form before any project mutation.
    pub(super) fn hierarchy_place(&mut self, source: ViewInput, kind: Kind, id: Option<u64>, at: Point) {
        self.views.active.schematic.tool = None;
        let Some(id) = id else {
            self.begin_hierarchy_edit(source, kind, None, at);
            return;
        };
        let Some(project) = self.views.active.snapshot(&self.session) else {
            return;
        };
        let mut params = json!({"id":format!("{}-{id}",if kind==Kind::Sheet{"sheet-symbol"}else{"label"}),"x_nm":at.x_nm,"y_nm":at.y_nm,"expected_revision":source.revision});
        if kind == Kind::Label {
            let Some(label) = project.hierarchical_labels().iter().find(|l| l.id == id) else {
                return;
            };
            params["name"] = json!(label.name);
            params["direction"] = json!(label.direction);
        }
        self.run(if kind == Kind::Sheet { engine::MOVE_SHEET } else { engine::EDIT_LABEL }, &params);
    }

    /// Capture label properties or a new/existing child choice without mutating authored data.
    /// New child names skip existing sheet names case-insensitively, including after deletion.
    fn begin_hierarchy_edit(&mut self, source: ViewInput, kind: Kind, id: Option<u64>, at: Point) {
        let Some(project) = self.views.active.snapshot(&self.session) else {
            return;
        };
        let label = id.and_then(|id| project.hierarchical_labels().iter().find(|l| l.id == id));
        let name = label.map_or_else(
            || {
                if kind != Kind::Sheet {
                    return "SIGNAL".into();
                }
                let mut number = project.sheets().len() + 1;
                while project.sheets().any(|sheet| sheet.name.eq_ignore_ascii_case(&format!("Sheet {number}"))) {
                    number += 1;
                }
                format!("Sheet {number}")
            },
            |l| l.name.clone(),
        );
        self.dialog = Dialog::Hierarchy(Edit {
            source,
            sheet: project.sheet_target(),
            kind,
            id,
            at,
            name,
            direction: label.map_or(Direction::Input, |l| l.direction),
            child: None,
            error: None,
            focused: false,
        });
    }

    /// Stale forms close without edits; validation errors remain editable. New child creation
    /// and placement commit through one engine command and therefore one project undo step.
    pub(super) fn hierarchy_dialog(&mut self, ctx: &egui::Context, mut edit: Edit) {
        let Some(project) = self.session.snapshot() else {
            return;
        };
        let mut submit = false;
        let mut cancel = false;
        let closing =
            shell::dialog(ctx, "hierarchy_dialog", if edit.kind == Kind::Sheet { "Hierarchical Sheet" } else { "Hierarchical Label" }, |ui| {
                if edit.kind == Kind::Sheet {
                    pcbcraft_chrome::controls::Choice::from_label("Child sheet")
                        .selected_text(edit.child.and_then(|t| project.sheets().find(|s| s.target == t).map(|s| s.name)).unwrap_or("New sheet"))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut edit.child, None, "New sheet");
                            for sheet in project.sheets().filter(|s| s.target != edit.sheet && project.parent_sheet(s.target).is_none()) {
                                // Cycles are also checked by the engine when the form is submitted.
                                ui.selectable_value(&mut edit.child, Some(sheet.target), sheet.name);
                            }
                        });
                }
                if edit.child.is_none() {
                    let label = ui.label("Name");
                    let field = ui.add(egui::TextEdit::singleline(&mut edit.name).desired_width(f32::INFINITY)).labelled_by(label.id);
                    if !edit.focused {
                        field.request_focus();
                        edit.focused = true;
                    }
                    submit = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                }
                if edit.kind == Kind::Label {
                    pcbcraft_chrome::controls::Choice::from_label("Direction").selected_text(direction_name(edit.direction)).show_ui(ui, |ui| {
                        for direction in Direction::ALL {
                            ui.selectable_value(&mut edit.direction, direction, direction_name(direction));
                        }
                    });
                }
                if let Some(error) = &edit.error {
                    ui.label(shell::error(error));
                }
                ui.horizontal(|ui| {
                    submit |= pcbcraft_chrome::controls::button(ui, "OK").clicked();
                    cancel = pcbcraft_chrome::controls::button(ui, "Cancel").clicked();
                });
            });
        if submit {
            if !self.views.view(edit.source.view()).is_some_and(|v| v.control_input(&self.session) == Some(edit.source)) {
                self.message = Some("View or project changed; reopen the hierarchy action to continue.".into());
                return;
            }
            let command = if edit.kind == Kind::Sheet {
                if edit.child.is_some() { engine::PLACE_SHEET } else { engine::CREATE_CHILD }
            } else if edit.id.is_some() {
                engine::EDIT_LABEL
            } else {
                engine::ADD_LABEL
            };
            let mut params = json!({"sheet":edit.sheet.id(),"x_nm":edit.at.x_nm,"y_nm":edit.at.y_nm,"expected_revision":edit.source.revision});
            if edit.kind == Kind::Label {
                params["direction"] = json!(edit.direction);
            }
            if let Some(child) = edit.child {
                params["child"] = json!(child.id());
            } else {
                params["name"] = json!(edit.name);
            }
            if let Some(id) = edit.id {
                params["id"] = json!(format!("label-{id}"));
            }
            match self.session.apply(command, &params) {
                Ok(_) => {
                    self.views.sync(&self.session);
                    self.message = None;
                    self.file_notice = None;
                    self.repaint_editors(ctx);
                }
                Err(error) => {
                    edit.error = Some(error.to_string());
                    self.dialog = Dialog::Hierarchy(edit);
                }
            }
        } else if !(closing || cancel) {
            self.dialog = Dialog::Hierarchy(edit);
        }
    }
}
