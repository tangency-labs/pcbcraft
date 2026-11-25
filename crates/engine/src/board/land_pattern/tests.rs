use super::*;

/// Lands are compared with the hand calculation before round-off, within one micrometer.
const MATCH_NM: i64 = 1_000;

fn limits(min_nm: i64, max_nm: i64) -> Limits {
    Limits { min_nm, max_nm }
}

/// Synthetic 1608 metric (0603) chip resistor limits; not a vendor data sheet.
fn chip_0603() -> LandPatternInput {
    LandPatternInput {
        family: PackageFamily::Chip,
        density: DensityLevel::B,
        lead_span: limits(1_500_000, 1_700_000),
        terminal_length: limits(200_000, 500_000),
        terminal_width: limits(700_000, 900_000),
        body_width: None,
        pitch_nm: None,
        pin_count: None,
        fabrication_tolerance_nm: 100_000,
        placement_tolerance_nm: 50_000,
    }
}

/// Synthetic SOIC-8 limits in the shape of a 3.9 mm body, 1.27 mm pitch package.
fn soic8() -> LandPatternInput {
    LandPatternInput {
        family: PackageFamily::GullWing,
        density: DensityLevel::B,
        lead_span: limits(5_800_000, 6_200_000),
        terminal_length: limits(400_000, 1_270_000),
        terminal_width: limits(310_000, 510_000),
        body_width: Some(limits(3_800_000, 4_000_000)),
        pitch_nm: Some(1_270_000),
        pin_count: Some(8),
        fabrication_tolerance_nm: 100_000,
        placement_tolerance_nm: 50_000,
    }
}

fn assert_near(actual: i64, expected_nm: f64, what: &str) {
    let difference = (actual as f64 - expected_nm).abs();
    assert!(difference <= MATCH_NM as f64, "{what}: {actual} nm differs from the hand value {expected_nm} nm by {difference} nm");
}

fn assert_land(land: &Land, number: &str, x_nm: f64, y_nm: f64, width_nm: f64, height_nm: f64) {
    assert_eq!(land.number, number);
    assert_near(land.center.x_nm, x_nm, &format!("pad {number} x"));
    assert_near(land.center.y_nm, y_nm, &format!("pad {number} y"));
    assert_near(land.width_nm, width_nm, &format!("pad {number} length"));
    assert_near(land.height_nm, height_nm, &format!("pad {number} width"));
}

/// IPC-7351 (2005) section 3.1.5 with Table 3-5 level B goals (toe 0.35, heel -0.05, side 0 mm),
/// worked by hand in millimeters:
/// CL = 0.20, CT = 0.30, CW = 0.20; Smin = 1.50 - 2(0.50) = 0.50;
/// Stol(RMS) = sqrt(0.20² + 2(0.30²)) = 0.469041576; Smax = 0.969041576;
/// Zmax = 1.50 + 0.70 + sqrt(0.20² + 0.10² + 0.05²) = 2.429128785;
/// Gmin = 0.969041576 + 0.10 - sqrt(0.469041576² + 0.10² + 0.05²) = 0.586859038;
/// Xmax = 0.70 + 0 + sqrt(0.20² + 0.10² + 0.05²) = 0.929128785;
/// land length (Z - G)/2 = 0.921134873 at X = ±(Z + G)/4 = ±0.753996956.
#[test]
fn level_b_0603_matches_hand_computed_lands() {
    let pattern = calculate(&chip_0603()).unwrap();
    assert_eq!(pattern.goals, FilletGoals { table: "3-5", toe_nm: 350_000, heel_nm: -50_000, side_nm: 0, courtyard_excess_nm: 250_000 });
    assert_near(pattern.outer_nm, 2_429_128.785, "Zmax");
    assert_near(pattern.inner_nm, 586_859.038, "Gmin");
    assert_near(pattern.land_width_nm, 929_128.785, "Xmax");
    assert_eq!(pattern.lands.len(), 2);
    assert_land(&pattern.lands[0], "1", -753_996.956, 0., 921_134.873, 929_128.785);
    assert_land(&pattern.lands[1], "2", 753_996.956, 0., 921_134.873, 929_128.785);
}

