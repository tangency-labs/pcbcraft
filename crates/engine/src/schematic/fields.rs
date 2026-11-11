//! Part instance metadata and per-placement field appearance are independent of pin geometry.
use super::{MAX_CENTER_NM, Point, TextAlign};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Standard optional fields retain stable identities even when their display names change.
pub const STANDARD: [(&str, &str); 4] = [
    ("datasheet", "Datasheet"),
    ("description", "Description"),
    ("manufacturer", "Manufacturer"),
    ("manufacturer_part_number", "Manufacturer part number"),
];

/// A named component field. The enclosing map key is its stable identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub name: String,
    pub value: String,
}

/// Authored component intent, retained for manufacturing and simulation consumers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attributes {
    pub do_not_populate: bool,
    pub exclude_from_bom: bool,
    pub exclude_from_board: bool,
    pub exclude_from_simulation: bool,
    pub exclude_from_position_files: bool,
}

/// Part instance content is shared by its units; library definitions remain pinned and unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub fields: BTreeMap<String, Field>,
    pub attributes: Attributes,
}

impl Metadata {
    /// Empty metadata is omitted from legacy-compatible saves.
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Bound field allocation and text, rejecting reserved/duplicate names and invalid IDs.
    pub fn validate(&self) -> Result<(), String> {
        if self.fields.len() > 64 {
            return Err("a component supports at most 64 optional fields".into());
        }
        let mut names = std::collections::BTreeSet::from(["reference".to_owned(), "value".into(), "footprint".into()]);
        for (id, field) in &self.fields {
            if !valid_id(id) {
                return Err("field identity must be a standard field ID or non-nil UUID".into());
            }
            if field.name.is_empty()
                || field.name.trim() != field.name
                || field.name.chars().count() > 64
                || field.name.chars().any(char::is_control)
                || !names.insert(field.name.to_lowercase())
            {
                return Err("field names must be unique, nonempty single-line text of at most 64 characters".into());
            }
            if field.value.chars().count() > 2048 || field.value.chars().any(char::is_control) {
                return Err("field values must be single-line text of at most 2048 characters".into());
            }
        }
        Ok(())
    }
}

/// Field positions are local unrotated offsets from the symbol center. None uses automatic
/// placement. A body move carries the field and a body turn rotates its offset; text orientation
/// remains explicitly controlled by `rotation`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    #[serde(deserialize_with = "required_offset")]
    pub offset: Option<Point>,
    pub visible: bool,
    pub show_name: bool,
    pub rotation: u16,
    pub align: TextAlign,
    pub height_nm: i64,
}

impl Default for Layout {
    fn default() -> Self {
        Self { offset: None, visible: true, show_name: false, rotation: 0, align: TextAlign::Middle, height_nm: 1_270_000 }
    }
}

/// Stable field IDs map to appearance on a single placed unit.
pub type Layouts = BTreeMap<String, Layout>;

/// Allocate an instance-field identity without making its name or row position its identity.
pub fn new_id() -> Result<String, String> {
    crate::fresh_id().map(|id| id.to_string()).map_err(|error| error.to_string())
}

impl crate::Project {
    /// Preview one unit's validated text appearance without changing revision, history or wiring.
    pub fn with_field_layout(&self, id: &str, layouts: Layouts) -> Result<Self, String> {
        let symbol = self.symbols().iter().find(|s| s.id() == id).ok_or("symbol is missing")?;
        validate_layouts(&layouts, &self.component(symbol).ok_or("component is missing")?.metadata)?;
        let mut next = self.clone();
        let symbol = std::sync::Arc::make_mut(&mut next.sheet_mut().symbols).iter_mut().find(|s| s.id() == id).ok_or("symbol is missing")?;
        symbol.field_layout = layouts;
        Ok(next)
    }
}

/// Reference/value always exist; optional fields belong to the component metadata.
pub fn known(id: &str, metadata: &Metadata) -> bool {
    matches!(id, "reference" | "value") || metadata.fields.contains_key(id)
}

/// Reject dangling field appearances and invalid geometry before a transaction or load.
pub fn validate_layouts(layouts: &Layouts, metadata: &Metadata) -> Result<(), String> {
    if layouts.len() > 66 {
        return Err("a symbol supports at most 66 field appearances".into());
    }
    for (id, layout) in layouts {
        if !known(id, metadata) {
            return Err(format!("appearance points to unknown field {id}"));
        }
        if !matches!(layout.rotation, 0 | 90 | 180 | 270)
            || !(100_000..=10_000_000).contains(&layout.height_nm)
            || layout.offset.is_some_and(|p| p.x_nm.unsigned_abs() > MAX_CENTER_NM as u64 || p.y_nm.unsigned_abs() > MAX_CENTER_NM as u64)
        {
            return Err("field appearance has invalid rotation, height or offset".into());
        }
    }
    Ok(())
}

/// Standard identities and UUIDs cannot collide with the mandatory reference/value fields.
fn valid_id(id: &str) -> bool {
    STANDARD.iter().any(|(key, _)| id == *key) || uuid::Uuid::parse_str(id).is_ok_and(|parsed| !parsed.is_nil() && parsed.to_string() == id)
}

/// An explicit null requests automatic placement; omission is an incomplete layout record.
fn required_offset<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Point>, D::Error> {
    Option::<Point>::deserialize(deserializer)
}

/// The public command schema mirrors the strict native metadata representation.
pub fn metadata_schema() -> serde_json::Value {
    use serde_json::json;
    let attributes = ["do_not_populate", "exclude_from_bom", "exclude_from_board", "exclude_from_simulation", "exclude_from_position_files"];
    json!({"type":"object","additionalProperties":false,"required":["fields","attributes"],"properties":{
        "fields":{"type":"object","maxProperties":64,"additionalProperties":{"type":"object","additionalProperties":false,
            "required":["name","value"],"properties":{"name":{"type":"string","minLength":1,"maxLength":64},"value":{"type":"string","maxLength":2048}}}},
        "attributes":{"type":"object","additionalProperties":false,"required":attributes,
            "properties":attributes.into_iter().map(|key| (key,json!({"type":"boolean"}))).collect::<BTreeMap<_,_>>()}
    }})
}

/// Layout replacement is bounded and validated against the resulting component fields.
pub fn layouts_schema() -> serde_json::Value {
    use serde_json::json;
    json!({"type":"object","maxProperties":66,"additionalProperties":{"type":"object","additionalProperties":false,
        "required":["offset","visible","show_name","rotation","align","height_nm"],"properties":{
        "offset":{"oneOf":[{"type":"null"},{"type":"object","additionalProperties":false,"required":["x_nm","y_nm"],"properties":{
            "x_nm":{"type":"integer","minimum":-MAX_CENTER_NM,"maximum":MAX_CENTER_NM},
            "y_nm":{"type":"integer","minimum":-MAX_CENTER_NM,"maximum":MAX_CENTER_NM}}}]},
        "visible":{"type":"boolean"},"show_name":{"type":"boolean"},"rotation":{"type":"integer","enum":[0,90,180,270]},
        "align":{"type":"string","enum":["start","middle","end"]},"height_nm":{"type":"integer","minimum":100000,"maximum":10000000}
    }}})
}

#[cfg(test)]
mod tests;
