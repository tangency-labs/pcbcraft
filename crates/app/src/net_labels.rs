//! Named-net tools submit the same engine commands as headless and live MCP clients.
use crate::{
    Dialog, PcbApp, ViewInput,
    schematic_canvas::{Camera, Click, Tool},
};
use egui::{Key, Modifiers, Pos2, Rect, Response, Ui};
use pcbcraft_chrome::{MenuItem, Tokens, net_label as chrome, shell};
use pcbcraft_engine::{
    Project, SheetTarget,
    net_labels::{self as engine, Label, Style},
    schematic::Point,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

/// Canvas and toolbar actions share one typed routing path.
#[derive(Clone, Copy, Debug)]
pub(super) enum Action {
    Properties,
    EditText,
    Move,
    Rotate,
    Delete,
}

/// A property form retains its source view, sheet and revision until submission.
#[derive(Clone, Debug)]
pub(super) struct Edit {
    pub(super) source: ViewInput,
    sheet: SheetTarget,
    label: Label,
    error: Option<String>,
    focused: bool,
    x: String,
    y: String,
    units: pcbcraft_chrome::units::Unit,
}

impl Edit {
    /// Capture a net marker's complete property draft without changing its electrical anchor.
    fn new(source: ViewInput, sheet: SheetTarget, label: Label, units: pcbcraft_chrome::units::Unit) -> Self {
        let x = pcbcraft_chrome::units::edit_label(label.at.x_nm, units);
        let y = pcbcraft_chrome::units::edit_label(label.at.y_nm, units);
        Self { source, sheet, label, error: None, focused: false, x, y, units }
    }

    /// Parse both axes and text together so invalid appearance cannot partially rename a net.
    fn read(&self) -> Result<Label, String> {
        let mut label = self.label.clone();
        label.at = Point {
            x_nm: pcbcraft_chrome::units::parse_length(&self.x, self.units).map_err(|e| format!("X: {e}"))?,
            y_nm: pcbcraft_chrome::units::parse_length(&self.y, self.units).map_err(|e| format!("Y: {e}"))?,
        };
        Ok(label)
    }
}

/// Map physical name appearance into chrome without importing engine types into the UI crate.
pub(crate) fn visual_text(label: &Label, step: f32) -> Option<chrome::Text> {
    let text = label.text.as_ref()?;
    let offset = text.offset.map(|p| {
        let v = egui::vec2(p.x_nm as f32, p.y_nm as f32) * (step / pcbcraft_engine::schematic::GRID_NM as f32);
        match label.rotation {
            90 => egui::vec2(v.y, -v.x),
            180 => -v,
            270 => egui::vec2(-v.y, v.x),
            _ => v,
        }
    });
    Some(chrome::Text {
        offset,
        height: step * text.height_nm as f32 / pcbcraft_engine::schematic::GRID_NM as f32,
        rotation: text.rotation,
        align: match text.align {
            pcbcraft_engine::schematic::TextAlign::Start => egui::Align::Min,
            pcbcraft_engine::schematic::TextAlign::Middle => egui::Align::Center,
            pcbcraft_engine::schematic::TextAlign::End => egui::Align::Max,
        },
        visible: text.visible,
        show_name: text.show_name,
    })
}

/// Map electrical styles into presentation without making chrome depend on the engine.
pub(super) fn visual(style: Style) -> chrome::Style {
    match style {
        Style::Local => chrome::Style::Local,
        Style::Global => chrome::Style::Global,
    }
}

/// Measure name/glyph bounds without adding any shapes to the scene.
pub(super) fn bounds(ui: &Ui, camera: &Camera, rect: Rect, label: &Label) -> Rect {
    chrome::bounds_with_text(
        ui.painter(),
        camera.screen(label.at, rect),
        &label.name,
        visual(label.style),
        label.rotation,
        camera.unit_step(),
        visual_text(label, camera.unit_step()).as_ref(),
    )
}

/// Resolve exact local physical contacts for painting, independently of matching remote net
/// names. Prepare each route once and stop checking anchors after their first contact.
pub(super) fn attached(project: &Project) -> BTreeSet<u64> {
    let mut remaining = BTreeMap::<(i64, i64), Vec<u64>>::new();
    for label in project.net_labels() {
        remaining.entry((label.at.x_nm, label.at.y_nm)).or_default().push(label.id);
    }
    let mut attached = BTreeSet::new();
    if remaining.is_empty() {
        return attached;
    }
    remaining.retain(|_, ids| {
        if ids.len() > 1 {
            attached.extend(ids.iter().copied());
            false
        } else {
            true
        }
    });
    // The hierarchy iterator includes named anchors themselves; an anchor is not its own contact.
    for at in project.symbols().iter().flat_map(|s| s.pins()).chain(
        project.hierarchy_endpoints().filter(|(end, _)| !matches!(end, pcbcraft_engine::schematic::WireEndpoint::NetLabel(_))).map(|(_, at)| at),
    ) {
        if let Some(ids) = remaining.remove(&(at.x_nm, at.y_nm)) {
            attached.extend(ids);
        }
    }
    for wire in project.wires() {
        if remaining.is_empty() {
            break;
        }
        let path = project.wire_path(wire);
        remaining.retain(|&(x_nm, y_nm), ids| {
            if pcbcraft_engine::wire::path_contains_point(&path, Point { x_nm, y_nm }) {
                attached.extend(ids.iter().copied());
                false
            } else {
                true
            }
        });
    }
    attached
}

/// Whether the hovered ink belongs to a named anchor; shared shortcut routing uses this before
/// component hotkeys so editing an enabled label cannot also edit a component beneath it.
pub(super) fn hovered(project: &Project, camera: &Camera, ctx: &egui::Context, at: Pos2, filters: crate::selection_filters::Filters) -> bool {
    let Some(rect) = camera.schematic_rect() else {
        return false;
    };
    project.net_labels().iter().filter(|label| filters.allows_net_label(label.style)).any(|label| {
        chrome::bounds_with_text(
            &ctx.debug_painter(),
            camera.screen(label.at, rect),
            &label.name,
            visual(label.style),
            label.rotation,
            camera.unit_step(),
            visual_text(label, camera.unit_step()).as_ref(),
        )
        .contains(at)
    })
}

/// Paint committed labels and pointer previews using the same geometry as picking. Name hover
/// outlines follow the owning view's input/filter state and do not change electrical selection.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    ui: &Ui,
    rect: Rect,
    camera: &Camera,
    project: &Project,
    tool: Option<&Tool>,
    pointer: Option<Point>,
    selected: &pcbcraft_engine::selection::Selection,
    editing: Option<&crate::inline_text::Target>,
) {
    let painter = ui.painter_at(rect);
    let attached = attached(project);
    let moving = match tool {
        Some(Tool::NetLabel { label, .. }) => Some(label),
        _ => None,
    };
    for label in project.net_labels().iter().filter(|label| moving.is_none_or(|m| m.id != label.id)) {
        let mut appearance = visual_text(label, camera.unit_step());
        if matches!(editing,Some(crate::inline_text::Target::NetName(id)) if *id==label.id) {
            let hidden = label.text.clone().unwrap_or_default();
            let hidden_label = Label { text: Some(pcbcraft_engine::schematic::fields::Layout { visible: false, ..hidden }), ..label.clone() };
            appearance = visual_text(&hidden_label, camera.unit_step());
        }
        chrome::paint_with_text(
            &painter,
            camera.screen(label.at, rect),
            &label.name,
            visual(label.style),
            label.rotation,
            camera.unit_step(),
            if selected.net_labels.contains(&label.object_id()) { Tokens::get().accent } else { Tokens::get().text },
            attached.contains(&label.id),
            appearance.as_ref(),
        );
    }
    if let Some(label) = moving
        && let Some(at) = pointer
    {
        chrome::paint_with_text(
            &painter,
            camera.screen(at, rect),
            &label.name,
            visual(label.style),
            label.rotation,
            camera.unit_step(),
            Tokens::get().accent,
            attached.contains(&label.id),
            visual_text(label, camera.unit_step()).as_ref(),
        );
    }
}

