//! Reusable generic parts link schematic, category and physical package definitions.
use super::{
    CURRENT_PART_FORMAT, Category, CategoryBinding, LEGACY_PART_FORMAT, MAX_DESCRIPTION_CHARS, MAX_KEYWORD_CHARS, MAX_KEYWORDS, MAX_LINK_LABEL_CHARS,
    MAX_LINK_URL_CHARS, MAX_LINKS, MAX_TERMINAL_RATINGS, METADATA_PART_FORMAT, PART_FORMAT, Package, RATED_PART_FORMAT, Reference, SupplyRange,
    Value,
    codec::{decode, encode, header, unique_map},
    field_id,
};
use crate::{
    board::definition::Source,
    schematic::{ElectricalType, SymbolDefinition, wiring::PinNumber},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
/// A bounded human-facing reference. Labels and URLs are descriptive metadata, never fetched on load.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartLink {
    /// Display label, 1 to [`MAX_LINK_LABEL_CHARS`] characters, stored trimmed.
    pub label: String,
    /// HTTP(S) reference, 1 to [`MAX_LINK_URL_CHARS`] characters, never fetched on load.
    pub url: String,
}

impl PartLink {
    /// Validate a trimmed link URL before it is stored. No URL is fetched.
    pub fn validate_url(url: &str) -> Result<(), String> {
        crate::library::check_http_url("Link URL", url, MAX_LINK_URL_CHARS)
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
    /// One-line human summary, 1 to [`MAX_DESCRIPTION_CHARS`] characters. Explicit null is refused.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "optional_description")]
    pub description: Option<String>,
    /// Short searchable tags, stored trimmed and unique by case-insensitive text.
    #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "keyword_list")]
    pub keywords: Vec<String>,
    /// Bounded reference links; their labels and URLs are never treated as search keywords.
    #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "link_list")]
    pub links: Vec<PartLink>,
}
impl PartDefinition {
    /// Parse an actual native part file; unresolved dependencies are checked separately by the catalog.
    pub fn read(text: &str) -> Result<Self, String> {
        Self::validate_decoded(decode(text)?)
    }

    /// Reopen a pinned project record under the same document cap as the canonical writer.
    /// The separate external source parser still applies its stricter file limit.
    pub(crate) fn read_saved(text: &str) -> Result<Self, String> {
        if text.len() > crate::persistence::MAX_DOCUMENT_BYTES {
            return Err("Pinned dependency exceeds its native document limit.".into());
        }
        Self::validate_decoded(serde_json::from_str(text).map_err(|error| error.to_string())?)
    }

    /// Both source and project inputs must pass the same semantic checks.
    fn validate_decoded(part: Self) -> Result<Self, String> {
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

    /// Lowercase search text made only from the authored description and keywords. Link labels
    /// and URLs are deliberately excluded: they are references, not searchable part metadata.
    /// Shared by catalog search and presentation surfaces so both match the same content.
    pub fn search_text(&self) -> String {
        let mut text = self.description.as_deref().unwrap_or_default().to_lowercase();
        for keyword in &self.keywords {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&keyword.to_lowercase());
        }
        text
    }

    /// Whether this definition carries descriptive metadata.
    fn has_metadata(&self) -> bool {
        self.description.is_some() || !self.keywords.is_empty() || !self.links.is_empty()
    }

    /// Check field-level bounds of the optional metadata, independent of the declared format.
    /// Drafts may be otherwise incomplete, but no metadata collection may exceed its limit or
    /// carry a duplicate identity (compared after trimming and case folding).
    pub(crate) fn validate_metadata(&self) -> Result<(), String> {
        if let Some(description) = &self.description {
            crate::library::check_text("description", description, MAX_DESCRIPTION_CHARS)?;
        }
        validate_keywords(&self.keywords)?;
        validate_links(&self.links)
    }

    /// Change one range in a draft. Parse both bounds before mutation; two blanks remove
    /// the rating. The normal publication validator checks the exact symbol dependency.
    pub fn set_supply_range(&mut self, terminal: &str, min: &str, max: &str) -> Result<(), String> {
        if PinNumber::new(terminal).is_none() {
            return Err(format!("Invalid supply range terminal ID {terminal:?}."));
        }
        let range = SupplyRange::parse(terminal, min, max)?;
        if self.terminal_ratings.get(terminal) == range.as_ref() {
            return Ok(());
        }
        if let Some(range) = range {
            if !self.terminal_ratings.contains_key(terminal) && self.terminal_ratings.len() >= MAX_TERMINAL_RATINGS {
                return Err(format!("A part supports at most {MAX_TERMINAL_RATINGS} terminal ratings."));
            }
            self.terminal_ratings.insert(terminal.into(), range);
        } else {
            self.terminal_ratings.remove(terminal);
        }
        self.format = CURRENT_PART_FORMAT.into();
        Ok(())
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

    /// Recognize a generic part format without validating the record's contents.
    pub(crate) fn supported_format(format: &str) -> Option<&'static str> {
        match format {
            LEGACY_PART_FORMAT => Some(LEGACY_PART_FORMAT),
            PART_FORMAT => Some(PART_FORMAT),
            RATED_PART_FORMAT => Some(RATED_PART_FORMAT),
            METADATA_PART_FORMAT => Some(METADATA_PART_FORMAT),
            CURRENT_PART_FORMAT => Some(CURRENT_PART_FORMAT),
            _ => None,
        }
    }

