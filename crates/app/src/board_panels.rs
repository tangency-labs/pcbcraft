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
        let input = self.views.active.input(&self.session);
        let state = &mut self.views.active.board;
        let browser_selection = state.probe_footprints();
        let Some(target) = state.target.and_then(|target| project.board_view(target)) else {
            return ([Rect::NOTHING; 3], Rect::NOTHING);
        };
        let host = Id::unique(("board_panels", self.views.host));
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
                    &browser_selection,
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
        let mut ratsnest_visible = !state.hidden_layers.contains(crate::editor_view::RATSNEST_KEY);
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
                    show_all = pcbcraft_chrome::controls::small_button(ui, "Show all").clicked();
                    isolate = ui.add_enabled_ui(can_isolate, |ui| pcbcraft_chrome::controls::small_button(ui, "Isolate active")).inner.clicked();
                    if target.has_accepted_connectivity() {
                        pcbcraft_chrome::board::ratsnest_visibility(ui, &mut ratsnest_visible);
                    }
                });
            },
            |ui| {
                layer_groups(ui, target, state, Id::unique((view_id, target.target)), &self.board_appearance, &mut color_pick);
            },
        );
        if collapse {
            state.layers_collapsed = !state.layers_collapsed;
        }
        state.set_ratsnest_visibility(ratsnest_visible);
        if show_all || isolate {
            state.set_layer_visibility(target.layers().iter().map(|layer| layer.id.to_string()), isolate);
        }
        let unit = self.preferences.units;
        // The ratsnest for this revision, through the same per-input cache the canvas reads.
        let unrouted = state.ratsnest(target, input).map(|ratsnest| ratsnest.lines.len());
        let mut copper_edit: Option<(&'static str, serde_json::Value)> = None;
        let mut reference_action = None;
        let mut reference_pick = None;
        let (properties, collapse) = self.views.active.panels[crate::workspace_panels::PROPERTIES].show(
            ui,
            host.with("properties"),
            "Properties",
            area,
            slots[crate::workspace_panels::PROPERTIES],
            state.properties_collapsed,
            workspace::panels::card_width(area, false),
            |ui| {
                if let Some(footprint) = state.reference.selected.as_ref().and_then(|id| target.footprints().find(|footprint| &footprint.id == id))
                    && let Some(input) = input
                {
                    let enabled = settings_enabled && !state.editing_footprint();
                    reference_action =
                        state.reference.show(ui, &footprint, input, unit, enabled, pcbcraft_chrome::properties::form_body_height(area));
                    return;
                }
                let selected: Vec<_> = target.footprints().filter(|footprint| state.selection.contains(&footprint.id)).collect();
                egui::Grid::new("board_properties").num_columns(2).show(ui, |ui| match selected.as_slice() {
                    [footprint] => {
                        property(ui, "Reference", footprint.reference);
                        property(ui, "Value", footprint.value);
                        property(ui, "Board", target.name());
                        property(ui, "Footprint", footprint.definition);
                        ui.label("Silkscreen");
                        if ui.button("Edit silkscreen text").clicked() {
                            reference_pick = Some(footprint.id.clone());
                        }
                        ui.end_row();
                        copper_edit = footprint_transform(ui, footprint, unit);
                        property(
                            ui,
                            if footprint.pads.iter().any(|pad| pad.copper.is_some()) { "Pads" } else { "Pad centers" },
                            &footprint.pads.len().to_string(),
                        );
                    }
                    [] if !state.tracks.selected.is_empty() => {
                        copper_edit = copper_properties(ui, target, &state.tracks.selected, unit);
                    }
                    [] => {
                        property(ui, "Board", target.name());
                        match target.outline_bounds() {
                            Some((min, max)) => property(
                                ui,
                                "Size",
                                &format!("{} × {}", units::label(max.x_nm - min.x_nm, unit), units::label(max.y_nm - min.y_nm, unit)),
                            ),
                            None => property(ui, "Size", "No outline"),
                        }
                        if let Some(setup) = target.setup() {
                            property(
                                ui,
                                "Copper layers",
                                &setup.layers().iter().filter(|layer| layer.purpose == LayerPurpose::Copper).count().to_string(),
                            );
                        }
                        property(ui, "Footprints", &target.footprints().count().to_string());
                        property(ui, "Tracks", &target.tracks().len().to_string());
                        property(ui, "Vias", &target.vias().len().to_string());
                        if let Some(lines) = unrouted {
                            property(ui, "Unrouted", &lines.to_string());
                        }
                    }
                    items => property(ui, "Selected footprints", &items.len().to_string()),
                });
            },
        );
        if collapse {
            state.properties_collapsed = !state.properties_collapsed;
        }
        if let Some(action) = reference_action {
            self.apply_reference_action(action);
            self.repaint_editors(ui.ctx());
        }
        if let Some(footprint) = reference_pick {
            self.views.select_board_reference(footprint, &self.session, self.preferences.selection_follow);
            self.repaint_editors(ui.ctx());
        }
        if let Some((command, mut params)) = copper_edit {
            params["board"] = serde_json::json!(target.target.board);
            params["expected_revision"] = serde_json::json!(self.session.revision());
            self.run(command, &params);
            self.repaint_editors(ui.ctx());
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

/// Edit one footprint's X, Y, rotation and side in place. Each field submits the full transform
/// through `pcb.set_footprint_transform` once, on release or confirmation, keeping the other
/// values; the caller adds the board and revision. Unchanged values submit nothing.
fn footprint_transform(
    ui: &mut Ui,
    footprint: &pcbcraft_engine::board::view::FootprintView<'_>,
    unit: units::Unit,
) -> Option<(&'static str, serde_json::Value)> {
    let limit = pcbcraft_engine::schematic::MAX_CENTER_NM;
    let (mut x, mut y, mut rotation, mut bottom) = (footprint.position.x_nm, footprint.position.y_nm, footprint.rotation, footprint.bottom);
    let mut changed = false;
    ui.label("X");
    if let Some(value) = committed_length(ui, ("Footprint X", &footprint.id), x, -limit..=limit, unit) {
        (x, changed) = (value, true);
    }
    ui.end_row();
    ui.label("Y");
    if let Some(value) = committed_length(ui, ("Footprint Y", &footprint.id), y, -limit..=limit, unit) {
        (y, changed) = (value, true);
    }
    ui.end_row();
    ui.label("Rotation");
    if let Some(value) = committed_degrees(ui, ("Footprint rotation", &footprint.id), rotation) {
        (rotation, changed) = (value, true);
    }
    ui.end_row();
    ui.label("Side");
    ui.horizontal(|ui| {
        for (label, value) in [("Top", false), ("Bottom", true)] {
            if ui.selectable_label(bottom == value, label).clicked() && bottom != value {
                (bottom, changed) = (value, true);
            }
        }
    });
    ui.end_row();
    changed.then(|| {
        (
            pcbcraft_engine::SET_FOOTPRINT_TRANSFORM,
            serde_json::json!({"footprint":footprint.id,"x_nm":x,"y_nm":y,"rotation":rotation,"side":if bottom { "back" } else { "front" }}),
        )
    })
}

/// A whole-degree rotation field, 0 through 359 with wrap-around, that keeps its draft while
/// dragged or typed into and returns once on release or confirmation, only when it changed.
fn committed_degrees(ui: &mut Ui, salt: (&str, impl std::hash::Hash + std::fmt::Debug), model: u16) -> Option<u16> {
    let id = ui.scope_id().with(&salt);
    let mut value = ui.data(|data| data.get_temp::<i64>(id)).unwrap_or(i64::from(model));
    let response = ui.add(egui::DragValue::new(&mut value).speed(1.0).suffix("°"));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::SpinButton, true, salt.0));
    let commit = response.drag_stopped() || (response.changed() && !response.dragged());
    if response.dragged() || response.has_focus() {
        ui.data_mut(|data| data.insert_temp(id, value));
    } else {
        ui.data_mut(|data| data.remove::<i64>(id));
    }
    let degrees = value.rem_euclid(360) as u16;
    (commit && degrees != model).then_some(degrees)
}

