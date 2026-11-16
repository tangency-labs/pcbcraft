//! Sourced insulation reports for clearance and creepage work; no spacing is derived here.

use super::{MAX_LENGTH_NM, MeasurementAxis, ReportBasis, Source, optional_text, present};
use serde::{Deserialize, Serialize};

pub(super) const MAX_REPORTS: usize = 16;
pub(super) const MAX_BREAKDOWN_V: u32 = 10_000_000;
pub(super) const MAX_CTI_V: u32 = 1_000;

/// One sourced dielectric strength: the voltage a specimen of the stated thickness withstood
/// before breakdown. Strength does not scale linearly with thickness, so the thickness is part
/// of the report and no per-millimetre value is derived from it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DielectricStrengthReport {
    /// Breakdown voltage in whole volts, 1 to 10,000,000.
    pub breakdown_v: u32,
    /// Thickness of the tested specimen, 1 to 100,000,000 nm.
    pub specimen_thickness_nm: i64,
    /// Direction of the applied field; absent means unknown.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub axis: Option<MeasurementAxis>,
    /// Test method or standard as written by the source, up to 80 characters.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Whether the value is typical, a minimum, a maximum or measured.
    pub basis: ReportBasis,
    /// Where the value comes from; required.
    pub source: Source,
}

/// Insulating material group by comparative tracking index, as IEC 60664-1:2007 clause
/// 4.8.1.3 classifies it for creepage distances.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MaterialGroup {
    #[serde(rename = "I")]
    One,
    #[serde(rename = "II")]
    Two,
    #[serde(rename = "IIIa")]
    ThreeA,
    #[serde(rename = "IIIb")]
    ThreeB,
}

impl MaterialGroup {
    /// The group whose CTI band holds `cti_v`, or `None` below 100 V, where IEC 60664-1:2007
    /// clause 4.8.1.3 assigns no group. Bands: I from 600 V; II from 400 V to below 600 V;
    /// IIIa from 175 V to below 400 V; IIIb from 100 V to below 175 V.
    pub fn for_cti(cti_v: u32) -> Option<Self> {
        match cti_v {
            600.. => Some(Self::One),
            400..600 => Some(Self::Two),
            175..400 => Some(Self::ThreeA),
            100..175 => Some(Self::ThreeB),
            _ => None,
        }
    }

    /// The group's name as written in IEC 60664-1 and in saved files.
    pub fn name(self) -> &'static str {
        match self {
            Self::One => "I",
            Self::Two => "II",
            Self::ThreeA => "IIIa",
            Self::ThreeB => "IIIb",
        }
    }
}

/// One sourced tracking report: a comparative tracking index in volts, a material group, or
/// both. When both are given they must agree; neither is inferred from the other on storage.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CtiReport {
    /// Comparative tracking index in whole volts, 1 to 1000.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub cti_v: Option<u32>,
    /// Material group; must hold `cti_v` when both are given.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub material_group: Option<MaterialGroup>,
    /// Test method or standard as written by the source, up to 80 characters.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Whether the value is typical, a minimum, a maximum or measured.
    pub basis: ReportBasis,
    /// Where the value comes from; required.
    pub source: Source,
}

/// Bound and canonicalize dielectric strength reports; the specimen thickness is required.
pub(super) fn validate_strength(reports: &mut [DielectricStrengthReport]) -> Result<(), String> {
    if reports.len() > MAX_REPORTS {
        return Err(format!("each material supports at most {MAX_REPORTS} dielectric strength reports"));
    }
    for report in reports.iter() {
        if !(1..=MAX_BREAKDOWN_V).contains(&report.breakdown_v) {
            return Err("dielectric strength must be 1 to 10,000,000 whole volts".to_owned());
        }
        if !(1..=MAX_LENGTH_NM).contains(&report.specimen_thickness_nm) {
            return Err("specimen thickness must be 1 to 100,000,000 whole nanometers".to_owned());
        }
        optional_text("dielectric strength test method", &report.method, 80)?;
        report.source.validate()?;
    }
    reports.sort();
    if reports.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("duplicate dielectric strength report".to_owned());
    }
    Ok(())
}

/// Bound and canonicalize tracking reports. A report needs a CTI or a group, and a CTI given
/// with a group must lie in that group's band.
pub(super) fn validate_cti(reports: &mut [CtiReport]) -> Result<(), String> {
    if reports.len() > MAX_REPORTS {
        return Err(format!("each material supports at most {MAX_REPORTS} tracking reports"));
    }
    for report in reports.iter() {
        if report.cti_v.is_none() && report.material_group.is_none() {
            return Err("a tracking report requires a CTI or a material group".to_owned());
        }
        if report.cti_v.is_some_and(|value| !(1..=MAX_CTI_V).contains(&value)) {
            return Err("CTI must be 1 to 1000 whole volts".to_owned());
        }
        if let (Some(cti), Some(group)) = (report.cti_v, report.material_group)
            && MaterialGroup::for_cti(cti) != Some(group)
        {
            return Err(format!("CTI {cti} V lies outside the band of material group {}", group.name()));
        }
        optional_text("tracking test method", &report.method, 80)?;
        report.source.validate()?;
    }
    reports.sort();
    if reports.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("duplicate tracking report".to_owned());
    }
    Ok(())
}
