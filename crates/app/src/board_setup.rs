//! Revision-bound board layer/stackup forms over the existing shared engine commands.
use crate::{Dialog, PcbApp, ViewId, ViewInput, Workspace};
use egui::{Context, Ui};
use pcbcraft_chrome::{properties, units};
use pcbcraft_engine::board::{
    setup::{BoardSetup, LayerPurpose, Side, Stratum},
    view::BoardTarget,
};
use serde_json::json;

/// Preserve exact stored nanometers when a rounded display string has not been edited.
#[derive(Debug)]
struct Length {
    original: Option<i64>,
    initial: String,
    text: String,
}
impl Length {
    /// Unknown thickness starts blank; it is never replaced with a guessed default or zero.
    fn new(value: Option<i64>, unit: units::Unit) -> Self {
        let text = value.map(|n| units::label(n, unit)).unwrap_or_default();
        Self { original: value, initial: text.clone(), text }
    }
    /// Blank clears a specified thickness; unchanged display text retains exact source precision.
    fn value(&self, unit: units::Unit) -> Result<Option<i64>, String> {
        if self.text == self.initial {
            return Ok(self.original);
        }
        if self.text.trim().is_empty() {
            return Ok(None);
        }
        units::parse_length(&self.text, unit).map(Some).map_err(|e| e.to_string())
    }
}

/// A draft retains every material/construction field while exposing names and finished thickness.
#[derive(Debug)]
struct Candidate {
    setup: BoardSetup,
    names: Vec<String>,
    thickness: Vec<Length>,
    material_labels: Vec<String>,
}
impl Candidate {
    /// Seed fields in the current display unit without adopting any project changes.
    fn new(setup: BoardSetup, unit: units::Unit) -> Self {
        let names = setup.layers().iter().map(|l| l.name.clone()).collect();
        let thickness = setup.stackup().strata.iter().map(|s| Length::new(s.thickness_nm(), unit)).collect();
        let material_labels = setup
            .stackup()
            .strata
            .iter()
            .map(|s| match s {
                Stratum::Dielectric { material, .. } => material.clone().unwrap_or_default(),
                _ => String::new(),
            })
            .collect();
        Self { setup, names, thickness, material_labels }
    }
    /// Build and validate the complete candidate, preserving layer identities, pinned materials,
    /// construction, tolerances and fields not edited by this first graphical form.
    fn value(&self, unit: units::Unit) -> Result<BoardSetup, String> {
        let mut value = serde_json::to_value(&self.setup).map_err(|e| e.to_string())?;
        for (i, name) in self.names.iter().enumerate() {
            value["layers"][i]["name"] = json!(name);
        }
        for (i, length) in self.thickness.iter().enumerate() {
            let entry = value["stackup"]["strata"][i].as_object_mut().ok_or("Invalid stackup draft")?;
            match length.value(unit)? {
                Some(n) => {
                    entry.insert("thickness_nm".into(), json!(n));
                }
                None => {
                    entry.remove("thickness_nm");
                }
            }
            if matches!(self.setup.stackup().strata[i], Stratum::Dielectric { .. }) {
                let material = &self.material_labels[i];
                if material.is_empty() {
                    entry.remove("material");
                } else {
                    entry.insert("material".into(), json!(material));
                }
            }
        }
        serde_json::from_value(value).map_err(|e| e.to_string())
    }
}

/// Captured board and view authority prevents an old form from editing a reopened project.
#[derive(Debug)]
pub(super) struct Edit {
    source: ViewInput,
    board: BoardTarget,
    unit: units::Unit,
    populated: bool,
    candidate: Option<Candidate>,
    copper_layers: u64,
    stackup_tab: bool,
    new_name: String,
    new_purpose: LayerPurpose,
    new_side: Option<Side>,
    error: Option<String>,
}

/// Friendly purpose names are presentation only; command inputs retain typed layer roles.
fn purpose_name(purpose: LayerPurpose) -> &'static str {
    match purpose {
        LayerPurpose::Copper => "Copper",
        LayerPurpose::Mask => "Solder mask",
        LayerPurpose::Paste => "Solder paste",
        LayerPurpose::Silkscreen => "Silkscreen",
        LayerPurpose::Adhesive => "Adhesive",
        LayerPurpose::Fabrication => "Fabrication",
        LayerPurpose::Courtyard => "Courtyard",
        LayerPurpose::Outline => "Board Edge",
        LayerPurpose::Mechanical => "Mechanical",
        LayerPurpose::User => "User",
    }
}

