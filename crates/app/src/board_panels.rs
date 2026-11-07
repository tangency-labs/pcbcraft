//! Board Browser, grouped Layers, placement readouts and editable board silkscreen settings.
use crate::{PcbApp, editor_view::BoardState};
use egui::{Id, Rect, Ui};
use pcbcraft_chrome::{board, units, workspace};
use pcbcraft_engine::board::setup::{Layer, LayerPurpose, Side, Stratum};

impl PcbApp {
    /// Render the focused board's contextual panels inside the host's header-free area, with shared edge anchors.
    /// Browsing explicitly selects and centers a placement; layer and collapse changes are local.
    pub(super) fn board_panels(&mut self, ui: &mut Ui, area: Rect) -> [Rect; 3] {
        let settings_enabled = matches!(self.dialog, crate::Dialog::None) && !self.project_action_pending() && !self.files.busy();
        let Some(project) = self.session.snapshot() else {
            return [Rect::NOTHING; 3];
        };
        let view_id = self.views.active.id();
        let state = &mut self.views.active.board;
        let Some(target) = state.target.and_then(|target| project.board_view(target)) else {
            return [Rect::NOTHING; 3];
        };
        let host = Id::new(("board_panels", self.views.host));
        let mut browser_area = area;
        browser_area.max.y = board::browser_bottom(area);
        let mut silk_apply = None;
        let mut pick = None;
        let (browser, collapse) = if self.browser.open {
            workspace::floating_card_with_id(ui, host.with("browser"), "Browser", browser_area, state.browser_collapsed, |ui| {
                pick = self.browser.tree(
                    ui,
                    project.clone(),
                    crate::editor_view::DocumentTarget::Board(target.target),
                    &self.views.active.schematic.selection,
                    &state.selection,
                );
            })
        } else {
            (Rect::NOTHING, false)
        };
        if collapse {
            state.browser_collapsed = !state.browser_collapsed;
        }
        let mut layers_area = area;
        if browser.is_finite() {
            layers_area.min.y = browser.max.y;
        }
        let (layers, collapse) = workspace::floating_card_with_id(ui, host.with("layers"), "Layers", layers_area, state.layers_collapsed, |ui| {
            layer_groups(ui, target, state, Id::new((view_id, target.target)));
        });
        if collapse {
            state.layers_collapsed = !state.layers_collapsed;
        }
        let (properties, collapse) =
            workspace::floating_card_aligned(ui, host.with("properties"), "Properties", area, state.properties_collapsed, true, |ui| {
                let selected: Vec<_> = target.footprints().filter(|footprint| state.selection.contains(&footprint.id)).collect();
                egui::Grid::new("board_properties").num_columns(2).show(ui, |ui| match selected.as_slice() {
                    [footprint] => {
                        property(ui, "Reference", footprint.reference);
                        property(ui, "Value", footprint.value);
                        property(ui, "Board", target.name());
                        property(ui, "Footprint", footprint.definition);
                        property(ui, "X", &units::label(footprint.position.x_nm, self.preferences.units));
                        property(ui, "Y", &units::label(footprint.position.y_nm, self.preferences.units));
                        property(ui, "Rotation", &format!("{}°", footprint.rotation));
                        property(
                            ui,
                            if footprint.pads.iter().any(|pad| pad.copper.is_some()) { "Pads" } else { "Pad centers" },
                            &footprint.pads.len().to_string(),
                        );
                    }
                    [] => {
                        property(ui, "Board", target.name());
                        property(ui, "Footprints", &target.footprints().count().to_string());
                        if let Some(setup) = target.setup() {
                            property(
                                ui,
                                "Copper layers",
                                &setup.layers().iter().filter(|layer| layer.purpose == LayerPurpose::Copper).count().to_string(),
                            );
                        }
                    }
                    items => property(ui, "Selected footprints", &items.len().to_string()),
                });
                ui.add_enabled_ui(settings_enabled, |ui| {
                    silk_apply = silkscreen_settings(ui, state, target, self.session.revision(), self.preferences.units);
                });
            });
        if collapse {
            state.properties_collapsed = !state.properties_collapsed;
        }
        if let Some(params) = silk_apply {
            self.run(pcbcraft_engine::SET_SILKSCREEN, &params);
            if self.message.is_none() {
                self.views.active.board.silkscreen_draft = None;
            }
            self.repaint_editors(ui.ctx());
        }
        if let Some(action) = pick {
            self.queue_view_action(ui.ctx(), view_id, crate::editor_windows::ViewAction::Browse(action));
        }
        [browser, layers, properties]
    }
}

