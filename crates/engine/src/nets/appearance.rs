//! Schematic wire drawing appearance owned by stable net identity.
//!
//! Appearance is presentation intent, not electrical data: it never changes connectivity,
//! net classes, PCB rules or geometry. Overrides are sparse on a net registry record, so an
//! untouched record keeps its exact saved bytes and old projects decode without promotion.

use super::{ClassTarget, Registry, State, graph::Graph};
use crate::{CommandError, Project, Session};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Replace the sparse drawing appearance of one logical net.
pub const SET_APPEARANCE: &str = "nets.set_appearance";
/// Inspect the effective drawing appearance of one logical net without promoting it.
pub const INSPECT_APPEARANCE: &str = "nets.inspect_appearance";

/// Smallest supported drawn stroke, in hundredths of a point (0.5 pt).
pub const MIN_WIDTH_CP: u16 = 50;
/// Largest supported drawn stroke, in hundredths of a point (8 pt).
pub const MAX_WIDTH_CP: u16 = 800;
/// Default schematic wire stroke, in hundredths of a point. Matches the shared canvas token.
pub const DEFAULT_WIDTH_CP: u16 = 160;

/// A saved, sparse override. Missing fields mean "use the theme default".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Appearance {
    /// Opaque RGB channels for a schematic wire, from 0 through 255.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "stored_color")]
    pub color: Option<[u8; 3]>,
    /// Drawn stroke width in hundredths of a point, kept in whole numbers for stable saves.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "stored_width")]
    pub width_cp: Option<u16>,
}

impl Appearance {
    /// Check the bounds that both the command schema and the native decoder enforce.
    pub fn validate(&self) -> Result<(), String> {
        match self.width_cp {
            Some(width) if !(MIN_WIDTH_CP..=MAX_WIDTH_CP).contains(&width) => {
                return Err(format!("Wire stroke must be {MIN_WIDTH_CP} to {MAX_WIDTH_CP} hundredths of a point."));
            }
            _ => {}
        }
        Ok(())
    }

    /// Build the canonical JSON contract exposed to inspection and command clients.
    pub fn schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "color": {"type": "array", "minItems": 3, "maxItems": 3, "items": {"type": "integer", "minimum": 0, "maximum": 255}},
                "width_cp": {"type": "integer", "minimum": MIN_WIDTH_CP, "maximum": MAX_WIDTH_CP}
            }
        })
    }
}

/// Reject an explicit null where a saved appearance is optional but present.
fn stored_color<'de, D: Deserializer<'de>>(de: D) -> Result<Option<[u8; 3]>, D::Error> {
    let color = Option::<[u8; 3]>::deserialize(de)?;
    if color.is_none() {
        return Err(serde::de::Error::custom("A saved wire color cannot be null; omit the field."));
    }
    Ok(color)
}

/// Reject explicit nulls and out-of-range saved stroke widths.
fn stored_width<'de, D: Deserializer<'de>>(de: D) -> Result<Option<u16>, D::Error> {
    let width = Option::<u16>::deserialize(de)?;
    if let Some(width) = width
        && !(MIN_WIDTH_CP..=MAX_WIDTH_CP).contains(&width)
    {
        return Err(serde::de::Error::custom("Saved wire stroke is out of range."));
    }
    Ok(width)
}

/// Reject an empty or explicitly null appearance object: absence is encoded by omitting the field.
pub(super) fn stored_appearance<'de, D: Deserializer<'de>>(de: D) -> Result<Option<Appearance>, D::Error> {
    let appearance = Option::<Appearance>::deserialize(de)?;
    match appearance {
        None => Err(serde::de::Error::custom("A net appearance cannot be null; omit the field.")),
        Some(appearance) if appearance.color.is_none() && appearance.width_cp.is_none() => {
            Err(serde::de::Error::custom("A net appearance must set a color or a stroke width."))
        }
        Some(appearance) => {
            appearance.validate().map_err(serde::de::Error::custom)?;
            Ok(Some(appearance))
        }
    }
}

impl Registry {
    /// A registry needing the appearance envelope selects the newer project and instance headers.
    pub(crate) fn has_appearance(&self) -> bool {
        self.records.iter().any(|record| record.appearance.is_some())
    }
}

/// A command patch: omitted fields are kept, an empty object or null resets to theme defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    #[serde(default)]
    color: Option<[u8; 3]>,
    #[serde(default)]
    width_cp: Option<u16>,
}

