use super::*;

#[test]
fn placing_a_rounded_profile_keeps_its_arc_clearance() {
    let core = [[0, 0], [800_000, 0], [800_000, 400_000], [0, 400_000]];
    let origin = [-71_000_000, 42_000_000];
    let absolute = core.map(|[x, y]| [x + origin[0], y + origin[1]]);
    let local = Regions::rounded_core(&core, 210_001.5).unwrap();
    let placed = local.place_profile(origin).unwrap();
    let direct = Regions::rounded_core(&absolute, 210_001.5).unwrap();
    assert!((placed.area().unwrap().value_nm2 - direct.area().unwrap().value_nm2).abs() < 5.);
    let [x, y] = origin;
    assert_eq!(placed.contains([x - 210_001, y + 200_000]).unwrap(), Some(true));
    assert_eq!(placed.contains([x - 210_002, y + 200_000]).unwrap(), Some(false));
    assert!(local.place_profile([MAX_LENGTH_NM - 500_000, 0]).is_none());
}

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

/// Boundaries expose the exact stored lines and circle in nanometres, with no chords.
#[test]
fn boundaries_report_exact_lines_and_arcs_in_nanometres() {
    let fill = rectangle(0, 0, 10 * MM, 10 * MM).boolean(&Regions::circle([4 * MM, 5 * MM], 2 * MM + 1).unwrap(), Operation::Difference).unwrap();
    let boundaries = fill.boundaries().unwrap();
    assert_eq!(boundaries.len(), 1);
    let corners: Vec<_> = boundaries[0]
        .outer
        .iter()
        .map(|edge| match *edge {
            Edge::Line { start_nm } => start_nm.map(|v| v.round() as i64),
            Edge::Arc { .. } => panic!("the outer loop is straight: {edge:?}"),
        })
        .collect();
    assert_eq!(corners.len(), 4);
    for corner in [[0, 0], [10 * MM, 0], [10 * MM, 10 * MM], [0, 10 * MM]] {
        assert!(corners.contains(&corner), "{corner:?} in {corners:?}");
    }
    assert_eq!(boundaries[0].holes.len(), 1);
    let mut sweep = 0.;
    for edge in &boundaries[0].holes[0] {
        let Edge::Arc { start_nm, center_nm, radius_nm, sweep_rad } = *edge else { panic!("the hole is circular: {edge:?}") };
        assert!((center_nm[0] - 4e6).abs() < 1e-6 && (center_nm[1] - 5e6).abs() < 1e-6, "{center_nm:?}");
        assert!((radius_nm - 1_000_000.5).abs() < 1e-6, "the half-nanometre radius is kept: {radius_nm}");
        assert!(((start_nm[0] - center_nm[0]).hypot(start_nm[1] - center_nm[1]) - radius_nm).abs() < 1e-6);
        assert!(sweep_rad < 0., "holes run clockwise: {sweep_rad}");
        sweep += sweep_rad;
    }
    assert!((sweep + std::f64::consts::TAU).abs() < 1e-12, "the arcs close one full circle: {sweep}");
    assert!(Regions::default().boundaries().unwrap().is_empty());
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

#[test]
fn physical_capsule_and_rounded_land_use_the_original_core_and_radius() {
    let capsule = Regions::rounded_core(&[[0, 0], [2 * MM, 0], [2 * MM, 0], [0, 0]], 500_000.5).unwrap();
    let r = 0.5000005;
    area_is(&capsule, 4. * r + std::f64::consts::PI * r * r);
    let rounded = Regions::rounded_core(&[[0, 0], [2 * MM, 0], [2 * MM, MM], [0, MM]], 100_000.).unwrap();
    area_is(&rounded, 2. + 0.6 + std::f64::consts::PI * 0.01);
}

#[test]
fn shared_mesher_draws_a_planar_hole_without_covering_its_center() {
    let hole = rectangle(0, 0, 10 * MM, 10 * MM).boolean(&Regions::circle([5 * MM, 5 * MM], 2 * MM).unwrap(), Operation::Difference).unwrap();
    let mesh = hole.display_mesh(5_000).unwrap();
    assert!(!mesh.triangles.is_empty());
    let cross = |a: [f64; 2], b: [f64; 2], p: [f64; 2]| (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
    for [a, b, c] in mesh.triangles {
        let (a, b, c) = (mesh.vertices_nm[a as usize], mesh.vertices_nm[b as usize], mesh.vertices_nm[c as usize]);
        let signs = [cross(a, b, [5e6, 5e6]), cross(b, c, [5e6, 5e6]), cross(c, a, [5e6, 5e6])];
        assert!(signs.iter().any(|v| *v < 0.) && signs.iter().any(|v| *v > 0.), "a display triangle covers the clearance hole");
    }
}

#[test]
fn nanometre_clearance_guard_survives_clipping_near_the_boundary() {
    let outer = rectangle(-2 * MM, -2 * MM, 4 * MM, 4 * MM);
    let keepout = Regions::circle([0, 0], 1_400_004).unwrap();
    let copper = outer.boolean(&keepout, Operation::Difference).unwrap();
    assert_eq!(copper.contains([699_999, 0]).unwrap(), Some(false));
    assert_eq!(copper.contains([700_001, 0]).unwrap(), Some(false));
    assert_eq!(copper.contains([700_004, 0]).unwrap(), Some(true));
    let forbidden = Regions::circle([0, 0], 1_400_000).unwrap();
    assert!(copper.boolean(&forbidden, Operation::Intersection).unwrap().is_empty());
}

/// Independent kernel copies preserve holes and a separate island inside a hole; warming,
/// cloning and serializing a source never adopt a Boolean operation's changed material.
#[test]
fn prepared_operands_preserve_nested_material_and_transport() {
    let outer = rectangle(0, 0, 10 * MM, 10 * MM);
    let void = rectangle(2 * MM, 2 * MM, 6 * MM, 6 * MM);
    let ring = outer.boolean(&void, Operation::Difference).unwrap();
    let center = rectangle(4 * MM, 4 * MM, 2 * MM, 2 * MM);
    let nested = ring.boolean(&center, Operation::Union).unwrap();
    let bytes = nested.worker_bytes().unwrap();
    let clone = nested.clone();
    area_is(&clone.boolean(&void, Operation::Intersection).unwrap(), 4.);
    area_is(&clone.boolean(&center, Operation::Difference).unwrap(), 64.);
    area_is(&nested, 68.);
    assert_eq!(nested.worker_bytes().unwrap(), bytes);
    let decoded = Regions::from_worker_bytes(&bytes).unwrap();
    assert_eq!(nested, decoded);
    area_is(&decoded.boolean(&void, Operation::Intersection).unwrap(), 4.);
    assert!(!clone.display_mesh(5_000).unwrap().triangles.is_empty());
    assert_eq!(clone.worker_bytes().unwrap(), bytes);
}

/// Concurrent consumers may populate geometry memos, but each operation owns an independent
/// body and kernel. Sharing a prepared source must not change its represented material.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn prepared_operands_support_concurrent_independent_operations() {
    let source = rectangle(0, 0, 10 * MM, 10 * MM).boolean(&Regions::circle([5 * MM, 5 * MM], 2 * MM).unwrap(), Operation::Difference).unwrap();
    let bytes = source.worker_bytes().unwrap();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let source = &source;
            scope.spawn(move || {
                let probe = rectangle(0, 0, MM, MM);
                area_is(&source.boolean(&probe, Operation::Intersection).unwrap(), 1.);
                assert!(source.boolean(&Regions::circle([5 * MM, 5 * MM], MM).unwrap(), Operation::Intersection).unwrap().is_empty());
            });
        }
    });
    assert_eq!(source.worker_bytes().unwrap(), bytes);
    area_is(&source, 100. - std::f64::consts::PI);
}

