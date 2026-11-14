//! Project-owned dimensional classes. Explicit sets resolve without assigning nets or editing copper.

mod commands;
mod resolve;
#[cfg(test)]
mod tests;

use crate::{Project, board::track};
pub(crate) use commands::{parse_set, rules_schema, set_schema};
pub use resolve::{Conflict, Contribution, PairResolution, Resolution, Source};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Bound authored catalog and request work independently of schematic or board size.
pub const MAX_CLASSES: usize = 256;
/// Create a class with a fresh stable identity.
pub const CREATE: &str = "net_classes.create";
/// Replace a class's name and sparse dimensional rules, retaining its identity.
pub const UPDATE: &str = "net_classes.update";
/// Remove an unbound class; saved net bindings are a later feature.
pub const REMOVE: &str = "net_classes.remove";
/// Inspect the catalog without preparing schematic connectivity.
pub const INSPECT: &str = "net_classes.inspect";
/// Resolve an explicit class set, or symmetric clearance between two supplied sets.
pub const RESOLVE: &str = "net_classes.resolve";

/// A supported dimensional property; every stored and command value is integer nanometres.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Property {
    ClearanceNm,
    MinTrackWidthNm,
    EdgeClearanceNm,
    MinAnnularRingNm,
    MinViaDiameterNm,
    MaxViaDiameterNm,
    MinViaDrillNm,
    MaxViaDrillNm,
    PreferredTrackWidthNm,
    PreferredViaDiameterNm,
    PreferredViaDrillNm,
}

impl Property {
    /// Stable vocabulary order, independent of file order and class display names.
    pub const ALL: [Self; 11] = [
        Self::ClearanceNm,
        Self::MinTrackWidthNm,
        Self::EdgeClearanceNm,
        Self::MinAnnularRingNm,
        Self::MinViaDiameterNm,
        Self::MaxViaDiameterNm,
        Self::MinViaDrillNm,
        Self::MaxViaDrillNm,
        Self::PreferredTrackWidthNm,
        Self::PreferredViaDiameterNm,
        Self::PreferredViaDrillNm,
    ];

    /// Exact JSON field name shared with board-default rules.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClearanceNm => "clearance_nm",
            Self::MinTrackWidthNm => "min_track_width_nm",
            Self::EdgeClearanceNm => "edge_clearance_nm",
            Self::MinAnnularRingNm => "min_annular_ring_nm",
            Self::MinViaDiameterNm => "min_via_diameter_nm",
            Self::MaxViaDiameterNm => "max_via_diameter_nm",
            Self::MinViaDrillNm => "min_via_drill_nm",
            Self::MaxViaDrillNm => "max_via_drill_nm",
            Self::PreferredTrackWidthNm => "preferred_track_width_nm",
            Self::PreferredViaDiameterNm => "preferred_via_diameter_nm",
            Self::PreferredViaDrillNm => "preferred_via_drill_nm",
        }
    }

    /// Whether this property is a nonbinding route-size preference rather than a hard limit.
    pub fn is_preference(self) -> bool {
        matches!(self, Self::PreferredTrackWidthNm | Self::PreferredViaDiameterNm | Self::PreferredViaDrillNm)
    }

    /// Hard maximums intersect by taking the smallest applicable value; other limits take the largest.
    pub fn is_maximum(self) -> bool {
        matches!(self, Self::MaxViaDiameterNm | Self::MaxViaDrillNm)
    }

    /// Supported geometry range, before relationships between dimensions are evaluated.
    pub fn range(self) -> (i64, i64) {
        match self {
            Self::MinTrackWidthNm => (0, track::MAX_WIDTH_NM),
            Self::PreferredTrackWidthNm => (track::MIN_WIDTH_NM, track::MAX_WIDTH_NM),
            Self::MinViaDiameterNm | Self::MaxViaDiameterNm | Self::PreferredViaDiameterNm => {
                (track::MIN_VIA_DRILL_NM + 1, track::MAX_VIA_DIAMETER_NM)
            }
            Self::MinViaDrillNm | Self::MaxViaDrillNm | Self::PreferredViaDrillNm => (track::MIN_VIA_DRILL_NM, track::MAX_VIA_DIAMETER_NM - 1),
            _ => (0, crate::schematic::MAX_CENTER_NM),
        }
    }

    /// Read the corresponding released fallback or addressed board default without JSON conversion.
    pub fn value(self, values: &crate::board::rules::Values) -> i64 {
        match self {
            Self::ClearanceNm => values.clearance_nm,
            Self::MinTrackWidthNm => values.min_track_width_nm,
            Self::EdgeClearanceNm => values.edge_clearance_nm,
            Self::MinAnnularRingNm => values.min_annular_ring_nm,
            Self::MinViaDiameterNm => values.min_via_diameter_nm,
            Self::MaxViaDiameterNm => values.max_via_diameter_nm,
            Self::MinViaDrillNm => values.min_via_drill_nm,
            Self::MaxViaDrillNm => values.max_via_drill_nm,
            Self::PreferredTrackWidthNm => values.preferred_track_width_nm,
            Self::PreferredViaDiameterNm => values.preferred_via_diameter_nm,
            Self::PreferredViaDrillNm => values.preferred_via_drill_nm,
        }
    }
}

