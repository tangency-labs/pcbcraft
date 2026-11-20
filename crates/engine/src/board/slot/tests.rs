use super::*;
use crate::board::definition::PadShape;

const WIDTH_NM: i64 = 560_000;
const LENGTH_NM: i64 = 1_560_000;
const ORIGIN: Point = Point { x_nm: 0, y_nm: 0 };

/// Make nanometer coordinates explicit in the geometric oracles.
fn point(x_nm: i64, y_nm: i64) -> Point {
    Point { x_nm, y_nm }
}

/// Standard synthetically dimensioned slot; this is not a vendor footprint or a design rule.
fn horizontal_slot() -> ResolvedPlatedSlot {
    PlatedSlot::new(WIDTH_NM, LENGTH_NM, 0).unwrap().resolve(ORIGIN, 0, false).unwrap()
}

/// Inclusive dimension limits, non-canonical angles and invalid placement centres.
#[test]
fn rejects_invalid_finished_sizes_and_angles() {
    let circle = PlatedSlot::new(1, 1, FULL_TURN_MDEG - 1).unwrap();
    assert_eq!(circle.dimensions(), (1, 1, FULL_TURN_MDEG - 1));
    assert_eq!(circle.resolve(ORIGIN, 0, false).unwrap().contour().bounds(), [-0.5, -0.5, 0.5, 0.5]);
    let max = PlatedSlot::new(MAX_OFFSET_NM, MAX_OFFSET_NM, 0).unwrap();
    assert!(max.resolve(point(MAX_PAD_CENTER_NM, -MAX_PAD_CENTER_NM), 0, false).is_some());
    assert!(max.resolve(point(MAX_PAD_CENTER_NM + 1, 0), 0, false).is_none());
    assert!(max.resolve(point(0, -MAX_PAD_CENTER_NM - 1), 0, false).is_none());
    for (width, length, rotation) in [(0, 1, 0), (2, 1, 0), (1, MAX_OFFSET_NM + 1, 0), (1, 1, FULL_TURN_MDEG)] {
        assert!(
            matches!(PlatedSlot::new(width, length, rotation), Err(CommandError::InvalidParams(message)) if message.contains("plated slot needs"))
        );
    }
}

/// Verify combined local/placement rotation and the X mirror applied before rotation. An
/// axis-aligned symmetric capsule would hide a missing mirror, so use a non-cardinal angle.
#[test]
fn placement_rotation_and_bottom_transform_the_spine() {
    let center = point(2_000_000, -1_000_000);
    let slot = PlatedSlot::new(WIDTH_NM, LENGTH_NM, 30_500).unwrap();
    assert_eq!(slot.resolve(center, 0, false).unwrap().cap_centres(), [point(1_569_185, -746_231), point(2_430_815, -1_253_769)]);
    assert_eq!(slot.resolve(center, 90, false).unwrap().cap_centres(), [point(2_253_769, -569_185), point(1_746_231, -1_430_815)]);
    assert_eq!(slot.resolve(center, 90, true).unwrap().cap_centres(), [point(2_253_769, -1_430_815), point(1_746_231, -569_185)]);
    assert_eq!(slot.resolve(center, 0, false).unwrap().width_nm(), WIDTH_NM);
}

/// The cap is an obstacle at the end of the hole, not a circle at the pad centre. Probes
/// use a synthetic radius so the test can check exact contact and the next nanometer gap.
#[test]
fn clearance_uses_round_caps_and_straight_sides() {
    let slot = horizontal_slot().contour();
    assert_eq!(slot.bounds(), [-780_000., -280_000., 780_000., 280_000.]);
    for (touch, outside) in [(point(800_000, 0), point(800_001, 0)), (point(0, 300_000), point(0, 300_001))] {
        assert!(slot.touches(&Contour::capsule(touch, touch, 20_000.)));
        assert_eq!(slot.gap(&Contour::capsule(outside, outside, 20_000.)), 1.);
    }
    let diagonal = point(780_000, 280_000);
    assert!(!slot.touches(&Contour::capsule(diagonal, diagonal, 20_000.)), "the bounds corner is not part of the hole");
}

/// The narrowest ring can be at an end even when both straight sides have plenty of
/// copper. A circle with the same width, an AABB comparison, or a zero-clamped ring
/// would each misclassify at least one of these independent land shapes.
#[test]
fn annular_ring_checks_end_margin_and_breakout_independent_of_land_shape() {
    let slot = horizontal_slot();
    let rectangle = Contour::pad(
        ORIGIN,
        [point(-800_000, -420_000), point(800_000, -420_000), point(800_000, 420_000), point(-800_000, 420_000)],
        PadShape::Rectangle,
        0,
        (1_600_000, 840_000),
    );
    assert_eq!(slot.annular_ring_nm(&rectangle), 20_000.);
    let same_bounds_circle = Contour::capsule(ORIGIN, ORIGIN, 779_999.);
    assert_eq!(slot.annular_ring_nm(&same_bounds_circle), -1.);
    let concentric_obround = Contour::capsule(point(-500_000, 0), point(500_000, 0), 370_000.);
    assert_eq!(slot.annular_ring_nm(&concentric_obround), 90_000.);
}