/// Show values supported by the model without inventing editable geometry or guessed dimensions.
fn property(ui: &mut Ui, label: &str, value: &str) {
    ui.label(label);
    ui.add(egui::Label::new(value).wrap());
    ui.end_row();
}

/// Group names describe presentation only; physical layers remain addressed by their engine IDs.
fn group(purpose: LayerPurpose) -> usize {
    match purpose {
        LayerPurpose::Copper => 0,
        LayerPurpose::Outline | LayerPurpose::Mechanical => 1,
        LayerPurpose::Silkscreen | LayerPurpose::Courtyard | LayerPurpose::Fabrication => 2,
        LayerPurpose::Mask | LayerPurpose::Paste | LayerPurpose::Adhesive => 3,
        LayerPurpose::User => 4,
    }
}

/// Keep copper in stack order and group technical layers consistently in the picker and card.
pub(super) fn ordered_layers(board: pcbcraft_engine::board::view::BoardView<'_>) -> Vec<Layer> {
    let mut layers = board.layers();
    layers.sort_by_cached_key(|layer| {
        (
            group(layer.purpose),
            board
                .setup()
                .and_then(|setup| setup.stackup().strata.iter().position(|stratum| matches!(stratum,Stratum::Copper{layer:id,..}if *id==layer.id)))
                .unwrap_or(usize::MAX),
            layer.name.to_lowercase(),
        )
    });
    layers
}

/// Retain older saved layer names while displaying the product's Board Edge terminology.
pub(super) fn layer_name(layer: &Layer) -> &str {
    if layer.purpose == LayerPurpose::Outline && layer.name == "Board outline" { "Board Edge" } else { &layer.name }
}

/// Copper keys reuse the board palette; technical keys use existing chrome ink roles.
fn layer_color(layer: &Layer) -> egui::Color32 {
    if layer.purpose == LayerPurpose::Copper {
        board::copper_color(layer.side.map(|side| side == Side::Front))
    } else {
        board::technical_layer_color(layer.purpose == LayerPurpose::Outline)
    }
}

/// Copper follows physical stack order; technical layers group by purpose and sort by display
/// name. Renaming or regrouping does not change local visibility keyed by stable layer identity.
/// A new active choice or resized viewport reveals the active row once, without fighting later scrolling.
fn layer_groups(ui: &mut Ui, board: pcbcraft_engine::board::view::BoardView<'_>, state: &mut BoardState, id: Id) {
    ui.horizontal(|ui| {
        if ui.small_button("Show all").clicked() {
            state.set_layer_visibility(board.layers().iter().map(|layer| layer.id.to_string()), false);
        }
        if ui.add_enabled(state.active_layer.is_some(), egui::Button::new("Isolate active").small()).clicked() {
            state.set_layer_visibility(board.layers().iter().map(|layer| layer.id.to_string()), true);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Visible");
        ui.add_space(pcbcraft_chrome::Tokens::get().space_24 + pcbcraft_chrome::Tokens::get().space_16);
        ui.label("Layer");
    });
    let current = (state.active_layer.clone(), ui.clip_rect().height().to_bits());
    let reveal = ui.ctx().data_mut(|data| {
        let key = id.with("last_active_layer");
        let before = data.get_temp::<(Option<String>, u32)>(key);
        let reveal = before.as_ref() != Some(&current);
        data.insert_temp(key, current);
        reveal
    });
    let layers = ordered_layers(board);
    for (index, title) in ["Copper", "Board", "Component details", "Manufacturing", "User"].into_iter().enumerate() {
        let items: Vec<_> = layers.iter().filter(|layer| group(layer.purpose) == index).collect();
        if items.is_empty() {
            continue;
        }
        let contains_active = items.iter().any(|layer| state.active_layer.as_deref() == Some(layer.id.to_string().as_str()));
        egui::CollapsingHeader::new(title).id_salt(id.with(index)).default_open(index <= 1).open((reveal && contains_active).then_some(true)).show(
            ui,
            |ui| {
                for layer in items {
                    layer_row(ui, layer, state, reveal);
                }
            },
        );
    }
}

/// Visibility and active layer are independent. Technical controls retain state for future
/// geometry; the current legacy through-hole inspection markers use visible copper only.
fn layer_row(ui: &mut Ui, layer: &Layer, state: &mut BoardState, reveal: bool) {
    let id = layer.id.to_string();
    let label = layer_name(layer);
    ui.push_id(&id, |ui| {
        ui.horizontal(|ui| {
            let mut visible = !state.hidden_layers.contains(&id);
            let response = ui.checkbox(&mut visible, "");
            response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, visible, format!("Show layer {}", label)));
            if response.changed() {
                state.set_layer(&id, false, Some(visible));
            }
            let active = state.active_layer.as_ref() == Some(&id);
            let row = workspace::tree_row(
                ui,
                ui.id().with("activate"),
                &workspace::TreeRow {
                    depth: 0,
                    fold: None,
                    icon: Some(workspace::TreeIcon::Layer { color: layer_color(layer), active }),
                    label,
                    accessibility_label: Some(&format!("Activate layer {label}")),
                    tip: if active { "Active layer" } else { "Set active layer" },
                    selected: active,
                },
            );
            if reveal && active {
                row.response.scroll_to_me(Some(egui::Align::Center));
            }
            if row.clicked {
                state.set_layer(&id, true, None);
            }
        })
    });
}