    /// Check the native format and its symbol, metadata and rating requirements without
    /// requiring publication identity, resolving references or inspecting package maps.
    pub(crate) fn validate_format(&self) -> Result<(), String> {
        if Self::supported_format(&self.format).is_none() {
            return Err(format!("Unsupported part format {:?}.", self.format));
        }
        let rated = !self.terminal_ratings.is_empty();
        let metadata = self.has_metadata();
        match self.format.as_str() {
            METADATA_PART_FORMAT if !metadata => return Err("pcbcraft-part/5 requires a description, keyword or link.".into()),
            LEGACY_PART_FORMAT | PART_FORMAT | RATED_PART_FORMAT if metadata => {
                return Err("Part description, keywords and links require pcbcraft-part/5 or pcbcraft-part/6.".into());
            }
            LEGACY_PART_FORMAT | PART_FORMAT if rated => {
                return Err("Terminal ratings require pcbcraft-part/4, pcbcraft-part/5 or pcbcraft-part/6.".into());
            }
            RATED_PART_FORMAT if !rated => return Err("pcbcraft-part/4 requires at least one terminal rating.".into()),
            _ => {}
        }
        self.validate_metadata()?;
        if self.symbol.is_none()
            && let Some(terminal) = self.terminal_ratings.keys().next()
        {
            return Err(format!("Supply range for terminal {terminal} requires an exact symbol."));
        }
        self.validate_rating_values()?;
        if self.format == LEGACY_PART_FORMAT && self.symbol.is_none() {
            return Err("Format 1 parts require a symbol; use pcbcraft-part/2 for optional drawings.".into());
        }
        Ok(())
    }

    /// Validate closed records and local invariants; empty package/category choices remain valid.
    pub fn validate(&self) -> Result<(), String> {
        let expected = Self::supported_format(&self.format).unwrap_or(PART_FORMAT);
        header(&self.format, expected, &self.reference(), &self.name, &self.license)?;
        self.validate_format()?;
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

/// An omitted description has no summary; an explicit null is not a description.
fn optional_description<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}

/// Validate the stored form of a keyword list: bounded count, canonical trimmed text and
/// identities unique after case folding. An empty list is the omitted default, not an error here.
fn validate_keywords(items: &[String]) -> Result<(), String> {
    if items.len() > MAX_KEYWORDS {
        return Err(format!("A part supports at most {MAX_KEYWORDS} keywords, not {}.", items.len()));
    }
    let mut seen = BTreeSet::new();
    for keyword in items {
        crate::library::check_text("keyword", keyword, MAX_KEYWORD_CHARS)?;
        if !seen.insert(keyword.to_lowercase()) {
            return Err(format!("Duplicate keyword {keyword:?} after trimming and case folding."));
        }
    }
    Ok(())
}

/// Validate stored links: bounded count, unique trimmed labels and fetched-free HTTP(S) URLs.
fn validate_links(links: &[PartLink]) -> Result<(), String> {
    if links.len() > MAX_LINKS {
        return Err(format!("A part supports at most {MAX_LINKS} links, not {}.", links.len()));
    }
    let mut seen = BTreeSet::new();
    for link in links {
        crate::library::check_text("link label", &link.label, MAX_LINK_LABEL_CHARS)?;
        if !seen.insert(link.label.to_lowercase()) {
            return Err(format!("Duplicate link label {:?} after trimming and case folding.", link.label));
        }
        PartLink::validate_url(&link.url)?;
    }
    Ok(())
}

/// Read keywords into their canonical stored form: trimmed, nonempty and case-insensitively unique.
/// Explicit null and an explicitly empty list are refused rather than silently defaulted.
fn keyword_list<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    let raw = Vec::<String>::deserialize(deserializer)?;
    if raw.is_empty() {
        return Err(serde::de::Error::custom("keywords must be omitted when empty"));
    }
    let trimmed: Vec<String> = raw.iter().map(|keyword| keyword.trim().to_owned()).collect();
    validate_keywords(&trimmed).map_err(serde::de::Error::custom)?;
    Ok(trimmed)
}

/// Read links into their canonical stored form: trimmed labels, unique identities and safe URLs.
/// Explicit null and an explicitly empty list are refused rather than silently defaulted.
fn link_list<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<PartLink>, D::Error> {
    let raw = Vec::<PartLink>::deserialize(deserializer)?;
    if raw.is_empty() {
        return Err(serde::de::Error::custom("links must be omitted when empty"));
    }
    let normalized: Vec<PartLink> = raw.into_iter().map(|link| PartLink { label: link.label.trim().to_owned(), url: link.url }).collect();
    validate_links(&normalized).map_err(serde::de::Error::custom)?;
    Ok(normalized)
}
