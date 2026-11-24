//! Inline batch fields keep mixed values until a deliberate set or clear action.

use super::width_field;
use pcbcraft_chrome::{MenuItem, controls, properties, settings, shell, theme::Tokens};
use pcbcraft_engine::{
    net_classes::Catalog,
    nets::{ClassTarget, DEFAULT_WIDTH_CP, PropertyValues},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Keep,
    Set,
    Clear,
}

#[derive(Debug)]
struct Field<T> {
    initial: T,
    value: T,
    mixed: bool,
    mode: Mode,
}

impl<T: Copy + PartialEq> Field<T> {
    /// Seed the editor with the common value, or the default for a mixed selection.
    fn new(mut values: impl Iterator<Item = Option<T>>, default: T) -> Self {
        let first = values.next().flatten();
        let mixed = !values.all(|value| value == first);
        let initial = first.filter(|_| !mixed).unwrap_or(default);
        Self { value: initial, initial, mixed, mode: Mode::Keep }
    }

    /// Discard both the requested action and its editable value.
    fn reset(&mut self) {
        self.value = self.initial;
        self.mode = Mode::Keep;
    }

    /// Show the aggregate status and the available changes for this individual channel.
    fn choose(&mut self, ui: &mut egui::Ui, label: &str) {
        let options = [(Mode::Keep, "Leave as is"), (Mode::Set, "Set for all"), (Mode::Clear, "Theme default")];
        let current = match self.mode {
            Mode::Keep if self.mixed => "Mixed",
            Mode::Keep => "Leave as is",
            Mode::Set => "Set for all",
            Mode::Clear => "Theme default",
        };
        controls::Choice::from_id_salt(label).selected_text(current).width(105.0).show_ui(ui, |ui| {
            for (mode, name) in options {
                if MenuItem::new(name).choice(mode == self.mode).show(ui) {
                    self.mode = mode;
                }
            }
        });
    }
}

/// Pending appearance and net class assignments for one set of distinct nets.
#[derive(Debug)]
pub(super) struct Edit {
    pub(super) targets: Vec<ClassTarget>,
    color: Field<[u8; 3]>,
    width: Field<u16>,
    classes: Vec<(Uuid, String, usize)>,
    class_index: usize,
    pending_classes: BTreeMap<Uuid, bool>,
}

impl Edit {
    /// Read both appearance channels and class counts once when the selection changes.
    pub(super) fn new(targets: Vec<ClassTarget>, values: &[PropertyValues], catalog: &Catalog) -> Self {
        let ink = Tokens::get().schematic_wire;
        let color = Field::new(values.iter().map(|value| value.appearance.and_then(|a| a.color)), [ink.r(), ink.g(), ink.b()]);
        let width = Field::new(values.iter().map(|value| value.appearance.and_then(|a| a.width_cp)), DEFAULT_WIDTH_CP);
        let classes = catalog
            .definitions
            .iter()
            .map(|class| (class.id, class.name.clone(), values.iter().filter(|value| value.classes.contains(&class.id)).count()))
            .collect();
        Self { targets, color, width, classes, class_index: 0, pending_classes: BTreeMap::new() }
    }

    /// Show the class chooser without making global rule definition changes.
    fn class_fields(&mut self, ui: &mut egui::Ui) {
        let Some((class, label, count)) = self.classes.get(self.class_index) else { return };
        let (class, count) = (*class, *count);
        ui.separator();
        properties::field(ui, "Net class", |ui| {
            controls::Choice::from_id_salt("bulk_net_class").selected_text(label.as_str()).width(ui.available_width()).show_ui(ui, |ui| {
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    for (i, (_, name, _)) in self.classes.iter().enumerate() {
                        if MenuItem::new(name).choice(i == self.class_index).show(ui) {
                            self.class_index = i;
                        }
                    }
                });
            });
        });
        ui.label(shell::dim(match self.pending_classes.get(&class) {
            Some(true) => "Assign to all on Apply".to_owned(),
            Some(false) => "Remove from all on Apply".to_owned(),
            None => format!("Assigned on {count} of {} selected nets", self.targets.len()),
        }));
        ui.horizontal(|ui| {
            if controls::button(ui, "Assign class").clicked() {
                if count == self.targets.len() {
                    self.pending_classes.remove(&class);
                } else {
                    self.pending_classes.insert(class, true);
                }
            }
            if controls::button(ui, "Remove class").clicked() {
                if count == 0 {
                    self.pending_classes.remove(&class);
                } else {
                    self.pending_classes.insert(class, false);
                }
            }
        });
    }

    /// Only include explicitly set or cleared fields in the shared request.
    fn changes(&self) -> Value {
        let mut appearance = json!({});
        if self.color.mode == Mode::Set {
            appearance["color"] = json!(self.color.value);
        }
        if self.width.mode == Mode::Set {
            appearance["width_cp"] = json!(self.width.value);
        }
        let mut clear = Vec::new();
        if self.color.mode == Mode::Clear {
            clear.push("color");
        }
        if self.width.mode == Mode::Clear {
            clear.push("width_cp");
        }
        if !clear.is_empty() {
            appearance["clear"] = json!(clear);
        }
        let mut changes = json!({});
        if self.color.mode != Mode::Keep || self.width.mode != Mode::Keep {
            changes["appearance"] = appearance;
        }
        for (key, assign) in [("add_classes", true), ("remove_classes", false)] {
            let classes: Vec<_> = self.pending_classes.iter().filter_map(|(class, value)| (*value == assign).then_some(class)).collect();
            if !classes.is_empty() {
                changes[key] = json!(classes);
            }
        }
        changes
    }

    /// Show the bulk draft and submit all changes through one shared command.
    pub(super) fn show(&mut self, ui: &mut egui::Ui) -> Option<Value> {
        ui.separator();
        ui.label(format!("{} nets selected", self.targets.len()));
        properties::field(ui, "Wire color", |ui| {
            self.color.choose(ui, "bulk_wire_color");
            if self.color.mode == Mode::Set {
                settings::color(ui, "Wire color", &mut self.color.value);
            }
        });
        properties::field(ui, "Wire width", |ui| {
            self.width.choose(ui, "bulk_wire_width");
            if self.width.mode == Mode::Set {
                width_field(ui, &mut self.width.value);
            }
        });
        self.class_fields(ui);
        let changed = self.color.mode != Mode::Keep || self.width.mode != Mode::Keep || !self.pending_classes.is_empty();
        if changed {
            let (apply, reset) =
                ui.horizontal(|ui| (controls::button(ui, "Apply changes").clicked(), controls::button(ui, "Reset changes").clicked())).inner;
            if apply {
                return Some(self.changes());
            }
            if reset {
                self.color.reset();
                self.width.reset();
                self.pending_classes.clear();
            }
        }
        None
    }
}
