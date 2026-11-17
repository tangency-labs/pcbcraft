use super::*;

const MM: i64 = 1_000_000;

/// Original rectangular profiles in nanometres, independent of kernel-generated expectations.
fn rectangle(x: i64, y: i64, width: i64, height: i64) -> Regions {
    Regions::polygon(&[[x, y], [x + width, y], [x + width, y + height], [x, y + height]]).unwrap()
}

/// Closed-form area in mm²; the adapter reports nm² and a separate numerical bound.
fn area_is(regions: &Regions, expected_mm2: f64) {
    let area = regions.area().unwrap();
    assert!((area.value_nm2 / 1e12 - expected_mm2).abs() < 1e-7, "{area:?}, expected {expected_mm2} mm²");
    assert!(area.error_bound_nm2.is_finite() && area.error_bound_nm2 >= 0.);
}

#[test]
fn circular_clearance_is_a_hole_with_exact_curved_area() {
    let board = rectangle(0, 0, 10 * MM, 10 * MM);
    let land = Regions::circle([5 * MM, 5 * MM], 2 * MM).unwrap();
    let fill = board.boolean(&land, Operation::Difference).unwrap();
    area_is(&fill, 100. - std::f64::consts::PI);
    assert_eq!(fill.contains([5 * MM, 5 * MM]).unwrap(), Some(false));
    assert_eq!(fill.contains([2 * MM, 5 * MM]).unwrap(), Some(true));
    assert_eq!(fill.contains([0, 5 * MM]).unwrap(), None);
    assert_eq!(fill.regions[0].holes.len(), 1);
    assert!(fill.regions[0].holes[0].segs.iter().all(|s| matches!(s, solidcore_record::Seg2::Arc { .. })));
    // A Boolean is immutable: making the void did not change either operand.
    area_is(&board, 100.);
    area_is(&land, std::f64::consts::PI);
}

#[test]
fn crossing_obstacle_splits_material_and_islands_keep_holes() {
    let board = rectangle(0, 0, 10 * MM, 10 * MM);
    let cut = rectangle(4 * MM, -MM, 2 * MM, 12 * MM);
    let hole = Regions::circle([2 * MM, 5 * MM], MM).unwrap();
    let fill = board.boolean(&cut, Operation::Difference).unwrap().boolean(&hole, Operation::Difference).unwrap();
    assert_eq!(fill.region_count(), 2);
    area_is(&fill, 80. - std::f64::consts::PI / 4.);
    assert_eq!(fill.islands().map(|r| r.regions[0].holes.len()).sum::<usize>(), 1);
    assert!(fill.boolean(&cut, Operation::Intersection).unwrap().is_empty());
}

#[test]
fn expansion_has_round_corners_and_erosion_retains_material_side() {
    let board = rectangle(0, 0, 10 * MM, 10 * MM);
    area_is(&board.offset(MM).unwrap(), 140. + std::f64::consts::PI);
    area_is(&board.offset(-MM).unwrap(), 64.);
    assert_eq!(board.offset(0).unwrap(), board);
    assert!(Regions::default().offset(MM).unwrap().is_empty());
}

#[test]
fn holes_shrink_on_expansion_and_grow_on_erosion() {
    let board = rectangle(0, 0, 10 * MM, 10 * MM);
    let hole = Regions::circle([5 * MM, 5 * MM], 4 * MM).unwrap();
    let donut = board.boolean(&hole, Operation::Difference).unwrap();
    area_is(&donut.offset(MM).unwrap(), 140.);
    area_is(&donut.offset(-MM).unwrap(), 64. - 9. * std::f64::consts::PI);
}

#[test]
fn inward_offset_can_split_a_narrow_neck() {
    let points = [
        [0, 0],
        [4_000_000, 0],
        [4_000_000, 1_500_000],
        [6_000_000, 1_500_000],
        [6_000_000, 0],
        [10_000_000, 0],
        [10_000_000, 4_000_000],
        [6_000_000, 4_000_000],
        [6_000_000, 2_500_000],
        [4_000_000, 2_500_000],
        [4_000_000, 4_000_000],
        [0, 4_000_000],
    ];
    let outline = Regions::polygon(&points).unwrap();
    let inset = outline.offset(-600_000).unwrap();
    assert_eq!(inset.region_count(), 2);
    assert_eq!(inset.contains([5 * MM, 2 * MM]).unwrap(), Some(false));
    assert_eq!(inset.contains([2 * MM, 2 * MM]).unwrap(), Some(true));
}

#[test]
fn odd_diameters_keep_half_nanometre_radius_and_large_coordinates_work() {
    let tiny = Regions::circle([0, 0], 10_001).unwrap();
    let area = tiny.area().unwrap().value_nm2;
    assert!((area - std::f64::consts::PI * 5_000.5_f64.powi(2)).abs() < 0.001);
    let base = MAX_LENGTH_NM - 20 * MM;
    let board = rectangle(base, base, 10 * MM, 10 * MM);
    let cut = Regions::circle([base, base + 5 * MM], 2 * MM).unwrap();
    let fill = board.boolean(&cut, Operation::Difference).unwrap();
    area_is(&fill, 100. - std::f64::consts::PI / 2.);
}