/// A track's clearance edge passing 2 nm from the point where a pad clearance's corner arc
/// meets its straight side, as two foreign obstacles produce with the fill's clearance reserve
/// (a 0.5 mm track entering a 0603 land on a real board). SolidCore resolves the crossing
/// onto the arc's end vertex within its internal zone tolerance but keeps the arc's full sweep,
/// so the loop read back has a 2 nm gap; it must still be read as closed material.
#[test]
fn intersection_within_kernel_zone_tolerance_of_an_arc_end_reads_back_closed() {
    let land =
        Regions::rounded_core(&[[81_150_000, 98_525_000], [81_550_000, 98_525_000], [81_550_000, 99_075_000], [81_150_000, 99_075_000]], 500_002.)
            .unwrap();
    let track = Regions::rounded_core(&[[81_000_000, 99_150_000], [81_000_000, 111_720_000]], 550_002.).unwrap();
    let union = Regions::union_all(&[land.clone(), track.clone()]).unwrap();
    assert_eq!(union.region_count(), 1);
    let shared = land.boolean(&track, Operation::Intersection).unwrap();
    let [u, l, t, s] = [&union, &land, &track, &shared].map(|r| r.area().unwrap().value_nm2);
    assert!((u - (l + t - s)).abs() < 1e3, "union {u} nm² != {l} + {t} - {s}");
    let pair = land.boolean(&track, Operation::Union).unwrap();
    // The fill feeds such results back to the kernel from their profiles (worker transport,
    // batch unions), where vertices are merged within the kernel's own zone tolerance.
    let reread = Regions::from_worker_bytes(&pair.worker_bytes().unwrap()).unwrap();
    assert_eq!(reread.region_count(), 1);
    let board = rectangle(70 * MM, 90 * MM, 20 * MM, 30 * MM);
    let copper = board.boolean(&reread, Operation::Difference).unwrap();
    area_is(&copper, 600. - u / 1e12);
    let again = Regions::union_all(&[reread, rectangle(75 * MM, 95 * MM, MM, MM)]).unwrap();
    assert_eq!(again.region_count(), 2);
}

