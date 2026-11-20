//! Schematic wire drawing appearance owned by stable net identity.
//!
//! Appearance is presentation intent, not electrical data: it never changes connectivity,
//! net classes, PCB rules or geometry. Overrides are sparse on a net registry record, so an
//! untouched record keeps its exact saved bytes and old projects decode without promotion.

use super::{Anchor, AnchorKind, ClassTarget, Registry, State, graph::Graph};
use crate::{CommandError, Project, Session, SheetTarget, WireEndpoint, schematic::wiring::Connectivity};
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

/// Reject an explicit null where an appearance color is optional but present.
/// Omission is handled by `serde(default)` at the field, so reaching here with a null is an error.
fn stored_color<'de, D: Deserializer<'de>>(de: D) -> Result<Option<[u8; 3]>, D::Error> {
    let color = Option::<[u8; 3]>::deserialize(de)?;
    if color.is_none() {
        return Err(serde::de::Error::custom("A wire color cannot be null; omit the field."));
    }
    Ok(color)
}

/// Reject an explicit null or an out-of-range stroke width. Omission is handled by
/// `serde(default)` at the field, so a null reaching this decoder is an error.
fn stored_width<'de, D: Deserializer<'de>>(de: D) -> Result<Option<u16>, D::Error> {
    let width = Option::<u16>::deserialize(de)?;
    if width.is_none() {
        return Err(serde::de::Error::custom("A wire stroke cannot be null; omit the field."));
    }
    if let Some(width) = width
        && !(MIN_WIDTH_CP..=MAX_WIDTH_CP).contains(&width)
    {
        return Err(serde::de::Error::custom("Wire stroke is out of range."));
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

/// One saved appearance channel that a command patch removes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClearChannel {
    /// Remove the saved RGB color while leaving the stroke width intact.
    Color,
    /// Remove the saved stroke width while leaving the RGB color intact.
    WidthCp,
}

/// A command patch: omitted fields are kept, an empty object or null resets to theme defaults.
/// An explicit null for either channel is rejected; use `clear` to remove a saved channel.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    #[serde(default, deserialize_with = "stored_color")]
    color: Option<[u8; 3]>,
    #[serde(default, deserialize_with = "stored_width")]
    width_cp: Option<u16>,
    /// Channels to remove from the saved override; setting and clearing one channel conflicts.
    #[serde(default)]
    clear: Vec<ClearChannel>,
}