impl Patch {
    /// Parse a strictly typed patch, rejecting unknown keys, null channels and unbounded widths.
    fn parse(value: &Value) -> Result<Self, CommandError> {
        if value.is_null() {
            return Ok(Self { color: None, width_cp: None });
        }
        let patch: Self = serde_json::from_value(value.clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
        let candidate = Appearance { color: patch.color, width_cp: patch.width_cp };
        candidate.validate().map_err(CommandError::InvalidParams)?;
        Ok(patch)
    }

    /// Return the full sparse appearance after applying this patch to a saved override.
    /// An empty patch (or an explicit null) is the documented reset to theme defaults.
    fn apply(self, saved: Option<Appearance>) -> Option<Appearance> {
        if self.color.is_none() && self.width_cp.is_none() {
            return None;
        }
        let color = self.color.or_else(|| saved.and_then(|appearance| appearance.color));
        let width_cp = self.width_cp.or_else(|| saved.and_then(|appearance| appearance.width_cp));
        (color.is_some() || width_cp.is_some()).then_some(Appearance { color, width_cp })
    }
}

impl Session {
    /// Apply one sparse appearance patch atomically. Eligible derived nets are promoted in the
    /// same history step; a patch that changes nothing never allocates an identity or a revision.
    pub(crate) fn set_net_appearance(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let original = self.open_project()?;
        let target = ClassTarget::parse(&params["target"])?;
        let patch = Patch::parse(&params["appearance"])?;
        let (existing, _) = original.class_target(&target)?;
        let saved = match existing {
            Some(id) => original.net_registry.record(id).ok_or_else(|| CommandError::NotFound("Unknown net identity.".into()))?.appearance,
            None => None,
        };
        let wanted = patch.apply(saved);
        if wanted == saved {
            return Ok(());
        }
        let (mut candidate, id) = original.with_promoted_net(&target)?;
        let mut registry = (*candidate.net_registry).clone();
        let record = registry
            .records
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or_else(|| CommandError::InvalidParams("Missing appearance identity.".into()))?;
        if !matches!(record.state, State::Active {}) {
            return Err(CommandError::InvalidParams("Appearance requires an active net identity.".into()));
        }
        record.appearance = wanted;
        candidate.net_registry = std::sync::Arc::new(registry);
        candidate.net_registry.validate(&candidate).map_err(CommandError::InvalidParams)?;
        self.change(candidate)
    }

    /// Inspect the effective appearance of a current or saved net without editing the project.
    pub(crate) fn inspect_net_appearance(&self, params: &Value) -> Result<Value, CommandError> {
        let project = self.open_project()?;
        let target = params.get("target").ok_or_else(|| CommandError::InvalidParams("A net target is required.".into()))?;
        let target = ClassTarget::parse(target)?;
        let (id, _) = project.class_target(&target)?;
        let Some(id) = id else {
            return Ok(json!({"target":target,"appearance":null,"status":"derived"}));
        };
        let record = project.net_registry.record(id).ok_or_else(|| CommandError::NotFound("Unknown net identity.".into()))?;
        let status = match &record.state {
            State::Retired { .. } => "retired",
            State::Ambiguous { .. } => "identity_ambiguous",
            State::Active {} => "ready",
        };
        Ok(json!({
            "target": ClassTarget::Persistent { project: project.id, scope: project.net_scope(), net: id },
            "appearance": record.appearance,
            "status": status,
        }))
    }
}

impl Project {
    /// Map every schematic wire with a saved net appearance to its effective override.
    ///
    /// This read builds the electrical graph only when at least one override exists, so ordinary
    /// projects keep the existing paint cost. Wires without a supported current identity are
    /// omitted; selection and highlight colors still take precedence when drawing.
    pub fn wire_appearances(&self) -> Result<BTreeMap<String, Appearance>, CommandError> {
        if !self.net_registry.has_appearance() {
            return Ok(BTreeMap::new());
        }
        let graph = Graph::build(self).map_err(CommandError::InvalidParams)?;
        let mut result = BTreeMap::new();
        for sheet in self.sheets() {
            let own = self.for_sheet(sheet.target).ok_or_else(|| CommandError::NotFound("Unknown appearance sheet.".into()))?;
            for wire in own.wires() {
                let Some(anchor) = graph.endpoint_anchor(sheet.target, wire.endpoints()[0]) else { continue };
                let Some(index) = graph.network(anchor) else { continue };
                let appearance = self
                    .net_registry
                    .records
                    .iter()
                    .find(|record| matches!(record.state, State::Active {}) && graph.network(&record.primary) == Some(index))
                    .and_then(|record| record.appearance);
                if let Some(appearance) = appearance {
                    result.insert(wire.id(), appearance);
                }
            }
        }
        Ok(result)
    }
}

/// Publish the strict patch schema used by the appearance command.
pub(crate) fn appearance_param_schema() -> Value {
    json!({"oneOf":[{"type":"null"}, Appearance::schema()]})
}
