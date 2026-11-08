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
    Move,
    Rotate,
    Delete,
}

/// A property form retains its source view, sheet and revision until submission.
#[derive(Debug)]
pub(super) struct Edit {
    source: ViewInput,
    sheet: SheetTarget,
    label: Label,
    error: Option<String>,
    focused: bool,
}

/// Map electrical styles into presentation without making chrome depend on the engine.
pub(super) fn visual(style: Style) -> chrome::Style {
    match style {
        Style::Local => chrome::Style::Local,
        Style::Global => chrome::Style::Global,
        Style::Power => chrome::Style::Power,
        Style::PowerBar => chrome::Style::PowerBar,
        Style::Ground => chrome::Style::Ground,
    }
}

/// Measure name/glyph bounds without adding any shapes to the scene.
pub(super) fn bounds(ui: &Ui, camera: &Camera, rect: Rect, label: &Label) -> Rect {
    chrome::bounds(ui.painter(), camera.screen(label.at, rect), &label.name, visual(label.style), label.rotation, camera.unit_step())
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
        chrome::bounds(&ctx.debug_painter(), camera.screen(label.at, rect), &label.name, visual(label.style), label.rotation, camera.unit_step())
            .contains(at)
    })
}

/// Paint committed labels and the pointer-bound preview. Inputs below use the same ink bounds.
pub(super) fn paint(
    ui: &Ui,
    rect: Rect,
    camera: &Camera,
    project: &Project,
    tool: Option<&Tool>,
    pointer: Option<Point>,
    selected: &pcbcraft_engine::selection::Selection,
) {
    let painter = ui.painter_at(rect);
    let attached = attached(project);
    let moving = match tool {
        Some(Tool::NetLabel { label, .. }) => Some(label),
        _ => None,
    };
    for label in project.net_labels().iter().filter(|label| moving.is_none_or(|m| m.id != label.id)) {
        chrome::paint(
            &painter,
            camera.screen(label.at, rect),
            &label.name,
            visual(label.style),
            label.rotation,
            camera.unit_step(),
            if selected.net_labels.contains(&label.object_id()) { Tokens::get().accent } else { Tokens::get().text },
            attached.contains(&label.id),
        );
    }
    if let Some(label) = moving
        && let Some(at) = pointer
    {
        chrome::paint(
            &painter,
            camera.screen(at, rect),
            &label.name,
            visual(label.style),
            label.rotation,
            camera.unit_step(),
            Tokens::get().accent,
            attached.contains(&label.id),
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
        let Some(source) = self.views.active.input(&self.session) else { return };
        self.parts.close(ctx);
        self.parts.release_focus(ctx);
        self.select(Default::default());
        let label = Label { id: 0, name: "GND".into(), style: Style::Ground, at: Point { x_nm: 0, y_nm: 0 }, rotation: 0 };
        self.views.active.schematic.tool = Some(Tool::NetLabel { label, revision: source.revision, drag: None });
    }

    /// Open a captured form. A new marker has ID zero until the engine allocates its identity.
    pub(super) fn begin_net_label(&mut self, style: Style) {
        self.select(Default::default());
        let Some(source) = self.views.active.input(&self.session) else {
            return;
        };
        let label = Label {
            id: 0,
            name: if style == Style::Ground {
                "GND"
            } else if matches!(style, Style::Power | Style::PowerBar) {
                "VCC"
            } else {
                "SIGNAL"
            }
            .into(),
            style,
            at: Point { x_nm: 0, y_nm: 0 },
            rotation: 0,
        };
        self.net_label_action(source, label, Action::Properties);
    }
    /// Apply only view-authorized actions; property edits and previews retain the source revision.
    pub(super) fn net_label_action(&mut self, source: ViewInput, mut label: Label, action: Action) {
        match action {
            Action::Properties => {
                let Some(project) = self.views.active.snapshot(&self.session) else {
                    return;
                };
                self.views.active.schematic.tool = None;
                self.dialog = Dialog::NetLabel(Edit { source, sheet: project.sheet_target(), label, error: None, focused: false });
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
    /// Engine failures remain visible in the app message without publishing partial state.
    pub(super) fn commit_net_label(&mut self, source: ViewInput, label: Label) {
        let repeat = label.id == 0 && self.parts.repeats();
        let next = label.clone();
        let mut params = json!({"name":label.name,"style":label.style,"x_nm":label.at.x_nm,"y_nm":label.at.y_nm,"rotation":label.rotation,"expected_revision":source.revision});
        if label.id != 0 {
            params["id"] = json!(label.object_id());
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
        let power = matches!(edit.label.style, Style::Power | Style::PowerBar | Style::Ground);
        let title = if power {
            "Power symbol properties".to_owned()
        } else if edit.label.id == 0 {
            "Place net label".into()
        } else {
            "Net label properties".into()
        };
        let mut submit = false;
        let action = pcbcraft_chrome::catalog::form(
            ctx,
            "net_label_properties",
            &title,
            560.,
            if edit.label.id == 0 { "Place" } else { "Apply" },
            true,
            false,
            true,
            |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Frame::new().inner_margin(Tokens::get().space_16).show(ui, |ui| {
                        pcbcraft_chrome::properties::section(ui, "Connection");
                        let field = pcbcraft_chrome::properties::text(ui, "Net name", &mut edit.label.name);
                        if !edit.focused {
                            field.request_focus();
                            edit.focused = true;
                        }
                        submit |= field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        if power {
                            pcbcraft_chrome::properties::field(ui, "Scope", |ui| {
                                ui.label("Global");
                            });
                        } else {
                            let mut scope = if edit.label.style == Style::Global { "global" } else { "local" }.to_owned();
                            pcbcraft_chrome::properties::choice(
                                ui,
                                "Scope",
                                ui.id().with("label_scope"),
                                &mut scope,
                                &[("local", "Local".into()), ("global", "Global".into())],
                            );
                            edit.label.style = if scope == "global" { Style::Global } else { Style::Local };
                        }
                        pcbcraft_chrome::properties::rotation(ui, &mut edit.label.rotation);
                        if let Some(error) = &edit.error {
                            ui.label(shell::error(error));
                        }
                    });
                });
            },
        );
        if action.close {
            return;
        }
        submit |= action.primary;
        if submit {
            if !self.views.view(edit.source.view()).is_some_and(|v| v.control_input(&self.session) == Some(edit.source)) {
                self.message = Some("View or project changed; reopen the label action to continue.".into());
                return;
            }
            if edit.label.name.is_empty()
                || edit.label.name.trim() != edit.label.name
                || edit.label.name.chars().count() > 200
                || edit.label.name.chars().any(char::is_control)
            {
                edit.error = Some("Enter a net name of 1–200 characters without surrounding whitespace or control characters.".into());
            } else if edit.label.id == 0 {
                self.views.active.schematic.tool = Some(Tool::NetLabel { label: edit.label, revision: edit.source.revision, drag: None });
                return;
            } else {
                let params = json!({"sheet":edit.sheet.id(),"id":edit.label.object_id(),"name":edit.label.name,"style":edit.label.style,"x_nm":edit.label.at.x_nm,"y_nm":edit.label.at.y_nm,"rotation":edit.label.rotation,"expected_revision":edit.source.revision});
                match self.session.apply(engine::SET, &params) {
                    Ok(_) => {
                        self.views.sync(&self.session);
                        self.repaint_editors(ctx);
                        return;
                    }
                    Err(error) => edit.error = Some(error.to_string()),
                }
            }
        }
        self.dialog = Dialog::NetLabel(edit);
    }
}