impl Patch {
    /// Parse a strictly typed patch, rejecting unknown keys, null channels, contradictory clear
    /// markers and unbounded widths. A null patch is the documented full reset.
    fn parse(value: &Value) -> Result<Self, CommandError> {
        if value.is_null() {
            return Ok(Self { color: None, width_cp: None, clear: Vec::new() });
        }
        let patch: Self = serde_json::from_value(value.clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
        let candidate = Appearance { color: patch.color, width_cp: patch.width_cp };
        candidate.validate().map_err(CommandError::InvalidParams)?;
        if patch.color.is_some() && patch.clear.contains(&ClearChannel::Color) {
            return Err(CommandError::InvalidParams("A patch cannot set and clear the color channel.".into()));
        }
        if patch.width_cp.is_some() && patch.clear.contains(&ClearChannel::WidthCp) {
            return Err(CommandError::InvalidParams("A patch cannot set and clear the width_cp channel.".into()));
        }
        Ok(patch)
    }

    /// Return the full sparse appearance after applying this patch to a saved override.
    /// An empty patch (or an explicit null) is the documented reset to theme defaults; a clear
    /// marker removes just that channel while the other is inherited from the saved override.
    fn apply(self, saved: Option<Appearance>) -> Option<Appearance> {
        let clear_color = self.clear.contains(&ClearChannel::Color);
        let clear_width = self.clear.contains(&ClearChannel::WidthCp);
        if self.color.is_none() && self.width_cp.is_none() && !clear_color && !clear_width {
            return None;
        }
        let color = if clear_color { None } else { self.color.or_else(|| saved.and_then(|appearance| appearance.color)) };
        let width_cp = if clear_width { None } else { self.width_cp.or_else(|| saved.and_then(|appearance| appearance.width_cp)) };
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

/// The current schematic endpoint of a live logical anchor. Junction anchors are codec-only and
/// have no schematic endpoint, so callers fall back to the electrical network in that case.
fn anchor_endpoint(project: &Project, anchor: &Anchor) -> Option<WireEndpoint> {
    match anchor.kind {
        AnchorKind::Terminal => project.component_terminal(anchor.object, &anchor.terminal).map(WireEndpoint::Terminal),
        AnchorKind::Label => Some(WireEndpoint::NetLabel(anchor.object)),
        AnchorKind::Port => Some(WireEndpoint::Label(anchor.object)),
        AnchorKind::Junction => None,
    }
}

/// Resolve a wire to the current logical network that owns it.
///
/// Terminals, labels and hierarchical ports project straight to a graph anchor. Junction-only
/// geometry (including junction-to-junction wires) is instead resolved through its electrical
/// network: find the electrical group of any endpoint, then the current logical network whose
/// anchors belong to that same group. Geometry with no supported identity resolves to `None`.
fn wire_network(project: &Project, graph: &Graph, electrical: &Connectivity, sheet: SheetTarget, endpoints: [WireEndpoint; 2]) -> Option<usize> {
    for endpoint in endpoints {
        if let Some(anchor) = graph.endpoint_anchor(sheet, endpoint)
            && let Some(index) = graph.network(anchor)
        {
            return Some(index);
        }
    }
    let net = endpoints.into_iter().find_map(|endpoint| electrical.net_index_for_endpoint(endpoint))?;
    graph
        .by_anchor
        .iter()
        .find_map(|(anchor, index)| (electrical.net_index_for_endpoint(anchor_endpoint(project, anchor)?) == Some(net)).then_some(*index))
}

impl Project {
    /// Map every schematic wire with a saved net appearance to its effective override.
    ///
    /// This read builds the electrical graph only when at least one override exists, so ordinary
    /// projects keep the existing paint cost. A single network-index-to-appearance map replaces a
    /// per-wire registry scan. Wires without a supported current identity are omitted; selection
    /// and highlight colors still take precedence when drawing.
    pub fn wire_appearances(&self) -> Result<BTreeMap<String, Appearance>, CommandError> {
        if !self.net_registry.has_appearance() {
            return Ok(BTreeMap::new());
        }
        let graph = Graph::build(self).map_err(CommandError::InvalidParams)?;
        let network_appearance: BTreeMap<usize, Appearance> = self
            .net_registry
            .records
            .iter()
            .filter(|record| matches!(record.state, State::Active {}))
            .filter_map(|record| Some((graph.network(&record.primary)?, record.appearance?)))
            .collect();
        if network_appearance.is_empty() {
            return Ok(BTreeMap::new());
        }
        let electrical = self.electrical_connectivity();
        let mut result = BTreeMap::new();
        for sheet in self.sheets() {
            let own = self.for_sheet(sheet.target).ok_or_else(|| CommandError::NotFound("Unknown appearance sheet.".into()))?;
            for wire in own.wires() {
                let Some(network) = wire_network(self, &graph, &electrical, sheet.target, wire.endpoints()) else { continue };
                if let Some(appearance) = network_appearance.get(&network) {
                    result.insert(wire.id(), *appearance);
                }
            }
        }
        Ok(result)
    }
}

/// Publish the strict patch schema used by the appearance command.
///
/// The object branch intentionally stays unchanged: adding a property inside this `oneOf` is a
/// breaking command-contract change that would require a `COMMAND_API_VERSION` bump. The `clear`
/// marker is documented on the parameter summary and enforced by [`validate_appearance_patch`].
pub(crate) fn appearance_param_schema() -> Value {
    json!({"oneOf":[{"type":"null"}, Appearance::schema()]})
}

/// Validate the command patch contract without promoting a net or editing the project. Accepts a
/// null full reset or an object of optional channels and clear markers; rejects unknown keys,
/// explicit null channels, contradictory markers and out-of-range widths.
pub(crate) fn validate_appearance_patch(value: &Value) -> Result<(), &'static str> {
    Patch::parse(value).map(|_| ()).map_err(|_| "must be null or an appearance patch object with RGB color, bounded width_cp and clear markers")
}
