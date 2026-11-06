//! Pinned material reports and dielectric construction, independent of board geometry and UI.
//! Raw records are validated when adopted as part of a board setup.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

mod electrical;
pub(crate) mod schema;
pub use electrical::{DielectricReport, DielectricValueKind, MeasurementAxis};
#[cfg(test)]
mod tests;

pub(crate) const MAX_MATERIALS: usize = 64;
pub(crate) const MAX_TG_REPORTS: usize = 8;
pub(crate) const MAX_PLIES_PER_GAP: usize = 32;
pub(crate) const MAX_TOTAL_PLIES: usize = 256;
pub(crate) const MAX_LENGTH_NM: i64 = 100_000_000;
pub(crate) const MIN_TEMPERATURE_MILLI_C: i64 = -273_149;
pub(crate) const MAX_TEMPERATURE_MILLI_C: i64 = 1_000_000;

/// Exact project-local material identity and revision; names never resolve a reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialRef {
    pub id: Uuid,
    pub revision: u32,
}

/// Provenance retained offline. A locator is descriptive text and is never fetched by the model.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub document: String,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
}

/// Measurement techniques are retained separately; this model does not equate their results.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TgMethod {
    Dsc,
    Tma,
    Dma,
}

/// What the source reports, not a design requirement or an independent qualification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportBasis {
    Typical,
    Minimum,
    Maximum,
    Measured,
}

/// A sourced glass-transition report in exact thousandths of a degree Celsius.
/// An absent report means unknown; zero Celsius is a valid value, not an omission sentinel.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TgReport {
    pub temperature_milli_c: i64,
    pub method: TgMethod,
    pub basis: ReportBasis,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub test_standard: Option<String>,
    pub source: Source,
}

/// An embedded material definition. Classification alone does not supply missing properties.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub id: Uuid,
    pub revision: u32,
    pub name: String,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tg: Vec<TgReport>,
    /// Frequency/condition-specific dielectric reports; absence supplies no Dk or Df default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dielectric: Vec<DielectricReport>,
}

impl Material {
    /// Address this exact embedded definition without resolving a name or a latest revision.
    pub fn reference(&self) -> MaterialRef {
        MaterialRef { id: self.id, revision: self.revision }
    }
}

/// One prepreg ply; its nominal stock thickness never contributes to finished board thickness.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepregPly {
    pub id: Uuid,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub material: Option<MaterialRef>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub glass_style: Option<String>,
    /// Resin fraction in millionths: 550,000 denotes 55 percent.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub resin_content_ppm: Option<u32>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub nominal_thickness_nm: Option<i64>,
}

/// A core or an ordered prepreg group within one dielectric gap. Material selection can remain
/// unknown. Core glass construction is a descriptive specification, not a resolved ply recipe.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Construction {
    Core {
        #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
        material: Option<MaterialRef>,
        #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
        glass_construction: Option<String>,
    },
    Prepreg {
        plies: Vec<PrepregPly>,
    },
}

impl Construction {
    /// Validate references and construction values. Global ply identity/count checks belong to setup.
    pub(crate) fn validate(&self, materials: &BTreeSet<MaterialRef>) -> Result<(), String> {
        match self {
            Self::Core { material, glass_construction } => {
                reference(material, materials)?;
                optional_text("core glass construction", glass_construction, 80)?;
            }
            Self::Prepreg { plies } => {
                if !(1..=MAX_PLIES_PER_GAP).contains(&plies.len()) {
                    return Err(format!("prepreg construction requires 1 to {MAX_PLIES_PER_GAP} ordered plies"));
                }
                for ply in plies {
                    reference(&ply.material, materials)?;
                    optional_text("glass style", &ply.glass_style, 80)?;
                    if ply.resin_content_ppm.is_some_and(|value| !(1..1_000_000).contains(&value)) {
                        return Err("resin content must be between 0 and 100 percent, exclusive, in millionths".to_owned());
                    }
                    if ply.nominal_thickness_nm.is_some_and(|value| !(1..=MAX_LENGTH_NM).contains(&value)) {
                        return Err("nominal ply thickness must be 1 to 100,000,000 whole nanometers".to_owned());
                    }
                }
            }
        }
        Ok(())
    }

