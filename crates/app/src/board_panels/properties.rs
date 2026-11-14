//! Board Properties editing: footprint transforms, copper sizes and locks, and the committed
//! length and rotation fields they share. Each returns at most one command for the caller.
use super::{layer_name, property};
use egui::Ui;
use pcbcraft_chrome::units;

/// Edit one footprint's X, Y, rotation and side in place. Each field submits the full transform
/// through `pcb.set_footprint_transform` once, on release or confirmation, keeping the other
/// values; the caller adds the board and `revision`. Length drafts belong to `view` and
/// `revision`, so another view or a newer revision never submits them. Unchanged values submit nothing.
pub(super) fn footprint_transform(
    ui: &mut Ui,
    footprint: &pcbcraft_engine::board::view::FootprintView<'_>,
    unit: units::Unit,
    view: crate::editor_view::ViewId,
    revision: u64,
) -> Option<(&'static str, serde_json::Value)> {
    let limit = pcbcraft_engine::schematic::MAX_CENTER_NM;
    let (mut x, mut y, mut rotation, mut bottom) = (footprint.position.x_nm, footprint.position.y_nm, footprint.rotation, footprint.bottom);
    let mut changed = false;
    ui.label("X");
    if let Some(value) = committed_length(ui, ("Footprint X", (view, &footprint.id)), x, -limit..=limit, unit, revision) {
        (x, changed) = (value, true);
    }
    ui.end_row();
    ui.label("Y");
    if let Some(value) = committed_length(ui, ("Footprint Y", (view, &footprint.id)), y, -limit..=limit, unit, revision) {
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
/// the board and `revision`. Length drafts belong to `view`, `revision` and the whole selection.
/// Mixed track widths show the first width and set all of them; vias
/// of different sizes show "Mixed" and stay read-only, since each via field sends both sizes.
pub(super) fn copper_properties(
    ui: &mut Ui,
    target: pcbcraft_engine::board::view::BoardView<'_>,
    selected: &std::collections::BTreeSet<uuid::Uuid>,
    unit: units::Unit,
    view: crate::editor_view::ViewId,
    revision: u64,
) -> Option<(&'static str, serde_json::Value)> {
    use pcbcraft_engine::board::track::{MAX_VIA_DIAMETER_NM, MAX_WIDTH_NM, MIN_VIA_DRILL_NM, MIN_WIDTH_NM};
    let tracks: Vec<_> = target.tracks().iter().filter(|track| selected.contains(&track.id)).collect();
    let vias: Vec<_> = target.vias().iter().filter(|via| selected.contains(&via.id)).collect();
    // The whole selection: a draft typed for some tracks never applies to a grown selection.
    let salt = (view, selected);
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
            .add_enabled_ui(!any_locked, |ui| {
                committed_length(ui, ("Selected track width", salt), first.width_nm, MIN_WIDTH_NM..=MAX_WIDTH_NM, unit, revision)
            })
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
                // The pad must stay larger than the drill; a smaller pad is refused rather than
                // silently shrinking the drill, matching the drill field's own limit.
                committed_length(ui, ("Selected via pad", salt), first.diameter_nm, first.drill_nm + 1..=MAX_VIA_DIAMETER_NM, unit, revision)
            })
            .inner
        {
            edit = Some((pcbcraft_engine::SET_VIA_SIZE, serde_json::json!({"vias":ids,"via_diameter_nm":pad,"via_drill_nm":first.drill_nm})));
        }
        ui.end_row();
        ui.label("Via drill");
        if let Some(drill) = ui
            .add_enabled_ui(!any_locked, |ui| {
                committed_length(ui, ("Selected via drill", salt), first.drill_nm, MIN_VIA_DRILL_NM..=first.diameter_nm - 1, unit, revision)
            })
            .inner
        {
            edit = Some((pcbcraft_engine::SET_VIA_SIZE, serde_json::json!({"vias":ids,"via_diameter_nm":first.diameter_nm,"via_drill_nm":drill})));
        }
        ui.end_row();
    }
    edit
}

/// An in-progress length edit, tied to the model value, display unit and project revision it
/// started from, and to a field shown in every pass since. A changed selection or view, any
/// other edit (including a peer's), undo or a unit switch discards it instead of submitting
/// stale text against the new revision.
#[derive(Clone, Debug)]
struct LengthDraft {
    text: String,
    model_nm: i64,
    unit: units::Unit,
    revision: u64,
    pass: u64,
    error: Option<String>,
}

/// A length text field using the shared parser: it shows exact editable text in the display unit,
/// accepts `mil`, `mm` or `in` suffixes over that default, and returns whole nanometres once, when
/// focus leaves with changed text that parses inside `range`. Untouched text never re-parses, so
/// no rounding reaches the model. Escape restores the model value; invalid or out-of-range text
/// stays editable with an error and submits nothing. A draft from an earlier `revision`, or one
/// whose field was hidden for a pass, is dropped unsubmitted. The salt's name is the accessible label.
fn committed_length(
    ui: &mut Ui,
    salt: (&str, impl std::hash::Hash + std::fmt::Debug),
    model_nm: i64,
    range: std::ops::RangeInclusive<i64>,
    unit: units::Unit,
    revision: u64,
) -> Option<i64> {
    let id = ui.scope_id().with(&salt);
    let shown = units::edit_label(model_nm, unit);
    let pass = ui.ctx().cumulative_pass_nr();
    let fresh = || LengthDraft { text: shown.clone(), model_nm, unit, revision, pass, error: None };
    let mut draft = ui
        .data(|data| data.get_temp::<LengthDraft>(id))
        .filter(|d| (d.model_nm, d.unit, d.revision) == (model_nm, unit, revision) && d.pass + 1 >= pass)
        .unwrap_or_else(fresh);
    draft.pass = pass;
    let mut committed = None;
    ui.vertical(|ui| {
        let response = pcbcraft_chrome::controls::text(ui, &mut draft.text).accessible_name(salt.0);
        if response.gained_focus() {
            // Like the spin fields this replaces, focusing selects the value so typing overwrites it.
            let mut state = egui::TextEdit::load_state(ui.ctx(), response.id).unwrap_or_default();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::select_all_str(&draft.text)));
            state.store(ui.ctx(), response.id);
        }
        if response.changed() {
            draft.error = None;
        }
        // egui drops focus on Escape before this frame's shortcuts run, so the board would also
        // clear its selection. Keep Escape for the field instead: it restores the model value.
        let escaped = response.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if response.has_focus() && !escaped {
            let filter = egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, escape: true, ..Default::default() };
            ui.memory_mut(|memory| memory.set_focus_lock_filter(response.id, filter));
        }
        if escaped {
            response.surrender_focus();
            draft = fresh();
        } else if response.lost_focus() {
            if draft.text == shown {
                draft = fresh();
            } else {
                match units::parse_length(&draft.text, unit) {
                    Err(error) => draft.error = Some(error.to_string()),
                    Ok(nm) if !range.contains(&nm) => {
                        draft.error = Some(format!("Enter {} to {}", units::label(*range.start(), unit), units::label(*range.end(), unit)));
                    }
                    Ok(nm) => {
                        committed = (nm != model_nm).then_some(nm);
                        draft = fresh();
                    }
                }
            }
        }
        if let Some(error) = &draft.error {
            ui.label(pcbcraft_chrome::shell::error(error.as_str()));
        }
        if response.has_focus() || draft.text != shown || draft.error.is_some() {
            ui.data_mut(|data| data.insert_temp(id, draft.clone()));
        } else {
            ui.data_mut(|data| data.remove::<LengthDraft>(id));
        }
    });
    committed
}
