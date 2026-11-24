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
            value["symbol_settings"] = json!({"behavior":item.behavior,"defaults":item.defaults,"category":item.category});
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
        if self.value["type"] == "pin" && self.value.get("appearance").is_none() && pcbcraft_chrome::controls::button(ui, "Add pin lead").clicked() {
            self.value["appearance"] = serde_json::to_value(pcbcraft_engine::schematic::PinAppearance::default()).unwrap_or(Value::Null);
        }
        let pad = self.value["type"] == "pad";
        let mut hidden = Vec::new();
        if self.value["type"] == "pin" {
            hidden.push("alternates");
            if self.value.get("appearance").is_some() {
                alternate_fields(ui, &mut self.value);
            }
        }
        // Preserve category references in metadata edits without exposing raw identity fields
        // as a substitute for a category-selection workflow.
        if self.kind == Kind::Symbol {
            hidden.push("category");
        }
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
            let mut kind = if self.value["pad"].get("plated_slot").is_some() {
                hidden.push("drill_nm");
                "plated_slot"
            } else if self.value["pad"]["copper"] == "all_copper" {
                "through_hole"
            } else {
                "smd"
            }
            .to_owned();
            let before = kind.clone();
            properties::choice(
                ui,
                "Pad type",
                egui::Id::unique("pad_type"),
                &mut kind,
                &[("smd", "SMD".into()), ("through_hole", "Through hole".into()), ("plated_slot", "Plated slot".into())],
            );
            if kind != before {
                self.value["pad"]["copper"] = json!(if kind == "smd" { "component_side" } else { "all_copper" });
                let pad = &mut self.value["pad"];
                let smaller_side = pad["width_nm"].as_i64().unwrap_or(600_000).min(pad["height_nm"].as_i64().unwrap_or(600_000));
                pad["drill_nm"] = if kind == "through_hole" { json!(smaller_side / 2) } else { Value::Null };
                if kind == "plated_slot" {
                    pad["plated_slot"] = json!({"width_nm":(smaller_side / 4).max(1),"length_nm":(smaller_side / 2).max(1),"rotation_mdeg":0});
                    hidden.push("drill_nm");
                } else if let Some(fields) = pad.as_object_mut() {
                    fields.remove("plated_slot");
                    hidden.retain(|field| *field != "drill_nm");
                }
                self.numbers.retain(|path, _| path != ".pad.drill_nm" && !path.starts_with(".pad.plated_slot."));
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
            let primitive: pcbcraft_engine::authoring::Primitive = serde_json::from_value(value).map_err(|e| e.to_string())?;
            if let Some((points, stroke_nm)) = pcbcraft_engine::authoring::courtyard::courtyard_polygon(&primitive) {
                let original_stroke = serde_json::from_value::<pcbcraft_engine::authoring::Primitive>(self.original.clone())
                    .ok()
                    .and_then(|existing| pcbcraft_engine::authoring::courtyard::courtyard_polygon(&existing).map(|(_, stroke)| stroke));
                if original_stroke != Some(stroke_nm) {
                    pcbcraft_engine::authoring::courtyard::validate_stroke(stroke_nm)?;
                }
                pcbcraft_engine::authoring::courtyard::validate_polygon(points)?;
            }
            Ok(Action::Replace { object, primitive })
        } else {
            serde_json::from_value(value).map_err(|e| e.to_string())
        }
    }

    /// Render the buffered stroke-width field for a selected courtyard outline.
    /// Returns `true` when Enter was pressed while the field had focus.
    pub(super) fn show_courtyard_stroke(&mut self, ui: &mut Ui) -> bool {
        let stroke_nm = self
            .value
            .get("artwork")
            .and_then(|a| a.get("graphic"))
            .and_then(|g| g.get("stroke_nm"))
            .and_then(Value::as_i64)
            .unwrap_or(pcbcraft_engine::authoring::courtyard::DEFAULT_STROKE_NM);
        let initial = format!("{}", stroke_nm as f64 / 1_000_000.0);
        let field = self.numbers.entry(".artwork.graphic.stroke_nm".into()).or_insert_with(|| (initial.clone(), initial));
        let response = properties::text(ui, "Stroke (mm)", &mut field.1);
        response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
    }

    /// Render the buffered layer purpose selector for a selected courtyard outline.
    pub(super) fn show_courtyard_purpose(&mut self, ui: &mut Ui) {
        let Some(Value::String(purpose)) = self.value.get_mut("artwork").and_then(|a| a.get_mut("purpose")) else {
            return;
        };
        let options = choices(".artwork.purpose").iter().map(|option| (*option, display(option))).collect::<Vec<_>>();
        properties::choice(ui, &label(".artwork.purpose"), egui::Id::unique(".artwork.purpose"), purpose, &options);
    }

    /// Render the buffered X and Y coordinate fields for the selected courtyard vertex.
    /// Returns `true` when Enter was pressed while either field had focus.
    pub(super) fn show_courtyard_vertex(&mut self, ui: &mut Ui, index: usize) -> bool {
        self.show_courtyard_vertex_labeled(ui, index, "Vertex X (mm)", "Vertex Y (mm)")
    }

    /// Render numbered vertex coordinate fields for the expanded properties dialog.
    /// Returns `true` when Enter was pressed while either field had focus.
    pub(super) fn show_courtyard_vertex_row(&mut self, ui: &mut Ui, index: usize) -> bool {
        let x_label = format!("Vertex {} X (mm)", index + 1);
        let y_label = format!("Vertex {} Y (mm)", index + 1);
        self.show_courtyard_vertex_labeled(ui, index, &x_label, &y_label)
    }

    /// Shared implementation for editing one courtyard polygon vertex's buffered X/Y lengths.
    fn show_courtyard_vertex_labeled(&mut self, ui: &mut Ui, index: usize, x_label: &str, y_label: &str) -> bool {
        let Some(point) = self
            .value
            .get("artwork")
            .and_then(|a| a.get("graphic"))
            .and_then(|g| g.get("geometry"))
            .and_then(|g| g.get("points"))
            .and_then(Value::as_array)
            .and_then(|pts| pts.get(index))
        else {
            return false;
        };
        let x_nm = point.get("x_nm").and_then(Value::as_i64).unwrap_or(0);
        let y_nm = point.get("y_nm").and_then(Value::as_i64).unwrap_or(0);
        let x_init = format!("{}", x_nm as f64 / 1_000_000.0);
        let y_init = format!("{}", y_nm as f64 / 1_000_000.0);
        let x_key = format!(".artwork.graphic.geometry.points.{index}.x_nm");
        let y_key = format!(".artwork.graphic.geometry.points.{index}.y_nm");
        let x_field = self.numbers.entry(x_key).or_insert_with(|| (x_init.clone(), x_init));
        let x_resp = properties::text(ui, x_label, &mut x_field.1);
        let y_field = self.numbers.entry(y_key).or_insert_with(|| (y_init.clone(), y_init));
        let y_resp = properties::text(ui, y_label, &mut y_field.1);
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        enter && (x_resp.lost_focus() || y_resp.lost_focus())
    }
}
/// Edit exact names and types in the existing one-transaction primitive draft.
/// Omit an empty list so legacy pin primitives retain their representation.
fn alternate_fields(ui: &mut Ui, value: &mut Value) {
    pcbcraft_chrome::controls::collapsing(ui, "pin_alternates", "Alternate pin functions", false, None, |ui| {
        let mut rows = value.get("alternates").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut remove = None;
        for (index, row) in rows.iter_mut().enumerate() {
            ui.push_id(index, |ui| {
                let mut name = row["name"].as_str().unwrap_or_default().to_owned();
                let mut electrical_type = row["electrical_type"].as_str().unwrap_or_default().to_owned();
                properties::text(ui, "Function name", &mut name);
                let types: Vec<_> = choices(".electrical_type").iter().map(|&key| (key, display(key))).collect();
                properties::choice(ui, "Electrical type", egui::Id::unique("alternate_electrical_type"), &mut electrical_type, &types);
                *row = json!({"name":name,"electrical_type":electrical_type});
                if pcbcraft_chrome::controls::button(ui, "Remove alternate").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            rows.remove(index);
        }
        if pcbcraft_chrome::controls::enabled_button(ui, rows.len() < pcbcraft_engine::pin_functions::MAX_ALTERNATES, "Add alternate").clicked() {
            rows.push(json!({"name":"","electrical_type":"passive"}));
        }
        if rows.is_empty() {
            if let Some(object) = value.as_object_mut() {
                object.remove("alternates");
            }
        } else {
            value["alternates"] = Value::Array(rows);
        }
    });
}

/// Field groups describe the selected object rather than its ordinal in a native array.
pub(super) fn title(draft: &Draft) -> String {
    match draft.value["type"].as_str() {
        Some("pin") => format!("Pin {}", draft.value["number"].as_str().unwrap_or("")),
        Some("pad") => format!("Pad {}", draft.value["pad"]["number"].as_str().unwrap_or("")),
        Some("non_plated_hole") => "Non-plated hole".into(),
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
            "diameter_nm" => "Finished diameter (mm)".into(),
            "reference_prefix" => "Ref prefix".into(),
            "default_value" => "Value".into(),
            "electrical_type" => "Electrical".into(),
            "length_nm" => "Length (mm)".into(),
            "plated_slot" => "Slot".into(),
            "rotation_mdeg" => "Rotation (°)".into(),
            "stroke_nm" => "Stroke (mm)".into(),
            "name_height_nm" => "Name size (mm)".into(),
            "number_height_nm" => "Number size (mm)".into(),
            "show_name" => "Show name".into(),
            "show_number" => "Show number".into(),
            "appearance" | "geometry" | "artwork" => String::new(),
            "at" | "center" | "hole" | "pad" | "graphic" | "path" | "points" => String::new(),
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
            pcbcraft_chrome::controls::checkbox(ui, value, label(path));
        }
        Value::Number(number) => {
            let text = number
                .as_i64()
                .map(|n| {
                    if path.ends_with("_nm") {
                        format!("{}", n as f64 / 1_000_000.0)
                    } else if path.ends_with("_mdeg") {
                        format_mdeg(n)
                    } else {
                        n.to_string()
                    }
                })
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
        } else if path.ends_with("_mdeg") {
            parse_mdeg(text).map_err(|e| format!("{} {e}", label(path)))?
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
/// Format whole millidegrees as exact degrees without trailing fractional zeros.
fn format_mdeg(mdeg: i64) -> String {
    let frac = (mdeg % 1_000).unsigned_abs();
    if frac == 0 {
        (mdeg / 1_000).to_string()
    } else {
        let whole = mdeg / 1_000;
        let sign = if mdeg < 0 && whole == 0 { "-" } else { "" };
        let trimmed = format!("{frac:03}");
        format!("{sign}{whole}.{}", trimmed.trim_end_matches('0'))
    }
}

/// Parse a non-negative degree value in `[0, 360)` with up to three decimal places into whole millidegrees.
fn parse_mdeg(text: &str) -> Result<i64, &'static str> {
    const RANGE_ERR: &str = "must be at least 0° and less than 360°.";
    let text = text.trim();
    let text = text.strip_suffix('°').unwrap_or(text).trim();
    if text.is_empty() {
        return Err("must be a non-negative number in degrees (up to 3 decimal places).");
    }
    let (whole, frac) = match text.split_once('.') {
        Some((w, f)) => (w, f),
        None => (text, ""),
    };
    if whole.is_empty() && frac.is_empty() {
        return Err("must be a non-negative number in degrees (up to 3 decimal places).");
    }
    if frac.len() > 3 || !whole.bytes().all(|b| b.is_ascii_digit()) || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return Err("must be a non-negative number in degrees (up to 3 decimal places).");
    }
    let whole: i64 = if whole.is_empty() { 0 } else { whole.parse().map_err(|_| RANGE_ERR)? };
    let mut frac_mdeg: i64 = if frac.is_empty() { 0 } else { frac.parse().map_err(|_| RANGE_ERR)? };
    for _ in frac.len()..3 {
        frac_mdeg *= 10;
    }
    let mdeg = whole.checked_mul(1_000).and_then(|v| v.checked_add(frac_mdeg)).ok_or(RANGE_ERR)?;
    if mdeg >= 360_000 {
        return Err(RANGE_ERR);
    }
    Ok(mdeg)
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

#[cfg(test)]
mod category_tests {
    use super::*;

    /// Editing ordinary definition metadata must not discard a loaded symbol's typed category.
    #[test]
    fn metadata_draft_retains_an_existing_category_binding() {
        let mut symbol = pcbcraft_engine::library::parse_symbol(include_str!("../../../../examples/native-parts/symbols/resistor.json")).unwrap();
        symbol.library = "parts-demo".into();
        symbol.provenance =
            Some(pcbcraft_engine::library::sources::Provenance { library_name: "Native Parts Demo".into(), license: "MIT OR Apache-2.0".into() });
        let item = Item::from_symbol(&symbol).unwrap();
        let mut draft = Draft::new(&item, None, 0);
        draft.value["name"] = json!("Renamed resistor");
        let mut session = Session::new(item, false).unwrap();
        session.execute(0, draft.action().unwrap()).unwrap();
        assert_eq!(session.snapshot().category, symbol.category);
        assert_eq!(session.snapshot().export_native().unwrap()["format"], "pcbcraft-symbol/3");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod slot_form_tests {
    use super::*;
    use pcbcraft_engine::{
        authoring::Primitive,
        board::{
            definition::{CopperBinding, PadShape},
            slot::PlatedSlot,
        },
    };

    /// Slot rotation uses degrees in the editor form while storing whole millidegrees.
    #[test]
    fn slot_rotation_field_formats_and_parses_degrees() {
        assert_eq!(label(".pad.plated_slot.rotation_mdeg"), "Slot Rotation (°)");
        assert_eq!(format_mdeg(0), "0");
        assert_eq!(format_mdeg(30_000), "30");
        assert_eq!(format_mdeg(30_500), "30.5");
        assert_eq!(format_mdeg(30_050), "30.05");
        assert_eq!(format_mdeg(30_005), "30.005");
        assert_eq!(format_mdeg(359_999), "359.999");

        assert_eq!(parse_mdeg("0"), Ok(0));
        assert_eq!(parse_mdeg("30"), Ok(30_000));
        assert_eq!(parse_mdeg("30.5"), Ok(30_500));
        assert_eq!(parse_mdeg("30.05"), Ok(30_050));
        assert_eq!(parse_mdeg("30.005"), Ok(30_005));
        assert_eq!(parse_mdeg("30.5°"), Ok(30_500));
        assert_eq!(parse_mdeg("359.999"), Ok(359_999));
        for invalid in ["", "  ", ".", "-5", "30,5", "30.1234", "360", "400", "abc"] {
            assert!(parse_mdeg(invalid).is_err(), "{invalid:?} should be rejected");
        }
        assert_eq!(parse_mdeg("360").unwrap_err(), "must be at least 0° and less than 360°.");
        assert_eq!(parse_mdeg("99999999999999999999").unwrap_err(), "must be at least 0° and less than 360°.");

        let item = Item::new(Kind::Footprint).unwrap();
        let mut session = Session::new(item, false).unwrap();
        let Primitive::Pad { mut pad } = session.snapshot().objects.get(&1).cloned().unwrap() else { panic!("expected pad 1") };
        pad.width_nm = 2_400_000;
        pad.height_nm = 1_200_000;
        pad.shape = PadShape::Rectangle;
        pad.corner_radius_nm = 0;
        pad.copper = CopperBinding::AllCopper;
        pad.drill_nm = None;
        pad.plated_slot = Some(PlatedSlot::new(600_000, 1_400_000, 0).unwrap());
        session.execute(0, Action::Replace { object: 1, primitive: Primitive::Pad { pad } }).unwrap();

        let mut draft = Draft::new(&session.snapshot(), Some(1), session.revision());
        draft.numbers.insert(".pad.plated_slot.rotation_mdeg".into(), ("0".into(), "30.5".into()));
        let Action::Replace { object: 1, primitive: Primitive::Pad { pad } } = draft.action().unwrap() else {
            panic!("expected pad replacement");
        };
        assert_eq!(pad.plated_slot.unwrap().dimensions(), (600_000, 1_400_000, 30_500));

        draft.numbers.insert(".pad.plated_slot.rotation_mdeg".into(), ("0".into(), "30.1234".into()));
        let error = draft.action().unwrap_err();
        assert!(error.contains("Slot Rotation (°)"), "{error}");

        draft.numbers.insert(".pad.plated_slot.rotation_mdeg".into(), ("0".into(), "360".into()));
        let error = draft.action().unwrap_err();
        assert_eq!(error, "Slot Rotation (°) must be at least 0° and less than 360°.");
    }
}
