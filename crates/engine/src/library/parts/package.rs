//! Physical package choices and complete logical-terminal to pad mappings.
use super::{Reference, codec::unique_map};
use crate::schematic::SymbolDefinition;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
/// One package option owns the exact footprint and complete terminal-to-pads mapping.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub name: String,
    pub footprint: Reference,
    #[serde(deserialize_with = "unique_map")]
    pub mapping: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unconnected_pads: Vec<String>,
}
impl Package {
    /// Check syntax and exclusive pad ownership even before its symbol/footprint is available.
    pub fn validate(&self) -> Result<(), String> {
        crate::library::check_text("package name", &self.name, 64)?;
        self.footprint.validate()?;
        if self.mapping.is_empty() || self.mapping.len() > 128 || self.unconnected_pads.len() > 128 {
            return Err("A package requires a bounded terminal mapping.".into());
        }
        let mut pads = BTreeSet::new();
        for (terminal, numbers) in &self.mapping {
            crate::library::check_pin_number(terminal)?;
            if numbers.is_empty() || numbers.len() > 128 {
                return Err(format!("Terminal {terminal} requires 1–128 pads."));
            }
            for pad in numbers {
                crate::library::check_pin_number(pad)?;
                if !pads.insert(pad) {
                    return Err(format!("Pad {pad} is mapped more than once."));
                }
            }
        }
        for pad in &self.unconnected_pads {
            crate::library::check_pin_number(pad)?;
            if !pads.insert(pad) {
                return Err(format!("Pad {pad} cannot be both mapped and unconnected, or repeated."));
            }
        }
        if pads.len() > 128 {
            return Err("A package supports at most 128 pads in this format.".into());
        }
        Ok(())
    }
    /// Require the exact footprint, every logical terminal and every physical pad, without inference.
    pub fn validate_definitions(&self, symbol: &SymbolDefinition, footprint: &crate::board::definition::FootprintDefinition) -> Result<(), String> {
        self.validate()?;
        if self.footprint != (Reference { library: footprint.library.clone(), id: footprint.id.clone(), revision: footprint.revision }) {
            return Err("Package footprint identity/revision does not match.".into());
        }
        let terminals: BTreeSet<_> = symbol.pins.iter().map(|pin| &pin.number).collect();
        if terminals != self.mapping.keys().collect() {
            return Err("Package must map every symbol terminal exactly once.".into());
        }
        let assigned: BTreeSet<_> = self.mapping.values().flatten().chain(&self.unconnected_pads).collect();
        if assigned != footprint.pads.iter().map(|pad| &pad.number).collect() {
            return Err("Package must account for every footprint pad, mapped or explicitly unconnected.".into());
        }
        Ok(())
    }
}