/// Individually proven curved holes use one oriented face and retain the same material,
/// winding, and visible voids as the general Boolean operation.
#[test]
fn independently_enclosed_cuts_preserve_the_exact_material() {
    let stock = Regions::polygon(&[[0, 0], [10_000_000, 0], [10_000_000, 10_000_000], [0, 10_000_000]]).unwrap();
    let centers = [[2_000_000, 3_000_000], [5_000_000, 5_000_000], [8_000_000, 7_000_000]];
    let cuts = centers.map(|center| Regions::circle(center, 1_000_000).unwrap());
    let expected = stock.boolean(&Regions::union_all(&cuts).unwrap(), Operation::Difference).unwrap();
    let fast = stock.subtract_enclosed(&cuts).unwrap().unwrap();
    let measured = fast.area().unwrap().value_nm2;
    assert!((expected.area().unwrap().value_nm2 - measured).abs() < 100.);
    for center in centers {
        assert_eq!(fast.contains(center).unwrap(), Some(false));
        assert_eq!(fast.contains([center[0] + 499_990, center[1]]).unwrap(), Some(false));
        assert_eq!(fast.contains([center[0] + 500_010, center[1]]).unwrap(), Some(true));
    }

    // Force the kernel to consume the prepared face, not just the analytic profile.
    let mesh = fast.display_mesh(5_000).unwrap();
    assert!(!mesh.triangles.is_empty());
    let area: f64 = mesh
        .triangles
        .iter()
        .map(|indices| {
            let [a, b, c] = indices.map(|i| mesh.vertices_nm[i as usize]);
            ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.
        })
        .sum();
    assert!((measured - area).abs() < 100_000_000_000., "{measured} vs {area}");
    assert!(stock.subtract_enclosed(&[cuts[0].clone(), cuts[0].clone()]).unwrap().is_none());
    let edge = Regions::circle([0, 6_000_000], 1_000_000).unwrap();
    assert!(stock.subtract_enclosed(&[edge]).unwrap().is_none());
}

/// Local subtraction preserves a copper island inside a closed clearance and the separate
/// foreign feature within that island. The assembled face is also usable by the kernel.
#[test]
fn enclosed_nested_cuts_keep_the_copper_between_them() {
    let stock = rectangle(0, 0, 30 * MM, 30 * MM);
    let outer = rectangle(4 * MM, 4 * MM, 12 * MM, 12 * MM);
    let inner = rectangle(6 * MM, 6 * MM, 8 * MM, 8 * MM);
    let ring = outer.boolean(&inner, Operation::Difference).unwrap();
    let dot = Regions::circle([10 * MM, 10 * MM], 2 * MM).unwrap();
    let cut = Regions::union_all(&[ring, dot]).unwrap();
    let expected = stock.boolean(&cut, Operation::Difference).unwrap();
    let actual = stock.subtract_enclosed(&[cut]).unwrap().unwrap();
    assert_eq!(actual.region_count(), 2);
    assert!((actual.area().unwrap().value_nm2 - expected.area().unwrap().value_nm2).abs() < 1_000.);
    for (x, expected) in [(2, true), (5, false), (8, true), (10, false)] {
        assert_eq!(actual.contains([x * MM, 10 * MM]).unwrap(), Some(expected));
    }
    let probe = rectangle(7 * MM, 8 * MM, MM, MM);
    area_is(&actual.boolean(&probe, Operation::Intersection).unwrap(), 1.);
}

/// Independent source groups can have nested envelopes. A clearance inside the trapped
/// copper must cut that island, and the island must still work as a later Boolean operand.
#[test]
fn independent_cut_inside_another_groups_void_uses_the_inner_face() {
    let stock = rectangle(0, 0, 30 * MM, 30 * MM);
    let outer = rectangle(4 * MM, 4 * MM, 12 * MM, 12 * MM);
    let inner = rectangle(6 * MM, 6 * MM, 8 * MM, 8 * MM);
    let ring = outer.boolean(&inner, Operation::Difference).unwrap();
    let dot = Regions::circle([10 * MM, 10 * MM], 2 * MM).unwrap();
    let expected = stock.boolean(&Regions::union_all(&[ring.clone(), dot.clone()]).unwrap(), Operation::Difference).unwrap();
    for cuts in [[ring.clone(), dot.clone()], [dot, ring]] {
        let actual = stock.subtract_independent_enclosed(&cuts).unwrap().unwrap();
        assert_eq!(actual.region_count(), 2);
        assert!((actual.area().unwrap().value_nm2 - expected.area().unwrap().value_nm2).abs() < 1_000.);
        for (x, retained) in [(2, true), (5, false), (8, true), (10, false)] {
            assert_eq!(actual.contains([x * MM, 10 * MM]).unwrap(), Some(retained));
        }
        let probe = rectangle(7 * MM, 8 * MM, 4 * MM, MM);
        let reference = expected.boolean(&probe, Operation::Intersection).unwrap();
        let overlap = actual.boolean(&probe, Operation::Intersection).unwrap();
        assert!((overlap.area().unwrap().value_nm2 - reference.area().unwrap().value_nm2).abs() < 1_000.);
    }
}