/// Sparse authored intent. Omission means no opinion; zero is an explicit supported value.
pub type Rules = BTreeMap<Property, i64>;

/// One independently revisioned rule set, addressed by UUID rather than display name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub id: Uuid,
    pub revision: u32,
    pub name: String,
    #[serde(deserialize_with = "unique_rules")]
    pub rules: Rules,
}

/// Reject repeated dimensional keys before JSON decoding can silently replace authored intent.
fn unique_rules<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Rules, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = Rules;
        /// Describe the sparse dimensional object in malformed-input diagnostics.
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("distinct supported dimensional properties")
        }
        /// Bound input and refuse duplicate properties rather than selecting the last value.
        fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Rules, M::Error> {
            let mut result = Rules::new();
            while let Some(property) = map.next_key::<Property>()? {
                if result.contains_key(&property) {
                    return Err(serde::de::Error::custom("Duplicate class property."));
                }
                result.insert(property, map.next_value()?);
            }
            Ok(result)
        }
    }
    deserializer.deserialize_map(Visitor)
}

/// Versioned project catalog; a project with no classes omits this record entirely.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub format: String,
    pub definitions: Vec<Definition>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self { format: "pcbcraft-net-classes/1".into(), definitions: Vec::new() }
    }
}

/// Refuse unsupported lengths and internally contradictory authored intent, without applying defaults.
pub(crate) fn validate_rules(rules: &Rules) -> Result<(), String> {
    for (&property, &value) in rules {
        let (min, max) = property.range();
        if !(min..=max).contains(&value) {
            return Err(format!("{} must be integer nanometres from {min} to {max}.", property.as_str()));
        }
    }
    if !resolve::relationships(rules).is_empty() {
        return Err("Class dimensions contain contradictory bounds or preferences.".into());
    }
    Ok(())
}

/// Names are presentation only, but blank, untrimmed and control-containing names are invalid.
pub(crate) fn validate_name(name: &str) -> Result<(), String> {
    if name.trim() != name || name.is_empty() || name.chars().count() > 200 || name.chars().any(char::is_control) {
        return Err("Class name must contain 1–200 trimmed characters without control characters.".into());
    }
    Ok(())
}

impl Catalog {
    /// Validate the complete bounded catalog before adoption. Duplicate IDs never overwrite records.
    pub fn validate(&self) -> Result<(), String> {
        if self.format != "pcbcraft-net-classes/1" || self.definitions.len() > MAX_CLASSES {
            return Err("Unsupported net-class format or class limit exceeded.".into());
        }
        let mut ids = BTreeSet::new();
        for class in &self.definitions {
            if class.id.is_nil() || !ids.insert(class.id) || class.revision == 0 {
                return Err("Class IDs must be unique non-nil UUIDs with positive revisions.".into());
            }
            validate_name(&class.name)?;
            validate_rules(&class.rules)?;
        }
        Ok(())
    }

    /// Resolve an exact class identity. Names and vector positions are never lookup authority.
    pub fn get(&self, id: Uuid) -> Option<&Definition> {
        self.definitions.iter().find(|class| class.id == id)
    }
}

impl Project {
    /// Inspect immutable project-level class intent without requiring any board or connectivity graph.
    pub fn net_classes(&self) -> &Catalog {
        &self.net_classes
    }
}
