//! Integration tests for circular arc copper contact, connectivity and short detection.
use super::*;
use crate::board::{
    drc::Kind,
    teardrop::{EDIT_TEARDROPS, Length, Parameters, Record, UNSUPPORTED as TEARDROP_UNSUPPORTED},
    track::{CopperItems, IslandBuilder, lands, short_for_copper_edit},
};

/// Build a semicircular arc connecting two horizontally aligned points `(a.x_nm, y)` and `(b.x_nm, y)`.
fn semicircle_between(id: u128, layer: Uuid, a: Point, b: Point, width_nm: i64, direction: Direction) -> ArcTrack {
    assert_eq!(a.y_nm, b.y_nm);
    let center = Point { x_nm: (a.x_nm + b.x_nm) / 2, y_nm: a.y_nm };
    ArcTrack { id: Uuid::from_u128(id), layer, width_nm, center, start: a, end: b, direction }
}

/// An arc connecting two same-net pads merges their islands, resolves the net, clears their
/// ratsnest airwire, and reports no shorts.
#[test]
fn arc_joins_same_net_pads_and_clears_ratsnest_airwire() {
    let mut s = routed_fixture();
    let front = copper_layers(&s)[0];
    let board_lands = lands(view(&s));
    let mut accepted: Vec<_> = board_lands.iter().filter(|l| l.net.as_deref() == Some("ACCEPTED")).collect();
    accepted.sort_by_key(|l| l.center.x_nm);
    assert_eq!(accepted.len(), 3);

    let before_edges = view(&s).ratsnest().unwrap().edges.iter().filter(|e| e.net == "ACCEPTED").count();
    assert_eq!(before_edges, 2);

    // Semicircle in +Y (Clockwise from right to left in X-right Y-down passes +pi/2) avoids neighboring pads.
    let arc = semicircle_between(0xc001, front, accepted[1].center, accepted[0].center, 250_000, Direction::Clockwise);
    s.apply(SET_ARCS, &params(&s, std::slice::from_ref(&arc))).unwrap();

    let v = view(&s);
    let (copper, nets) = v.copper_with_nets().unwrap();
    let island0 = copper.pads[&accepted[0].key];
    let island1 = copper.pads[&accepted[1].key];
    let island2 = copper.pads[&accepted[2].key];
    assert_eq!(island0, island1);
    assert_eq!(copper.arcs[&arc.id], island0);
    assert_ne!(island0, island2);
    assert_eq!(nets.get(&arc.id).map(String::as_str), Some("ACCEPTED"));
    assert_eq!(v.copper_nets().unwrap(), nets);
    assert_eq!(v.copper().unwrap(), copper);

    let after_edges = v.ratsnest().unwrap().edges.iter().filter(|e| e.net == "ACCEPTED").count();
    assert_eq!(after_edges, 1);
    let shorts = v.copper_shorts().unwrap();
    assert_eq!(shorts.total, 0);
    assert!(shorts.violations.is_empty());
}

/// Arcs respect copper layer isolation and join multi-layer chains through vias.
#[test]
fn arc_layer_isolation_and_multilayer_via_chain() {
    let mut s = routed_fixture();
    let back = copper_layers(&s)[1];
    let board_lands = lands(view(&s));
    let mut accepted: Vec<_> = board_lands.iter().filter(|l| l.net.as_deref() == Some("ACCEPTED")).collect();
    accepted.sort_by_key(|l| l.center.x_nm);

    // Back-layer arc over Front-only SMD pads does NOT connect them.
    let back_arc = semicircle_between(0xc002, back, accepted[1].center, accepted[0].center, 250_000, Direction::Clockwise);
    s.apply(SET_ARCS, &params(&s, std::slice::from_ref(&back_arc))).unwrap();
    let v = view(&s);
    let (copper, nets) = v.copper_with_nets().unwrap();
    assert_ne!(copper.pads[&accepted[0].key], copper.pads[&accepted[1].key]);
    assert_ne!(copper.arcs[&back_arc.id], copper.pads[&accepted[0].key]);
    assert!(!nets.contains_key(&back_arc.id));
    assert_eq!(v.ratsnest().unwrap().edges.iter().filter(|e| e.net == "ACCEPTED").count(), 2);

    // Remove the arc, place through vias at both pad centers, then re-add the Back-layer arc.
    s.apply(SET_ARCS, &params(&s, &[])).unwrap();
    let board = view(&s).target.board;
    for pad in [accepted[0], accepted[1]] {
        s.apply(crate::ADD_VIA, &json!({"board":board,"at":{"x_nm":pad.center.x_nm,"y_nm":pad.center.y_nm},"expected_revision":s.revision()}))
            .unwrap();
    }
    let via_ids: Vec<_> = view(&s).vias().iter().map(|v| v.id).collect();
    assert_eq!(via_ids.len(), 2);
    s.apply(SET_ARCS, &params(&s, std::slice::from_ref(&back_arc))).unwrap();

    let v = view(&s);
    let (copper, nets) = v.copper_with_nets().unwrap();
    let island = copper.pads[&accepted[0].key];
    assert_eq!(copper.pads[&accepted[1].key], island);
    assert_eq!(copper.arcs[&back_arc.id], island);
    for id in &via_ids {
        assert_eq!(copper.vias[id], island);
        assert_eq!(nets.get(id).map(String::as_str), Some("ACCEPTED"));
    }
    assert_eq!(nets.get(&back_arc.id).map(String::as_str), Some("ACCEPTED"));
    assert_eq!(v.ratsnest().unwrap().edges.iter().filter(|e| e.net == "ACCEPTED").count(), 1);
    assert_eq!(v.copper_shorts().unwrap().total, 0);
}