impl Edit {
    /// Edit catalog names and stage technical-layer creation without committing any metadata.
    fn layers(&mut self, ui: &mut Ui) {
        let Some(candidate) = &mut self.candidate else { return };
        let t = pcbcraft_chrome::Tokens::get();
        let room = (ui.available_width() - 2.0 * ui.spacing().item_spacing.x).max(0.0);
        let widths = [room * 0.55, room * 0.28, room * 0.17];
        egui::Grid::new("setup_layers").num_columns(3).striped(true).show(ui, |ui| {
            for (label, width) in ["Layer", "Purpose", "Side"].into_iter().zip(widths) {
                ui.add_sized(egui::vec2(width, t.control_height), egui::Label::new(label).halign(egui::Align::Min));
            }
            ui.end_row();
            for layer in crate::board_panels::ordered_catalog(candidate.setup.layers().to_vec(), Some(&candidate.setup)) {
                let Some(index) = candidate.setup.layers().iter().position(|l| l.id == layer.id) else { continue };
                let name = &mut candidate.names[index];
                ui.add_sized(egui::vec2(widths[0], t.control_height), egui::TextEdit::singleline(name).id_salt(layer.id))
                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, format!("Layer name {}", layer.name)));
                ui.add_sized(egui::vec2(widths[1], t.control_height), egui::Label::new(purpose_name(layer.purpose)).halign(egui::Align::Min));
                let side = match layer.side {
                    Some(Side::Front) => "Front",
                    Some(Side::Back) => "Back",
                    None if layer.purpose == LayerPurpose::Copper => "Internal",
                    None => "—",
                };
                ui.add_sized(egui::vec2(widths[2], t.control_height), egui::Label::new(side).halign(egui::Align::Min));
                ui.end_row();
            }
        });
        properties::section(ui, "Add technical layer");
        if self.populated {
            ui.label("Layer membership can currently be changed only before footprints are placed.");
            return;
        }
        properties::text(ui, "Layer name", &mut self.new_name);
        properties::field(ui, "Purpose", |ui| {
            egui::ComboBox::from_id_salt("new_layer_purpose").selected_text(purpose_name(self.new_purpose)).show_ui(ui, |ui| {
                for purpose in [
                    LayerPurpose::Mechanical,
                    LayerPurpose::User,
                    LayerPurpose::Outline,
                    LayerPurpose::Silkscreen,
                    LayerPurpose::Fabrication,
                    LayerPurpose::Courtyard,
                    LayerPurpose::Mask,
                    LayerPurpose::Paste,
                    LayerPurpose::Adhesive,
                ] {
                    if ui.selectable_value(&mut self.new_purpose, purpose, purpose_name(purpose)).changed() {
                        self.new_side = match purpose {
                            LayerPurpose::Mechanical | LayerPurpose::User | LayerPurpose::Outline => None,
                            _ => Some(Side::Front),
                        };
                    }
                }
            });
        });
        if self.new_purpose != LayerPurpose::Outline {
            properties::field(ui, "Side", |ui| {
                egui::ComboBox::from_id_salt("new_layer_side")
                    .selected_text(match self.new_side {
                        Some(Side::Front) => "Front",
                        Some(Side::Back) => "Back",
                        None => "Neither",
                    })
                    .show_ui(ui, |ui| {
                        if matches!(self.new_purpose, LayerPurpose::Mechanical | LayerPurpose::User) {
                            ui.selectable_value(&mut self.new_side, None, "Neither");
                        }
                        ui.selectable_value(&mut self.new_side, Some(Side::Front), "Front");
                        ui.selectable_value(&mut self.new_side, Some(Side::Back), "Back");
                    });
            });
        }
        if ui.button("Add layer").clicked() {
            let result = candidate.value(self.unit).and_then(|setup| {
                setup.with_added_technical_layer(self.new_name.clone(), self.new_purpose, self.new_side).map_err(|e| e.to_string())
            });
            match result {
                Ok(setup) => {
                    self.candidate = Some(Candidate::new(setup, self.unit));
                    self.new_name.clear();
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
    }

    /// Edit finished copper/dielectric thickness in physical order; blank values stay unknown.
    fn stackup(&mut self, ui: &mut Ui) {
        let Some(candidate) = &mut self.candidate else { return };
        ui.label(format!("Front to back · thickness in {}", self.unit.symbol()));
        let width = ((ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0).max(0.0);
        let height = pcbcraft_chrome::Tokens::get().control_height;
        egui::Grid::new("setup_stackup").num_columns(3).striped(true).show(ui, |ui| {
            for label in ["Layer".to_owned(), format!("Thickness ({})", self.unit.symbol()), "Material label".to_owned()] {
                ui.add_sized(egui::vec2(width, height), egui::Label::new(label).halign(egui::Align::Min));
            }
            ui.end_row();
            for (i, stratum) in candidate.setup.stackup().strata.iter().enumerate() {
                let label = match stratum {
                    Stratum::Copper { layer, .. } => candidate
                        .setup
                        .layers()
                        .iter()
                        .position(|l| l.id == *layer)
                        .map(|n| candidate.names[n].clone())
                        .unwrap_or_else(|| "Copper".into()),
                    Stratum::Dielectric { .. } => format!("Dielectric {}", i.div_ceil(2)),
                };
                ui.add_sized(egui::vec2(width, height), egui::Label::new(&label).truncate().halign(egui::Align::Min));
                ui.add_sized(
                    egui::vec2(width, height),
                    egui::TextEdit::singleline(&mut candidate.thickness[i].text)
                        .id_salt(("stratum_thickness", i))
                        .desired_width(width)
                        .hint_text("Unspecified"),
                )
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, format!("{label} thickness")));
                if matches!(stratum, Stratum::Dielectric { .. }) {
                    ui.add_sized(
                        egui::vec2(width, height),
                        egui::TextEdit::singleline(&mut candidate.material_labels[i])
                            .id_salt(("stratum_material", i))
                            .desired_width(width)
                            .hint_text("Unspecified"),
                    )
                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, format!("{label} material label")));
                } else {
                    ui.add_sized(egui::vec2(width, height), egui::Label::new("—").halign(egui::Align::Min));
                }
                ui.end_row();
            }
        });
    }
}