/// Resolve property/move/rotate/delete actions on actual label ink. Active placement consumes
/// pointer input until completion or cancellation; no project state changes during a preview.
/// Filters restrict existing ink while explicit placement remains available.
#[allow(clippy::too_many_arguments)]
pub(super) fn input(
    ui: &mut Ui,
    response: &Response,
    rect: Rect,
    camera: &Camera,
    project: &Project,
    tool: Option<&Tool>,
    enabled: bool,
    selection: &pcbcraft_engine::selection::Selection,
    placement: Option<Point>,
    filters: crate::selection_filters::Filters,
) -> Option<Option<Click>> {
    if let Some(Tool::NetLabel { label, drag, .. }) = tool {
        if !enabled || ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(false)))) {
            return Some(Some(Click::NetLabelCancel));
        }
        if response.secondary_clicked() {
            return Some(Some(Click::NetLabelCancel));
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::R)) {
            return Some(Some(Click::NetLabelRotate));
        }
        if drag.is_some() && ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary)) {
            return Some(Some(if response.contains_pointer() {
                placement
                    .map(|at| {
                        let mut label = label.clone();
                        label.at = at;
                        Click::NetLabelCommit(label)
                    })
                    .unwrap_or(Click::NetLabelCancel)
            } else {
                Click::NetLabelCancel
            }));
        }
        if drag.is_none()
            && response.clicked()
            && let Some(at) = placement
        {
            let mut label = label.clone();
            label.at = at;
            return Some(Some(Click::NetLabelCommit(label)));
        }
        return Some(None);
    }
    if tool.is_some() {
        return None;
    }
    if enabled
        && response.drag_started_by(egui::PointerButton::Primary)
        && !ui.input(|i| i.modifiers.shift || i.modifiers.ctrl || i.modifiers.command)
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        && let Some(label) =
            project.net_labels().iter().rev().find(|label| filters.allows_net_label(label.style) && bounds(ui, camera, rect, label).contains(origin))
    {
        if selection.len() <= 1 || !selection.net_labels.contains(&label.object_id()) {
            return Some(Some(Click::NetLabelDrag(label.clone(), camera.screen(label.at, rect) - origin)));
        }
        return None;
    }
    if !enabled && !response.context_menu_opened() {
        return None;
    }
    if selection.len() > 1 {
        return None;
    }
    let hit = response
        .hover_pos()
        .or_else(|| response.interact_pointer_pos())
        .and_then(|p| {
            project.net_labels().iter().rev().find(|label| filters.allows_net_label(label.style) && bounds(ui, camera, rect, label).contains(p))
        })
        .cloned();
    let menu_id = response.id.with("named_anchor_context");
    if response.secondary_clicked() {
        ui.data_mut(|data| match &hit {
            Some(label) => {
                data.insert_temp(menu_id, label.id);
            }
            None => data.remove::<u64>(menu_id),
        });
    }
    if let Some(id) = ui.data(|data| data.get_temp::<u64>(menu_id))
        && let Some(label) = project.net_labels().iter().find(|label| label.id == id && filters.allows_net_label(label.style))
        && let Some(menu) = pcbcraft_chrome::menu::context(response, |ui| {
            if MenuItem::new("Edit text…").show(ui) {
                return Some(Click::NetLabelAction(label.clone(), Action::EditText));
            }
            for (name, action) in
                [("Properties…", Action::Properties), ("Move", Action::Move), ("Rotate", Action::Rotate), ("Delete Net Label", Action::Delete)]
            {
                if MenuItem::new(name).show(ui) {
                    return Some(Click::NetLabelAction(label.clone(), action));
                }
            }
            None
        })
    {
        return Some(menu.inner);
    }
    if !enabled || ui.ctx().egui_wants_keyboard_input() {
        return None;
    }
    let hovered = hit.is_some();
    let label = hit.or_else(|| {
        selection.single_net_label().and_then(|id| project.net_labels().iter().find(|l| l.id == id && filters.allows_net_label(l.style)).cloned())
    })?;
    for (key, action) in [
        (Key::E, Action::Properties),
        (Key::M, Action::Move),
        (Key::R, Action::Rotate),
        (Key::Delete, Action::Delete),
        (Key::Backspace, Action::Delete),
    ] {
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
            return Some(Some(Click::NetLabelAction(label, action)));
        }
    }
    if hovered && response.double_clicked() {
        return Some(Some(Click::NetLabelAction(label, Action::Properties)));
    }
    // Keep the anchor available to click-to-wire; clicking ink selects only this marker.
    if hovered && response.clicked() && response.hover_pos().is_some_and(|p| camera.screen(label.at, rect).distance(p) > 6.) {
        let modifiers = ui.input(|i| i.modifiers);
        return Some(Some(Click::SelectItems(selection.combine(
            &pcbcraft_engine::selection::Selection::one_net_label(label.id),
            modifiers.shift,
            modifiers.ctrl || modifiers.command,
        ))));
    }
    None
}

