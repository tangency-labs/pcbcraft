//! Board Browser, grouped Layers and contextual placement readouts.
use crate::{PcbApp, editor_view::BoardState};
use egui::{Id, Rect, Ui};
use pcbcraft_chrome::{board, units, workspace};
use pcbcraft_engine::board::setup::{Layer, LayerPurpose, Stratum};

impl PcbApp {
    /// Render the focused board's contextual panels inside the host's header-free area, with shared edge anchors.
    /// Browsing explicitly selects and centers a placement; layer and collapse changes are local.
    /// Dock slots keep Layers, Properties and Selection filters separate on either edge.
    pub(super) fn board_panels(&mut self, ui: &mut Ui, area: Rect) -> ([Rect; 3], Rect) {
        let settings_enabled = matches!(self.dialog, crate::Dialog::None) && !self.project_action_pending() && !self.files.busy();
        let Some(project) = self.session.snapshot() else {
            return ([Rect::NOTHING; 3], Rect::NOTHING);
        };
        let view_id = self.views.active.id();
        let state = &mut self.views.active.board;
        let Some(target) = state.target.and_then(|target| project.board_view(target)) else {
            return ([Rect::NOTHING; 3], Rect::NOTHING);
        };
        let host = Id::new(("board_panels", self.views.host));
        let mut browser_area = area;
        browser_area.max.y = board::browser_bottom(area);
        let mut color_pick = None;
        let mut setup_open = false;
        let (mut show_all, mut isolate) = (false, false);
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
        let slots = crate::workspace_panels::areas(area, browser, &self.views.active.panels, [true, true, self.views.active.filters_open], false);
        let can_isolate = state.active_layer.is_some();
        let mut ratsnest_visible = !state.hidden_layers.contains("ratsnest");
        let (layers, collapse) = self.views.active.panels[crate::workspace_panels::LAYERS].show_with_toolbar(
            ui,
            host.with("layers"),
            "Layers",
            area,
            slots[crate::workspace_panels::LAYERS],
            state.layers_collapsed,
            workspace::panels::card_width(area, false),
            pcbcraft_chrome::board_setup::layer_actions_height(),
            |ui| {
                setup_open = pcbcraft_chrome::board_setup::open_button(ui, settings_enabled).clicked();
                ui.horizontal(|ui| {
                    show_all = ui.small_button("Show all").clicked();
                    isolate = ui.add_enabled(can_isolate, egui::Button::new("Isolate active").small()).clicked();
                    if target.has_accepted_connectivity() {
                        pcbcraft_chrome::board::ratsnest_visibility(ui, &mut ratsnest_visible);
                    }
                });
            },
            |ui| {
                layer_groups(ui, target, state, Id::new((view_id, target.target)), &self.board_appearance, &mut color_pick);
            },
        );
        if collapse {
            state.layers_collapsed = !state.layers_collapsed;
        }
        state.set_ratsnest_visibility(ratsnest_visible);
        if show_all || isolate {
            state.set_layer_visibility(target.layers().iter().map(|layer| layer.id.to_string()), isolate);
        }
        let (properties, collapse) = self.views.active.panels[crate::workspace_panels::PROPERTIES].show(
            ui,
            host.with("properties"),
            "Properties",
            area,
            slots[crate::workspace_panels::PROPERTIES],
            state.properties_collapsed,
            workspace::panels::card_width(area, false),
            |ui| {
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
            },
        );
        if collapse {
            state.properties_collapsed = !state.properties_collapsed;
        }
        if let Some(action) = pick {
            self.queue_view_action(ui.ctx(), view_id, crate::editor_windows::ViewAction::Browse(action));
        }
        if let Some(layer) = color_pick {
            self.begin_layer_color(view_id, layer);
            self.repaint_editors(ui.ctx());
        }
        if setup_open {
            self.begin_board_setup(view_id);
        }
        ([browser, layers, properties], slots[crate::workspace_panels::FILTERS])
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
    ordered_catalog(board.layers(), board.setup())
}

/// Preview catalogs use the same physical copper order and technical groups as the live board.
pub(super) fn ordered_catalog(mut layers: Vec<Layer>, setup: Option<&pcbcraft_engine::board::setup::BoardSetup>) -> Vec<Layer> {
    layers.sort_by_cached_key(|layer| {
        (
            group(layer.purpose),
            setup
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

/// Layer keys share the effective palette used by their supported canvas geometry.
fn layer_color(layer: &Layer, board: pcbcraft_engine::board::view::BoardView<'_>, appearance: &crate::board_appearance::Appearance) -> egui::Color32 {
    appearance.color(board, layer)
}

/// Copper follows physical stack order; technical layers group by purpose and sort by display
/// name. Renaming or regrouping does not change local visibility keyed by stable layer identity.
/// A new active choice or resized viewport reveals the active row once, without fighting later scrolling.
fn layer_groups(
    ui: &mut Ui,
    board: pcbcraft_engine::board::view::BoardView<'_>,
    state: &mut BoardState,
    id: Id,
    appearance: &crate::board_appearance::Appearance,
    pick: &mut Option<String>,
) {
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
                    if layer_row(ui, layer, state, reveal, board, appearance) {
                        *pick = Some(layer.id.to_string());
                    }
                }
            },
        );
    }
}

/// Visibility and activation have separate hit targets. Eye changes retain the active layer;
/// both copper and technical visibility remain view-local and keyed by stable layer identity.
fn layer_row(
    ui: &mut Ui,
    layer: &Layer,
    state: &mut BoardState,
    reveal: bool,
    board: pcbcraft_engine::board::view::BoardView<'_>,
    appearance: &crate::board_appearance::Appearance,
) -> bool {
    let id = layer.id.to_string();
    let label = layer_name(layer);
    let mut color_clicked = false;
    ui.push_id(&id, |ui| {
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let visible = !state.hidden_layers.contains(&id);
                if board::layer_visibility(ui, label, visible).clicked() {
                    state.set_layer(&id, false, Some(!visible));
                }
                let active = state.active_layer.as_ref() == Some(&id);
                let row = workspace::tree_row(
                    ui,
                    ui.id().with("activate"),
                    &workspace::TreeRow {
                        depth: 0,
                        fold: None,
                        icon: Some(workspace::TreeIcon::Layer { color: layer_color(layer, board, appearance), active, editable: true }),
                        label,
                        accessibility_label: Some(&format!("Activate layer {label}")),
                        tip: if active { "Active layer" } else { "Set active layer" },
                        selected: active,
                    },
                );
                if reveal && active {
                    row.response.scroll_to_me(Some(egui::Align::Center));
                }
                color_clicked = row.icon_clicked;
                if row.clicked {
                    state.set_layer(&id, true, None);
                }
            });
        })
    });
    color_clicked
}