/// Inspect selected copper and atomically change its lock state; locked or mixed-locked selections keep sizes read-only.
/// Returns at most one command, submitted when a field is released or confirmed; the caller adds
/// the board and revision. Mixed track widths show the first width and set all of them; vias
/// of different sizes show "Mixed" and stay read-only, since each via field sends both sizes.
fn copper_properties(
    ui: &mut Ui,
    target: pcbcraft_engine::board::view::BoardView<'_>,
    selected: &std::collections::BTreeSet<uuid::Uuid>,
    unit: units::Unit,
) -> Option<(&'static str, serde_json::Value)> {
    use pcbcraft_engine::board::track::{MAX_VIA_DIAMETER_NM, MAX_WIDTH_NM, MIN_VIA_DRILL_NM, MIN_WIDTH_NM};
    let tracks: Vec<_> = target.tracks().iter().filter(|track| selected.contains(&track.id)).collect();
    let vias: Vec<_> = target.vias().iter().filter(|via| selected.contains(&via.id)).collect();
    let salt = selected.iter().next().copied().unwrap_or_default();
    let mut edit = None;
    property(ui, "Selected tracks", &tracks.len().to_string());
    property(ui, "Selected vias", &vias.len().to_string());
    let any_locked = tracks.iter().any(|track| track.locked) || vias.iter().any(|via| via.locked);
    let all_locked = tracks.iter().all(|track| track.locked) && vias.iter().all(|via| via.locked);
    let mut lock = if all_locked {
        "Locked"
    } else if any_locked {
        "Mixed"
    } else {
        "Unlocked"
    };
    let original_lock = lock;
    ui.label("Lock");
    let response = pcbcraft_chrome::controls::Choice::from_id_salt(("copper_lock", salt)).selected_text(lock).show_ui(ui, |ui| {
        ui.selectable_value(&mut lock, "Unlocked", "Unlocked");
        ui.selectable_value(&mut lock, "Locked", "Locked");
    });
    response.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, ui.is_enabled(), "Copper lock"));
    ui.end_row();
    if lock != original_lock {
        let mut params = serde_json::json!({"locked":lock == "Locked"});
        if !tracks.is_empty() {
            params["tracks"] = serde_json::json!(tracks.iter().map(|track| track.id).collect::<Vec<_>>());
        }
        if !vias.is_empty() {
            params["vias"] = serde_json::json!(vias.iter().map(|via| via.id).collect::<Vec<_>>());
        }
        return Some((pcbcraft_engine::SET_COPPER_LOCKED, params));
    }
    if let Some(first) = tracks.first() {
        let layers = target.layers();
        if tracks.iter().all(|track| track.layer == first.layer)
            && let Some(layer) = layers.iter().find(|layer| layer.id == first.layer)
        {
            property(ui, "Layer", layer_name(layer, &layers));
        }
        ui.label("Width");
        if let Some(width) = ui
            .add_enabled_ui(!any_locked, |ui| committed_length(ui, ("Selected track width", salt), first.width_nm, MIN_WIDTH_NM..=MAX_WIDTH_NM, unit))
            .inner
        {
            edit = Some((
                pcbcraft_engine::SET_TRACK_WIDTH,
                serde_json::json!({"tracks":tracks.iter().map(|t| t.id).collect::<Vec<_>>(),"width_nm":width}),
            ));
        }
        ui.end_row();
    }
    if let Some(first) = vias.first()
        && !vias.iter().all(|via| (via.diameter_nm, via.drill_nm) == (first.diameter_nm, first.drill_nm))
    {
        // One field would overwrite the other dimension on every via; select matching vias to edit.
        property(ui, "Via pad", "Mixed");
        property(ui, "Via drill", "Mixed");
    } else if let Some(first) = vias.first() {
        let ids: Vec<_> = vias.iter().map(|via| via.id).collect();
        ui.label("Via pad");
        if let Some(pad) = ui
            .add_enabled_ui(!any_locked, |ui| {
                committed_length(ui, ("Selected via pad", salt), first.diameter_nm, MIN_VIA_DRILL_NM + 1..=MAX_VIA_DIAMETER_NM, unit)
            })
            .inner
        {
            edit = Some((
                pcbcraft_engine::SET_VIA_SIZE,
                serde_json::json!({"vias":ids,"via_diameter_nm":pad,"via_drill_nm":first.drill_nm.min(pad - 1)}),
            ));
        }
        ui.end_row();
        ui.label("Via drill");
        if let Some(drill) = ui
            .add_enabled_ui(!any_locked, |ui| {
                committed_length(ui, ("Selected via drill", salt), first.drill_nm, MIN_VIA_DRILL_NM..=first.diameter_nm - 1, unit)
            })
            .inner
        {
            edit = Some((pcbcraft_engine::SET_VIA_SIZE, serde_json::json!({"vias":ids,"via_diameter_nm":first.diameter_nm,"via_drill_nm":drill})));
        }
        ui.end_row();
    }
    edit
}

