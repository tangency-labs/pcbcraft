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
        if object.is_none() && item.kind == Kind::Symbol {
            value["symbol_settings"] = json!({"behavior":item.behavior,"defaults":item.defaults});
        }
        if object.is_none() && item.kind == Kind::Footprint {
            value["footprint_reference"] = json!({
                "enabled": item.reference_font.is_some(),
                "offset": item.reference_offset.unwrap_or(pcbcraft_engine::board::DEFAULT_REFERENCE_OFFSET),
            });
        }
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
        if self.value["type"] == "pin" && self.value.get("appearance").is_none() && ui.button("Add pin lead").clicked() {
            self.value["appearance"] = serde_json::to_value(pcbcraft_engine::schematic::PinAppearance::default()).unwrap_or(Value::Null);
        }
        let pad = self.value["type"] == "pad";
        let mut hidden = Vec::new();
        if self.kind == Kind::Footprint {
            hidden.push("fill");
        }
        if self.kind == Kind::Footprint && self.object.is_none() {
            hidden.extend(["reference_prefix", "default_value", "footprint_reference"]);
            let reference = &mut self.value["footprint_reference"];
            let mut enabled = reference["enabled"].as_bool().unwrap_or_default();
            if ui.checkbox(&mut enabled, "Generate silkscreen reference").changed() {
                reference["enabled"] = json!(enabled);
            }
            if enabled {
                fields(ui, &mut reference["offset"], &mut self.numbers, ".footprint_reference.offset", expanded, &[]);
            } else {
                self.numbers.remove(".footprint_reference.offset.x_nm");
                self.numbers.remove(".footprint_reference.offset.y_nm");
            }
        }
        if pad {
            hidden.push("copper");
            if self.value["pad"]["shape"] != "rounded_rectangle" {
                hidden.push("corner_radius_nm");
            }
            if self.value["pad"]["shape"] == "circle" {
                hidden.push("height_nm");
            }
            let mut kind = if self.value["pad"]["copper"] == "all_copper" { "through_hole" } else { "smd" }.to_owned();
            let before = kind.clone();
            properties::choice(
                ui,
                "Pad type",
                egui::Id::unique("pad_type"),
                &mut kind,
                &[("smd", "SMD".into()), ("through_hole", "Through hole".into())],
            );
            if kind != before {
                self.value["pad"]["copper"] = json!(if kind == "smd" { "component_side" } else { "all_copper" });
                self.value["pad"]["drill_nm"] = if kind == "smd" {
                    Value::Null
                } else {
                    json!(
                        self.value["pad"]["width_nm"].as_i64().unwrap_or(600_000).min(self.value["pad"]["height_nm"].as_i64().unwrap_or(600_000)) / 2
                    )
                };
                self.numbers.remove(".pad.drill_nm");
            }
        }
        let old_shape = self.value["pad"]["shape"].clone();
        fields(ui, &mut self.value, &mut self.numbers, "", expanded, &hidden);
        if pad && self.value["pad"]["shape"] != old_shape {
            self.value["pad"]["corner_radius_nm"] = json!(if self.value["pad"]["shape"] == "rounded_rectangle" {
                self.value["pad"]["width_nm"].as_i64().unwrap_or(600_000).min(self.value["pad"]["height_nm"].as_i64().unwrap_or(600_000)) / 4
            } else {
                0
            });
            self.numbers.remove(".pad.corner_radius_nm");
        }
    }
    /// Parse all typed lengths once at Apply; native schema validation then checks the transaction.
    pub(super) fn action(&self) -> Result<Action, String> {
        let mut value = self.value.clone();
        resolve(&mut value, &self.numbers, "")?;
        if value["type"] == "pin" && value["name"].as_str().is_some_and(|name| name.trim().is_empty()) {
            value["name"] = Value::Null;
        }
        if value["type"] == "pad" && value["pad"]["shape"] == "circle" {
            value["pad"]["height_nm"] = value["pad"]["width_nm"].clone();
        }
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
    if let Some(axis) = path.strip_prefix("footprint_reference.offset.") {
        return if axis == "x_nm" { "Reference X offset (mm)" } else { "Reference Y offset (mm)" }.into();
    }
    let names = path
        .split('.')
        .map(|part| match part {
            "x_nm" => "X (mm)".into(),
            "y_nm" => "Y (mm)".into(),
            "width_nm" => "Width (mm)".into(),
            "height_nm" => if path.contains("geometry") { "Text size (mm)" } else { "Height (mm)" }.into(),
            "radius_nm" => "Radius (mm)".into(),
            "corner_radius_nm" => "Corner (mm)".into(),
            "drill_nm" => "Drill (mm)".into(),
            "reference_prefix" => "Ref prefix".into(),
            "default_value" => "Value".into(),
            "electrical_type" => "Electrical".into(),
            "length_nm" => "Length (mm)".into(),
            "stroke_nm" => "Stroke (mm)".into(),
            "name_height_nm" => "Name size (mm)".into(),
            "number_height_nm" => "Number size (mm)".into(),
            "show_name" => "Show name".into(),
            "show_number" => "Show number".into(),
            "appearance" | "geometry" | "artwork" => String::new(),
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
        "electrical_type" => &[
            "passive",
            "input",
            "output",
            "bidirectional",
            "power_in",
            "power_out",
            "unspecified",
            "tri_state",
            "open_collector",
            "open_emitter",
            "no_connect",
        ],
        "shape" => &["rectangle", "rounded_rectangle", "circle", "oval"],
        "orientation" => &["right", "down", "left", "up"],
        "style" => &["line", "inverted", "clock", "inverted_clock"],
        "fill" => &["none", "solid", "background"],
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
                properties::choice(ui, &label(path), egui::Id::unique(path), text, &options);
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
        "appearance",
        "orientation",
        "length_nm",
        "style",
        "show_name",
        "show_number",
        "name_height_nm",
        "number_height_nm",
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
