//! Physical package choices with logical-terminal maps when a symbol is present.
use super::{Reference, codec::terminal_map};
use crate::{board::definition::MAX_PADS, library::MAX_TERMINALS, schematic::SymbolDefinition};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
/// One package option owns an exact footprint and its optional logical-terminal map.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub name: String,
    pub footprint: Reference,
    #[serde(deserialize_with = "terminal_map")]
    pub mapping: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unconnected_pads: Vec<String>,
}
impl Package {
    /// Check syntax and exclusive pad ownership even before its symbol/footprint is available.
    /// The terminal and pad limits are those of symbols and footprints; size errors name the field, limit and observed count.
    pub fn validate(&self) -> Result<(), String> {
        crate::library::check_text("package name", &self.name, 64)?;
        self.footprint.validate()?;
        if self.mapping.len() > MAX_TERMINALS {
            return Err(format!("A package's `mapping` holds at most {MAX_TERMINALS} terminal entries, not {}.", self.mapping.len()));
        }
        if self.unconnected_pads.len() > MAX_PADS {
            return Err(format!("A package's `unconnected_pads` holds at most {MAX_PADS} pads, not {}.", self.unconnected_pads.len()));
        }
        let mut pads = BTreeSet::new();
        for (terminal, numbers) in &self.mapping {
            crate::library::check_pin_number(terminal)?;
            if numbers.is_empty() || numbers.len() > MAX_PADS {
                return Err(format!("A package's `mapping` for terminal {terminal} requires 1–{MAX_PADS} pads, not {}.", numbers.len()));
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
        if pads.len() > MAX_PADS {
            return Err(format!("A package has at most {MAX_PADS} pads, not {}.", pads.len()));
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
