//! Standard physical quantity identities and their canonical unit metadata.
use serde::{Deserialize, Serialize};

/// Stable physical meanings; serde names are the existing native category identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quantity {
    Resistance,
    Capacitance,
    Inductance,
    Voltage,
    Current,
    Power,
    Frequency,
    Time,
    Length,
    Temperature,
    Ratio,
}

/// Metadata for one built-in quantity. Aliases supplement the exact canonical unit.
/// A copied definition does not mutate the standard registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuantityDefinition {
    /// Stable physical meaning used by fields and serialized category identifiers.
    pub quantity: Quantity,
    /// Exact base-unit identifier accepted in canonical records.
    pub canonical_unit: &'static str,
    /// Unit symbol used in presentation after applying the prefix policy.
    pub display_symbol: &'static str,
    /// Additional input spellings; none may establish a different saved identity.
    pub input_aliases: &'static [&'static str],
}

impl QuantityDefinition {
    /// Accept the canonical unit or an explicitly declared, case-sensitive input alias.
    pub fn accepts(self, suffix: &str) -> bool {
        suffix == self.canonical_unit || self.input_aliases.contains(&suffix)
    }
}

impl Quantity {
    /// All standard quantity identities, independent of library names or field labels.
    pub const ALL: [Self; 11] = [
        Self::Resistance,
        Self::Capacitance,
        Self::Inductance,
        Self::Voltage,
        Self::Current,
        Self::Power,
        Self::Frequency,
        Self::Time,
        Self::Length,
        Self::Temperature,
        Self::Ratio,
    ];

    /// Resolve the shared canonical identity, display symbol and accepted aliases.
    pub fn definition(self) -> QuantityDefinition {
        match self {
            Self::Resistance => {
                QuantityDefinition { quantity: self, canonical_unit: "Ohm", display_symbol: "Ω", input_aliases: &["ohm", "ohms", "Ω", "R"] }
            }
            Self::Capacitance => QuantityDefinition { quantity: self, canonical_unit: "F", display_symbol: "F", input_aliases: &[] },
            Self::Inductance => QuantityDefinition { quantity: self, canonical_unit: "H", display_symbol: "H", input_aliases: &[] },
            Self::Voltage => QuantityDefinition { quantity: self, canonical_unit: "V", display_symbol: "V", input_aliases: &[] },
            Self::Current => QuantityDefinition { quantity: self, canonical_unit: "A", display_symbol: "A", input_aliases: &[] },
            Self::Power => QuantityDefinition { quantity: self, canonical_unit: "W", display_symbol: "W", input_aliases: &[] },
            Self::Frequency => QuantityDefinition { quantity: self, canonical_unit: "Hz", display_symbol: "Hz", input_aliases: &[] },
            Self::Time => QuantityDefinition { quantity: self, canonical_unit: "s", display_symbol: "s", input_aliases: &[] },
            Self::Length => QuantityDefinition { quantity: self, canonical_unit: "m", display_symbol: "m", input_aliases: &[] },
            Self::Temperature => QuantityDefinition { quantity: self, canonical_unit: "K", display_symbol: "K", input_aliases: &[] },
            Self::Ratio => QuantityDefinition { quantity: self, canonical_unit: "1", display_symbol: "1", input_aliases: &[] },
        }
    }

    /// Return the exact stored base-unit identifier, never an alias or display spelling.
    pub fn unit(self) -> &'static str {
        self.definition().canonical_unit
    }
}

/// Enumerate the built-in registry without allowing host code to alter its meanings.
pub fn standard_quantities() -> impl Iterator<Item = QuantityDefinition> {
    Quantity::ALL.into_iter().map(Quantity::definition)
}

/// Resolve only a canonical stored unit; input aliases do not establish saved identity.
pub fn definition_for_unit(unit: &str) -> Option<QuantityDefinition> {
    standard_quantities().find(|definition| definition.canonical_unit == unit)
}

/// Require an exact canonical decimal and a known base-unit identifier.
/// This validation needs no project, category, UI or installed library source.
pub fn validate_quantity(value: &str, unit: &str) -> Result<(), String> {
    if !super::decimal(value) || definition_for_unit(unit).is_none() {
        return Err("Quantity requires a canonical decimal and base-unit identifier.".into());
    }
    Ok(())
}

/// A validated canonical numeric quantity, independent of any native part value record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalQuantity {
    value: String,
    unit: String,
}

impl CanonicalQuantity {
    /// Adopt canonical parts only; aliases, prefixes and noncanonical decimals are refused.
    pub fn new(value: impl Into<String>, unit: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        let unit = unit.into();
        validate_quantity(&value, &unit)?;
        Ok(Self { value, unit })
    }

    /// Exact normalized decimal text, with no display rounding.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Canonical base-unit identifier rather than a display symbol or input alias.
    pub fn unit(&self) -> &str {
        &self.unit
    }

    /// Transfer the exact strings into a host's own typed value or command payload.
    pub fn into_parts(self) -> (String, String) {
        (self.value, self.unit)
    }

    /// Format for display through the same engineering-prefix policy as parsing.
    pub fn display(&self) -> String {
        super::display_quantity(&self.value, &self.unit)
    }
}
