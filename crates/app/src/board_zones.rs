//! Zone-specific intent and forms compose the shared board path editor and engine commands.
use crate::{
    Dialog, PcbApp,
    board_drawing::Tool,
    editor_view::{ViewInput, Workspace},
};
use pcbcraft_chrome::{controls, properties, shell};
use pcbcraft_engine::{
    board::{view::BoardView, zones::Connection},
    nets::ClassTarget,
};
use serde_json::{Value, json};

/// A creation chooser retains its exact view/revision and typed net targets until armed or cancelled.
#[derive(Debug)]
pub(super) struct Edit {
    source: ViewInput,
    choices: Vec<NetChoice>,
    selected: String,
    rectangle: bool,
}
impl PcbApp {
    /// Capture meaningful current nets without promotion; only a completed outline authors intent.
    pub(super) fn begin_zone(&mut self) {
        if self.views.active.workspace != Workspace::Board {
            return;
        }
        let Some(source) = self.views.active.input(&self.session) else { return };
        let Some(project) = self.session.project() else { return };
        let rows = match project.inspect_net_registry() {
            Ok(rows) => rows,
            Err(e) => {
                self.message = Some(e);
                return;
            }
        };
        let choices = net_choices(&rows);
        if choices.is_empty() {
            self.message = Some("No current nets are available for a zone.".into());
            return;
        }
        self.views.active.cancel_gesture();
        self.dialog = Dialog::Zone(Box::new(Edit { source, choices, selected: "0".into(), rectangle: false }));
    }
    /// Arm one existing polygon/rectangle gesture after a valid captured net selection.
    pub(super) fn zone_dialog(&mut self, ctx: &egui::Context, mut edit: Edit) {
        let (mut submit, mut cancel) = (false, false);
        let closing = shell::dialog(ctx, "zone_setup", "New Zone", |ui| {
            let choices: Vec<_> = edit.choices.iter().enumerate().map(|(i, choice)| (i.to_string(), choice.label.clone())).collect();
            let refs: Vec<_> = choices.iter().map(|(key, label)| (key.as_str(), label.clone())).collect();
            properties::choice(ui, "Net", egui::Id::unique("zone_net"), &mut edit.selected, &refs);
            ui.horizontal(|ui| {
                controls::checkbox(ui, &mut edit.rectangle, "Rectangle");
            });
            ui.horizontal(|ui| {
                submit = controls::button(ui, "Draw").clicked();
                cancel = controls::button(ui, "Cancel").clicked();
            });
        });
        if closing || cancel {
            return;
        }
        if submit {
            if !self.views.active.accepts(edit.source, &self.session) {
                self.message = Some("The board changed; choose the zone net again.".into());
                return;
            }
            if let Some(choice) = edit.selected.parse::<usize>().ok().and_then(|i| edit.choices.get(i)) {
                let state = &mut self.views.active.board;
                state.tracks.cancel();
                state.drawing.cancel();
                state.zones.zone = true;
                state.zones.zone_target = Some(choice.target.clone());
                state.zones.start(if edit.rectangle { Tool::Rectangle } else { Tool::Lines }, state.active_layer.clone());
                state.zones.zone_source = Some(edit.source);
            }
            return;
        }
        self.dialog = Dialog::Zone(Box::new(edit));
    }
}