/// A length field in the display units that keeps its draft while dragged or typed into and
/// returns whole nanometers once, on release or confirmation, only when the value changed.
/// The salt's name is the field's accessible label; its key keeps drafts per selection.
fn committed_length(
    ui: &mut Ui,
    salt: (&str, impl std::hash::Hash + std::fmt::Debug),
    model_nm: i64,
    range: std::ops::RangeInclusive<i64>,
    unit: units::Unit,
) -> Option<i64> {
    let per_unit = match unit {
        units::Unit::Mm => units::NM_PER_MM,
        units::Unit::Mil => units::NM_PER_MIL,
    } as f64;
    let id = ui.scope_id().with(&salt);
    let mut value = ui.data(|data| data.get_temp::<f64>(id)).unwrap_or(model_nm as f64 / per_unit);
    let response = ui.add(
        egui::DragValue::new(&mut value)
            .range(*range.start() as f64 / per_unit..=*range.end() as f64 / per_unit)
            .speed(if unit == units::Unit::Mm { 0.01 } else { 0.5 })
            .max_decimals(3)
            .suffix(format!(" {}", unit.symbol())),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::SpinButton, true, salt.0));
    let commit = response.drag_stopped() || (response.changed() && !response.dragged());
    if response.dragged() || response.has_focus() {
        ui.data_mut(|data| data.insert_temp(id, value));
    } else {
        ui.data_mut(|data| data.remove::<f64>(id));
    }
    let nm = ((value * per_unit).round() as i64).clamp(*range.start(), *range.end());
    (commit && nm != model_nm).then_some(nm)
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