impl PcbApp {
    /// Open an explicitly addressed board's disposable draft, with the same revision used by MCP.
    pub(super) fn begin_board_setup(&mut self, view: ViewId) {
        if !matches!(self.dialog, Dialog::None) || self.files.busy() || self.project_action_pending() {
            return;
        }
        let Some(editor) = self.views.view(view).filter(|v| v.workspace == Workspace::Board && v.definition.is_none()) else { return };
        let Some(source) = editor.input(&self.session) else { return };
        let Some(board) = self.session.project().and_then(|p| editor.board.target.and_then(|t| p.board_view(t))) else { return };
        self.dialog = Dialog::BoardSetup(Box::new(Edit {
            source,
            board: board.target,
            unit: self.preferences.units,
            populated: board.footprints().next().is_some(),
            candidate: board.setup().cloned().map(|s| Candidate::new(s, self.preferences.units)),
            copper_layers: 2,
            stackup_tab: false,
            new_name: String::new(),
            new_purpose: LayerPurpose::User,
            new_side: None,
            error: None,
        }));
    }

    /// Submit one validated candidate through the shared command path. Failed/stale forms retain
    /// their fields; Cancel has no effect on model, revision, undo or saved bytes.
    pub(super) fn board_setup_dialog(&mut self, ctx: &Context, mut edit: Box<Edit>) {
        let valid = self.views.view(edit.source.view()).is_some_and(|v| v.accepts(edit.source, &self.session));
        if !valid {
            edit.error = Some("The project or view changed. Close Board setup and reopen it to edit the current board.".into());
        }
        let mut apply = false;
        let mut cancel = false;
        let can_apply = valid;
        let apply_label = if edit.candidate.is_some() { "Apply board setup" } else { "Create board setup" };
        let dismissed = pcbcraft_chrome::board::setup_dialog(
            ctx,
            |ui| {
                if edit.candidate.is_some() {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut edit.stackup_tab, false, "Layers");
                        ui.selectable_value(&mut edit.stackup_tab, true, "Stackup");
                    });
                    if edit.stackup_tab {
                        edit.stackup(ui);
                    } else {
                        edit.layers(ui);
                    }
                } else {
                    ui.label("Create copper and technical layers. Material and thickness values remain unspecified until you enter them.");
                    if edit.populated {
                        ui.label("Existing surface-mount pads will use front copper; through-hole pads will span all copper layers. Placements and package assignments are retained, and marker-only pads remain markers.");
                    }
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut edit.copper_layers, 2, "2 copper layers");
                        ui.radio_value(&mut edit.copper_layers, 4, "4 copper layers");
                    });
                }
                edit.error.clone()
            },
            |ui| {
                ui.horizontal(|ui| {
                    apply = ui.add_enabled(can_apply, egui::Button::new(apply_label)).clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            },
        );
        if cancel || dismissed {
            return;
        }
        if apply && valid {
            let command = if edit.candidate.is_some() { pcbcraft_engine::SET_BOARD_SETUP } else { pcbcraft_engine::INITIALIZE_BOARD_SETUP };
            let result = match &edit.candidate {
                Some(candidate) => candidate
                    .value(edit.unit)
                    .and_then(|setup| serde_json::to_value(setup).map_err(|e| e.to_string()))
                    .map(|setup| json!({"setup":setup})),
                None => Ok(json!({"copper_layers":edit.copper_layers,"resolve_existing_footprints":edit.populated})),
            };
            match result {
                Ok(mut params) => {
                    params["board"] = json!(edit.board.board);
                    params["expected_revision"] = json!(edit.source.revision);
                    self.run(command, &params);
                    if let Some(error) = self.message.take() {
                        edit.error = Some(error);
                    } else {
                        self.repaint_editors(ctx);
                        return;
                    }
                }
                Err(error) => edit.error = Some(error),
            }
        }
        self.dialog = Dialog::BoardSetup(edit);
    }
}

#[cfg(test)]
mod tests;