/// IPC-7351 (2005) section 3.1.5 with Table 3-2 level B goals (toe 0.35, heel 0.35, side
/// 0.03 mm). The terminal tolerance 0.87 mm exceeds 0.5 mm, so note 1's reduced heel does not
/// apply although Smin is inside the body. Worked by hand in millimeters:
/// CL = 0.40, CT = 0.87, CW = 0.20; Smin = 5.80 - 2(1.27) = 3.26;
/// Stol(RMS) = sqrt(0.40² + 2(0.87²)) = 1.293754; Smax = 4.553754;
/// Zmax = 5.80 + 0.70 + sqrt(0.40² + 0.10² + 0.05²) = 6.915331193;
/// Gmin = 4.553754 - 0.70 - sqrt(1.293754² + 0.10² + 0.05²) = 2.555178084;
/// Xmax = 0.31 + 0.06 + sqrt(0.20² + 0.10² + 0.05²) = 0.599128785;
/// land length 2.180076555 at X = ±2.367627319, rows at ±0.635 and ±1.905 mm.
#[test]
fn level_b_soic8_matches_hand_computed_lands() {
    let pattern = calculate(&soic8()).unwrap();
    assert_eq!(pattern.goals, FilletGoals { table: "3-2", toe_nm: 350_000, heel_nm: 350_000, side_nm: 30_000, courtyard_excess_nm: 250_000 });
    assert_near(pattern.outer_nm, 6_915_331.193, "Zmax");
    assert_near(pattern.inner_nm, 2_555_178.084, "Gmin");
    assert_near(pattern.land_width_nm, 599_128.785, "Xmax");
    let (x, length, width) = (2_367_627.319, 2_180_076.555, 599_128.785);
    let expected = [
        ("1", -x, -1_905_000.),
        ("2", -x, -635_000.),
        ("3", -x, 635_000.),
        ("4", -x, 1_905_000.),
        ("5", x, 1_905_000.),
        ("6", x, 635_000.),
        ("7", x, -635_000.),
        ("8", x, -1_905_000.),
    ];
    assert_eq!(pattern.lands.len(), expected.len());
    for (land, (number, x, y)) in pattern.lands.iter().zip(expected) {
        assert_land(land, number, x, y, length, width);
    }
}

/// Every density level reads its own column of Tables 3-5 and 3-2.
#[test]
fn density_levels_select_their_table_columns() {
    for (density, chip_toe, chip_side, gull_heel, courtyard) in
        [(DensityLevel::A, 550_000, 50_000, 450_000, 500_000), (DensityLevel::C, 150_000, -50_000, 250_000, 100_000)]
    {
        let chip = calculate(&LandPatternInput { density, ..chip_0603() }).unwrap().goals;
        assert_eq!((chip.toe_nm, chip.heel_nm, chip.side_nm, chip.courtyard_excess_nm), (chip_toe, -50_000, chip_side, courtyard));
        let gull = calculate(&LandPatternInput { density, ..soic8() }).unwrap().goals;
        assert_eq!((gull.heel_nm, gull.courtyard_excess_nm), (gull_heel, courtyard));
    }
}

/// Table 3-5 starts at the 1608 metric nominal length; one nanometer shorter uses Table 3-6.
#[test]
fn chip_table_switches_at_the_1608_nominal_length() {
    assert_eq!(calculate(&chip_0603()).unwrap().goals.table, "3-5");
    let shorter = LandPatternInput { lead_span: limits(1_500_000, 1_699_998), ..chip_0603() };
    assert_eq!(
        calculate(&shorter).unwrap().goals,
        FilletGoals { table: "3-6", toe_nm: 100_000, heel_nm: -50_000, side_nm: 0, courtyard_excess_nm: 150_000 }
    );
}

/// Narrow synthetic leads at a given pitch, for the fine-pitch table and the land gap.
fn fine_pitch(pitch_nm: i64) -> LandPatternInput {
    LandPatternInput { terminal_width: limits(170_000, 270_000), pitch_nm: Some(pitch_nm), ..soic8() }
}

