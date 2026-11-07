//! Scalar property fields retain invalid typed lengths until an explicit validated Apply.
use super::*;
use pcbcraft_chrome::units::{self, Unit};

#[derive(Debug)]
pub(super) struct Draft {
    pub(super) revision: u64,
    object: Option<u64>,
    kind: Kind,
    value: Value,
    original: Value,
    numbers: BTreeMap<String, (String, String)>,
}
impl Draft {
    /// Capture the selected primitive or definition metadata with its document authority.
    pub(super) fn new(item: &Item, object: Option<u64>, revision: u64) -> Self {
        let mut value=object.and_then(|id|item.objects.get(&id)).and_then(|primitive|serde_json::to_value(primitive).ok()).unwrap_or_else(||json!({"operation":"metadata","name":item.name,"license":item.license,"source":item.source,"reference_prefix":item.reference_prefix,"default_value":item.default_value}));
        if let Some(pad) = value.get_mut("pad").and_then(Value::as_object_mut) {
            pad.entry("corner_radius_nm").or_insert(json!(0));
            pad.entry("drill_nm").or_insert(Value::Null);
        }
        Self { revision, object, kind: item.kind, original: value.clone(), value, numbers: BTreeMap::new() }
    }
    /// Invalid numeric text is also unsaved input even while the last valid scalar is unchanged.
    pub(super) fn dirty(&self) -> bool {
        self.value != self.original || self.numbers.values().any(|(old, new)| old != new)
    }
    /// Use human-readable fields rather than exposing serialization keys or a JSON text editor.
    pub(super) fn show(&mut self, ui: &mut Ui, expanded: bool) {
        fields(
            ui,
            &mut self.value,
            &mut self.numbers,
            "",
            expanded,
            if self.kind == Kind::Footprint && self.object.is_none() { &["reference_prefix", "default_value"] } else { &[] },
        );
    }
    /// Parse all typed lengths once at Apply; native schema validation then checks the transaction.
    pub(super) fn action(&self) -> Result<Action, String> {
        let mut value = self.value.clone();
        resolve(&mut value, &self.numbers, "")?;
        if let Some(object) = self.object {
            Ok(Action::Replace { object, primitive: serde_json::from_value(value).map_err(|e| e.to_string())? })
        } else {
            serde_json::from_value(value).map_err(|e| e.to_string())
        }
    }
}
/// Field groups describe the selected object rather than its ordinal in a native array.
pub(super) fn title(draft: &Draft) -> String {
    match draft.value["type"].as_str() {
        Some("pin") => format!("Pin {}", draft.value["number"].as_str().unwrap_or("")),
        Some("pad") => format!("Pad {}", draft.value["pad"]["number"].as_str().unwrap_or("")),
        Some("outline") => format!("{} rectangle", display(draft.value["graphic"]["purpose"].as_str().unwrap_or("Technical"))),
        Some(kind) => display(kind),
        None => "Definition".into(),
    }
}
/// Labels follow geometry semantics; explicit units remain beside length input.
fn label(path: &str) -> String {
    let path = path.trim_start_matches('.');
    let names = path
        .split('.')
        .map(|part| match part {
            "x_nm" => "X (mm)".into(),
            "y_nm" => "Y (mm)".into(),
            "width_nm" => "Width (mm)".into(),
            "height_nm" => "Height (mm)".into(),
            "radius_nm" => "Radius (mm)".into(),
            "corner_radius_nm" => "Corner (mm)".into(),
            "drill_nm" => "Drill (mm)".into(),
            "reference_prefix" => "Ref prefix".into(),
            "default_value" => "Value".into(),
            "electrical_type" => "Electrical".into(),
            "at" | "center" | "pad" | "graphic" | "path" | "points" => String::new(),
            "number" => "Number".into(),
            "name" => "Name".into(),
            "copper" => "Layers".into(),
            "shape" => "Shape".into(),
            "rotation" => "Rotation (°)".into(),
            _ => part.parse::<usize>().map(|index| format!("Point {}", index + 1)).unwrap_or_else(|_| display(part)),
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<String>>();
    names.join(" ")
}
/// Known native enum values are choices, while electrical names/numbers remain editable strings.
fn choices(path: &str) -> &'static [&'static str] {
    match path.rsplit('.').next().unwrap_or("") {
        "electrical_type" => &["passive", "input", "output", "bidirectional", "power_in", "power_out", "unspecified"],
        "shape" => &["rectangle", "rounded_rectangle", "circle"],
        "copper" => &["component_side", "all_copper"],
        "purpose" => &["fabrication", "silkscreen", "courtyard"],
        "align" => &["start", "middle", "end"],
        _ => &[],
    }
}
/// Recurse over typed native properties. Compact forms omit long provenance/point tables;
/// expanded forms retain the exact same field values and revision as the inspector.
fn fields(ui: &mut Ui, value: &mut Value, numbers: &mut BTreeMap<String, (String, String)>, path: &str, expanded: bool, hidden: &[&str]) {
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().cloned().collect::<Vec<_>>();
            keys.sort_by_key(|key| priority(key));
            for key in keys {
                let Some(value) = object.get_mut(&key) else { continue };
                if ["type", "operation"].contains(&key.as_str()) || hidden.contains(&key.as_str()) {
                    continue;
                }
                if !expanded && ["source", "license", "points"].contains(&key.as_str()) {
                    continue;
                }
                fields(ui, value, numbers, &format!("{path}.{key}"), expanded, hidden);
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter_mut().enumerate() {
                fields(ui, value, numbers, &format!("{path}.{index}"), expanded, hidden);
            }
        }
        Value::Bool(value) => {
            ui.checkbox(value, label(path));
        }
        Value::Number(number) => {
            let length = path.ends_with("_nm");
            let text = number
                .as_i64()
                .map(|n| if length { format!("{}", n as f64 / 1_000_000.0) } else { n.to_string() })
                .unwrap_or_else(|| number.to_string());
            let field = numbers.entry(path.into()).or_insert_with(|| (text.clone(), text));
            properties::text(ui, &label(path), &mut field.1);
        }
        Value::String(text) => {
            let options = choices(path);
            if options.is_empty() {
                properties::text(ui, &label(path), text);
            } else {
                let options = options.iter().map(|option| (*option, display(option))).collect::<Vec<_>>();
                properties::choice(ui, &label(path), egui::Id::new(path), text, &options);
            }
        }
        Value::Null if path.ends_with("drill_nm") => {
            let field = numbers.entry(path.into()).or_insert_with(|| (String::new(), String::new()));
            properties::text(ui, &label(path), &mut field.1);
        }
        Value::Null if path.ends_with("name") => {
            let mut text = String::new();
            if properties::text(ui, &label(path), &mut text).changed() {
                *value = Value::String(text);
            }
        }
        _ => {}
    }
}
/// Resolve buffered numeric text without changing the draft after a parse failure.
fn resolve(value: &mut Value, numbers: &BTreeMap<String, (String, String)>, path: &str) -> Result<(), String> {
    if let Some((_, text)) = numbers.get(path) {
        if text.trim().is_empty() && path.ends_with("drill_nm") {
            *value = Value::Null;
            return Ok(());
        }
        let number = if path.ends_with("_nm") {
            units::parse_length(text, Unit::Mm).map_err(|e| format!("{}: {e}", label(path)))?
        } else {
            text.parse::<i64>().map_err(|_| format!("{} must be a whole number.", label(path)))?
        };
        *value = json!(number);
        return Ok(());
    }
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                resolve(value, numbers, &format!("{path}.{key}"))?;
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter_mut().enumerate() {
                resolve(value, numbers, &format!("{path}.{index}"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Native serialization order is stable for Git; form order instead follows how dimensions
/// and electrical identities are normally entered.
fn priority(key: &str) -> usize {
    [
        "name",
        "number",
        "electrical_type",
        "at",
        "center",
        "from",
        "to",
        "min",
        "max",
        "shape",
        "width_nm",
        "height_nm",
        "radius_nm",
        "corner_radius_nm",
        "copper",
        "drill_nm",
        "rotation",
        "reference_prefix",
        "default_value",
        "license",
        "source",
    ]
    .iter()
    .position(|candidate| *candidate == key)
    .unwrap_or(100)
}
/// Enum storage names remain unchanged while controls use ordinary readable labels.
fn display(text: &str) -> String {
    let text = text.replace('_', " ");
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
