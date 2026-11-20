//! Reusable generic parts link schematic, category and physical package definitions.
use super::{
    Category, CategoryBinding, LEGACY_PART_FORMAT, MAX_TERMINAL_RATINGS, PART_FORMAT, Package, RATED_PART_FORMAT, Reference, SupplyRange, Value,
    codec::{decode, encode, header, unique_map},
    field_id,
};
use crate::{
    board::definition::Source,
    schematic::{ElectricalType, SymbolDefinition, wiring::PinNumber},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
/// A reusable generic part contains no drawings, manufacturer identity, stock or prices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartDefinition {
    pub format: String,
    pub library: String,
    pub id: String,
    pub revision: u32,
    pub name: String,
    pub license: String,
    pub source: Source,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "optional_symbol")]
    pub symbol: Option<Reference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<CategoryBinding>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", deserialize_with = "unique_map")]
    pub fixed_parameters: BTreeMap<String, Value>,
    #[serde(deserialize_with = "unique_map")]
    pub packages: BTreeMap<String, Package>,
    /// Device limits keyed by the exact symbol's logical terminal ID, never its display name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", deserialize_with = "rating_map")]
    pub terminal_ratings: BTreeMap<String, SupplyRange>,
}
impl PartDefinition {
    /// Parse an actual native part file; unresolved dependencies are checked separately by the catalog.
    pub fn read(text: &str) -> Result<Self, String> {
        let part: Self = decode(text)?;
        part.validate()?;
        Ok(part)
    }
    /// Its immutable address, independent of source registrations and paths.
    pub fn reference(&self) -> Reference {
        Reference { library: self.library.clone(), id: self.id.clone(), revision: self.revision }
    }
    /// The authored range on one logical terminal, or `None` when both bounds are unknown.
    pub fn supply_range(&self, terminal: &str) -> Option<&SupplyRange> {
        self.terminal_ratings.get(terminal)
    }

    /// Change one range in a draft. Parse both bounds before mutation; two blanks remove
    /// the rating. The normal publication validator checks the exact symbol dependency.
    pub fn set_supply_range(&mut self, terminal: &str, min: &str, max: &str) -> Result<(), String> {
        if PinNumber::new(terminal).is_none() {
            return Err(format!("Invalid supply range terminal ID {terminal:?}."));
        }
        if let Some(range) = SupplyRange::parse(terminal, min, max)? {
            if !self.terminal_ratings.contains_key(terminal) && self.terminal_ratings.len() >= MAX_TERMINAL_RATINGS {
                return Err(format!("A part supports at most {MAX_TERMINAL_RATINGS} terminal ratings."));
            }
            self.terminal_ratings.insert(terminal.into(), range);
        } else {
            self.terminal_ratings.remove(terminal);
        }
        self.select_rating_format();
        Ok(())
    }

    /// Select the current rated envelope for an authored map. Otherwise retain the
    /// original unrated format, or use /2 after the last rating was removed from /4.
    /// Only draft transitions use this; native readers always enforce the declared header.
    pub(crate) fn select_rating_format(&mut self) {
        match self.format.as_str() {
            LEGACY_PART_FORMAT | PART_FORMAT if !self.terminal_ratings.is_empty() => self.format = RATED_PART_FORMAT.into(),
            RATED_PART_FORMAT if self.terminal_ratings.is_empty() => self.format = PART_FORMAT.into(),
            _ => {}
        }
    }

    /// Check bounds and map size without resolving the symbol or requiring other draft fields.
    pub(crate) fn validate_rating_values(&self) -> Result<(), String> {
        if self.terminal_ratings.len() > MAX_TERMINAL_RATINGS {
            return Err(format!("A part supports at most {MAX_TERMINAL_RATINGS} terminal ratings."));
        }
        for (terminal, range) in &self.terminal_ratings {
            if PinNumber::new(terminal).is_none() {
                return Err(format!("Invalid supply range terminal ID {terminal:?}."));
            }
            range.validate(terminal)?;
        }
        Ok(())
    }

    /// Validate ratings against the resolved exact logical terminal table, independent of
    /// packages and graphical units. Callers supply the symbol resolved for this definition.
    pub(crate) fn validate_rating_terminals(&self, symbol: Option<&SymbolDefinition>) -> Result<(), String> {
        if self.terminal_ratings.is_empty() {
            return Ok(());
        }
        let symbol = symbol.ok_or("Terminal ratings require an exact symbol.")?;
        let mut found = 0;
        for pin in symbol.pins() {
            if self.terminal_ratings.contains_key(&pin.number) {
                if pin.electrical_type != ElectricalType::PowerIn {
                    return Err(format!("Terminal {} must be power_in to carry a supply range.", pin.number));
                }
                found += 1;
            }
        }
        if found != self.terminal_ratings.len()
            && let Some(terminal) = self.terminal_ratings.keys().find(|id| symbol.pin_index(id).is_none())
        {
            return Err(format!("Unknown supply range terminal {terminal} in the exact symbol."));
        }
        Ok(())
    }