/// Edit the selected authored zone through the same command as MCP, retaining full settings.
/// Missing/retired net identities remain visible and are never silently substituted.
/// Popup choices commit when their values change, independently of the trigger's response flags.
pub(super) fn properties(
    ui: &mut egui::Ui,
    view: BoardView<'_>,
    state: &mut crate::board_drawing::State,
    unit: pcbcraft_chrome::units::Unit,
    revision: u64,
) -> Option<(&'static str, Value)> {
    let zone = view.zones().iter().find(|z| state.selected.as_ref() == Some(&z.id.to_string()))?;
    if state.zone_choices.as_ref().is_none_or(|(target, rev, _)| *target != view.target || *rev != revision) {
        let choices = view.inspect_zone_nets().map(|rows| net_choices(&rows)).unwrap_or_default();
        state.zone_choices = Some((view.target, revision, choices));
    }
    let choices = &state.zone_choices.as_ref()?.2;
    properties::identity(ui, "Zone outline", "No copper fill");
    let status = view.zone_net_state(zone.net);
    let current = choices.iter().find(|c| c.id == Some(zone.net)).map_or_else(|| zone.net.to_string(), |c| c.label.clone());
    let mut net = "current".to_owned();
    let mut net_refs = vec![("current".to_owned(), format!("{current} ({status})"))];
    net_refs.extend(choices.iter().enumerate().filter(|(_, c)| c.id != Some(zone.net)).map(|(i, c)| (i.to_string(), c.label.clone())));
    let refs: Vec<_> = net_refs.iter().map(|(id, label)| (id.as_str(), label.clone())).collect();
    properties::choice(ui, "Net", egui::Id::unique(("zone_net_edit", zone.id)), &mut net, &refs);
    let net_changed = net != "current";
    if status != "active" {
        ui.label(shell::error(match status {
            "ambiguous" => "Ambiguous net identity.",
            "retired" => "Retired net identity.",
            _ => "Missing net identity.",
        }));
    }
    let mut changed = net_changed;
    let mut layer = zone.layer.to_string();
    let layers: Vec<_> = view
        .layers()
        .into_iter()
        .filter(|l| l.purpose == pcbcraft_engine::board::setup::LayerPurpose::Copper)
        .map(|l| (l.id.to_string(), l.name))
        .collect();
    let refs: Vec<_> = layers.iter().map(|(id, label)| (id.as_str(), label.clone())).collect();
    properties::choice(ui, "Layer", egui::Id::unique(("zone_layer", zone.id)), &mut layer, &refs);
    changed |= layer != zone.layer.to_string();
    let mut options = zone.options.clone();
    let mut priority = i64::from(options.priority);
    ui.label("Priority");
    if let Some(value) = scalar(ui, "Priority", zone.id, priority as u64, 1, 1000, revision) {
        priority = value as i64;
        changed = true;
    }
    options.priority = priority as u16;
    let mut clearance = options.clearance_nm.is_some();
    changed |= controls::checkbox(ui, &mut clearance, "Clearance override").changed();
    if clearance {
        let current = options.clearance_nm.unwrap_or(250_000);
        ui.label("Clearance");
        if let Some(value) = crate::board_panels::properties::zone_length(ui, ("Clearance", zone.id), current, unit, revision) {
            options.clearance_nm = Some(value);
            changed = true;
        } else {
            options.clearance_nm = Some(current);
        }
    } else {
        options.clearance_nm = None;
    }
    for (label, value) in [("Thermal gap", &mut options.thermal_gap_nm), ("Spoke width", &mut options.spoke_width_nm)] {
        ui.label(label);
        if let Some(next) = crate::board_panels::properties::zone_length(ui, (label, zone.id), *value, unit, revision) {
            *value = next;
            changed = true;
        }
    }
    let original_connection = match options.connection {
        Connection::Thermal => "thermal",
        Connection::Solid => "solid",
        Connection::None => "none",
    };
    let mut connection = original_connection.to_owned();
    properties::choice(
        ui,
        "Connection",
        egui::Id::unique(("zone_connection", zone.id)),
        &mut connection,
        &[("thermal", "Thermal".into()), ("solid", "Solid".into()), ("none", "None".into())],
    );
    changed |= connection != original_connection;
    options.connection = match connection.as_str() {
        "solid" => Connection::Solid,
        "none" => Connection::None,
        _ => Connection::Thermal,
    };
    ui.label("Minimum island area (mm²)");
    if let Some(value) = scalar(
        ui,
        "Minimum island area",
        zone.id,
        options.min_island_area_nm2,
        1_000_000_000_000,
        pcbcraft_engine::board::zones::MAX_ISLAND_AREA_NM2,
        revision,
    ) {
        options.min_island_area_nm2 = value;
        changed = true;
    }
    changed.then(|| {
        let mut params = json!({"zone":zone.id,"layer":layer,"options":options});
        if let Some(choice) = net.parse::<usize>().ok().and_then(|index| choices.get(index)) {
            params["target"] = json!(choice.target);
        }
        (pcbcraft_engine::board::zones::SET_ZONE, params)
    })
}

/// A revision-qualified derived/persistent target and display label; names never grant authority.
#[derive(Clone, Debug)]
pub(super) struct NetChoice {
    label: String,
    target: ClassTarget,
    id: Option<uuid::Uuid>,
}
/// Build picker presentation once from an engine inspection; unnamed nets remain distinguishable.
fn net_choices(rows: &Value) -> Vec<NetChoice> {
    rows["nets"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, row)| {
            let anchor = serde_json::from_value(row["anchor"].clone()).ok()?;
            let label = row["name"].as_str().filter(|name| !name.is_empty()).map_or_else(|| format!("Net {}", index + 1), str::to_owned);
            Some(NetChoice { label, target: ClassTarget::Derived { anchor }, id: serde_json::from_value(row["id"].clone()).ok() })
        })
        .collect()
}
/// Scalars commit on focus loss; stale revisions and Escape discard text without a transaction.
#[derive(Clone)]
struct Scalar {
    text: String,
    revision: u64,
    model: u64,
    error: bool,
    edited: bool,
    pass: u64,
}
/// Reuse the shared text control for priority and mm² area, preserving exact untouched model values.
fn scalar(ui: &mut egui::Ui, label: &str, zone: uuid::Uuid, model: u64, scale: u64, max: u64, revision: u64) -> Option<u64> {
    let id = ui.scope_id().with((label, zone));
    let pass = ui.ctx().cumulative_pass_nr();
    let shown = if scale == 1 { model.to_string() } else { format!("{}", model as f64 / scale as f64) };
    let fresh = || Scalar { text: shown.clone(), revision, model, error: false, edited: false, pass };
    let mut draft =
        ui.data(|d| d.get_temp::<Scalar>(id)).filter(|d| d.revision == revision && d.model == model && d.pass + 1 >= pass).unwrap_or_else(fresh);
    draft.pass = pass;
    let response = controls::text(ui, &mut draft.text).accessible_name(label);
    draft.edited |= response.changed();
    let mut result = None;
    if response.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        response.surrender_focus();
        draft = fresh();
    } else if response.lost_focus() && draft.edited && draft.text != shown {
        let value = draft
            .text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite() && *n >= 0. && *n <= max as f64 / scale as f64 && (scale != 1 || n.fract() == 0.))
            .map(|n| (n * scale as f64).round() as u64)
            .filter(|n| *n <= max);
        if let Some(value) = value {
            result = (value != model).then_some(value);
            draft = fresh();
        } else {
            draft.error = true;
        }
    }
    if draft.error {
        ui.label(shell::error("Value is outside the supported range."));
    }
    ui.data_mut(|d| d.insert_temp(id, draft));
    result
}
