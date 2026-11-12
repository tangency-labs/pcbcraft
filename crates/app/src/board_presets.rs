//! View-local routing sizes and reusable numeric presets, independent of board setup profiles.
use crate::{Dialog, PcbApp, ViewInput};
use pcbcraft_chrome::{
    controls, shell,
    units::{self, Unit},
    workspace,
};
use pcbcraft_engine::board::track::{MAX_VIA_DIAMETER_NM, MAX_WIDTH_NM, MIN_VIA_DRILL_NM, MIN_WIDTH_NM};
use serde::{Deserialize, Serialize};

pub(super) const STORAGE_KEY: &str = "pcbcraft.routing-size-presets.v1";
const MAX_PRESETS: usize = 128;
const WIDTHS: &[i64] = &[100_000, 150_000, 200_000, 250_000, 300_000, 400_000, 500_000, 750_000, 1_000_000];
const VIAS: &[[i64; 2]] = &[[400_000, 200_000], [600_000, 300_000], [800_000, 400_000], [1_000_000, 500_000]];

/// User-added sizes are stored in local application settings; built-in choices are implicit.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Presets {
    widths: Vec<i64>,
    vias: Vec<[i64; 2]>,
    #[serde(skip)]
    pub(super) dirty: bool,
}

impl Presets {
    /// Invalid or oversized settings fall back to the built-in choices without touching a board.
    pub(super) fn from_storage(text: Option<String>) -> Self {
        text.filter(|text| text.len() <= 16_384)
            .and_then(|text| serde_json::from_str::<Self>(&text).ok())
            .filter(|saved| {
                saved.widths.len() <= MAX_PRESETS
                    && saved.vias.len() <= MAX_PRESETS
                    && saved.widths.iter().all(|width| Size::Width(*width).valid())
                    && saved.vias.iter().all(|via| Size::Via(*via).valid())
            })
            .map(|mut saved| {
                saved.widths.sort_unstable();
                saved.widths.dedup();
                saved.vias.sort_unstable();
                saved.vias.dedup();
                saved
            })
            .unwrap_or_default()
    }

    /// Persist only user-added canonical nanometres, never display-unit approximations.
    pub(super) fn storage_text(&self) -> Option<String> {
        serde_json::to_string(self).ok()
    }

    /// Keep the current value selectable even when it came from an older view or another tool.
    fn choices(&self, current: Size) -> Vec<Size> {
        let mut choices: Vec<_> = match current {
            Size::Width(_) => WIDTHS.iter().chain(&self.widths).copied().map(Size::Width).collect(),
            Size::Via(_) => VIAS.iter().chain(&self.vias).copied().map(Size::Via).collect(),
        };
        choices.push(current);
        choices.sort_unstable();
        choices.dedup();
        choices
    }

    /// Validate before saving and bound local collections; repeated sizes consume no new slot.
    fn remember(&mut self, size: Size) -> Result<(), String> {
        if !size.valid() {
            return Err("Size is outside the supported range.".into());
        }
        let count = self.widths.len() + self.vias.len();
        let result = match size {
            Size::Width(width) => insert(&mut self.widths, WIDTHS, width),
            Size::Via(via) => insert(&mut self.vias, VIAS, via),
        };
        self.dirty |= count != self.widths.len() + self.vias.len();
        result
    }
}

/// Add one distinct custom choice atomically; built-in values need no saved duplicate.
fn insert<T: Ord + Copy>(saved: &mut Vec<T>, defaults: &[T], value: T) -> Result<(), String> {
    if saved.contains(&value) || defaults.contains(&value) {
        return Ok(());
    }
    if saved.len() >= MAX_PRESETS {
        return Err("The preset list is full. Uncheck Save as preset to use this value once.".into());
    }
    saved.push(value);
    saved.sort_unstable();
    Ok(())
}

/// Via pad and drill dimensions are one choice so a dropdown never constructs an invalid pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Size {
    Width(i64),
    Via([i64; 2]),
}

impl Size {
    /// Use the engine's supported bounds; preset selection does not qualify manufacturing rules.
    fn valid(self) -> bool {
        match self {
            Self::Width(width) => (MIN_WIDTH_NM..=MAX_WIDTH_NM).contains(&width),
            Self::Via([pad, drill]) => pad <= MAX_VIA_DIAMETER_NM && (MIN_VIA_DRILL_NM..pad).contains(&drill),
        }
    }

    /// Format paired values with the same unit and retain exact integers behind the display.
    fn label(self, unit: Unit) -> String {
        match self {
            Self::Width(width) => units::label(width, unit),
            Self::Via([pad, drill]) => format!("{} / {}", units::label(pad, unit).trim_end_matches(unit.symbol()).trim(), units::label(drill, unit)),
        }
    }
}