    /// Validate closed records and local invariants; empty package/category choices remain valid.
    pub fn validate(&self) -> Result<(), String> {
        let expected = match self.format.as_str() {
            LEGACY_PART_FORMAT => LEGACY_PART_FORMAT,
            RATED_PART_FORMAT => RATED_PART_FORMAT,
            _ => PART_FORMAT,
        };
        header(&self.format, expected, &self.reference(), &self.name, &self.license)?;
        if (self.format == RATED_PART_FORMAT) == self.terminal_ratings.is_empty() {
            return Err("Only pcbcraft-part/4 carries terminal_ratings, and it requires at least one entry.".into());
        }
        if self.symbol.is_none()
            && let Some(terminal) = self.terminal_ratings.keys().next()
        {
            return Err(format!("Supply range for terminal {terminal} requires an exact symbol."));
        }
        self.validate_rating_values()?;
        if self.format == LEGACY_PART_FORMAT && self.symbol.is_none() {
            return Err("Format 1 parts require a symbol; use pcbcraft-part/2 for optional drawings.".into());
        }
        if let Some(symbol) = &self.symbol {
            symbol.validate()?;
        }
        for (name, value, limit) in [
            ("source kind", &self.source.kind, 64),
            ("source reference", &self.source.reference, 2048),
            ("source locator", &self.source.locator, 1024),
        ] {
            crate::library::check_text(name, value, limit)?;
        }
        if self.packages.len() > 64 || self.fixed_parameters.len() > 64 {
            return Err("A part supports at most 64 packages and 64 fixed parameters.".into());
        }
        if let Some(category) = &self.category {
            category.validate()?;
        } else if !self.fixed_parameters.is_empty() {
            return Err("Fixed parameters require an explicit category binding.".into());
        }
        for (id, value) in &self.fixed_parameters {
            if !field_id(id) {
                return Err(format!("Invalid namespaced parameter {id:?}."));
            }
            value.validate_entry(id)?;
        }
        for (id, package) in &self.packages {
            crate::library::check_id(id)?;
            package.validate()?;
            if self.symbol.is_none() && (!package.mapping.is_empty() || !package.unconnected_pads.is_empty()) {
                return Err("A part without a symbol has no terminal map; leave mappings and unconnected pads empty.".into());
            }
            if self.symbol.is_some() && package.mapping.is_empty() {
                return Err("A package with a symbol requires a terminal mapping.".into());
            }
        }
        Ok(())
    }
    /// Resolve fixed requirements against one exact category; never treat untyped values as quantities.
    pub fn validate_category(&self, category: Option<&Category>) -> Result<(), String> {
        self.validate()?;
        match (&self.category, category) {
            (Some(binding), Some(category)) => {
                binding.validate_category(category)?;
                for (id, value) in &self.fixed_parameters {
                    let field = category.fields.get(id).ok_or_else(|| format!("Unknown category field {id}."))?;
                    value.validate_field(field).map_err(|e| format!("{id}: {e}"))?;
                }
                Ok(())
            }
            (None, None) => Ok(()),
            _ => Err("Part category is missing or was supplied without a binding.".into()),
        }
    }
    /// Encode a validated standalone record with stable map order and no runtime/source cache data.
    pub fn encode(&self) -> Result<String, String> {
        self.validate()?;
        encode(self)
    }
}

/// When present the rating map must have content, cannot repeat a terminal key, and enforces
/// [`MAX_TERMINAL_RATINGS`].
fn rating_map<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<BTreeMap<String, SupplyRange>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = BTreeMap<String, SupplyRange>;
        /// Describe the terminal rating map in malformed-input diagnostics.
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a nonempty map of distinct terminal supply ranges")
        }
        /// Bound allocation to [`MAX_TERMINAL_RATINGS`] and reject repeated keys.
        fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
            let mut ratings = BTreeMap::new();
            while let Some(key) = map.next_key::<String>()? {
                if ratings.contains_key(&key) {
                    return Err(serde::de::Error::custom("Duplicate terminal_ratings key."));
                }
                if ratings.len() >= MAX_TERMINAL_RATINGS {
                    return Err(serde::de::Error::custom(format!("A part supports at most {MAX_TERMINAL_RATINGS} terminal ratings.")));
                }
                ratings.insert(key, map.next_value()?);
            }
            if ratings.is_empty() {
                return Err(serde::de::Error::custom("terminal_ratings must be omitted when empty"));
            }
            Ok(ratings)
        }
    }
    deserializer.deserialize_map(Visitor)
}

/// An absent association is optional; explicit null is not an exact reference.
fn optional_symbol<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Reference>, D::Error> {
    Reference::deserialize(deserializer).map(Some)
}
