//! Orderable manufacturer records share native libraries with generic part definitions.
use super::{
    Reference, Value,
    codec::{decode, encode, header, unique_map},
};
use crate::{board::definition::Source, library::sources::ResolvedPart};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Versioned manufacturer metadata and typed specifications, independent of supplier services.
pub const MANUFACTURER_PART_FORMAT: &str = "pcbcraft-manufacturer-part/1";

/// One manufacturer/MPN identifies one reusable part and physical package variant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManufacturerPart {
    pub format: String,
    pub library: String,
    pub id: String,
    pub revision: u32,
    pub name: String,
    pub license: String,
    pub source: Source,
    pub manufacturer: String,
    pub mpn: String,
    pub part: Reference,
    pub package: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datasheet: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", deserialize_with = "unique_map")]
    pub specifications: BTreeMap<String, Value>,
}
impl ManufacturerPart {
    /// Read a bounded original record without fetching the datasheet or resolving source paths.
    pub fn read(text: &str) -> Result<Self, String> {
        let value: Self = decode(text)?;
        value.validate()?;
        Ok(value)
    }
    /// Exact immutable identity survives source removal and vendor-adapter changes.
    pub fn reference(&self) -> Reference {
        Reference { library: self.library.clone(), id: self.id.clone(), revision: self.revision }
    }
    /// Validate ownership and canonical specification values before dependency resolution.
    pub fn validate(&self) -> Result<(), String> {
        header(&self.format, MANUFACTURER_PART_FORMAT, &self.reference(), &self.name, &self.license)?;
        self.part.validate()?;
        crate::library::check_id(&self.package)?;
        for (name, text, limit) in [
            ("manufacturer", &self.manufacturer, 128),
            ("MPN", &self.mpn, 128),
            ("source kind", &self.source.kind, 64),
            ("source reference", &self.source.reference, 2048),
            ("source locator", &self.source.locator, 1024),
        ] {
            crate::library::check_text(name, text, limit)?;
        }
        if let Some(url) = &self.datasheet {
            crate::library::check_text("datasheet", url, 2048)?;
            let host = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or("").split(['/', '?', '#']).next().unwrap_or("");
            if host.is_empty() || url.chars().any(char::is_whitespace) {
                return Err("Datasheet must be an HTTP(S) URL; loading performs no fetch.".into());
            }
        }
        if self.specifications.len() > 64 {
            return Err("A manufacturer record supports at most 64 specifications.".into());
        }
        for (id, value) in &self.specifications {
            if !super::field_id(id) {
                return Err(format!("Invalid specification field {id}."));
            }
            value.validate()?;
        }
        Ok(())
    }
    /// Verify the exact part, package and category meanings without claiming suitability for a design.
    pub fn validate_part(&self, part: &ResolvedPart) -> Result<(), String> {
        self.validate()?;
        if self.part != part.definition.reference() || !part.definition.packages.contains_key(&self.package) {
            return Err("Manufacturer record requires its exact part and declared package variant.".into());
        }
        for (id, value) in &self.specifications {
            let field = part
                .category
                .as_ref()
                .and_then(|category| category.fields.get(id))
                .ok_or_else(|| format!("Unknown manufacturer specification field {id}."))?;
            value.validate_field(field)?;
        }
        Ok(())
    }
    /// Write canonical standalone data; manufacturer names and MPNs never become native identity keys.
    pub fn encode(&self) -> Result<String, String> {
        self.validate()?;
        encode(self)
    }
}
