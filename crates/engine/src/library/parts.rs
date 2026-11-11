//! Native parts connect exact symbol/footprint revisions; categories give parameters their meaning.
//! These are library definitions, independent of project instances and supplier offers.
use crate::{board::definition::Source, schematic::SymbolDefinition};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
mod tests;

/// Published generic-part file format.
pub const PART_FORMAT: &str = "pcbcraft-part/1";
/// Published typed category file format.
pub const CATEGORY_FORMAT: &str = "pcbcraft-category/1";

/// An immutable definition address. The containing field supplies the item kind; paths are not identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub library: String,
    pub id: String,
    pub revision: u32,
}
impl Reference {
    /// Reject invalid library/item identities and unversioned references before resolution.
    pub fn validate(&self) -> Result<(), String> {
        super::check_id(&self.library)?;
        super::check_id(&self.id)?;
        if self.revision == 0 {
            return Err("Definition references require a positive revision.".into());
        }
        Ok(())
    }
}

/// Explicit category/Value binding. Omission leaves a symbol or part uncategorized, never inferred.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategoryBinding {
    pub definition: Reference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_value: Option<String>,
}
impl CategoryBinding {
    /// Validate the address and optional namespaced field without fetching its category.
    pub fn validate(&self) -> Result<(), String> {
        self.definition.validate()?;
        if self.primary_value.as_deref().is_some_and(|id| !field_id(id)) {
            return Err("Primary value must name a namespaced category field.".into());
        }
        Ok(())
    }
    /// Require the exact category and a quantity field for the unit-aware Value control.
    pub fn validate_category(&self, category: &Category) -> Result<(), String> {
        self.validate()?;
        category.validate()?;
        if self.definition != category.reference() {
            return Err("Category identity/revision does not match the binding.".into());
        }
        if let Some(id) = &self.primary_value
            && !matches!(category.fields.get(id).map(|field| &field.value_type), Some(FieldType::Quantity { .. }))
        {
            return Err(format!("Primary value {id} must name a quantity field in the category."));
        }
        Ok(())
    }
}

/// Physical quantities use one canonical unit each; display-unit parsing is a separate operation.
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
impl Quantity {
    /// Canonical case-sensitive unit identifiers avoid interpreting display labels or SI prefixes.
    pub fn unit(self) -> &'static str {
        match self {
            Self::Resistance => "Ohm",
            Self::Capacitance => "F",
            Self::Inductance => "H",
            Self::Voltage => "V",
            Self::Current => "A",
            Self::Power => "W",
            Self::Frequency => "Hz",
            Self::Time => "s",
            Self::Length => "m",
            Self::Temperature => "K",
            Self::Ratio => "1",
        }
    }

    /// Normalize unit-aware input into an exact base-unit value. Prefixes are case-sensitive;
    /// omitted units use this quantity, and incompatible units or excess precision are rejected.
    pub fn parse(self, text: &str) -> Result<Value, String> {
        if text.len() > 128 {
            return Err("Quantity input exceeds 128 bytes.".into());
        }
        let text = text.trim();
        let number_end = text.find(|c: char| !c.is_ascii_digit() && !matches!(c, '+' | '-' | '.' | 'e' | 'E')).unwrap_or(text.len());
        let (number, suffix) = text.split_at(number_end);
        let suffix: String = suffix.chars().filter(|c| !c.is_whitespace()).collect();
        let aliases: &[&str] = match self {
            Self::Resistance => &["Ohm", "ohm", "ohms", "Ω", "R"],
            _ => &[self.unit()],
        };
        let shift = if suffix.is_empty() || aliases.contains(&suffix.as_str()) {
            0
        } else if self == Self::Ratio && suffix == "%" {
            -2
        } else {
            let prefixes =
                [("p", -12), ("n", -9), ("u", -6), ("µ", -6), ("μ", -6), ("m", -3), ("k", 3), ("M", 6), ("G", 9), ("T", 12), ("milli", -3)];
            prefixes
                .into_iter()
                .find_map(|(prefix, shift)| suffix.strip_prefix(prefix).filter(|rest| rest.is_empty() || aliases.contains(rest)).map(|_| shift))
                .ok_or_else(|| format!("Expected {} or a compatible SI-prefixed unit.", self.unit()))?
        };
        let value = normalized_decimal(number, shift)?;
        Ok(Value::Quantity { value, unit: self.unit().into() })
    }
}

