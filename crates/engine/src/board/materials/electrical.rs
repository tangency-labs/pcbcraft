//! Authored dielectric characterization, kept separate from a future impedance solver.

use super::{MAX_LENGTH_NM, MAX_TEMPERATURE_MILLI_C, MIN_TEMPERATURE_MILLI_C, ReportBasis, Source, optional_text, present};
use serde::{Deserialize, Serialize};

pub(super) const MAX_REPORTS: usize = 32;
pub(super) const MAX_FREQUENCY_HZ: u64 = 1_000_000_000_000_000;

/// A supplier's design value is distinct from a test value; neither is inferred from the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DielectricValueKind {
    Test,
    Design,
}

/// Direction in the material specimen, not a board rotation or an isotropy assumption.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementAxis {
    X,
    Y,
    Z,
}

/// One sourced Dk and/or Df report with its known applicability conditions. Values are exact
/// dimensionless millionths: 4,200,000 means Dk 4.2; 15,000 means Df 0.015. Missing frequency,
/// method, specimen or direction stays unknown and is not permission to use a report universally.
/// No interpolation, effective-permittivity calculation or impedance result is supplied here.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DielectricReport {
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub dk_millionths: Option<u32>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub df_millionths: Option<u32>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub frequency_hz: Option<u64>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub value_kind: Option<DielectricValueKind>,
    pub basis: ReportBasis,
    pub source: Source,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub temperature_milli_c: Option<i64>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub axis: Option<MeasurementAxis>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub glass_construction: Option<String>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub resin_content_ppm: Option<u32>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub specimen_thickness_nm: Option<i64>,
}

/// Bound and canonicalize reports without choosing one for an electrical calculation. The
/// supported numeric ranges are storage limits, not proof of a manufacturable material.
pub(super) fn validate_reports(reports: &mut [DielectricReport]) -> Result<(), String> {
    if reports.len() > MAX_REPORTS {
        return Err(format!("each material supports at most {MAX_REPORTS} dielectric reports"));
    }
    for report in reports.iter() {
        if report.dk_millionths.is_none() && report.df_millionths.is_none() {
            return Err("a dielectric report requires Dk or Df".to_owned());
        }
        if report.dk_millionths.is_some_and(|value| !(1_000_000..=1_000_000_000).contains(&value))
            || report.df_millionths.is_some_and(|value| value > 1_000_000)
        {
            return Err("dielectric reports support Dk 1 to 1000 and Df 0 to 1, in whole millionths".to_owned());
        }
        if report.frequency_hz.is_some_and(|value| !(1..=MAX_FREQUENCY_HZ).contains(&value)) {
            return Err("report frequency must be 1 to 1,000,000,000,000,000 whole hertz".to_owned());
        }
        if report.temperature_milli_c.is_some_and(|value| !(MIN_TEMPERATURE_MILLI_C..=MAX_TEMPERATURE_MILLI_C).contains(&value)) {
            return Err("report temperature must be above absolute zero and at most 1000 Celsius, in whole milli-Celsius".to_owned());
        }
        if report.resin_content_ppm.is_some_and(|value| !(1..1_000_000).contains(&value)) {
            return Err("specimen resin content must be between 0 and 100 percent, exclusive, in millionths".to_owned());
        }
        if report.specimen_thickness_nm.is_some_and(|value| !(1..=MAX_LENGTH_NM).contains(&value)) {
            return Err("specimen thickness must be 1 to 100,000,000 whole nanometers".to_owned());
        }
        optional_text("dielectric test method", &report.method, 80)?;
        optional_text("specimen glass construction", &report.glass_construction, 80)?;
        report.source.validate()?;
    }
    reports.sort();
    if reports.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("duplicate dielectric report".to_owned());
    }
    Ok(())
}