/// Table 3-2 applies above 0.625 mm pitch and Table 3-3 at 0.625 mm and below.
#[test]
fn gull_wing_table_switches_above_0_625_mm_pitch() {
    assert_eq!(calculate(&fine_pitch(625_001)).unwrap().goals.table, "3-2");
    let goals = calculate(&fine_pitch(625_000)).unwrap().goals;
    assert_eq!((goals.table, goals.side_nm), ("3-3", -20_000));
}

/// Note 1 to Table 3-2: the reduced heel applies up to a 0.5 mm terminal tolerance and only
/// when Smin does not clear the body's maximum width.
#[test]
fn gull_wing_reduced_heel_follows_note_1() {
    let terminal = |max_nm| LandPatternInput { terminal_length: limits(400_000, max_nm), ..soic8() };
    assert_eq!(calculate(&terminal(900_000)).unwrap().goals.heel_nm, 150_000);
    assert_eq!(calculate(&terminal(900_001)).unwrap().goals.heel_nm, 350_000);
    let narrow_body = LandPatternInput { body_width: Some(limits(3_000_000, 3_999_999)), ..terminal(900_000) };
    assert_eq!(calculate(&narrow_body).unwrap().goals.heel_nm, 350_000);
}

/// Missing body width cannot select a heel at or below 0.5 mm terminal tolerance.
#[test]
fn gull_wing_body_width_is_required_only_at_or_below_note_1_tolerance() {
    let without_body = |max_nm| LandPatternInput { body_width: None, terminal_length: limits(400_000, max_nm), ..soic8() };
    for max_nm in [899_999, 900_000] {
        assert_eq!(
            calculate(&without_body(max_nm)).unwrap_err(),
            "a gull-wing pattern needs a body width when the terminal length tolerance is at most 0.5 mm"
        );
    }
    assert_eq!(calculate(&without_body(900_001)).unwrap().goals.heel_nm, 350_000);
}

/// A dimension may have equal limits; a minimum above its maximum is refused.
#[test]
fn limits_accept_equal_bounds_and_refuse_reversed_bounds() {
    let exact = LandPatternInput { terminal_width: limits(800_000, 800_000), ..chip_0603() };
    assert!(calculate(&exact).is_ok());
    let reversed = LandPatternInput { terminal_width: limits(800_001, 800_000), ..chip_0603() };
    assert_eq!(calculate(&reversed).unwrap_err(), "terminal width minimum 800001 nm exceeds its maximum 800000 nm");
    let zero = LandPatternInput { lead_span: limits(0, 1_700_000), ..chip_0603() };
    assert_eq!(calculate(&zero).unwrap_err(), "lead span limits must be between 1 and 100000000 nm");
    let negative = LandPatternInput { placement_tolerance_nm: -1, ..chip_0603() };
    assert_eq!(calculate(&negative).unwrap_err(), "placement tolerance must be between 0 and 100000000 nm");
    assert!(calculate(&LandPatternInput { placement_tolerance_nm: 0, ..chip_0603() }).is_ok());
}

/// Terminals that reach each other inside the body leave no inner span to compute from.
#[test]
fn terminals_meeting_inside_the_body_are_refused() {
    let meeting = LandPatternInput { terminal_length: limits(200_000, 750_000), ..chip_0603() };
    assert_eq!(
        calculate(&meeting).unwrap_err(),
        "the terminals meet inside the body: a 1500000 nm minimum lead span leaves 0 nm between terminals of maximum length"
    );
    let narrow = LandPatternInput { terminal_length: limits(200_000, 749_999), ..chip_0603() };
    assert!(calculate(&narrow).is_ok());
}

/// A heel goal larger than the realistic inner span would make opposite lands meet.
#[test]
fn opposite_lands_that_would_meet_are_refused() {
    let long_feet = LandPatternInput { terminal_length: limits(400_000, 2_800_000), ..soic8() };
    assert!(calculate(&long_feet).unwrap_err().starts_with("the computed lands do not separate"));
}