/// One view's unapplied, unit-aware board settings. A captured revision prevents overwriting
/// a concurrent edit; Cancel and field typing never change authored geometry.
#[derive(Debug)]
pub(super) struct SilkscreenDraft {
    revision: u64,
    units: units::Unit,
    values: [String; 3],
    initial: [String; 3],
    original: [i64; 3],
    error: Option<String>,
}
impl SilkscreenDraft {
    /// Seed display strings in the current unit while retaining exact nanometers for fields
    /// that are left unchanged; rounded presentation must not introduce an authored edit.
    fn new(revision: u64, unit: units::Unit, style: pcbcraft_engine::board::silkscreen::Settings) -> Self {
        let original = [style.text_height_nm, style.stroke_width_nm, style.pad_clearance_nm];
        let initial = original.map(|n| units::label(n, unit));
        Self { revision, units: unit, values: initial.clone(), initial, original, error: None }
    }
    /// Parse all fields and let the engine validate the complete candidate atomically.
    fn params(&self, board: pcbcraft_engine::board::view::BoardTarget) -> Result<serde_json::Value, String> {
        let lengths = self
            .values
            .iter()
            .enumerate()
            .map(|(index, text)| {
                if text.trim() == self.initial[index] {
                    Ok(self.original[index])
                } else {
                    units::parse_length(text, self.units).map_err(|e| e.to_string())
                }
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(serde_json::json!({"board":board.board,"expected_revision":self.revision,"settings":{
            "text_height_nm":lengths[0],"stroke_width_nm":lengths[1],"pad_clearance_nm":lengths[2]}}))
    }
}

/// Edit real board settings in Properties. Display-only pad numbers and net annotations are
/// deliberately independent from this printable silkscreen style.
fn silkscreen_settings(
    ui: &mut Ui,
    state: &mut BoardState,
    target: pcbcraft_engine::board::view::BoardView<'_>,
    revision: u64,
    unit: units::Unit,
) -> Option<serde_json::Value> {
    let mut apply = None;
    ui.collapsing("Silkscreen", |ui| {
        if let Some(draft) = &mut state.silkscreen_draft {
            egui::Grid::new("silkscreen_edit").num_columns(2).show(ui, |ui| {
                for (label, value) in ["Text height", "Stroke width", "Pad clearance"].into_iter().zip(&mut draft.values) {
                    let caption = ui.label(label);
                    ui.add(
                        egui::TextEdit::singleline(value)
                            .desired_width(pcbcraft_chrome::Tokens::get().form_label_width)
                            .hint_text(unit.symbol())
                            .id_salt(label),
                    )
                    .labelled_by(caption.id);
                    ui.end_row();
                }
            });
            if ui.button("Apply silkscreen settings").clicked() {
                match draft.params(target.target) {
                    Ok(params) => apply = Some(params),
                    Err(error) => draft.error = Some(error),
                }
            }
            let cancel = ui.button("Cancel silkscreen changes").clicked();
            if let Some(error) = &draft.error {
                ui.colored_label(pcbcraft_chrome::Tokens::get().error, error);
            }
            if cancel {
                state.silkscreen_draft = None;
            }
        } else {
            let style = target.silkscreen();
            egui::Grid::new("silkscreen_values").num_columns(2).show(ui, |ui| {
                for (label, value) in
                    [("Text height", style.text_height_nm), ("Stroke width", style.stroke_width_nm), ("Pad clearance", style.pad_clearance_nm)]
                {
                    property(ui, label, &units::label(value, unit));
                }
            });
            if ui.button("Edit silkscreen settings").clicked() {
                state.silkscreen_draft = Some(SilkscreenDraft::new(revision, unit, style));
            }
        }
    });
    apply
}
