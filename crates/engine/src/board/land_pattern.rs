//! Surface-mount land patterns computed from a component's dimension limits.
//!
//! The calculator follows the land pattern equations of IPC-7351 (February 2005), section
//! 3.1.5: the outer land extent `Z`, the inner gap `G` and the land width `X` combine the
//! component's least-material dimensions, a solder fillet goal for the toe, heel and side,
//! and the root-sum-square of the component, fabrication (`F`) and placement (`P`)
//! tolerances. The fillet goals come from Tables 3-2, 3-3, 3-5 and 3-6 for the selected
//! density level. Values are returned before the standard's round-off, in whole nanometers.
//!
//! Every length is an integer number of nanometers, as in the footprint definition. The
//! resulting lands use the footprint's local axes (positive Y down): two-terminal chips lie
//! on the X axis with pad 1 on the left, and gull-wing rows run along Y with pin 1 at the
//! top of the left row, numbered counterclockwise as seen from the top.

use crate::schematic::Point;
use serde::{Deserialize, Serialize};

use super::definition::{MAX_OFFSET_NM, MAX_PADS};

/// The edition whose equations and goal tables this calculator implements.
pub const STANDARD_EDITION: &str = "IPC-7351 (February 2005)";

/// Nominal chip body length from which IPC-7351 Table 3-5 applies instead of Table 3-6:
/// the 1608 metric (0603 inch) size.
const CHIP_TABLE_3_5_MIN_LENGTH_NM: i64 = 1_600_000;
/// Gull-wing pitch above which IPC-7351 Table 3-2 applies instead of Table 3-3.
const GULL_WING_TABLE_3_2_PITCH_ABOVE_NM: i64 = 625_000;
/// Largest terminal length tolerance for which the reduced gull-wing heel goals of
/// note 1 to IPC-7351 Tables 3-2 and 3-3 apply.
const GULL_WING_SHORT_HEEL_MAX_TERMINAL_TOLERANCE_NM: i64 = 500_000;

/// Land protrusion goal from IPC-7351 section 3.1.5.1: most (A), nominal (B) or least (C).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DensityLevel {
    /// Maximum (most) land protrusion, for low-density or hand-soldered boards.
    A,
    /// Median (nominal) land protrusion, for ordinary reflow boards.
    B,
    /// Minimum (least) land protrusion, for high-density boards.
    C,
}

/// The terminal styles this calculator supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageFamily {
    /// Rectangular or square-end two-terminal chip resistors and capacitors.
    Chip,
    /// Flat ribbon L and gull-wing leads in two rows, such as SOIC and SOP.
    GullWing,
}

/// Inclusive minimum and maximum of one component dimension from its data sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub min_nm: i64,
    pub max_nm: i64,
}

impl Limits {
    /// Difference between the largest and smallest permitted size.
    fn tolerance(self) -> i64 {
        self.max_nm - self.min_nm
    }
}

/// Component dimensions and process tolerances for one land pattern.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LandPatternInput {
    pub family: PackageFamily,
    pub density: DensityLevel,
    /// Overall terminal-to-terminal length `L`, or the lead span of a gull-wing package.
    pub lead_span: Limits,
    /// Length `T` of one terminal or of the lead's flat foot.
    pub terminal_length: Limits,
    /// Width `W` of one terminal or lead.
    pub terminal_width: Limits,
    /// Body width `A` of a gull-wing package, required when the terminal length tolerance
    /// is at most 0.5 mm to determine the heel goal under note 1 to Tables 3-2 and 3-3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_width: Option<Limits>,
    /// Lead pitch of a gull-wing package; chips have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch_nm: Option<i64>,
    /// Total lead count of a gull-wing package, split evenly between its two rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_count: Option<u16>,
    /// Land pattern fabrication tolerance `F`.
    pub fabrication_tolerance_nm: i64,
    /// Placement accuracy `P`, as a diameter of true position.
    pub placement_tolerance_nm: i64,
}

/// Solder fillet goals and courtyard excess taken from the IPC-7351 table for the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct FilletGoals {
    /// Number of the table the goals come from, such as `3-5`.
    pub table: &'static str,
    pub toe_nm: i64,
    pub heel_nm: i64,
    pub side_nm: i64,
    pub courtyard_excess_nm: i64,
}

/// One computed rectangular land in local footprint coordinates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Land {
    pub number: String,
    pub center: Point,
    /// Extent along local X.
    pub width_nm: i64,
    /// Extent along local Y.
    pub height_nm: i64,
}

/// The computed pattern: the standard's three dimensions and the lands they place.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LandPattern {
    pub goals: FilletGoals,
    /// `Zmax`: distance across the outer ends of opposite lands.
    pub outer_nm: i64,
    /// `Gmin`: gap between the inner ends of opposite lands.
    pub inner_nm: i64,
    /// `Xmax`: land width across the terminal.
    pub land_width_nm: i64,
    pub lands: Vec<Land>,
}

