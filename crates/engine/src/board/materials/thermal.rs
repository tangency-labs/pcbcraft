//! Sourced thermal conductivity and expansion reports; no heat-flow or warpage model is supplied.

use super::{MAX_TEMPERATURE_MILLI_C, MIN_TEMPERATURE_MILLI_C, MeasurementAxis, ReportBasis, Source, optional_text, present};
use serde::{Deserialize, Serialize};

pub(super) const MAX_REPORTS: usize = 16;
pub(super) const MAX_CONDUCTIVITY_MW_PER_M_K: u32 = 10_000_000;
pub(super) const MIN_CTE_PPB_PER_K: i64 = -1_000_000;
pub(super) const MAX_CTE_PPB_PER_K: i64 = 10_000_000;

/// One sourced thermal conductivity in exact milliwatts per metre-kelvin: 1000 means
/// 1 W/(m·K). A laminate conducts very differently in-plane and through its thickness, so a
/// report without an axis is not permission to use it in either direction.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThermalConductivityReport {
    /// Conductivity in whole mW/(m·K), 1 to 10,000,000, as a JSON integer-kind token.
    pub conductivity_mw_per_m_k: u32,
    /// Direction measured; absent means unknown, not isotropic.
    #[serde(default, deserialize_with = "present_string_enum", skip_serializing_if = "Option::is_none")]
    pub axis: Option<MeasurementAxis>,
    /// Specimen temperature, in the Tg report range, as a JSON integer-kind token.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub temperature_milli_c: Option<i64>,
    /// Test method or standard as written by the source, up to 80 characters.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Whether the value is typical, a minimum, a maximum or measured.
    #[serde(deserialize_with = "string_enum")]
    pub basis: ReportBasis,
    /// Where the value comes from; required.
    pub source: Source,
}

/// Which side of the glass transition a thermal expansion value describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TgRegion {
    BelowTg,
    AboveTg,
}

/// One sourced coefficient of thermal expansion in exact parts per billion per kelvin:
/// 50,000 means 50 ppm/K. The value is signed because some reinforcements contract in-plane.
/// A stated temperature range has both ends; region and range are retained as reported and
/// neither is derived from the other or from a Tg report.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CteReport {
    /// Expansion in whole ppb/K, -1,000,000 to 10,000,000, as a JSON integer-kind token.
    pub cte_ppb_per_k: i64,
    /// Direction measured; absent means unknown, not isotropic.
    #[serde(default, deserialize_with = "present_string_enum", skip_serializing_if = "Option::is_none")]
    pub axis: Option<MeasurementAxis>,
    /// Side of the glass transition, when the source states it.
    #[serde(default, deserialize_with = "present_string_enum", skip_serializing_if = "Option::is_none")]
    pub region: Option<TgRegion>,
    /// Lower end of the averaging range, as a JSON integer-kind token; given with `to_milli_c`.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub from_milli_c: Option<i64>,
    /// Upper end, as a JSON integer-kind token, above `from_milli_c`.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub to_milli_c: Option<i64>,
    /// Test method or standard as written by the source, up to 80 characters.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Whether the value is typical, a minimum, a maximum or measured.
    #[serde(deserialize_with = "string_enum")]
    pub basis: ReportBasis,
    /// Where the value comes from; required.
    pub source: Source,
}

/// Decode the published string representation, not Serde's alternative singleton enum map.
/// The enum itself remains responsible for its variant names.
pub(super) fn string_enum<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<T, D::Error> {
    let value = String::deserialize(deserializer)?;
    T::deserialize(serde::de::value::StringDeserializer::<D::Error>::new(value))
}

/// An omitted enum stays unknown; a present value must be a string, never a map or null.
pub(super) fn present_string_enum<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<T>, D::Error> {
    string_enum(deserializer).map(Some)
}

/// Whether a temperature lies in the representable range shared with Tg reports.
fn temperature(value: i64) -> bool {
    (MIN_TEMPERATURE_MILLI_C..=MAX_TEMPERATURE_MILLI_C).contains(&value)
}

/// Bound and canonicalize conductivity reports; duplicates and out-of-range values are refused.
pub(super) fn validate_conductivity(reports: &mut [ThermalConductivityReport]) -> Result<(), String> {
    if reports.len() > MAX_REPORTS {
        return Err(format!("each material supports at most {MAX_REPORTS} thermal conductivity reports"));
    }
    for report in reports.iter() {
        if !(1..=MAX_CONDUCTIVITY_MW_PER_M_K).contains(&report.conductivity_mw_per_m_k) {
            return Err("thermal conductivity must be 1 to 10,000,000 whole mW/(m·K)".to_owned());
        }
        if report.temperature_milli_c.is_some_and(|value| !temperature(value)) {
            return Err("report temperature must be above absolute zero and at most 1000 Celsius, in whole milli-Celsius".to_owned());
        }
        optional_text("thermal conductivity test method", &report.method, 80)?;
        report.source.validate()?;
    }
    reports.sort();
    if reports.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("duplicate thermal conductivity report".to_owned());
    }
    Ok(())
}

/// Bound and canonicalize expansion reports. A temperature range needs both ends in rising
/// order; a single end would not say which interval the coefficient averages over.
pub(super) fn validate_cte(reports: &mut [CteReport]) -> Result<(), String> {
    if reports.len() > MAX_REPORTS {
        return Err(format!("each material supports at most {MAX_REPORTS} thermal expansion reports"));
    }
    for report in reports.iter() {
        if !(MIN_CTE_PPB_PER_K..=MAX_CTE_PPB_PER_K).contains(&report.cte_ppb_per_k) {
            return Err("thermal expansion must be -1,000,000 to 10,000,000 whole ppb/K".to_owned());
        }
        match (report.from_milli_c, report.to_milli_c) {
            (None, None) => {}
            (Some(from), Some(to)) if temperature(from) && temperature(to) && from < to => {}
            _ => {
                return Err(
                    "a thermal expansion temperature range requires both ends, rising, above absolute zero and at most 1000 Celsius".to_owned()
                );
            }
        }
        optional_text("thermal expansion test method", &report.method, 80)?;
        report.source.validate()?;
    }
    reports.sort();
    if reports.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("duplicate thermal expansion report".to_owned());
    }
    Ok(())
}
