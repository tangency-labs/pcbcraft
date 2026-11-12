//! Shared terminal-map vocabulary preserves legacy one-pad strings and adds explicit multi-pad lists.
use crate::{board::definition::FootprintDefinition, schematic::SymbolDefinition};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One or more physical lands for one logical terminal; single-pad records keep their old bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PadIds {
    One(String),
    Many(Vec<String>),
}
impl PadIds {
    /// Pad identity spelling is preserved, including leading zeros and alphanumeric names.
    pub fn pads(&self) -> &[String] {
        match self {
            Self::One(value) => std::slice::from_ref(value),
            Self::Many(values) => values,
        }
    }
    /// Bound and validate the list before accepting it into a project or shared command.
    pub fn valid(&self) -> bool {
        let pads = self.pads();
        !pads.is_empty()
            && pads.len() <= 128
            && pads.iter().all(|pad| crate::schematic::PinNumber::new(pad).is_some())
            && pads.iter().collect::<std::collections::BTreeSet<_>>().len() == pads.len()
    }
    /// Cross-probing may choose one representative pad; electrical graphs still visit every land.
    pub fn first(&self) -> Option<&str> {
        self.pads().first().map(String::as_str)
    }
    /// Reverse correspondence uses explicit membership, never numeric equivalence or geometry.
    pub fn contains(&self, pad: &str) -> bool {
        self.pads().iter().any(|number| number == pad)
    }
}
impl From<String> for PadIds {
    fn from(value: String) -> Self {
        Self::One(value)
    }
}
impl From<&str> for PadIds {
    fn from(value: &str) -> Self {
        Self::One(value.into())
    }
}
impl From<Vec<String>> for PadIds {
    fn from(mut values: Vec<String>) -> Self {
        if values.len() == 1 { Self::One(values.remove(0)) } else { Self::Many(values) }
    }
}
impl PartialEq<&str> for PadIds {
    fn eq(&self, value: &&str) -> bool {
        self.pads().len() == 1 && self.first() == Some(*value)
    }
}
impl std::fmt::Display for PadIds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.pads().join(", "))
    }
}

/// Resolve a physical assignment through the same complete-map validator as a reusable package.
pub(crate) fn validate(
    mapping: &BTreeMap<String, PadIds>,
    unconnected: &[String],
    symbol: &SymbolDefinition,
    footprint: &FootprintDefinition,
) -> Result<(), String> {
    super::Package {
        name: "Assigned package".into(),
        footprint: super::Reference { library: footprint.library.clone(), id: footprint.id.clone(), revision: footprint.revision },
        mapping: mapping.iter().map(|(terminal, pads)| (terminal.clone(), pads.pads().to_vec())).collect(),
        unconnected_pads: unconnected.to_vec(),
    }
    .validate_definitions(symbol, footprint)
}

/// New physical mapping semantics require an explicit newer board or baseline format.
pub(crate) fn extended(mapping: &BTreeMap<String, PadIds>, unconnected: &[String]) -> bool {
    !unconnected.is_empty() || mapping.values().any(|pads| matches!(pads, PadIds::Many(_)))
}