/// Compute the land pattern for `input`.
///
/// Errors name the first refused value: a dimension outside 1 nm to 100 mm, a minimum above its
/// maximum, a tolerance outside 0 to 100 mm, terminals that meet inside the body, opposite lands
/// that would meet after rounding, adjacent gull-wing lands that would touch, lands beyond the
/// footprint size or coordinate limits, missing required fields, or fields for the wrong family.
pub fn calculate(input: &LandPatternInput) -> Result<LandPattern, String> {
    validate(input)?;
    let (lead_span, terminal_length, terminal_width) = (input.lead_span, input.terminal_length, input.terminal_width);
    let inner_span_min_nm = lead_span.min_nm - 2 * terminal_length.max_nm;
    if inner_span_min_nm <= 0 {
        return Err(format!(
            "the terminals meet inside the body: a {} nm minimum lead span leaves {} nm between terminals of maximum length",
            lead_span.min_nm, inner_span_min_nm
        ));
    }
    let goals = goals(input, inner_span_min_nm)?;
    let (fabrication, placement) = (input.fabrication_tolerance_nm as f64, input.placement_tolerance_nm as f64);
    let process = |component: f64| (component * component + fabrication * fabrication + placement * placement).sqrt();
    let length_tolerance = lead_span.tolerance() as f64;
    let terminal_tolerance = terminal_length.tolerance() as f64;
    // Section 3.1.1: the worst-case tolerance of the inner span is replaced by its RMS value,
    // which is added to its minimum to give the realistic maximum used for Gmin.
    let inner_span_tolerance = (length_tolerance * length_tolerance + 2. * terminal_tolerance * terminal_tolerance).sqrt();
    let inner_span_max = inner_span_min_nm as f64 + inner_span_tolerance;
    // Section 3.1.5: Zmax, Gmin and Xmax.
    let outer = lead_span.min_nm as f64 + 2. * goals.toe_nm as f64 + process(length_tolerance);
    let inner = inner_span_max - 2. * goals.heel_nm as f64 - process(inner_span_tolerance);
    let land_width = terminal_width.min_nm as f64 + 2. * goals.side_nm as f64 + process(terminal_width.tolerance() as f64);
    let (inner_nm, land_width_nm) = (nm(inner), nm(land_width));
    let length_nm = nm((outer - inner) / 2.);
    let row_offset_nm = nm((outer + inner) / 4.);
    if inner_nm < 1 || land_width_nm < 1 || length_nm < 1 {
        return Err(format!("the computed lands do not separate: outer {} nm, gap {inner_nm} nm, width {land_width_nm} nm", nm(outer)));
    }
    // Use the same integer X centers and size that the emitted rectangles will use.
    let gap_nm = 2 * row_offset_nm - length_nm;
    if gap_nm < 1 {
        return Err(format!("opposite lands do not separate after rounding: {gap_nm} nm gap"));
    }
    let lands = match input.family {
        PackageFamily::Chip => vec![land("1", -row_offset_nm, 0, length_nm, land_width_nm), land("2", row_offset_nm, 0, length_nm, land_width_nm)],
        PackageFamily::GullWing => gull_wing_lands(input, row_offset_nm, length_nm, land_width_nm)?,
    };
    if !lands.iter().all(within_bounds) {
        return Err(format!("the computed lands exceed the {MAX_OFFSET_NM} nm footprint size or coordinate limits"));
    }
    Ok(LandPattern { goals, outer_nm: nm(outer), inner_nm, land_width_nm, lands })
}

/// Refuse out-of-range dimensions and fields that belong to the other package family.
fn validate(input: &LandPatternInput) -> Result<(), String> {
    let named = [("lead span", input.lead_span), ("terminal length", input.terminal_length), ("terminal width", input.terminal_width)];
    for (name, limits) in named.into_iter().chain(input.body_width.map(|limits| ("body width", limits))) {
        if !(1..=MAX_OFFSET_NM).contains(&limits.min_nm) || !(1..=MAX_OFFSET_NM).contains(&limits.max_nm) {
            return Err(format!("{name} limits must be between 1 and {MAX_OFFSET_NM} nm"));
        }
        if limits.min_nm > limits.max_nm {
            return Err(format!("{name} minimum {} nm exceeds its maximum {} nm", limits.min_nm, limits.max_nm));
        }
    }
    for (name, value) in [("fabrication tolerance", input.fabrication_tolerance_nm), ("placement tolerance", input.placement_tolerance_nm)] {
        if !(0..=MAX_OFFSET_NM).contains(&value) {
            return Err(format!("{name} must be between 0 and {MAX_OFFSET_NM} nm"));
        }
    }
    match input.family {
        PackageFamily::Chip if input.pitch_nm.is_some() || input.pin_count.is_some() || input.body_width.is_some() => {
            Err("a chip land pattern takes no pitch, pin count or body width".into())
        }
        PackageFamily::Chip => Ok(()),
        PackageFamily::GullWing => {
            let pitch = input.pitch_nm.ok_or("a gull-wing land pattern needs a pitch")?;
            let pins = input.pin_count.ok_or("a gull-wing land pattern needs a pin count")?;
            if !(1..=MAX_OFFSET_NM).contains(&pitch) {
                return Err(format!("pitch must be between 1 and {MAX_OFFSET_NM} nm"));
            }
            if pins < 2 || !pins.is_multiple_of(2) || usize::from(pins) > MAX_PADS {
                return Err(format!("a gull-wing pin count must be even and between 2 and {MAX_PADS}, not {pins}"));
            }
            Ok(())
        }
    }
}