/// An isolated via placed away from every pad reaches a pad solely through an arc, toggling
/// `connected_to_pad` and resolved net in `inspect_vias()`.
#[test]
fn via_connected_to_pad_solely_through_arc() {
    let mut s = routed_fixture();
    let front = copper_layers(&s)[0];
    let board = view(&s).target.board;
    let pad = lands(view(&s)).into_iter().find(|l| l.net.as_deref() == Some("ACCEPTED")).unwrap();
    let via_at = Point { x_nm: pad.center.x_nm, y_nm: pad.center.y_nm + 4_000_000 };
    s.apply(crate::ADD_VIA, &json!({"board":board,"at":{"x_nm":via_at.x_nm,"y_nm":via_at.y_nm},"expected_revision":s.revision()})).unwrap();
    let before = view(&s).inspect_vias();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0]["connected_to_pad"], false);
    assert_eq!(before[0]["net"], Value::Null);

    // Connect pad to via exclusively through a semicircle on Front copper.
    let center = Point { x_nm: pad.center.x_nm, y_nm: (pad.center.y_nm + via_at.y_nm) / 2 };
    let arc = ArcTrack {
        id: Uuid::from_u128(0xc004),
        layer: front,
        width_nm: 250_000,
        center,
        start: pad.center,
        end: via_at,
        direction: Direction::Clockwise,
    };
    s.apply(SET_ARCS, &params(&s, &[arc])).unwrap();

    let after = view(&s).inspect_vias();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0]["connected_to_pad"], true);
    assert_eq!(after[0]["net"], "ACCEPTED");
    assert!(after[0].get("geometry_error").is_none());
}

/// An arc bridging two distinct intended nets reports a copper short, omits display net
/// membership for the conflicting island, and triggers edit short detection.
#[test]
fn arc_touching_foreign_copper_reports_short_and_omits_display_net() {
    let mut s = routed_fixture();
    let front = copper_layers(&s)[0];
    let board_lands = lands(view(&s));
    let pad_a = board_lands.iter().find(|l| l.net.as_deref() == Some("ACCEPTED")).unwrap().clone();
    let pad_b =
        board_lands.iter().find(|l| l.net.is_some() && l.net.as_deref() != Some("ACCEPTED") && l.center.y_nm == pad_a.center.y_nm).unwrap().clone();

    let arc = semicircle_between(0xc003, front, pad_a.center, pad_b.center, 250_000, Direction::Clockwise);
    s.apply(SET_ARCS, &params(&s, std::slice::from_ref(&arc))).unwrap();

    let v = view(&s);
    let (copper, nets) = v.copper_with_nets().unwrap();
    assert_eq!(copper.pads[&pad_a.key], copper.pads[&pad_b.key]);
    assert_eq!(copper.arcs[&arc.id], copper.pads[&pad_a.key]);
    assert!(!nets.contains_key(&arc.id));

    let shorts = v.copper_shorts().unwrap();
    assert_eq!(shorts.total, 1);
    assert_eq!(shorts.violations.len(), 1);
    assert_eq!(shorts.violations[0].kind, Kind::Short);
    assert_eq!(shorts.violations[0].nets.len(), 2);
    assert!(shorts.violations[0].items.contains(&arc.id.to_string()));
    assert!(shorts.violations[0].items.contains(&format!("{}:{}", pad_a.key.0, pad_a.key.1)));
    assert!(shorts.violations[0].items.contains(&format!("{}:{}", pad_b.key.0, pad_b.key.1)));

    let edit_short = short_for_copper_edit(s.project().unwrap(), &BTreeSet::from([arc.id]), &BTreeSet::new()).unwrap();
    assert!(edit_short.is_some());
}

