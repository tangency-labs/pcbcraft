//! Integration tests for circular arc copper contact, connectivity and short detection.
use super::*;
use crate::board::{
    drc::Kind,
    track::{IslandBuilder, lands, short_for_copper_edit},
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

    // Semicircle in -Y (CCW from right to left in X-right Y-down) avoids neighboring pads.
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
    assert!(v.copper_shorts().unwrap().is_empty());
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
    assert!(v.copper_shorts().unwrap().is_empty());
    // Both vias now report connected_to_pad == true and resolved net in inspect_vias().
    for inspected in v.inspect_vias() {
        assert_eq!(inspected["connected_to_pad"], true);
        assert_eq!(inspected["net"], "ACCEPTED");
        assert!(inspected.get("geometry_error").is_none());
    }
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
    assert_eq!(shorts.len(), 1);
    assert_eq!(shorts[0].kind, Kind::Short);
    assert_eq!(shorts[0].nets.len(), 2);
    assert!(shorts[0].items.contains(&arc.id.to_string()));
    assert!(shorts[0].items.contains(&format!("{}:{}", pad_a.key.0, pad_a.key.1)));
    assert!(shorts[0].items.contains(&format!("{}:{}", pad_b.key.0, pad_b.key.1)));

    let edit_short = short_for_copper_edit(s.project().unwrap(), &BTreeSet::from([arc.id]), &BTreeSet::new());
    assert!(edit_short.is_some());
}

/// Pairwise and four-axis indexed island building produce identical islands with arcs.
#[test]
fn island_builder_with_arcs_matches_between_pairwise_and_indexed_paths() {
    let mut s = routed_fixture();
    let layers = copper_layers(&s);
    let (front, back) = (layers[0], layers[1]);
    let board_lands = lands(view(&s));
    let mut accepted: Vec<_> = board_lands.iter().filter(|l| l.net.as_deref() == Some("ACCEPTED")).collect();
    accepted.sort_by_key(|l| l.center.x_nm);

    let arcs = vec![
        semicircle_between(0xc010, front, accepted[1].center, accepted[0].center, 250_000, Direction::Clockwise),
        semicircle_between(0xc011, back, accepted[2].center, accepted[1].center, 250_000, Direction::Counterclockwise),
    ];
    s.apply(SET_ARCS, &params(&s, &arcs)).unwrap();

    let v = view(&s);
    let board_lands = lands(v);
    let (tracks, vias, arcs) = (v.tracks(), v.vias(), v.arc_tracks());

    let mut pairwise = IslandBuilder::with_arcs(&board_lands, tracks, vias, arcs).on_path(true);
    while !pairwise.step(usize::MAX) {}
    let pairwise_result = pairwise.finish_with_arcs(&board_lands, tracks, vias, arcs);

    let mut indexed = IslandBuilder::with_arcs(&board_lands, tracks, vias, arcs).on_path(false);
    while !indexed.step(usize::MAX) {}
    let indexed_result = indexed.finish_with_arcs(&board_lands, tracks, vias, arcs);

    assert_eq!(pairwise_result, indexed_result);
}
