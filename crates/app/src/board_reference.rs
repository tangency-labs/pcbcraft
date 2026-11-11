//! Reference text selection and revision-bound local print properties.
use crate::{PcbApp, ViewInput};
use egui::Ui;
use pcbcraft_chrome::{
    properties,
    units::{self, Unit},
};
use pcbcraft_engine::{
    board::{
        reference::{Resolved, Size, Text},
        view::FootprintView,
    },
    schematic::{Point, TextAlign},
};
use serde_json::json;

/// A reference is its own edit target; its owning footprint supplies correspondence only.
#[derive(Debug, Default)]
pub(super) struct State {
    pub(super) selected: Option<String>,
    draft: Option<Edit>,
}

#[derive(Debug)]
struct Edit {
    source: ViewInput,
    footprint: String,
    component: String,
    appearance: Text,
    x: String,
    y: String,
    height: String,
    stroke: String,
    unit: Unit,
    dirty: bool,
    rename: Option<String>,
    error: Option<String>,
}

pub(super) enum Action {
    Appearance { source: ViewInput, footprint: String, appearance: Option<Text>, offset: Point },
    Rename { source: ViewInput, component: String, reference: String },
}

impl Edit {
    /// Capture one placement and its resolved defaults without freezing inherited print settings.
    fn new(footprint: &FootprintView<'_>, source: ViewInput, unit: Unit) -> Self {
        Self {
            source,
            footprint: footprint.id.clone(),
            component: footprint.component.clone(),
            appearance: footprint.reference_text.clone().unwrap_or_default(),
            x: units::edit_label(footprint.reference_offset.x_nm, unit),
            y: units::edit_label(footprint.reference_offset.y_nm, unit),
            height: units::edit_label(footprint.reference_appearance.size.height_nm, unit),
            stroke: units::edit_label(footprint.reference_appearance.size.stroke_width_nm, unit),
            unit,
            dirty: false,
            rename: None,
            error: None,
        }
    }

    /// Reformat valid dimensions on a display-unit change while retaining invalid typed input.
    fn set_units(&mut self, unit: Unit) {
        if self.unit != unit {
            for value in [&mut self.x, &mut self.y, &mut self.height, &mut self.stroke] {
                if let Ok(nm) = units::parse_length(value, self.unit) {
                    *value = units::edit_label(nm, unit);
                }
            }
            self.unit = unit;
        }
    }

    /// Parse the whole draft before creating a command, including paired physical dimensions.
    fn action(&self) -> Result<Action, String> {
        let length = |name: &str, value: &str| units::parse_length(value, self.unit).map_err(|error| format!("{name}: {error}"));
        let mut appearance = self.appearance.clone();
        if appearance.size.is_some() {
            appearance.size =
                Some(Size { height_nm: length("Text height", &self.height)?, stroke_width_nm: length("Text stroke width", &self.stroke)? });
        }
        let appearance = (appearance != Text::default()).then_some(appearance);
        if let Some(appearance) = &appearance {
            appearance.validate()?;
        }
        Ok(Action::Appearance {
            source: self.source,
            footprint: self.footprint.clone(),
            appearance,
            offset: Point { x_nm: length("Text X offset", &self.x)?, y_nm: length("Text Y offset", &self.y)? },
        })
    }
}

impl State {
    /// Preserve a draft for repeated selection of the same reference; changing targets drops its draft.
    pub(super) fn select(&mut self, footprint: String) {
        if self.selected.as_ref() != Some(&footprint) {
            self.selected = Some(footprint);
            self.draft = None;
        }
    }

    /// Keep errors editable after a refused command and reload resolved fields after success.
    fn completed(&mut self, error: Option<String>) {
        if let Some(error) = error {
            if let Some(edit) = &mut self.draft {
                edit.error = Some(error);
            }
        } else {
            self.draft = None;
        }
    }

