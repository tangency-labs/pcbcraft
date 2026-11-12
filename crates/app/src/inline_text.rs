//! In-place field drafts belong to one canvas opening and publish one validated command on Enter.
use crate::{PcbApp, ViewInput};
use egui::{Key, Modifiers, Ui};
use pcbcraft_engine::{Project, SET_SYMBOL_FIELDS, SheetTarget};
use serde_json::json;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Symbol { symbol: String, field: String },
    NetName(u64),
}

#[derive(Debug)]
pub(super) struct Edit {
    pub(super) target: Target,
    source: ViewInput,
    sheet: SheetTarget,
    name: String,
    value: String,
    original: String,
    focused: bool,
    outside_press: bool,
    error: Option<String>,
    widget: Option<egui::Id>,
}

impl PcbApp {
    /// Resolve a visible component field or label name from its captured view, without changing selection,
    /// appearance, geometry or history. Missing/stale targets cannot start an editor.
    pub(super) fn begin_inline_text(&mut self, source: ViewInput, target: Target) {
        if !self.views.active.accepts(source, &self.session) {
            return;
        }
        let Some(project) = self.views.active.snapshot(&self.session) else { return };
        let content = match &target {
            Target::Symbol { symbol, field } => {
                project.symbols().iter().find(|s| s.id() == *symbol).and_then(|s| project.component(s)).and_then(|c| match field.as_str() {
                    "reference" => Some(("Reference".into(), c.reference().into())),
                    "value" => Some(("Value".into(), c.display_value().into_owned())),
                    _ => c.metadata.fields.get(field).map(|f| (f.name.clone(), f.value.clone())),
                })
            }
            Target::NetName(id) => project.net_labels().iter().find(|label| label.id == *id).map(|label| ("Net name".into(), label.name.clone())),
        };
        let Some((name, value)) = content else { return };
        self.views.active.schematic.text_edit = Some(Edit {
            source,
            sheet: project.sheet_target(),
            target,
            name,
            original: value.clone(),
            value,
            focused: false,
            outside_press: false,
            error: None,
            widget: None,
        });
    }

    /// Paint a normal text box in the owning canvas, hiding the old name while editing. Enter
    /// applies once; Escape, outside clicks, focus loss or a changed opening/revision discard it.
    pub(super) fn show_inline_text(&mut self, ui: &mut Ui, project: &Project) {
        let Some(mut edit) = self.views.active.schematic.text_edit.take() else { return };
        let id = *edit.widget.get_or_insert_with(|| ui.scope_id().with(("inline_field", ui.ctx().cumulative_pass_nr())));
        if !self.views.active.accepts(edit.source, &self.session) || !matches!(self.dialog, crate::Dialog::None) {
            ui.memory_mut(|memory| memory.surrender_focus(id));
            ui.ctx().request_repaint();
            self.message = Some("View or project changed; text edit cancelled.".into());
            return;
        }
        if ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(false)))) {
            ui.memory_mut(|memory| memory.surrender_focus(id));
            ui.ctx().request_repaint();
            return;
        }
        let Some(canvas) = self.views.active.camera.schematic_rect() else {
            ui.memory_mut(|memory| memory.surrender_focus(id));
            ui.ctx().request_repaint();
            return;
        };
        let camera = &self.views.active.camera;
        let geometry = match &edit.target {
            Target::Symbol { symbol, field } if self.views.active.filters.symbols => {
                crate::schematic_canvas::fields::edit_geometry(ui, project, camera, canvas, symbol, field)
            }
            Target::NetName(id) => {
                project.net_labels().iter().find(|label| label.id == *id && self.views.active.filters.allows_net_label(label.style)).and_then(
                    |label| {
                        pcbcraft_chrome::net_label::name_ink(
                            ui.painter(),
                            camera.screen(label.at, canvas),
                            &label.name,
                            crate::net_labels::visual(label.style),
                            label.rotation,
                            camera.unit_step(),
                            crate::net_labels::visual_text(label, camera.unit_step()).as_ref(),
                        )
                        .map(|ink| {
                            (
                                ink.bounds,
                                label.text.as_ref().map_or(camera.unit_step(), |text| {
                                    camera.unit_step() * text.height_nm as f32 / pcbcraft_engine::schematic::GRID_NM as f32
                                }),
                            )
                        })
                    },
                )
            }
            _ => None,
        };
        let Some((bounds, size)) = geometry else {
            ui.memory_mut(|memory| memory.surrender_focus(id));
            ui.ctx().request_repaint();
            return;
        };
        let enter = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        let escape = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let first = !edit.focused;
        let response = pcbcraft_chrome::properties::inline_text(
            ui,
            id,
            &format!("Edit {}", edit.name),
            &mut edit.value,
            canvas,
            bounds,
            size,
            first,
            edit.error.as_deref(),
        );
        edit.focused = true;
        if !first && ui.input(|i| i.pointer.primary_pressed()) && !response.contains_pointer() {
            edit.outside_press = true;
        }
        if escape || (edit.outside_press && ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary))) {
            response.surrender_focus();
            ui.ctx().request_repaint();
            return;
        }
        if enter {
            if edit.value == edit.original {
                response.surrender_focus();
                ui.ctx().request_repaint();
                return;
            }
            match self.apply_inline_text(project, &edit) {
                Ok(()) => {
                    response.surrender_focus();
                    self.repaint_editors(ui.ctx());
                    return;
                }
                Err(error) => {
                    edit.error = Some(error);
                    response.request_focus();
                    ui.ctx().request_repaint();
                }
            }
        }
        self.views.active.schematic.text_edit = Some(edit);
    }

    /// Change only the addressed string through existing shared commands, preserving all other
    /// content and appearance. The source sheet, opening and revision are rechecked atomically.
    fn apply_inline_text(&mut self, project: &Project, edit: &Edit) -> Result<(), String> {
        if !self.views.active.accepts(edit.source, &self.session) {
            return Err("View or project changed; text edit cancelled.".into());
        }
        let (command, params) = match &edit.target {
            Target::Symbol { symbol, field } => {
                let placement = project.symbols().iter().find(|s| s.id() == *symbol).ok_or("Symbol no longer exists")?;
                let component = project.component(placement).ok_or("Part no longer exists")?;
                let mut params = json!({"sheet":edit.sheet.id(),"symbol":symbol,"reference":component.reference(),"value":component.value(),"expected_revision":edit.source.revision});
                match field.as_str() {
                    "reference" => params["reference"] = json!(edit.value),
                    "value" => params["value"] = json!(edit.value),
                    _ => {
                        let mut metadata = component.metadata.clone();
                        metadata.fields.get_mut(field).ok_or("Field no longer exists")?.value = edit.value.clone();
                        params["metadata"] = json!(metadata);
                    }
                }
                (SET_SYMBOL_FIELDS, params)
            }
            Target::NetName(id) => {
                let label = project.net_labels().iter().find(|label| label.id == *id).ok_or("Net name no longer exists")?;
                let mut params = json!({"sheet":edit.sheet.id(),"id":label.object_id(),"name":edit.value,"style":label.style,"rotation":label.rotation,"x_nm":label.at.x_nm,"y_nm":label.at.y_nm,"expected_revision":edit.source.revision});
                if let Some(text) = &label.text {
                    params["text"] = json!(text);
                }
                (pcbcraft_engine::net_labels::SET, params)
            }
        };
        self.session.apply(command, &params).map_err(|e| e.to_string())?;
        self.views.sync(&self.session);
        self.message = None;
        Ok(())
    }
}