/// A field's type is pinned with the category, independently of its editable display name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldType {
    Quantity { quantity: Quantity, unit: String },
    Integer { minimum: i64, maximum: i64 },
    Choice { choices: Vec<String> },
    Text {},
    Boolean {},
}
impl FieldType {
    /// Shared field identities retain type/quantity/unit meaning; category-specific bounds and
    /// permitted choices may differ without changing how an existing value is interpreted.
    pub fn same_meaning(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Quantity { quantity, unit }, Self::Quantity { quantity: other_quantity, unit: other_unit }) => {
                quantity == other_quantity && unit == other_unit
            }
            (Self::Integer { .. }, Self::Integer { .. })
            | (Self::Choice { .. }, Self::Choice { .. })
            | (Self::Text {}, Self::Text {})
            | (Self::Boolean {}, Self::Boolean {}) => true,
            _ => false,
        }
    }
}

/// Typed category field; namespaced map keys are its stable identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub name: String,
    pub value_type: FieldType,
}

/// A category defines fields and units, never part-instance values or manufacturer specifications.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub format: String,
    pub library: String,
    pub id: String,
    pub revision: u32,
    pub name: String,
    pub license: String,
    #[serde(deserialize_with = "unique_map")]
    pub fields: BTreeMap<String, Field>,
}
impl Category {
    /// Read a bounded standalone category with strict fields and semantic type/unit validation.
    pub fn read(text: &str) -> Result<Self, String> {
        let category: Self = decode(text)?;
        category.validate()?;
        Ok(category)
    }
    /// Its permanent exact address, independent of where a source stores the document.
    pub fn reference(&self) -> Reference {
        Reference { library: self.library.clone(), id: self.id.clone(), revision: self.revision }
    }
    /// Validate field identity/type, canonical units and bounded, distinct enumeration choices.
    pub fn validate(&self) -> Result<(), String> {
        header(&self.format, CATEGORY_FORMAT, &self.reference(), &self.name, &self.license)?;
        if self.fields.is_empty() || self.fields.len() > 64 {
            return Err("A category requires 1–64 fields.".into());
        }
        for (id, field) in &self.fields {
            if !field_id(id) {
                return Err(format!("Invalid namespaced category field {id:?}."));
            }
            super::check_text("field name", &field.name, 64)?;
            match &field.value_type {
                FieldType::Quantity { quantity, unit } if unit != quantity.unit() => {
                    return Err(format!("Field {id} requires canonical unit {}.", quantity.unit()));
                }
                FieldType::Choice { choices } => {
                    if choices.is_empty() || choices.len() > 64 || choices.iter().collect::<BTreeSet<_>>().len() != choices.len() {
                        return Err(format!("Field {id} requires 1–64 distinct choices."));
                    }
                    for choice in choices {
                        super::check_text("choice", choice, 64)?;
                    }
                }
                FieldType::Integer { minimum, maximum }
                    if minimum > maximum || minimum.unsigned_abs() > 9_007_199_254_740_991 || maximum.unsigned_abs() > 9_007_199_254_740_991 =>
                {
                    return Err(format!("Field {id} requires ordered, JSON-safe integer bounds."));
                }
                _ => {}
            }
        }
        Ok(())
    }
    /// Serialize validated content deterministically, with sorted field identities and no caches.
    pub fn encode(&self) -> Result<String, String> {
        self.validate()?;
        encode(self)
    }
}

