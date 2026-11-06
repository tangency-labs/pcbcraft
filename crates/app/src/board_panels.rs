//! Board Browser, grouped Layers and read-only Properties follow the FigCraft workspace layout.
use crate::{PcbApp, editor_view::BoardState};
use egui::{Id, Rect, Ui};
use pcbcraft_chrome::{board, units, workspace};
use pcbcraft_engine::board::setup::{BoardSetup, Layer, LayerPurpose, Side, Stratum};

impl PcbApp {
    /// Render the focused board's contextual panels inside the host's header-free area, with shared edge anchors.
    /// Browsing explicitly selects and centers a placement; layer and collapse changes are local.
    pub(super) fn board_panels(&mut self, ui: &mut Ui, area: Rect) -> [Rect; 3] {
        let Some(project) = self.session.project() else {
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
        let mut pick = None;
        let mut sheet = None;
        let (browser, collapse) =
            workspace::floating_card_with_id(ui, host.with("browser"), "Browser", browser_area, state.browser_collapsed, |ui| {
                ui.label(&project.name);
                egui::CollapsingHeader::new("Sheets").id_salt("board_browser_sheets").default_open(true).show(ui, |ui| {
                    if ui.selectable_label(false, "Main").clicked() {
                        sheet = Some(project.sheet_target());
                    }
                });
                egui::CollapsingHeader::new(target.name()).id_salt((target.target, "board")).default_open(true).show(ui, |ui| {
                    egui::CollapsingHeader::new("Footprints").id_salt((target.target, "footprints")).default_open(true).show(ui, |ui| {
                        for footprint in target.footprints() {
                            let response = ui
                                .push_id(&footprint.id, |ui| {
                                    ui.selectable_label(
                                        state.selection.contains(&footprint.id),
                                        format!("{}  {}", footprint.reference, footprint.value),
                                    )
                                })
                                .inner;
                            if response.clicked() {
                                pick = Some((footprint.id, footprint.position));
                            }
                        }
                    });
                });
            });
        if collapse {
            state.browser_collapsed = !state.browser_collapsed;
        }
        let mut layers_area = area;
        if browser.is_finite() {
            layers_area.min.y = browser.max.y;
        }
        let (layers, collapse) = workspace::floating_card_with_id(ui, host.with("layers"), "Layers", layers_area, state.layers_collapsed, |ui| {
            if let Some(setup) = target.setup() {
                layer_groups(ui, setup, state, Id::new((view_id, target.target)));
            } else {
                ui.label("Stackup unspecified");
            }
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
                        property(ui, "Pad centers", &footprint.pads.len().to_string());
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
            });
        if collapse {
            state.properties_collapsed = !state.properties_collapsed;
        }
        if let Some((id, position)) = pick {
            self.views.select_board(std::collections::BTreeSet::from([id]), &self.session, self.preferences.selection_follow);
            self.views.active.camera.center_on(position);
            self.repaint_editors(ui.ctx());
        }
        if let Some(target) = sheet {
            self.queue_view_action(
                ui.ctx(),
                view_id,
                crate::editor_windows::ViewAction::OpenDocument(crate::editor_view::DocumentTarget::Sheet(target)),
            );
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

/// Copper follows physical stack order; technical layers group by purpose and sort by display
/// name. Renaming or regrouping does not change local visibility keyed by stable layer identity.
fn layer_groups(ui: &mut Ui, setup: &BoardSetup, state: &mut BoardState, id: Id) {
    let mut layers: Vec<_> = setup.layers().iter().collect();
    layers.sort_by_cached_key(|layer| {
        (
            setup.stackup().strata.iter().position(|stratum| matches!(stratum,Stratum::Copper{layer:id,..}if *id==layer.id)).unwrap_or(usize::MAX),
            layer.name.to_lowercase(),
        )
    });
    for (index, title) in ["Copper", "Board", "Component details", "Manufacturing", "User"].into_iter().enumerate() {
        let items: Vec<_> = layers.iter().filter(|layer| group(layer.purpose) == index).collect();
        if items.is_empty() {
            continue;
        }
        egui::CollapsingHeader::new(title).id_salt(id.with(index)).default_open(index == 0).show(ui, |ui| {
            for layer in items {
                layer_row(ui, layer, state);
            }
        });
    }
}

/// Visibility and active layer are independent. Technical controls retain state for future
/// geometry; the current legacy through-hole inspection markers use visible copper only.
fn layer_row(ui: &mut Ui, layer: &Layer, state: &mut BoardState) {
    let id = layer.id.to_string();
    ui.push_id(&id, |ui| {
        ui.horizontal(|ui| {
            let mut visible = !state.hidden_layers.contains(&id);
            let response = ui.checkbox(&mut visible, "");
            response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, visible, format!("Show layer {}", layer.name)));
            if response.changed() {
                if visible {
                    state.hidden_layers.remove(&id);
                } else {
                    state.hidden_layers.insert(id.clone());
                }
            }
            if layer.purpose == LayerPurpose::Copper {
                board::layer_swatch(ui, board::copper_color(layer.side.map(|side| side == Side::Front)));
            }
            if ui.selectable_label(state.active_layer.as_ref() == Some(&id), &layer.name).clicked() {
                state.active_layer = Some(id.clone());
            }
        })
    });
}