/// `short_for_copper_edit` fails closed when a snapshot carries an invalid circular arc rather
/// than reporting "no short" and allowing the edit.
#[test]
fn short_for_copper_edit_fails_closed_on_invalid_arc() {
    let s = routed_fixture();
    let front = copper_layers(&s)[0];
    let mut project = s.snapshot().unwrap().as_ref().clone();
    let bad_arc = ArcTrack {
        id: Uuid::from_u128(0xc099),
        layer: front,
        width_nm: 250_000,
        center: Point { x_nm: 0, y_nm: 0 },
        start: Point { x_nm: 0, y_nm: 0 },
        end: Point { x_nm: 0, y_nm: 0 },
        direction: Direction::Clockwise,
    };
    Arc::make_mut(&mut project.board).arcs.push(bad_arc.clone());
    assert!(short_for_copper_edit(&project, &BTreeSet::from([bad_arc.id]), &BTreeSet::new()).is_err());
}

/// Pairwise and four-axis indexed island building produce identical islands with arcs,
/// including contact exclusively through an arc's mid-sweep apex/bulge.
#[test]
fn island_builder_with_arcs_matches_between_pairwise_and_indexed_paths() {
    let mut s = routed_fixture();
    let layers = copper_layers(&s);
    let (front, back) = (layers[0], layers[1]);
    let board = view(&s).target.board;
    let board_lands = lands(view(&s));
    let mut accepted: Vec<_> = board_lands.iter().filter(|l| l.net.as_deref() == Some("ACCEPTED")).collect();
    accepted.sort_by_key(|l| l.center.x_nm);

    // Place a via at the +Y apex of the front semicircle so contact occurs strictly at the bulge,
    // not at either arc endpoint.
    let front_arc = semicircle_between(0xc010, front, accepted[1].center, accepted[0].center, 250_000, Direction::Clockwise);
    let radius = (accepted[1].center.x_nm - accepted[0].center.x_nm).abs() / 2;
    let apex = Point { x_nm: front_arc.center.x_nm, y_nm: front_arc.center.y_nm + radius };
    s.apply(crate::ADD_VIA, &json!({"board":board,"at":{"x_nm":apex.x_nm,"y_nm":apex.y_nm},"expected_revision":s.revision()})).unwrap();
    let apex_via = view(&s).vias()[0].id;

    let back_arc = semicircle_between(0xc011, back, accepted[2].center, accepted[1].center, 250_000, Direction::Counterclockwise);
    s.apply(SET_ARCS, &params(&s, &[front_arc.clone(), back_arc.clone()])).unwrap();

    let v = view(&s);
    let board_lands = lands(v);
    let items = CopperItems { lands: &board_lands, tracks: v.tracks(), vias: v.vias(), arcs: v.arc_tracks() };

    let mut pairwise = IslandBuilder::try_with_arcs(items).unwrap().on_path(true);
    while !pairwise.step(usize::MAX) {}
    let pairwise_result = pairwise.finish_with_arcs(items);

    let mut indexed = IslandBuilder::try_with_arcs(items).unwrap().on_path(false);
    while !indexed.step(usize::MAX) {}
    let indexed_result = indexed.finish_with_arcs(items);

    assert_eq!(pairwise_result, indexed_result);
    let copper = &indexed_result.0;
    let island = copper.pads[&accepted[0].key];
    assert_eq!(copper.pads[&accepted[1].key], island);
    assert_eq!(copper.arcs[&front_arc.id], island);
    assert_eq!(copper.vias[&apex_via], island);
    assert_ne!(copper.arcs[&back_arc.id], island);
}

