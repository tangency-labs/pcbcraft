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
    pub conductivity_mw_per_m_k: u32,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub axis: Option<MeasurementAxis>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub temperature_milli_c: Option<i64>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    pub basis: ReportBasis,
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
    pub cte_ppb_per_k: i64,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub axis: Option<MeasurementAxis>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub region: Option<TgRegion>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub from_milli_c: Option<i64>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub to_milli_c: Option<i64>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    pub basis: ReportBasis,
    pub source: Source,
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