impl PcbApp {
    /// Start a GND placement directly in the active schematic, without a chooser or property
    /// form. Only the canvas commit edits history; the preview keeps the current repeat mode.
    pub(super) fn begin_ground(&mut self, ctx: &egui::Context) {
        if self.views.active.input(&self.session).is_none() {
            return;
        }
        let Some(definition) = self.session.libraries().latest_symbol("power", "gnd") else {
            self.message = Some("The Ground library definition is unavailable; check Library Sources.".into());
            return;
        };
        self.parts.close(ctx);
        self.parts.release_focus(ctx);
        self.select(Default::default());
        self.begin_template(crate::parts_browser::Placement::new(definition));
    }

    /// Open a captured form. A new marker has ID zero until the engine allocates its identity.
    pub(super) fn begin_net_label(&mut self, style: Style) {
        self.select(Default::default());
        let Some(source) = self.views.active.input(&self.session) else {
            return;
        };
        let label = Label { id: 0, name: "SIGNAL".into(), style, at: Point { x_nm: 0, y_nm: 0 }, rotation: 0, text: None };
        self.net_label_action(source, label, Action::Properties);
    }
    /// Apply only view-authorized actions; property edits and previews retain the source revision.
    pub(super) fn net_label_action(&mut self, source: ViewInput, mut label: Label, action: Action) {
        match action {
            Action::EditText => self.begin_inline_text(source, crate::inline_text::Target::NetName(label.id)),
            Action::Properties => {
                let Some(project) = self.views.active.snapshot(&self.session) else {
                    return;
                };
                self.views.active.schematic.tool = None;
                let edit = Edit::new(source, project.sheet_target(), label, self.preferences.units);
                self.dialog = Dialog::NetLabel(Box::new(edit));
            }
            Action::Move => self.views.active.schematic.tool = Some(Tool::NetLabel { label, revision: source.revision, drag: None }),
            Action::Rotate => {
                label.rotation = (label.rotation + 90) % 360;
                self.commit_net_label(source, label);
            }
            Action::Delete => self.run(engine::DELETE, &json!({"id":label.object_id(),"expected_revision":source.revision})),
        }
    }
    /// Commit one completed preview through the shared engine, which allocates new identities.
    /// Existing marker moves retain touching wires independent of placement order. Text-only
    /// edits leave the electrical anchor fixed, and failures publish no partial state.
    pub(super) fn commit_net_label(&mut self, source: ViewInput, label: Label) {
        let repeat = label.id == 0 && self.parts.repeats();
        let next = label.clone();
        let mut params = json!({"name":label.name,"style":label.style,"x_nm":label.at.x_nm,"y_nm":label.at.y_nm,"rotation":label.rotation,"expected_revision":source.revision});
        if let Some(text) = &label.text {
            params["text"] = json!(text);
        }
        if label.id != 0 {
            params["id"] = json!(label.object_id());
            params["keep_connected"] = json!(true);
        }
        self.run(if label.id == 0 { engine::ADD } else { engine::SET }, &params);
        self.views.active.schematic.tool = None;
        if repeat && self.message.is_none() && self.session.revision() > source.revision {
            self.views.active.schematic.tool = Some(Tool::NetLabel { label: next, revision: self.session.revision(), drag: None });
        }
    }
    /// Place starts a new marker preview; Apply commits an existing marker edit once.
    /// Cancel/stale forms never publish data, and engine errors stay visible beside the fields.
    pub(super) fn net_label_dialog(&mut self, ctx: &egui::Context, mut edit: Edit) {
        edit.set_units(self.preferences.units);
        let title = if edit.label.id == 0 { "Place net label" } else { "Net label properties" };
        let mut submit = false;
        let action = pcbcraft_chrome::catalog::property_form(
            ctx,
            "net_label_properties",
            title,
            760.,
            if edit.label.id == 0 { "Place" } else { "Apply" },
            |ui| {
                submit |= label_fields(ui, &mut edit, true);
                if let Some(error) = &edit.error {
                    ui.label(shell::error(error));
                }
            },
        );
        if action.close {
            if let Some(view) = self.views.view_mut(edit.source.view()) {
                view.schematic.properties.net_draft = None;
            }
            return;
        }
        submit |= action.primary;
        if submit {
            if !self.views.view(edit.source.view()).is_some_and(|v| v.control_input(&self.session) == Some(edit.source)) {
                self.message = Some("View or project changed; reopen the label action to continue.".into());
                return;
            }
            match edit.read() {
                Ok(label)
                    if label.name.is_empty()
                        || label.name.trim() != label.name
                        || label.name.chars().count() > 200
                        || label.name.chars().any(char::is_control) =>
                {
                    edit.error = Some("Enter a net name of 1–200 characters without surrounding whitespace or control characters.".into());
                }
                Ok(label) if label.id == 0 => {
                    self.views.active.schematic.tool = Some(Tool::NetLabel { label, revision: edit.source.revision, drag: None });
                    return;
                }
                Ok(label) => match self.apply_label_edit(&edit, label) {
                    Ok(()) => {
                        self.repaint_editors(ctx);
                        return;
                    }
                    Err(error) => edit.error = Some(error),
                },
                Err(error) => edit.error = Some(error),
            }
        }
        self.dialog = Dialog::NetLabel(Box::new(edit));
    }