/// A validated arc whose `atan2` endpoint angles collide in `f64` still participates in
/// connectivity instead of being silently dropped.
#[test]
fn sub_ulp_atan2_arc_participates_in_connectivity_and_shorts() {
    let mut s = routed_fixture();
    let front = copper_layers(&s)[0];
    let board = view(&s).target.board;
    let start = Point { x_nm: -100_000_000, y_nm: 1 };
    let end = Point { x_nm: -99_999_999, y_nm: 1 };
    let via_a = Point { x_nm: -100_000_000, y_nm: 300_000 };
    let via_b = Point { x_nm: -99_999_999, y_nm: -300_000 };
    for at in [via_a, via_b] {
        s.apply(
            crate::ADD_VIA,
            &json!({"board":board,"at":{"x_nm":at.x_nm,"y_nm":at.y_nm},"via_diameter_nm":500_000,"via_drill_nm":300_000,"expected_revision":s.revision()}),
        )
        .unwrap();
    }
    let vias: Vec<_> = view(&s).vias().iter().map(|v| v.id).collect();
    assert_eq!(vias.len(), 2);
    // Without the arc, the two vias (600_000 nm apart, diameter 500_000 nm) are separate islands.
    let before = view(&s).copper().unwrap();
    assert_ne!(before.vias[&vias[0]], before.vias[&vias[1]]);

    let arc = ArcTrack {
        id: Uuid::from_u128(0xc020),
        layer: front,
        width_nm: 200_000,
        center: Point { x_nm: 0, y_nm: 0 },
        start,
        end,
        direction: Direction::Counterclockwise,
    };
    s.apply(SET_ARCS, &params(&s, std::slice::from_ref(&arc))).unwrap();
    let after = view(&s).copper().unwrap();
    assert_eq!(after.vias[&vias[0]], after.arcs[&arc.id]);
    assert_eq!(after.vias[&vias[1]], after.arcs[&arc.id]);
}

/// Teardrop boards (both teardrop-only and arcs+teardrops) keep returning `teardrop::UNSUPPORTED`
/// across connectivity, ratsnest, short detection and via inspection.
#[test]
fn teardrops_still_refused_in_connectivity_and_short_detection() {
    let mut s = routed_fixture();
    let front = copper_layers(&s)[0];
    let board = view(&s).target.board;
    let at = Point { x_nm: 0, y_nm: 20_000_000 };
    let end = Point { x_nm: 600_000, y_nm: 20_000_000 };
    s.apply(crate::ADD_VIA, &json!({"board":board,"at":at,"via_diameter_nm":500_000,"via_drill_nm":300_000,"expected_revision":s.revision()}))
        .unwrap();
    s.apply(crate::ADD_TRACKS, &json!({"layer":front,"width_nm":100_000,"points":[at,end],"expected_revision":s.revision()})).unwrap();
    let via = view(&s).vias()[0].id;
    let track = view(&s).tracks()[0].id;
    let record = Record {
        id: Uuid::from_u128(0x7ea1),
        via,
        track,
        parameters: Parameters { length: Length::Percent(20), max_length_nm: None, width_pct: 80, max_width_nm: None },
    };
    s.apply(EDIT_TEARDROPS, &json!({"board":board,"expected_revision":s.revision(),"edits":{"add":[record],"remove":[]}})).unwrap();

    let assert_refused = |v: crate::board::BoardView<'_>| {
        assert_eq!(v.copper().unwrap_err(), TEARDROP_UNSUPPORTED);
        assert_eq!(v.copper_nets().unwrap_err(), TEARDROP_UNSUPPORTED);
        assert_eq!(v.copper_with_nets().unwrap_err(), TEARDROP_UNSUPPORTED);
        assert_eq!(v.ratsnest().unwrap_err(), TEARDROP_UNSUPPORTED);
        assert_eq!(v.copper_shorts().unwrap_err(), TEARDROP_UNSUPPORTED);
        assert_eq!(v.inspect_vias()[0]["geometry_error"], TEARDROP_UNSUPPORTED);
    };

    assert_refused(view(&s));

    // Also verify a snapshot carrying both arcs and teardrops still refuses with teardrop::UNSUPPORTED.
    let mut combined = s.snapshot().unwrap().as_ref().clone();
    Arc::make_mut(&mut combined.board).arcs.push(ArcTrack {
        id: Uuid::from_u128(0x7ea2),
        layer: front,
        width_nm: 250_000,
        center: Point { x_nm: 0, y_nm: 0 },
        start: Point { x_nm: 5_000_000, y_nm: 0 },
        end: Point { x_nm: 0, y_nm: 5_000_000 },
        direction: Direction::Clockwise,
    });
    let combined_view = combined.board_view(combined.board_target().unwrap()).unwrap();
    assert_refused(combined_view);
}