/// Independently rounded centers and length can erase a positive equation gap.
#[test]
fn opposite_lands_must_separate_after_rounding() {
    let input = |span_nm| LandPatternInput {
        lead_span: limits(span_nm, span_nm),
        terminal_length: limits(400_000, 500_000),
        body_width: Some(limits(650_000, 700_000)),
        ..soic8()
    };
    assert_eq!(calculate(&input(1_738_857)).unwrap_err(), "opposite lands do not separate after rounding: 0 nm gap");
    let lands = calculate(&input(1_738_858)).unwrap().lands;
    let (left, right) = (&lands[0], &lands[7]);
    let left_end = left.center.x_nm + left.width_nm - left.width_nm / 2;
    let right_start = right.center.x_nm - right.width_nm / 2;
    assert!(right_start > left_end);
}

/// Full pad width is limited even when both edges lie inside the coordinate envelope.
#[test]
fn computed_lands_obey_the_native_full_size_limit() {
    let input = |terminal_width_nm| LandPatternInput {
        density: DensityLevel::A,
        lead_span: limits(1_600_000, 1_600_000),
        terminal_length: limits(300_000, 300_000),
        terminal_width: limits(terminal_width_nm, terminal_width_nm),
        fabrication_tolerance_nm: 0,
        placement_tolerance_nm: 0,
        ..chip_0603()
    };
    let exact = calculate(&input(MAX_OFFSET_NM - 100_000)).unwrap();
    assert_eq!(exact.lands[0].height_nm, MAX_OFFSET_NM);
    assert_eq!(
        calculate(&input(MAX_OFFSET_NM - 99_999)).unwrap_err(),
        "the computed lands exceed the 100000000 nm footprint size or coordinate limits"
    );
}

/// Adjacent gull-wing lands need a gap: the land width must stay below the pitch.
#[test]
fn adjacent_gull_wing_lands_must_leave_a_gap() {
    let width = calculate(&fine_pitch(500_000)).unwrap().land_width_nm;
    assert!(calculate(&fine_pitch(width + 1)).is_ok());
    assert_eq!(
        calculate(&fine_pitch(width)).unwrap_err(),
        format!("adjacent lands touch: the {width} nm land width is not less than the {width} nm pitch")
    );
}

/// Each family takes only its own fields, and gull-wing rows need an even pin count.
#[test]
fn family_fields_are_checked() {
    let chip_with_pitch = LandPatternInput { pitch_nm: Some(1_270_000), ..chip_0603() };
    assert_eq!(calculate(&chip_with_pitch).unwrap_err(), "a chip land pattern takes no pitch, pin count or body width");
    let no_pitch = LandPatternInput { pitch_nm: None, ..soic8() };
    assert_eq!(calculate(&no_pitch).unwrap_err(), "a gull-wing land pattern needs a pitch");
    let two = LandPatternInput { pin_count: Some(2), ..soic8() };
    assert_eq!(calculate(&two).unwrap().lands.len(), 2);
    let odd = LandPatternInput { pin_count: Some(7), ..soic8() };
    assert_eq!(calculate(&odd).unwrap_err(), "a gull-wing pin count must be even and between 2 and 1024, not 7");
}

/// The saved field names use the documented snake_case spelling.
#[test]
fn input_reads_documented_field_names() {
    let input: LandPatternInput = serde_json::from_value(serde_json::json!({
        "family": "gull_wing",
        "density": "b",
        "lead_span": { "min_nm": 5_800_000, "max_nm": 6_200_000 },
        "terminal_length": { "min_nm": 400_000, "max_nm": 1_270_000 },
        "terminal_width": { "min_nm": 310_000, "max_nm": 510_000 },
        "body_width": { "min_nm": 3_800_000, "max_nm": 4_000_000 },
        "pitch_nm": 1_270_000,
        "pin_count": 8,
        "fabrication_tolerance_nm": 100_000,
        "placement_tolerance_nm": 50_000
    }))
    .unwrap();
    assert_eq!(input, soic8());
}