    /// Text labels use the same captured property draft in the inspector and expanded form.
    pub(super) fn net_label_inspector(&mut self, ui: &mut Ui, canvas: Rect, area: Rect) -> Option<Rect> {
        let source = self.views.active.input(&self.session)?;
        let project = self.views.active.snapshot(&self.session)?;
        let id = self.views.active.schematic.selection.single_net_label()?;
        let label = project.net_labels().iter().find(|l| l.id == id)?.clone();
        let current = Edit::new(source, project.sheet_target(), label, self.preferences.units);
        let mut edit = self
            .views
            .active
            .schematic
            .properties
            .net_draft
            .take()
            .filter(|d| d.source == source && d.label.id == id)
            .unwrap_or_else(|| current.clone());
        edit.set_units(self.preferences.units);
        let mut apply = false;
        let mut more = false;
        let enabled = matches!(self.dialog, Dialog::None);
        let collapsed = self.views.active.schematic.properties.collapsed;
        let width = pcbcraft_chrome::workspace::panels::card_width(canvas, true);
        let (rect, collapse) = self.views.active.panels[crate::workspace_panels::PROPERTIES].show(
            ui,
            egui::Id::unique(("selection_properties", self.views.host)),
            "Properties",
            canvas,
            area,
            collapsed,
            width,
            |ui| {
                pcbcraft_chrome::properties::inspector_body(ui);
                pcbcraft_chrome::properties::identity(ui, &edit.label.name, "Net label");
                if !enabled {
                    return;
                }
                apply |= label_fields(ui, &mut edit, false);
                if let Some(error) = &edit.error {
                    ui.label(shell::error(error));
                }
                if edit.read().as_ref() != Ok(&current.label) {
                    apply |= ui.button("Apply changes").clicked();
                    if ui.button("Reset changes").clicked() {
                        edit = current.clone();
                    }
                }
                more = ui.button("More properties ↗").clicked();
            },
        );
        if collapse {
            self.views.active.schematic.properties.collapsed = !collapsed;
        }
        self.views.active.schematic.properties.rect = rect;
        if apply {
            match edit.read().and_then(|label| self.apply_label_edit(&edit, label)) {
                Ok(()) => {
                    self.views.active.schematic.properties.net_draft = None;
                    return Some(rect);
                }
                Err(error) => edit.error = Some(error),
            }
        }
        if more {
            edit.focused = false;
            self.dialog = Dialog::NetLabel(Box::new(edit.clone()));
        }
        self.views.active.schematic.properties.net_draft = Some(edit);
        self.views.active.schematic.properties.rect = rect;
        Some(rect)
    }