    /// Retain authored ply order; there is no implied copper layer between these IDs.
    pub(crate) fn ply_ids(&self) -> impl Iterator<Item = Uuid> + '_ {
        let plies = match self {
            Self::Core { .. } => &[][..],
            Self::Prepreg { plies } => plies.as_slice(),
        };
        plies.iter().map(|ply| ply.id)
    }
}

/// Validate and canonicalize embedded definitions before resolving any construction references.
/// Multiple revisions of one material may coexist, but each exact key has one definition.
pub(crate) fn validate_catalog(materials: &mut [Material]) -> Result<BTreeSet<MaterialRef>, String> {
    if materials.len() > MAX_MATERIALS {
        return Err(format!("board setup supports at most {MAX_MATERIALS} pinned material revisions"));
    }
    let mut keys = BTreeSet::new();
    for material in materials.iter_mut() {
        if material.id.is_nil() || material.revision == 0 || !keys.insert(material.reference()) {
            return Err("material IDs must be non-nil with positive, unique pinned revisions".to_owned());
        }
        text("material name", &material.name, 80)?;
        optional_text("material family", &material.family, 80)?;
        optional_text("manufacturer", &material.manufacturer, 80)?;
        optional_text("product", &material.product, 80)?;
        if let Some(source) = &material.source {
            source.validate()?;
        }
        if material.tg.len() > MAX_TG_REPORTS {
            return Err(format!("each material supports at most {MAX_TG_REPORTS} Tg reports"));
        }
        for report in &material.tg {
            if !(MIN_TEMPERATURE_MILLI_C..=MAX_TEMPERATURE_MILLI_C).contains(&report.temperature_milli_c) {
                return Err("Tg must be above absolute zero and at most 1000 Celsius, in whole milli-Celsius".to_owned());
            }
            optional_text("Tg test standard", &report.test_standard, 80)?;
            report.source.validate()?;
        }
        material.tg.sort();
        if material.tg.windows(2).any(|reports| reports[0] == reports[1]) {
            return Err("duplicate Tg report".to_owned());
        }
        electrical::validate_reports(&mut material.dielectric)?;
    }
    materials.sort_by_key(Material::reference);
    Ok(keys)
}

impl Source {
    /// Bound provenance text without interpreting a locator as an instruction or fetching it.
    fn validate(&self) -> Result<(), String> {
        text("source document", &self.document, 200)?;
        optional_text("source revision", &self.revision, 80)?;
        optional_text("source locator", &self.locator, 1024)
    }
}

/// Unknown selection is allowed; a present reference must resolve to an exact embedded revision.
fn reference(value: &Option<MaterialRef>, materials: &BTreeSet<MaterialRef>) -> Result<(), String> {
    if value.is_some_and(|key| !materials.contains(&key)) { Err("construction references a missing material revision".to_owned()) } else { Ok(()) }
}

/// Omission retains unknown metadata; present text is validated without silently trimming it.
fn optional_text(field: &str, value: &Option<String>, max: usize) -> Result<(), String> {
    if let Some(value) = value {
        text(field, value, max)?;
    }
    Ok(())
}

/// Keep single-line authored names and provenance bounded and unambiguous.
fn text(field: &str, value: &str, max: usize) -> Result<(), String> {
    if value.is_empty() || value.trim() != value || value.chars().count() > max || value.chars().any(char::is_control) {
        Err(format!("{field} requires 1 to {max} characters, without surrounding whitespace or control characters"))
    } else {
        Ok(())
    }
}

/// Optional fields may be omitted, but explicit null is not an authored value.
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}