/// Boundary clipping can split the stock. Enclosed cuts still go in their respective
/// faces without repacking them into the boundary Boolean.
#[test]
fn independent_cuts_survive_boundary_clipping_that_splits_stock() {
    let stock = rectangle(0, 0, 20 * MM, 20 * MM);
    let crossing = rectangle(9 * MM, -MM, 2 * MM, 22 * MM);
    let stock = stock.boolean(&crossing, Operation::Difference).unwrap();
    let cuts = [[5 * MM, 10 * MM], [15 * MM, 10 * MM]].map(|p| Regions::circle(p, 2 * MM).unwrap());
    let actual = stock.subtract_independent_enclosed(&cuts).unwrap().unwrap();
    let expected = stock.boolean(&Regions::union_all(&cuts).unwrap(), Operation::Difference).unwrap();
    assert_eq!(actual.region_count(), 2);
    assert!((actual.area().unwrap().value_nm2 - expected.area().unwrap().value_nm2).abs() < 1_000.);
    for (x, retained) in [(3, true), (5, false), (10, false), (15, false), (17, true)] {
        assert_eq!(actual.contains([x * MM, 10 * MM]).unwrap(), Some(retained));
    }
}

/// A contact box in a clearance hole or outside a diagonal edge can be rejected. Partial
/// contact, including when the contour surrounds a piece of material, must still be tested.
#[test]
fn contact_box_rejection_keeps_boundary_and_partial_contacts() {
    let stock = Regions::polygon(&[[0, 0], [12 * MM, 0], [0, 12 * MM]]).unwrap();
    let hole = Regions::circle([4 * MM, 4 * MM], 3_800_000).unwrap();
    let material = stock.subtract_enclosed(&[hole]).unwrap().unwrap();
    let may_intersect = |box_nm: [i64; 4]| material.may_intersect_box(box_nm.map(|v| v as f64)).unwrap();
    assert!(!may_intersect([3 * MM, 3 * MM, 5 * MM, 5 * MM]));
    assert!(!may_intersect([7 * MM, 7 * MM, 8 * MM, 8 * MM]));
    assert!(may_intersect([5 * MM, 4 * MM, 6 * MM, 5 * MM]));
    assert!(may_intersect([5 * MM, 6 * MM, 7 * MM, 7 * MM]));
    assert!(may_intersect([-MM, -MM, 13 * MM, 13 * MM]));
}

/// A covered hole can leave the union calculation; partial cover and exterior
/// material must remain in the Boolean operands.
#[test]
fn covered_holes_preserve_the_union_with_adjacent_cutters() {
    let plane = rectangle(0, 0, 10 * MM, 10 * MM);
    let first = Regions::circle([3 * MM, 5 * MM], MM).unwrap();
    let second = Regions::circle([7 * MM, 5 * MM], 2 * MM).unwrap();
    let holed = plane.boolean(&first, Operation::Difference).unwrap().boolean(&second, Operation::Difference).unwrap();
    let whole_cover = Regions::circle([3 * MM, 5 * MM], 2 * MM).unwrap();
    let partial_cover = rectangle(7 * MM, 4 * MM, 4 * MM, 2 * MM);
    let filled = holed.fill_covered_holes(|_| vec![&whole_cover, &partial_cover]).unwrap().unwrap();
    assert!(filled.covers_box(whole_cover.bounds_nm().unwrap()).unwrap());
    assert!(!filled.covers_box(partial_cover.bounds_nm().unwrap()).unwrap());
    assert_eq!(filled.contains([6_500_000, 5 * MM]).unwrap(), Some(false));
    let expected = Regions::union_all(&[holed, whole_cover, partial_cover.clone()]).unwrap();
    let actual = Regions::union_all(&[filled, partial_cover]).unwrap();
    assert!(actual.boolean(&expected, Operation::Difference).unwrap().is_empty());
    assert!(expected.boolean(&actual, Operation::Difference).unwrap().is_empty());
}