/// A captured custom-size form, retaining partially typed input until Use or cancellation.
#[derive(Debug)]
pub(super) struct Edit {
    source: ViewInput,
    current: Size,
    unit: Unit,
    first: String,
    second: String,
    save: bool,
    focused: bool,
    error: Option<String>,
}

impl Edit {
    /// Capture both display units and view authority so other windows cannot redirect this edit.
    fn new(source: ViewInput, current: Size, unit: Unit) -> Self {
        let (first, second) = match current {
            Size::Width(width) => (width, 0),
            Size::Via([pad, drill]) => (pad, drill),
        };
        Self {
            source,
            current,
            unit,
            first: units::edit_label(first, unit),
            second: units::edit_label(second, unit),
            save: true,
            focused: false,
            error: None,
        }
    }

    /// Explicit units override the captured display unit. Invalid pairs remain editable as a draft.
    fn value(&self) -> Result<Size, String> {
        let first = units::parse_length(&self.first, self.unit).map_err(|e| e.to_string())?;
        let value = match self.current {
            Size::Width(_) => Size::Width(first),
            Size::Via(_) => Size::Via([first, units::parse_length(&self.second, self.unit).map_err(|e| e.to_string())?]),
        };
        if value.valid() {
            Ok(value)
        } else {
            Err(match value {
                Size::Width(_) => {
                    format!("Width must be between {} and {}.", units::label(MIN_WIDTH_NM, self.unit), units::label(MAX_WIDTH_NM, self.unit))
                }
                Size::Via(_) => format!(
                    "Drill must be at least {} and smaller than the pad; pad must be at most {}.",
                    units::label(MIN_VIA_DRILL_NM, self.unit),
                    units::label(MAX_VIA_DIAMETER_NM, self.unit)
                ),
            })
        }
    }
}

/// Draw a themed ribbon dropdown and return a preset selection or a request for custom entry.
fn picker(ui: &mut egui::Ui, title: &str, current: Size, presets: &Presets, unit: Unit, enabled: bool, compact: bool) -> Option<Option<Size>> {
    let mut action = None;
    workspace::ribbon_field(ui, title, compact, |ui| {
        ui.add_enabled_ui(enabled, |ui| {
            let response = controls::Choice::from_id_salt(title)
                .selected_text(current.label(unit))
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for size in presets.choices(current) {
                        if ui.selectable_label(size == current, size.label(unit)).clicked() {
                            action = Some(Some(size));
                        }
                    }
                    ui.separator();
                    if controls::button(ui, "Custom…").clicked() {
                        action = Some(None);
                    }
                })
                .response;
            response.accessible_name(title)
        })
        .inner
    });
    action
}

impl PcbApp {
    /// Dropdowns select upcoming geometry only. Their presets remain separate from board setup.
    pub(super) fn board_size_controls(&mut self, ui: &mut egui::Ui, enabled: bool, compact: bool) {
        if let Some(project) = self.views.active.snapshot(&self.session)
            && let Some(board) = self.views.active.board.target.and_then(|target| project.board_view(target))
        {
            self.views.active.board.tracks.sync_rules(board);
        }
        let unit = self.preferences.units;
        let width = Size::Width(self.views.active.board.tracks.width_nm);
        let via = Size::Via([self.views.active.board.tracks.via_diameter_nm, self.views.active.board.tracks.via_drill_nm]);
        for (title, current) in [("Track / line width", width), ("Via pad / drill", via)] {
            match picker(ui, title, current, &self.routing_presets, unit, enabled, compact) {
                Some(Some(value)) => set_size(&mut self.views.active.board.tracks, value),
                Some(None) => {
                    if let Some(source) = self.views.active.input(&self.session) {
                        self.dialog = Dialog::RoutingSize(Edit::new(source, current, unit));
                    }
                }
                None => {}
            }
        }
    }

    /// Validate custom dimensions and optional local storage before changing the originating view.
    /// Escape, backdrop dismissal and stale authority leave both its dimensions and presets alone.
    pub(super) fn routing_size_dialog(&mut self, ctx: &egui::Context, mut edit: Edit) {
        let via = matches!(edit.current, Size::Via(_));
        let (mut submit, mut cancel) = (false, false);
        let closing = shell::dialog(ctx, "routing_size", if via { "Custom Via Size" } else { "Custom Width" }, |ui| {
            let label = ui.label(if via { "Pad diameter" } else { "Width" });
            let response = ui.add(egui::TextEdit::singleline(&mut edit.first).desired_width(f32::INFINITY)).labelled_by(label.id);
            if !edit.focused {
                response.request_focus();
                edit.focused = true;
            }
            submit |= response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if via {
                let label = ui.label("Drill diameter");
                let response = ui.add(egui::TextEdit::singleline(&mut edit.second).desired_width(f32::INFINITY)).labelled_by(label.id);
                submit |= response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            }
            ui.checkbox(&mut edit.save, "Save as preset");
            if let Some(error) = &edit.error {
                ui.label(shell::error(error.as_str()));
            }
            ui.horizontal(|ui| {
                submit |= controls::button(ui, "Use").clicked();
                cancel = controls::button(ui, "Cancel").clicked();
            });
        });
        if closing || cancel {
            return;
        }
        if submit {
            if !self.views.view(edit.source.view()).is_some_and(|view| view.accepts(edit.source, &self.session)) {
                self.message = Some("View or project changed; reopen the size control to continue.".into());
                return;
            }
            match edit.value().and_then(|value| {
                if edit.save {
                    self.routing_presets.remember(value)?;
                }
                Ok(value)
            }) {
                Ok(value) => {
                    if let Some(view) = self.views.view_mut(edit.source.view()) {
                        set_size(&mut view.board.tracks, value);
                    }
                    return;
                }
                Err(error) => edit.error = Some(error),
            }
        }
        self.dialog = Dialog::RoutingSize(edit);
    }
}