/// Select the fillet goals of IPC-7351 Tables 3-2 and 3-3 (gull-wing above and up to 0.625 mm
/// pitch) or 3-5 and 3-6 (chips from and below the 1608 metric nominal length). A gull-wing
/// package whose minimum inner span `Smin` is at most its maximum body width uses the reduced
/// heel goals of note 1 to Tables 3-2 and 3-3, unless its terminal length tolerance exceeds 0.5 mm.
fn goals(input: &LandPatternInput, inner_span_min_nm: i64) -> Result<FilletGoals, String> {
    let level = |a: i64, b: i64, c: i64| match input.density {
        DensityLevel::A => a,
        DensityLevel::B => b,
        DensityLevel::C => c,
    };
    let courtyard = level(500_000, 250_000, 100_000);
    Ok(match input.family {
        PackageFamily::Chip if nominal(input.lead_span) >= CHIP_TABLE_3_5_MIN_LENGTH_NM => FilletGoals {
            table: "3-5",
            toe_nm: level(550_000, 350_000, 150_000),
            heel_nm: -50_000,
            side_nm: level(50_000, 0, -50_000),
            courtyard_excess_nm: courtyard,
        },
        PackageFamily::Chip => FilletGoals {
            table: "3-6",
            toe_nm: level(200_000, 100_000, 0),
            heel_nm: -50_000,
            side_nm: level(50_000, 0, 0),
            courtyard_excess_nm: level(200_000, 150_000, 100_000),
        },
        PackageFamily::GullWing => {
            let wide_pitch = input.pitch_nm.is_some_and(|pitch| pitch > GULL_WING_TABLE_3_2_PITCH_ABOVE_NM);
            let short_heel = if input.terminal_length.tolerance() <= GULL_WING_SHORT_HEEL_MAX_TERMINAL_TOLERANCE_NM {
                let body = input.body_width.ok_or("a gull-wing pattern needs a body width when the terminal length tolerance is at most 0.5 mm")?;
                inner_span_min_nm <= body.max_nm
            } else {
                false
            };
            FilletGoals {
                table: if wide_pitch { "3-2" } else { "3-3" },
                toe_nm: level(550_000, 350_000, 150_000),
                heel_nm: if short_heel { level(250_000, 150_000, 50_000) } else { level(450_000, 350_000, 250_000) },
                side_nm: if wide_pitch { level(50_000, 30_000, 10_000) } else { level(10_000, -20_000, -40_000) },
                courtyard_excess_nm: courtyard,
            }
        }
    })
}

/// Place two rows of leads: pin 1 at the top of the left row, counting down the left row and
/// back up the right row. Adjacent lands of a row must leave a gap.
fn gull_wing_lands(input: &LandPatternInput, row_offset_nm: i64, length_nm: i64, width_nm: i64) -> Result<Vec<Land>, String> {
    let (Some(pitch), Some(pins)) = (input.pitch_nm, input.pin_count) else {
        return Err("a gull-wing land pattern needs a pitch and a pin count".into());
    };
    if width_nm >= pitch {
        return Err(format!("adjacent lands touch: the {width_nm} nm land width is not less than the {pitch} nm pitch"));
    }
    let per_row = usize::from(pins / 2);
    let row_y = |index: usize| nm(pitch as f64 * (index as f64 - (per_row as f64 - 1.) / 2.));
    let left = (0..per_row).map(|index| land(&(index + 1).to_string(), -row_offset_nm, row_y(index), length_nm, width_nm));
    let right = (0..per_row).map(|index| land(&(per_row + index + 1).to_string(), row_offset_nm, row_y(per_row - 1 - index), length_nm, width_nm));
    Ok(left.chain(right).collect())
}

/// Build one land with its length along the X axis.
fn land(number: &str, x_nm: i64, y_nm: i64, length_nm: i64, width_nm: i64) -> Land {
    Land { number: number.into(), center: Point { x_nm, y_nm }, width_nm: length_nm, height_nm: width_nm }
}

/// Keep every full land extent within the native size limit and each edge within the envelope.
fn within_bounds(land: &Land) -> bool {
    let axis_fits = |center: i64, size: i64| (1..=MAX_OFFSET_NM).contains(&size) && center.abs() + (size + 1) / 2 <= MAX_OFFSET_NM;
    axis_fits(land.center.x_nm, land.width_nm) && axis_fits(land.center.y_nm, land.height_nm)
}

/// Middle of a validated dimension range.
fn nominal(limits: Limits) -> i64 {
    limits.min_nm + limits.tolerance() / 2
}

/// Round a computed length to the nearest nanometer, halves away from zero.
fn nm(value: f64) -> i64 {
    value.round() as i64
}

#[cfg(test)]
mod tests;