/// Known fixed requirements of a generic part. Unknown/missing specifications are not fixed values.
/// Decimal strings are canonical base-unit values, preserving precision across JSON consumers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Value {
    Quantity { value: String, unit: String },
    Integer { value: i64 },
    Choice { value: String },
    Text { value: String },
    Boolean { value: bool },
}
impl Value {
    /// Bound values before category resolution; quantities use plain canonical decimal notation.
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Quantity { value, unit } => {
                if !decimal(value) || !["Ohm", "F", "H", "V", "A", "W", "Hz", "s", "m", "K", "1"].contains(&unit.as_str()) {
                    return Err("Quantity requires a canonical decimal and base-unit identifier.".into());
                }
            }
            Self::Choice { value } => super::check_text("choice value", value, 64)?,
            Self::Integer { value } if value.unsigned_abs() > 9_007_199_254_740_991 => {
                return Err("Integer must be exactly representable by JSON consumers.".into());
            }
            Self::Text { value } => super::check_text("text value", value, 128)?,
            Self::Boolean { .. } | Self::Integer { .. } => {}
        }
        Ok(())
    }
    /// Check meaning against the pinned category, never the field's display label.
    pub fn validate_field(&self, field: &Field) -> Result<(), String> {
        self.validate()?;
        let valid = match (self, &field.value_type) {
            (Self::Quantity { unit, .. }, FieldType::Quantity { unit: expected, .. }) => unit == expected,
            (Self::Choice { value }, FieldType::Choice { choices }) => choices.contains(value),
            (Self::Integer { value }, FieldType::Integer { minimum, maximum }) => (*minimum..=*maximum).contains(value),
            (Self::Text { .. }, FieldType::Text {}) | (Self::Boolean { .. }, FieldType::Boolean {}) => true,
            _ => false,
        };
        if valid { Ok(()) } else { Err("Value type, unit or choice does not match its category field.".into()) }
    }
}

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
        super::check_text("package name", &self.name, 64)?;
        self.footprint.validate()?;
        if self.mapping.is_empty() || self.mapping.len() > 128 || self.unconnected_pads.len() > 128 {
            return Err("A package requires a bounded terminal mapping.".into());
        }
        let mut pads = BTreeSet::new();
        for (terminal, numbers) in &self.mapping {
            super::check_pin_number(terminal)?;
            if numbers.is_empty() || numbers.len() > 128 {
                return Err(format!("Terminal {terminal} requires 1–128 pads."));
            }
            for pad in numbers {
                super::check_pin_number(pad)?;
                if !pads.insert(pad) {
                    return Err(format!("Pad {pad} is mapped more than once."));
                }
            }
        }
        for pad in &self.unconnected_pads {
            super::check_pin_number(pad)?;
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
    pub symbol: Reference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<CategoryBinding>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", deserialize_with = "unique_map")]
    pub fixed_parameters: BTreeMap<String, Value>,
    #[serde(deserialize_with = "unique_map")]
    pub packages: BTreeMap<String, Package>,
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
    /// Validate closed records and local invariants; empty package/category choices remain valid.
    pub fn validate(&self) -> Result<(), String> {
        header(&self.format, PART_FORMAT, &self.reference(), &self.name, &self.license)?;
        self.symbol.validate()?;
        for (name, value, limit) in [
            ("source kind", &self.source.kind, 64),
            ("source reference", &self.source.reference, 2048),
            ("source locator", &self.source.locator, 1024),
        ] {
            super::check_text(name, value, limit)?;
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
            value.validate()?;
        }
        for (id, package) in &self.packages {
            super::check_id(id)?;
            package.validate()?;
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

/// Field IDs are namespace:item, with portable independently validated segments.
pub fn field_id(id: &str) -> bool {
    id.split_once(':').is_some_and(|(namespace, field)| super::check_id(namespace).is_ok() && super::check_id(field).is_ok())
}

/// Canonical decimals have at most 36 digits and 18 fractional places; zero has exactly one spelling.
pub fn decimal(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    let (whole, fraction) = digits.split_once('.').map_or((digits, None), |(a, b)| (a, Some(b)));
    !whole.is_empty()
        && (whole == "0" || !whole.starts_with('0'))
        && whole.bytes().all(|b| b.is_ascii_digit())
        && fraction.is_none_or(|f| !f.is_empty() && f.len() <= 18 && !f.ends_with('0') && f.bytes().all(|b| b.is_ascii_digit()))
        && whole.len() + fraction.map_or(0, str::len) <= 36
        && value != "-0"
}

/// Shift decimal text by a power of ten without binary floating-point conversion or rounding.
fn normalized_decimal(number: &str, shift: i32) -> Result<String, String> {
    let (mantissa, exponent) = match number.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<i32>().map_err(|_| "Invalid decimal exponent.")?),
        None => (number, 0),
    };
    if !(-48..=48).contains(&exponent) {
        return Err("Decimal exponent is outside supported bounds.".into());
    }
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.strip_prefix(['-', '+']).unwrap_or(mantissa);
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if (whole.is_empty() && fraction.is_empty()) || !whole.bytes().chain(fraction.bytes()).all(|b| b.is_ascii_digit()) {
        return Err("Invalid decimal quantity.".into());
    }
    let digits = format!("{whole}{fraction}");
    let mut digits = digits.trim_start_matches('0').to_owned();
    if digits.is_empty() {
        return Ok("0".into());
    }
    let mut scale = exponent + shift - i32::try_from(fraction.len()).map_err(|_| "Quantity precision overflow.")?;
    while digits.ends_with('0') {
        digits.pop();
        scale += 1;
    }
    let value = if scale >= 0 {
        format!("{digits}{}", "0".repeat(scale as usize))
    } else {
        let split = digits.len() as i32 + scale;
        if split > 0 {
            let (a, b) = digits.split_at(split as usize);
            format!("{a}.{b}")
        } else {
            format!("0.{}{digits}", "0".repeat((-split) as usize))
        }
    };
    let value = if negative { format!("-{value}") } else { value };
    if decimal(&value) { Ok(value) } else { Err("Quantity exceeds 36 decimal digits or 18 fractional places; no rounding was applied.".into()) }
}

/// Validate a shared native header before any semantic reference resolution.
fn header(format: &str, expected: &str, reference: &Reference, name: &str, license: &str) -> Result<(), String> {
    if format != expected {
        return Err(format!("Unsupported native format {format:?}; expected {expected}."));
    }
    reference.validate()?;
    super::check_text("name", name, 64)?;
    super::check_text("license", license, 128)
}

/// All native record parsers share source-loader bounds even when called outside a loaded source.
fn decode<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, String> {
    if text.len() > super::sources::MAX_FILE_BYTES {
        return Err("Native library record exceeds 1 MiB.".into());
    }
    serde_json::from_str(text).map_err(|error| error.to_string())
}

/// Stable pretty JSON with a final newline; validation is performed by the record's encoder.
fn encode<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string_pretty(value).map(|text| text + "\n").map_err(|error| error.to_string())
}

/// Reject duplicate authored map identities rather than allowing the last JSON entry to win.
fn unique_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Visitor<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> {
        type Value = BTreeMap<String, T>;
        /// Describe identity-keyed native maps in malformed-input diagnostics.
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a map of distinct native identities")
        }
        /// Bound map allocation and reject repeated keys before adopting a record.
        fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
            let mut result = BTreeMap::new();
            while let Some(key) = map.next_key::<String>()? {
                if result.len() >= 128 || result.contains_key(&key) {
                    return Err(serde::de::Error::custom("Too many or duplicate native record identities."));
                }
                result.insert(key, map.next_value()?);
            }
            Ok(result)
        }
    }
    deserializer.deserialize_map(Visitor(std::marker::PhantomData))
}