/// Apply a validated preset without changing the project, undo history or another view's tool.
fn set_size(tracks: &mut crate::board_tracks::State, value: Size) {
    match value {
        Size::Width(width) => tracks.width_nm = width,
        Size::Via([pad, drill]) => {
            tracks.via_diameter_nm = pad;
            tracks.via_drill_nm = drill;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opening Custom and accepting unchanged mil values preserves every stored nanometre.
    #[test]
    fn unchanged_custom_mil_values_round_trip_exactly() {
        let mut h = crate::tests::harness();
        h.state_mut().run("project.new", &serde_json::json!({}));
        h.run();
        let source = h.state().views.active.input(h.state().session()).unwrap();
        for current in [Size::Width(300_000), Size::Width(254_001), Size::Via([600_000, 300_000]), Size::Via([762_001, 304_801])] {
            let edit = Edit::new(source, current, Unit::Mil);
            assert_eq!(edit.value().unwrap(), current);
        }
    }

    /// Saved custom choices round-trip exactly and cannot bypass engine bounds or grow unbounded.
    #[test]
    fn custom_presets_round_trip_and_reject_invalid_storage() {
        let mut saved = Presets::default();
        saved.remember(Size::Width(508_000)).unwrap();
        saved.remember(Size::Via([762_000, 304_800])).unwrap();
        let text = saved.storage_text();
        let restored = Presets::from_storage(text.clone());
        assert_eq!(restored.storage_text(), text);
        assert!(restored.choices(Size::Width(250_000)).contains(&Size::Width(508_000)));
        assert!(restored.choices(Size::Via([600_000, 300_000])).contains(&Size::Via([762_000, 304_800])));
        saved.remember(Size::Width(508_000)).unwrap();
        saved.remember(Size::Via([600_000, 300_000])).unwrap();
        assert_eq!(saved.storage_text(), text);
        for invalid in [Size::Width(0), Size::Width(MAX_WIDTH_NM + 1), Size::Via([300_000, 300_000]), Size::Via([400_000, 49_999])] {
            assert!(saved.remember(invalid).is_err());
            assert_eq!(saved.storage_text(), text);
        }
        let empty = Presets::default().storage_text();
        for invalid in [
            "{\"widths\":[0],\"vias\":[]}".to_owned(),
            "{\"widths\":[],\"vias\":[[300000,300000]]}".to_owned(),
            " ".repeat(16_385),
            serde_json::to_string(&Presets { widths: vec![250_000; MAX_PRESETS + 1], ..Presets::default() }).unwrap(),
        ] {
            assert_eq!(Presets::from_storage(Some(invalid)).storage_text(), empty);
        }
    }

    /// Presets use the launcher's ordinary settings path and flush without a project save.
    #[test]
    fn custom_presets_survive_launcher_restart() {
        #[derive(Default)]
        struct Storage(std::collections::BTreeMap<String, String>);
        impl eframe::Storage for Storage {
            fn get_string(&self, key: &str) -> Option<String> {
                self.0.get(key).cloned()
            }
            fn set_string(&mut self, key: &str, value: String) {
                self.0.insert(key.into(), value);
            }
            fn remove_string(&mut self, key: &str) {
                self.0.remove(key);
            }
            fn flush(&mut self) {}
        }
        let mut app = PcbApp::default();
        app.routing_presets.remember(Size::Width(508_000)).unwrap();
        app.routing_presets.remember(Size::Via([762_000, 304_800])).unwrap();
        assert!(app.routing_presets.dirty);
        let saved = app.routing_presets.storage_text();
        let mut storage = Storage::default();
        eframe::App::save(&mut app, &mut storage);
        assert!(!app.routing_presets.dirty);
        let mut cc = eframe::CreationContext::_new_kittest(egui::Context::default());
        cc.storage = Some(&storage);
        let restored = PcbApp::new(&cc);
        assert_eq!(restored.routing_presets.storage_text(), saved);
        assert!(!restored.routing_presets.dirty);
    }
}