/// Display previous default outline names as Board Outline unless that would duplicate another
/// layer's label. A collision retains authored names; saved names and IDs are never rewritten.
pub(super) fn layer_name<'a>(layer: &'a Layer, layers: &[Layer]) -> &'a str {
    let collision = layers.iter().any(|other| {
        other.id != layer.id
            && (other.name.eq_ignore_ascii_case("Board Outline") || (other.purpose == LayerPurpose::Outline && other.name == "Board Edge"))
    });
    if layer.purpose == LayerPurpose::Outline && matches!(layer.name.as_str(), "Board Edge" | "Board outline") && !collision {
        "Board Outline"
    } else {
        &layer.name
    }
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
    for (index, title) in ["Copper", "Board", "Part details", "Manufacturing", "User"].into_iter().enumerate() {
        let items: Vec<_> = layers.iter().filter(|layer| group(layer.purpose) == index).collect();
        if items.is_empty() {
            continue;
        }
        let contains_active = items.iter().any(|layer| state.active_layer.as_deref() == Some(layer.id.to_string().as_str()));
        pcbcraft_chrome::controls::collapsing(ui, id.with(index), title, index <= 1, (reveal && contains_active).then_some(true), |ui| {
            for layer in items {
                if layer_row(ui, layer, &layers, state, reveal, board, appearance) {
                    *pick = Some(layer.id.to_string());
                }
            }
        });
    }
}

/// Visibility and activation have separate hit targets. Eye changes retain the active layer;
/// both copper and technical visibility remain view-local and keyed by stable layer identity.
fn layer_row(
    ui: &mut Ui,
    layer: &Layer,
    layers: &[Layer],
    state: &mut BoardState,
    reveal: bool,
    board: pcbcraft_engine::board::view::BoardView<'_>,
    appearance: &crate::board_appearance::Appearance,
) -> bool {
    let id = layer.id.to_string();
    let label = layer_name(layer, layers);
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
                    ui.scope_id().with("activate"),
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
