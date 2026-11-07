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
pub(super) fn paint(painter: &Painter, rect: Rect, camera: &Camera, project: &Project, tool: Option<&Tool>, pointer: Option<Pos2>) {
    let tokens = Tokens::get();
    let moving = match tool {
        Some(Tool::Hierarchy { kind, id, offset, .. }) => Some((*kind, *id, pointer.map(|p| camera.snap(p + *offset, rect)))),
        _ => None,
    };
    for block in project.sheet_symbols() {
        let at = moving.filter(|(kind, id, _)| *kind == Kind::Sheet && *id == Some(block.id)).and_then(|(_, _, at)| at).unwrap_or(block.at);
        let size = project.sheet_symbol_size(block);
        let bounds = Rect::from_min_max(camera.screen(at, rect), camera.screen(Point { x_nm: at.x_nm + size.x_nm, y_nm: at.y_nm + size.y_nm }, rect));
        let stroke = Stroke::new(theme::SCHEMATIC_SYMBOL_STROKE, tokens.text);
        painter.rect_stroke(bounds, 0, stroke, egui::StrokeKind::Middle);
        paint_bounded_text(
            painter,
            bounds.left_top() + Vec2::splat(tokens.space_8),
            project.child_name(block),
            FontId::proportional(tokens.font_button),
            tokens.text,
            (bounds.width() - tokens.space_16).max(0.),
            false,
            false,
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
                right,
                true,
            );
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
fn paint_bounded_text(painter: &Painter, at: Pos2, text: &str, font: FontId, color: egui::Color32, width: f32, right: bool, center: bool) {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap.max_width = width;
    job.wrap.max_rows = 1;
    let galley = painter.layout_job(job);
    let offset = egui::vec2(if right { galley.size().x } else { 0. }, if center { galley.size().y / 2. } else { 0. });
    painter.galley(at - offset, galley, color);
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

/// Hit label text before block interiors; pin anchors retain wire-target priority on plain clicks.
fn hit(project: &Project, camera: &Camera, rect: Rect, pointer: Pos2) -> Option<(Kind, u64, Point)> {
    let t = Tokens::get();
    for label in project.hierarchical_labels().iter().rev() {
        let pos = camera.screen(label.at, rect);
        let width = (label.name.chars().count() as f32 * t.font_button).max(t.space_24);
        let offset = if label.direction.right() { 0. } else { -width };
        if Rect::from_min_size(pos + egui::vec2(offset, -t.space_8), egui::vec2(width, t.space_16)).contains(pointer) {
            return Some((Kind::Label, label.id, label.at));
        }
    }
    for block in project.sheet_symbols().iter().rev() {
        let size = project.sheet_symbol_size(block);
        if Rect::from_min_max(
            camera.screen(block.at, rect),
            camera.screen(Point { x_nm: block.at.x_nm + size.x_nm, y_nm: block.at.y_nm + size.y_nm }, rect),
        )
        .contains(pointer)
        {
            return Some((Kind::Sheet, block.id, block.at));
        }
    }
    None
}

/// Let a hovered hierarchy object handle its edit/delete shortcuts before an older component
/// selection. The canvas repeats the hit test against current layout before issuing an action.
pub(super) fn hovered(project: &Project, camera: &Camera, pointer: Pos2) -> bool {
    camera.schematic_rect().is_some_and(|rect| rect.contains(pointer) && hit(project, camera, rect, pointer).is_some())
}

/// Consume hierarchy placement, shortcuts and context menus before generic component selection.
/// A nested result distinguishes an owned menu with no action from unrelated canvas input.
pub(super) fn input(
    ui: &mut Ui,
    response: &Response,
    project: &Project,
    camera: &Camera,
    rect: Rect,
    tool: Option<&Tool>,
    enabled: bool,
) -> Option<Option<Click>> {
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
        let target = response.interact_pointer_pos().and_then(|p| hit(project, camera, rect, p));
        ui.data_mut(|data| match target {
            Some(value) => {
                data.insert_temp(menu_id, value);
            }
            None => data.remove::<(Kind, u64, Point)>(menu_id),
        });
    }
    if (enabled || response.context_menu_opened())
        && let Some((kind, id, _)) = ui.data(|data| data.get_temp::<(Kind, u64, Point)>(menu_id))
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
            rows.into_iter()
                .find_map(|(name, key, action)| MenuItem::new(name).shortcut(key).show(ui).then_some(Click::HierarchyAction(kind, id, action)))
        })
    {
        return Some(menu.inner);
    }
    if !enabled || !response.hovered() || ui.ctx().egui_wants_keyboard_input() {
        return None;
    }
    let (kind, id, _) = hit(project, camera, rect, response.hover_pos()?)?;
    let action = if kind == Kind::Sheet && response.double_clicked() {
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
    action.map(|action| Some(Click::HierarchyAction(kind, id, action)))
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
    fn begin_hierarchy_edit(&mut self, source: ViewInput, kind: Kind, id: Option<u64>, at: Point) {
        let Some(project) = self.views.active.snapshot(&self.session) else {
            return;
        };
        let label = id.and_then(|id| project.hierarchical_labels().iter().find(|l| l.id == id));
        let name = label
            .map_or_else(|| if kind == Kind::Sheet { format!("Sheet {}", project.sheets().len() + 1) } else { "SIGNAL".into() }, |l| l.name.clone());
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
                    egui::ComboBox::from_label("Child sheet")
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
                    egui::ComboBox::from_label("Direction").selected_text(direction_name(edit.direction)).show_ui(ui, |ui| {
                        for direction in Direction::ALL {
                            ui.selectable_value(&mut edit.direction, direction, direction_name(direction));
                        }
                    });
                }
                if let Some(error) = &edit.error {
                    ui.label(shell::error(error));
                }
                ui.horizontal(|ui| {
                    submit |= ui.button("OK").clicked();
                    cancel = ui.button("Cancel").clicked();
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