    /// Submit complete marker properties and retained contacts atomically to the source
    /// sheet/revision, using the current automatic route-cleanup preference for moves.
    fn apply_label_edit(&mut self, edit: &Edit, label: Label) -> Result<(), String> {
        if !self.views.active.accepts(edit.source, &self.session) {
            return Err("View or project changed; reopen properties to continue.".into());
        }
        let mut params = json!({"sheet":edit.sheet.id(),"id":label.object_id(),"name":label.name,"style":label.style,"x_nm":label.at.x_nm,"y_nm":label.at.y_nm,"rotation":label.rotation,"expected_revision":edit.source.revision,"keep_connected":true,"auto_heal":self.preferences.auto_heal_wires});
        if let Some(text) = label.text {
            params["text"] = json!(text);
        }
        self.session.apply(engine::SET, &params).map_err(|e| e.to_string())?;
        self.views.sync(&self.session);
        self.message = None;
        Ok(())
    }
}

impl Edit {
    /// Display-unit changes reformat valid coordinates and text settings without applying them.
    fn set_units(&mut self, unit: pcbcraft_chrome::units::Unit) {
        if self.units == unit {
            return;
        }
        for value in [&mut self.x, &mut self.y] {
            if let Ok(nm) = pcbcraft_chrome::units::parse_length(value, self.units) {
                *value = pcbcraft_chrome::units::edit_label(nm, unit);
            }
        }
        self.units = unit;
    }
}

/// Text-label properties edit name, scope and placement through the shared label command.
fn label_fields(ui: &mut Ui, edit: &mut Edit, focus: bool) -> bool {
    use pcbcraft_chrome::properties as form;
    let name = form::text(ui, "Net name", &mut edit.label.name);
    if focus && !edit.focused {
        name.request_focus();
        edit.focused = true;
    }
    let enter = name.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    let mut key = if edit.label.style == Style::Global { "global" } else { "local" }.to_owned();
    form::choice(ui, "Scope", ui.scope_id().with("label_style"), &mut key, &[("local", "Local".into()), ("global", "Global".into())]);
    edit.label.style = if key == "global" { Style::Global } else { Style::Local };
    form::rotation(ui, &mut edit.label.rotation);
    let position_enter = if edit.label.id != 0 {
        let x = form::text(ui, "X", &mut edit.x);
        let y = form::text(ui, "Y", &mut edit.y);
        (x.lost_focus() || y.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter))
    } else {
        false
    };
    enter || position_enter
}
