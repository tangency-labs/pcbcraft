//! Shared typed comparison drafts; partial text stays outside persisted model values.
use pcbcraft_chrome::properties;
use pcbcraft_engine::library::parts::{Field, Requirement, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operator {
    Unspecified,
    Exact,
    AtLeast,
    AtMost,
    Range,
    OneOf,
}
/// Incomplete text stays in a view draft rather than becoming partially validated project values.
#[derive(Clone, Debug)]
pub(crate) struct Draft {
    operator: Operator,
    values: Vec<String>,
}
impl Draft {
    /// Restore every supported native requirement without flattening it into formatted Value text.
    pub(crate) fn new(requirement: Option<&Requirement>) -> Self {
        let (operator, values) = match requirement {
            None => (Operator::Unspecified, vec![String::new()]),
            Some(Requirement::Exact { value }) => (Operator::Exact, vec![value.display()]),
            Some(Requirement::AtLeast { value }) => (Operator::AtLeast, vec![value.display()]),
            Some(Requirement::AtMost { value }) => (Operator::AtMost, vec![value.display()]),
            Some(Requirement::Range { minimum, maximum }) => (Operator::Range, vec![minimum.display(), maximum.display()]),
            Some(Requirement::OneOf { values }) => (Operator::OneOf, values.iter().map(Value::display).collect()),
        };
        Self { operator, values }
    }
    /// Canonical conversion uses the engine field parser; invalid input leaves the draft editable.
    pub(crate) fn requirement(&self, field: &Field) -> Result<Option<Requirement>, String> {
        if self.operator == Operator::Unspecified {
            return Ok(None);
        }
        let first = || field.parse_input(self.values.first().map_or("", String::as_str));
        let requirement = match self.operator {
            Operator::Unspecified => return Ok(None),
            Operator::Exact => Requirement::Exact { value: first()? },
            Operator::AtLeast => Requirement::AtLeast { value: first()? },
            Operator::AtMost => Requirement::AtMost { value: first()? },
            Operator::Range => Requirement::Range { minimum: first()?, maximum: field.parse_input(self.values.get(1).map_or("", String::as_str))? },
            Operator::OneOf => Requirement::OneOf { values: self.values.iter().map(|value| field.parse_input(value)).collect::<Result<_, _>>()? },
        };
        requirement.validate(field)?;
        Ok(Some(requirement))
    }
    /// Operators and values are independent controls, retaining incomplete ranges and allowed sets.
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, name: &str) {
        self.show_presence(ui, name, "Unspecified");
    }
    /// Reuse the same typed controls with a caller-owned label for the absence of a comparison.
    pub(crate) fn show_presence(&mut self, ui: &mut egui::Ui, name: &str, none_label: &str) {
        ui.label(name);
        self.show_presence_controls(ui, name, none_label);
    }
    /// Draw comparison controls when the containing form already supplies the field label.
    pub(crate) fn show_presence_controls(&mut self, ui: &mut egui::Ui, name: &str, none_label: &str) {
        egui::ComboBox::from_id_salt("requirement_operator")
            .selected_text(match self.operator {
                Operator::Unspecified => none_label,
                Operator::Exact => "Exact",
                Operator::AtLeast => "At least",
                Operator::AtMost => "At most",
                Operator::Range => "Range",
                Operator::OneOf => "One of",
            })
            .show_ui(ui, |ui| {
                for (operator, label) in [
                    (Operator::Unspecified, none_label),
                    (Operator::Exact, "Exact"),
                    (Operator::AtLeast, "At least"),
                    (Operator::AtMost, "At most"),
                    (Operator::Range, "Range"),
                    (Operator::OneOf, "One of"),
                ] {
                    ui.selectable_value(&mut self.operator, operator, label);
                }
            })
            .response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, name));
        let count = if self.operator == Operator::Range { 2 } else { 1 };
        while self.values.len() < count {
            self.values.push(String::new());
        }
        if self.operator == Operator::Unspecified {
            return;
        }
        let count = if self.operator == Operator::OneOf { self.values.len() } else { count };
        let mut remove = None;
        for (index, value) in self.values.iter_mut().take(count).enumerate() {
            let suffix = match self.operator {
                Operator::Range => {
                    if index == 0 {
                        "minimum".into()
                    } else {
                        "maximum".into()
                    }
                }
                Operator::OneOf => format!("value {}", index + 1),
                _ => "value".into(),
            };
            let label = format!("{name} {suffix}");
            if self.operator == Operator::OneOf && count > 1 {
                if properties::text_action(ui, &label, value, "Remove") {
                    remove = Some(index);
                }
            } else {
                properties::text(ui, &label, value);
            }
        }
        if let Some(index) = remove {
            self.values.remove(index);
        }
        if self.operator == Operator::OneOf && pcbcraft_chrome::controls::enabled_button(ui, self.values.len() < 64, "Add allowed value").clicked() {
            self.values.push(String::new());
        }
    }
}
