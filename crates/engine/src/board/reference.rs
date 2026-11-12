//! Board-local printed reference overrides, independent of component identity and library data.
use crate::{
    CommandError, Session,
    schematic::{Point, TextAlign},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// Change one placement's local print appearance and optional offset in one undoable transaction.
pub const SET_REFERENCE_TEXT: &str = "pcb.set_footprint_reference_text";

/// A local size override keeps height and stroke together so global style edits stay independent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Size {
    pub height_nm: i64,
    pub stroke_width_nm: i64,
}

/// Sparse print overrides. An absent text follows the component reference, and Reset removes overrides.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Text {
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    pub font: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    pub size: Option<Size>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    pub rotation: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    pub align: Option<TextAlign>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    pub visible: Option<bool>,
}

/// Absence inherits a field; an explicitly present null is rejected to preserve canonical saves.
pub(crate) fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// Resolved print appearance retains component identity separately from the displayed words.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Resolved {
    pub text: String,
    pub font: String,
    pub size: Size,
    pub rotation: u16,
    pub align: TextAlign,
    pub visible: bool,
}

impl Text {
    /// Reject empty override records, unsupported fonts, control characters and invalid physical sizes.
    pub fn validate(&self) -> Result<(), String> {
        if self == &Self::default() {
            return Err("Omit an empty reference appearance to inherit its defaults.".into());
        }
        if self.text.as_ref().is_some_and(|text| text.trim().is_empty() || text.chars().count() > 128 || text.chars().any(char::is_control)) {
            return Err("Silkscreen text must contain 1–128 characters without control characters.".into());
        }
        if self.font.as_deref().is_some_and(|font| !matches!(font, "pcbcraft-technical-1" | "pcbcraft-stroke-1")) {
            return Err("Unsupported silkscreen font.".into());
        }
        if self.size.is_some_and(|size| !(1..=10_000_000).contains(&size.height_nm) || !(1..=size.height_nm / 2).contains(&size.stroke_width_nm)) {
            return Err("Text height must be positive and at most 10 mm, with stroke width at most half the height.".into());
        }
        if self.rotation.is_some_and(|rotation| rotation >= 360) {
            return Err("Reference rotation must be from 0 through 359 degrees.".into());
        }
        Ok(())
    }

    /// Describe the nullable override accepted by GUI and MCP, with cross-field sizes checked by validation.
    pub(crate) fn schema() -> Value {
        json!({
            "type": ["object", "null"],
            "minProperties": 1,
            "additionalProperties": false,
            "properties": {
                "text": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 128,
                    "pattern": r"^[^\u0000-\u001f\u007f-\u009f]+(?![\s\S])"
                },
                "font": {
                    "type": "string",
                    "enum": ["pcbcraft-technical-1", "pcbcraft-stroke-1"]
                },
                "size": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["height_nm", "stroke_width_nm"],
                    "properties": {
                        "height_nm": {"type": "integer", "minimum": 1, "maximum": 10_000_000},
                        "stroke_width_nm": {"type": "integer", "minimum": 1, "maximum": 5_000_000}
                    }
                },
                "rotation": {"type": "integer", "minimum": 0, "maximum": 359},
                "align": {"type": "string", "enum": ["start", "middle", "end"]},
                "visible": {"type": "boolean"}
            }
        })
    }
}

impl Resolved {
    /// Resolve validated sparse placement overrides against the pinned definition and board print settings.
    /// Callers must validate `Text` and settings before rendering or persisting the resolved appearance.
    pub fn new(
        definition: Option<&super::definition::FootprintDefinition>,
        reference: &str,
        settings: super::silkscreen::Settings,
        text: Option<&Text>,
    ) -> Self {
        let empty = Text::default();
        let text = text.unwrap_or(&empty);
        Self {
            text: text.text.as_deref().unwrap_or(reference).to_owned(),
            font: text.font.as_deref().or_else(|| definition.and_then(|d| d.reference.as_deref())).unwrap_or("pcbcraft-technical-1").to_owned(),
            size: text.size.unwrap_or(Size { height_nm: settings.text_height_nm, stroke_width_nm: settings.stroke_width_nm }),
            rotation: text.rotation.unwrap_or(0),
            align: text.align.unwrap_or(TextAlign::Middle),
            visible: text.visible.unwrap_or_else(|| definition.is_some_and(|d| d.reference.is_some())),
        }
    }
}

impl Session {
    /// Set board-local print overrides without changing the component, its reference, library or pads.
    /// Unknown/stale targets and invalid input reject the whole edit, including a supplied offset.
    pub(crate) fn set_footprint_reference_text(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let id = params["footprint"].as_str().and_then(|id| crate::schematic::wiring::parse_id(id, "footprint-"));
        let index = project
            .board
            .footprints
            .binary_search_by_key(&id.unwrap_or_default(), |f| f.id)
            .map_err(|_| CommandError::InvalidParams("unknown footprint ID".into()))?;
        let text: Option<Text> =
            serde_json::from_value(params["appearance"].clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))?;
        if let Some(text) = &text {
            text.validate().map_err(CommandError::InvalidParams)?;
            if project.board.definition_for(&project.board.footprints[index]).is_none() {
                return Err(CommandError::InvalidParams("Physical silkscreen appearance requires a native footprint definition.".into()));
            }
        }
        let mut footprint = project.board.footprints[index].clone();
        footprint.reference_text = text;
        if let Some(value) = params.get("offset") {
            let offset: Point = serde_json::from_value(value.clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))?;
            footprint.reference_offset = (offset != super::DEFAULT_REFERENCE_OFFSET).then_some(offset);
        }
        if !footprint.valid() {
            return Err(CommandError::InvalidParams("Reference position or appearance is invalid.".into()));
        }
        if footprint == project.board.footprints[index] {
            return Ok(());
        }
        let mut next = project.clone();
        Arc::make_mut(&mut next.board).footprints[index] = footprint;
        self.change(next)?;
        Ok(())
    }
}