    /// Present local print controls and an explicit shared-reference rename, bound to this view/revision.
    pub(super) fn show(
        &mut self,
        ui: &mut Ui,
        footprint: &FootprintView<'_>,
        source: ViewInput,
        unit: Unit,
        enabled: bool,
        body_height: f32,
    ) -> Option<Action> {
        let replace =
            self.draft.as_ref().is_none_or(|edit| edit.footprint != footprint.id || (edit.source != source && !edit.dirty && edit.rename.is_none()));
        if replace {
            self.draft = Some(Edit::new(footprint, source, unit));
        }
        let edit = self.draft.as_mut()?;
        edit.set_units(unit);
        ui.strong("Silkscreen text");
        properties::field(ui, "Component", |ui| {
            ui.label(footprint.reference);
            if ui
                .add_enabled(enabled && !edit.dirty && edit.source == source, egui::Button::new("Rename…").small())
                .on_hover_text("Rename the component in the schematic, PCB and outputs. Apply or reset pending silkscreen edits first.")
                .clicked()
            {
                edit.rename = Some(footprint.reference.to_owned());
            }
        });
        if edit.source != source {
            ui.label("The board changed. Reload properties to continue.");
            if ui.button("Reload properties").clicked() {
                self.draft = None;
            }
            return None;
        }
        if let Some(reference) = &mut edit.rename {
            properties::text(ui, "Reference", reference);
            let apply = ui.add_enabled(enabled, egui::Button::new("Rename component")).clicked();
            let reference = reference.clone();
            if ui.button("Cancel rename").clicked() {
                edit.rename = None;
                edit.error = None;
            }
            if let Some(error) = &edit.error {
                ui.label(error);
            }
            return apply.then(|| Action::Rename { source: edit.source, component: edit.component.clone(), reference });
        }
        let mut action = None;
        ui.add_enabled_ui(enabled, |ui| {
            egui::ScrollArea::vertical().id_salt("reference_fields").max_height(body_height).show(ui, |ui| {
                appearance_fields(
                    ui,
                    edit,
                    &footprint.reference_appearance,
                    footprint.reference,
                    footprint.pads.iter().any(|pad| pad.copper.is_some()),
                );
                egui::CollapsingHeader::new("Position").show(ui, |ui| {
                    if properties::text(ui, "X offset", &mut edit.x).changed() {
                        edit.dirty = true;
                    }
                    if properties::text(ui, "Y offset", &mut edit.y).changed() {
                        edit.dirty = true;
                    }
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.add_enabled(edit.dirty, pcbcraft_chrome::settings::action_button("Apply changes", true)).clicked() {
                    match edit.action() {
                        Ok(value) => action = Some(value),
                        Err(error) => edit.error = Some(error),
                    }
                }
                if ui
                    .add(pcbcraft_chrome::settings::action_button("Reset silkscreen", false))
                    .on_hover_text("Restore component reference text, default appearance and the library's reference position.")
                    .clicked()
                {
                    action = Some(Action::Appearance {
                        source: edit.source,
                        footprint: edit.footprint.clone(),
                        appearance: None,
                        offset: pcbcraft_engine::board::DEFAULT_REFERENCE_OFFSET,
                    });
                }
            });
        });
        if let Some(error) = &edit.error {
            ui.label(error);
        }
        action
    }
}

/// Edit sparse appearance values. Explicit values remain overrides until the user resets them.
fn appearance_fields(ui: &mut Ui, edit: &mut Edit, resolved: &Resolved, reference: &str, physical: bool) {
    ui.add_enabled_ui(physical, |ui| {
        let mut linked = edit.appearance.text.is_none();
        if ui.checkbox(&mut linked, "Use component reference").changed() {
            edit.appearance.text = (!linked).then(|| reference.to_owned());
            edit.dirty = true;
        }
        if !linked {
            let mut text = edit.appearance.text.clone().unwrap_or_else(|| reference.to_owned());
            if properties::text(ui, "Printed text", &mut text).changed() {
                edit.appearance.text = Some(text);
                edit.dirty = true;
            }
        }
        let original = edit.appearance.clone();
        let label = match edit.appearance.font.as_deref() {
            Some("pcbcraft-technical-1") => "Technical",
            Some(_) => "Legacy stroke",
            None => "Library default",
        };
        properties::select(ui, "Font", ui.scope_id().with("reference_font"), label, |ui| {
            ui.selectable_value(&mut edit.appearance.font, None, "Library default");
            ui.selectable_value(&mut edit.appearance.font, Some("pcbcraft-technical-1".into()), "Technical");
            ui.selectable_value(&mut edit.appearance.font, Some("pcbcraft-stroke-1".into()), "Legacy stroke");
        });
        let height = properties::text(ui, "Height", &mut edit.height).changed();
        let stroke = properties::text(ui, "Stroke", &mut edit.stroke).changed();
        if height || stroke {
            edit.appearance.size.get_or_insert(resolved.size);
            edit.dirty = true;
        }
        let mut rotation = edit.appearance.rotation.unwrap_or(0);
        if properties::angle(ui, "Rotation", &mut rotation).changed() {
            edit.appearance.rotation = Some(rotation);
        }
        let align = match edit.appearance.align {
            None => "Default",
            Some(TextAlign::Start) => "Left",
            Some(TextAlign::Middle) => "Center",
            Some(TextAlign::End) => "Right",
        };
        properties::select(ui, "Alignment", ui.scope_id().with("reference_align"), align, |ui| {
            ui.selectable_value(&mut edit.appearance.align, None, "Default");
            ui.selectable_value(&mut edit.appearance.align, Some(TextAlign::Start), "Left");
            ui.selectable_value(&mut edit.appearance.align, Some(TextAlign::Middle), "Center");
            ui.selectable_value(&mut edit.appearance.align, Some(TextAlign::End), "Right");
        });
        let mut visible = edit.appearance.visible.unwrap_or(resolved.visible);
        if ui.checkbox(&mut visible, "Show silkscreen text").changed() {
            edit.appearance.visible = Some(visible);
        }
        edit.dirty |= original != edit.appearance;
    });
}

impl PcbApp {
    /// Delete on reference selection hides its ink and retains the physical package and component.
    pub(super) fn hide_selected_reference(&mut self) -> bool {
        let Some(id) = self.views.active.board.reference.selected.clone() else { return false };
        let Some(source) = self.views.active.input(&self.session) else { return true };
        let footprint = self
            .session
            .project()
            .and_then(|project| project.board_view(self.views.active.board.target?))
            .and_then(|board| board.footprints().find(|footprint| footprint.id == id));
        if let Some(footprint) = footprint {
            let mut appearance = footprint.reference_text.clone().unwrap_or_default();
            appearance.visible = Some(false);
            let offset = footprint.reference_offset;
            self.apply_reference_action(Action::Appearance { source, footprint: id, appearance: Some(appearance), offset });
        }
        true
    }

    /// Validate form authority, then submit local appearance or a shared component rename through engine commands.
    pub(super) fn apply_reference_action(&mut self, action: Action) {
        let source = match &action {
            Action::Appearance { source, .. } | Action::Rename { source, .. } => *source,
        };
        if !self.views.active.accepts(source, &self.session) {
            self.views.active.board.reference.completed(Some("View or board changed; reload properties to continue.".into()));
            return;
        }
        match action {
            Action::Appearance { footprint, appearance, offset, .. } => {
                if let Some(target) = self.views.active.board.target {
                    self.run(pcbcraft_engine::SET_FOOTPRINT_REFERENCE_TEXT, &json!({"board":target.board,"footprint":footprint,"appearance":appearance,"offset":offset,"expected_revision":source.revision}));
                }
            }
            Action::Rename { component, reference, .. } => {
                let symbol = self.session.project().and_then(|project| {
                    project.sheets().find_map(|sheet| {
                        let scoped = project.for_sheet(sheet.target)?;
                        let symbol = scoped.symbols().iter().find(|symbol| scoped.component(symbol).is_some_and(|value| value.id() == component))?;
                        Some((sheet.target.id(), symbol.id(), scoped.value(symbol).to_owned()))
                    })
                });
                if let Some((sheet, symbol, value)) = symbol {
                    self.run(
                        pcbcraft_engine::SET_SYMBOL_FIELDS,
                        &json!({"sheet":sheet,"symbol":symbol,"reference":reference,"value":value,"expected_revision":source.revision}),
                    );
                } else {
                    self.message = Some("The component is no longer available for renaming.".into());
                }
            }
        }
        self.views.active.board.reference.completed(self.message.clone());
    }
}