#[test]
fn empty_boolean_algebra_and_winding_are_explicit() {
    let board = rectangle(0, 0, MM, MM);
    let clockwise = Regions::polygon(&[[0, 0], [0, MM], [MM, MM], [MM, 0]]).unwrap();
    area_is(&clockwise, 1.);
    assert_eq!(board.boolean(&Regions::default(), Operation::Union).unwrap(), board);
    assert!(board.boolean(&Regions::default(), Operation::Intersection).unwrap().is_empty());
    assert!(board.boolean(&board, Operation::Difference).unwrap().is_empty());
}

#[test]
fn invalid_arguments_are_rejected_and_do_not_poison_later_calls() {
    let invalids = [
        Regions::polygon(&[[0, 0], [1, 0]]),
        Regions::polygon(&[[0, 0], [1, 0], [0, 0]]),
        Regions::circle([0, 0], 0),
        Regions::circle([MAX_LENGTH_NM, 0], 1),
        Regions::polygon(&[[MAX_LENGTH_NM + 1, 0], [0, 1], [1, 0]]),
    ];
    for result in invalids {
        assert_eq!(result.unwrap_err().code(), ErrorCode::InvalidParams);
    }
    let board = rectangle(0, 0, MM, MM);
    assert_eq!(board.contains([MAX_LENGTH_NM + 1, 0]).unwrap_err().code(), ErrorCode::InvalidParams);
    assert_eq!(board.offset(i64::MIN).unwrap_err().code(), ErrorCode::InvalidParams);
    area_is(&board.boolean(&board, Operation::Intersection).unwrap(), 1.);
}

#[test]
fn repeated_identical_operations_return_identical_derived_profiles() {
    let board = rectangle(0, 0, 10 * MM, 10 * MM);
    let hole = Regions::circle([5 * MM, 5 * MM], 2 * MM).unwrap();
    assert_eq!(board.boolean(&hole, Operation::Difference).unwrap(), board.boolean(&hole, Operation::Difference).unwrap());
}

#[test]
fn covering_cuts_and_disjoint_intersections_produce_proved_empty_material() {
    let board = rectangle(0, 0, MM, MM);
    let cover = rectangle(-MM, -MM, 3 * MM, 3 * MM);
    assert!(board.boolean(&cover, Operation::Difference).unwrap().is_empty());
    let disjoint = rectangle(4 * MM, 0, MM, MM);
    assert!(board.boolean(&disjoint, Operation::Intersection).unwrap().is_empty());
    let touching = rectangle(MM, 0, MM, MM);
    assert!(board.boolean(&touching, Operation::Intersection).unwrap().is_empty());
}

#[test]
fn derived_coordinate_overflow_is_refused_without_changing_the_operand() {
    let board = rectangle(MAX_LENGTH_NM - MM, 0, MM, MM);
    assert_eq!(board.offset(MM).unwrap_err().code(), ErrorCode::OperationFailed);
    area_is(&board, 1.);
}

#[test]
fn expansion_removes_a_collapsed_hole_instead_of_reversing_it() {
    let board = rectangle(0, 0, 10 * MM, 10 * MM);
    let hole = Regions::circle([5 * MM, 5 * MM], 4 * MM).unwrap();
    let donut = board.boolean(&hole, Operation::Difference).unwrap();
    let expanded = donut.offset(3 * MM).unwrap();
    assert_eq!(expanded.contains([5 * MM, 5 * MM]).unwrap(), Some(true));
    area_is(&expanded, 220. + 9. * std::f64::consts::PI);
}

#[test]
fn multi_island_offset_preserves_holes_and_merges_expanding_neighbours() {
    let left = rectangle(0, 0, 4 * MM, 4 * MM);
    let hole = Regions::circle([2 * MM, 2 * MM], MM).unwrap();
    let ring = left.boolean(&hole, Operation::Difference).unwrap();
    let right = rectangle(5 * MM, 0, 4 * MM, 4 * MM);
    let separate = ring.boolean(&right, Operation::Union).unwrap();
    assert_eq!(separate.region_count(), 2);
    let expanded = separate.offset(200_000).unwrap();
    assert_eq!(expanded.region_count(), 2);
    assert_eq!(expanded.contains([2 * MM, 2 * MM]).unwrap(), Some(false));
    let merged = separate.offset(600_000).unwrap();
    assert_eq!(merged.region_count(), 1);
    assert_eq!(merged.contains([4_500_000, 2 * MM]).unwrap(), Some(true));
    assert_eq!(merged.contains([2 * MM, 2 * MM]).unwrap(), Some(true));
    area_is(&separate, 32. - std::f64::consts::PI / 4.);
}
