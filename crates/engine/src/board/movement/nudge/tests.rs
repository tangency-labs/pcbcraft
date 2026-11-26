use super::*;
use crate::{
    ADD_TRACKS, ADD_VIA, SET_COPPER_LOCKED, Session,
    board::track::{
        DEFAULT_VIA_DIAMETER_NM, DEFAULT_VIA_DRILL_NM,
        tests::{copper_layers, routed_fixture, view},
    },
};
use serde_json::{Value, json};

fn p(x_nm: i64, y_nm: i64) -> Point {
    Point { x_nm, y_nm }
}

/// All fixture edits use the catalog and revision checks used by the app and MCP. History
/// commands declare no parameters, so only commands that accept a revision receive one.
fn edit(s: &mut Session, command: &str, mut params: Value) {
    if crate::command(command).is_some_and(|cmd| cmd.params.iter().any(|p| p.name == "expected_revision")) {
        params["expected_revision"] = json!(s.revision());
    }
    let result = s.apply(command, &params);
    assert!(result.is_ok(), "{command}: {params}: {result:?}");
}

/// The shared two-layer routed fixture: three placed 0402 resistors near the origin. Free copper
/// in these tests stays well clear of them, at 30 mm and beyond.
fn fixture() -> (Session, Uuid) {
    let s = routed_fixture();
    let front = copper_layers(&s)[0];
    (s, front)
}

fn params(s: &Session, items: Value, dx: i64, dy: i64) -> Value {
    json!({"board":view(s).target.board,"items":items,"dx_nm":dx,"dy_nm":dy,"preserve_contacts":true,"expected_revision":s.revision()})
}

fn refused(s: &mut Session, params: &Value) -> String {
    let revision = s.revision();
    let snapshot = s.snapshot().unwrap_or_else(|| panic!("missing project"));
    let error = s.apply(super::super::MOVE_ITEMS, params).err().unwrap_or_else(|| panic!("move was accepted: {params}"));
    assert_eq!(s.revision(), revision);
    assert!(s.snapshot().is_some_and(|live| Arc::ptr_eq(&snapshot, &live)), "a refused move leaves the project untouched");
    error.to_string()
}

/// An exact grid displacement of the middle leg changes only the neighboring endpoints at the
/// junctions. Undo and redo cover all three tracks; a locked neighbor refuses the entire edit.
#[test]
fn nudges_selected_leg_and_stretches_only_attached_ends() {
    let (mut s, layer) = fixture();
    let (a, b, c, d) = (p(30_000_000, 30_000_000), p(30_000_000, 35_000_000), p(35_000_000, 35_000_000), p(35_000_000, 30_000_000));
    edit(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":100_000,"points":[a,b,c,d]}));
    let before = view(&s).tracks().to_vec();
    assert_eq!(before.len(), 3);
    let joins = |s: &Session, from, to| {
        let tracks = view(s).tracks();
        assert!(tracks.iter().any(|t| (t.start, t.end) == (from, to) || (t.start, t.end) == (to, from)), "missing {from:?} to {to:?}");
    };
    let id =
        |from, to| before.iter().find(|t| [(from, to), (to, from)].contains(&(t.start, t.end))).unwrap_or_else(|| panic!("missing fixture track")).id;
    let selected = id(b, c);
    let revision = s.revision();
    let mv = params(&s, json!({"tracks":[selected]}), 0, 254_000);
    let applied = s.apply(super::super::MOVE_ITEMS, &mv);
    assert!(applied.is_ok(), "{applied:?}");
    assert_eq!(s.revision(), revision + 1, "one nudge is one edit");
    let (moved_b, moved_c) = (p(b.x_nm, b.y_nm + 254_000), p(c.x_nm, c.y_nm + 254_000));
    joins(&s, a, moved_b);
    joins(&s, moved_b, moved_c);
    joins(&s, moved_c, d);
    assert_eq!(view(&s).tracks().len(), 3);
    let after = view(&s).tracks().to_vec();
    edit(&mut s, "edit.undo", json!({}));
    assert_eq!(view(&s).tracks(), before);
    edit(&mut s, "edit.redo", json!({}));
    assert_eq!(view(&s).tracks(), after);
    edit(&mut s, "edit.undo", json!({}));
    let board_id = view(&s).target.board;
    edit(&mut s, SET_COPPER_LOCKED, json!({"board":board_id,"tracks":[id(a,b)],"locked":true}));
    let mv = params(&s, json!({"tracks":[selected]}), 0, 254_000);
    refused(&mut s, &mv);
}

/// The crossbar of an H meets the middle of both unselected rails, so neither end can move.
/// Selecting it with a free via refuses the whole step: the via, every track, the revision and
/// the undo history stay as they were. The via alone still moves.
#[test]
fn a_selected_leg_held_at_both_ends_refuses_the_whole_mixed_step() {
    let (mut s, layer) = fixture();
    let rails = [(p(30_000_000, 30_000_000), p(30_000_000, 40_000_000)), (p(35_000_000, 30_000_000), p(35_000_000, 40_000_000))];
    for (from, to) in rails {
        edit(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":100_000,"points":[from,to]}));
    }
    let (left, right) = (p(30_000_000, 35_000_000), p(35_000_000, 35_000_000));
    edit(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":100_000,"points":[left,right]}));
    let free = p(45_000_000, 45_000_000);
    edit(&mut s, ADD_VIA, json!({"at":free,"via_diameter_nm":DEFAULT_VIA_DIAMETER_NM,"via_drill_nm":DEFAULT_VIA_DRILL_NM}));
    let tracks = view(&s).tracks().to_vec();
    assert_eq!(tracks.len(), 3);
    let crossbar =
        tracks.iter().find(|t| [(left, right), (right, left)].contains(&(t.start, t.end))).unwrap_or_else(|| panic!("missing crossbar")).id;
    let via = view(&s).vias()[0].id;

    for items in [json!({"tracks":[crossbar],"vias":[via]}), json!({"tracks":[crossbar]})] {
        let mv = params(&s, items, 0, 254_000);
        let message = refused(&mut s, &mv);
        assert!(message.contains("held at both ends"), "{message}");
        assert_eq!(view(&s).tracks(), tracks);
        assert_eq!(view(&s).vias()[0].at, free);
    }
    // The refusals left history alone: the next undo still removes the via, the last real edit.
    edit(&mut s, "edit.undo", json!({}));
    assert!(view(&s).vias().is_empty());
    assert_eq!(view(&s).tracks(), tracks);
    edit(&mut s, "edit.redo", json!({}));

    let mv = params(&s, json!({"vias":[via]}), 0, 254_000);
    let applied = s.apply(super::super::MOVE_ITEMS, &mv);
    assert!(applied.is_ok(), "{applied:?}");
    assert_eq!(view(&s).vias()[0].at, p(free.x_nm, free.y_nm + 254_000));
    assert_eq!(view(&s).tracks(), tracks);
}

/// A via anchor and a pad anchor never detach. Including an unlocked via in the selection
/// permits a legitimate shared displacement; moving just the via carries its coincident track end.
#[test]
fn refusing_to_detach_fixed_pad_or_via_is_atomic() {
    let (mut s, layer) = fixture();
    let at = p(38_000_000, 36_000_000);
    let other = p(48_000_000, 36_000_000);
    edit(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":100_000,"points":[at,other]}));
    edit(&mut s, ADD_VIA, json!({"at":at,"via_diameter_nm":DEFAULT_VIA_DIAMETER_NM,"via_drill_nm":DEFAULT_VIA_DRILL_NM}));
    let t = view(&s).tracks()[0].id;
    let v = view(&s).vias()[0].id;
    let mv = params(&s, json!({"tracks":[t]}), 0, 2_540_000);
    let message = refused(&mut s, &mv);
    assert!(message.contains("detach"), "{message}");
    let mv = params(&s, json!({"vias":[v]}), 0, 2_540_000);
    let applied = s.apply(super::super::MOVE_ITEMS, &mv);
    assert!(applied.is_ok(), "{applied:?}");
    let moved_via = p(at.x_nm, at.y_nm + 2_540_000);
    let after = view(&s);
    let end = after.tracks()[0].clone();
    assert_eq!(after.vias()[0].at, moved_via);
    assert!([end.start, end.end].contains(&moved_via), "the coincident end follows its via");
    assert!([end.start, end.end].contains(&other), "the far end stays put");
    let mv = params(&s, json!({"tracks":[t],"vias":[v]}), 0, 2_540_000);
    let applied = s.apply(super::super::MOVE_ITEMS, &mv);
    assert!(applied.is_ok(), "{applied:?}");

    // A free leg from a fixed SMD pad: moving the leg alone would leave the pad behind.
    let footprint = view(&s).footprints().next().unwrap_or_else(|| panic!("fixture footprint missing"));
    let pad = &footprint.pads[0];
    let Some(crate::board::view::CopperLayers::Resolved { layers, .. }) = &pad.copper_layers else { panic!("fixture pad unresolved") };
    let (center, pad_layer) = (pad.center, layers[0]);
    edit(&mut s, ADD_TRACKS, json!({"layer":pad_layer,"width_nm":100_000,"points":[center,p(center.x_nm,center.y_nm - 10_000_000)]}));
    let track = view(&s).tracks().iter().find(|t| t.start == center || t.end == center).unwrap_or_else(|| panic!("missing pad leg")).id;
    let mv = params(&s, json!({"tracks":[track]}), 0, -2_540_000);
    let message = refused(&mut s, &mv);
    assert!(message.contains("detach"), "{message}");
}

/// Each remaining refusal of the reshape leaves the project untouched. A junction asked to follow
/// a pivoting leg and a moving via at once has no single answer; a selected leg's end held by
/// the middle of an unselected rail cannot follow the via or pad that moves under it.
#[test]
fn conflicting_junctions_and_held_ends_refuse_the_whole_step() {
    let via = |s: &mut Session, at: Point| {
        edit(s, ADD_VIA, json!({"at":at,"via_diameter_nm":DEFAULT_VIA_DIAMETER_NM,"via_drill_nm":DEFAULT_VIA_DRILL_NM}));
        view(s).vias().iter().find(|v| v.at == at).unwrap_or_else(|| panic!("missing via")).id
    };
    let track = |s: &Session, from: Point, to: Point| {
        view(s).tracks().iter().find(|t| [(from, to), (to, from)].contains(&(t.start, t.end))).unwrap_or_else(|| panic!("missing track")).id
    };
    let (rail_top, rail_bottom, held, free) =
        (p(30_000_000, 30_000_000), p(30_000_000, 40_000_000), p(30_000_000, 35_000_000), p(40_000_000, 35_000_000));

    // The leg pivots about its held end, so its middle asks the junction for half the step,
    // while a via there asks for all of it.
    let (mut s, layer) = fixture();
    edit(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":100_000,"points":[rail_top,rail_bottom]}));
    edit(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":100_000,"points":[held,free]}));
    let junction = p(35_000_000, 35_000_000);
    edit(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":100_000,"points":[junction,p(35_000_000,45_000_000)]}));
    let leg = track(&s, held, free);
    let v = via(&mut s, junction);
    let mv = params(&s, json!({"tracks":[leg],"vias":[v]}), 0, 254_000);
    let message = refused(&mut s, &mv);
    assert!(message.contains("two different ways"), "{message}");

    // The held end of the selected leg sits on a selected via.
    let (mut s, layer) = fixture();
    edit(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":100_000,"points":[rail_top,rail_bottom]}));
    edit(&mut s, ADD_TRACKS, json!({"layer":layer,"width_nm":100_000,"points":[held,free]}));
    let leg = track(&s, held, free);
    let v = via(&mut s, held);
    let mv = params(&s, json!({"tracks":[leg],"vias":[v]}), 0, 254_000);
    let message = refused(&mut s, &mv);
    assert!(message.contains("cannot follow its moving via"), "{message}");

    // The held end of the selected leg sits on a pad of a selected footprint; the rail crosses
    // that pad diagonally, away from the footprint's other pad.
    let (mut s, _) = fixture();
    let footprint = view(&s).footprints().next().unwrap_or_else(|| panic!("fixture footprint missing"));
    let (id, center, other) = (footprint.id.clone(), footprint.pads[0].center, footprint.pads[1].center);
    let Some(crate::board::view::CopperLayers::Resolved { layers, .. }) = &footprint.pads[0].copper_layers else { panic!("fixture pad unresolved") };
    let pad_layer = layers[0];
    let away = if other.x_nm > center.x_nm { -1 } else { 1 };
    let (a, b) = (p(center.x_nm - away * 3_000_000, center.y_nm + 3_000_000), p(center.x_nm + away * 3_000_000, center.y_nm - 3_000_000));
    edit(&mut s, ADD_TRACKS, json!({"layer":pad_layer,"width_nm":100_000,"points":[a,b]}));
    let end = p(center.x_nm, center.y_nm - 10_000_000);
    edit(&mut s, ADD_TRACKS, json!({"layer":pad_layer,"width_nm":100_000,"points":[center,end]}));
    let leg = track(&s, center, end);
    let mv = params(&s, json!({"footprints":[id],"tracks":[leg]}), 0, -254_000);
    let message = refused(&mut s, &mv);
    assert!(message.contains("cannot follow its moving pad"), "{message}");
}

/// Two selected legs cross at their shared midpoint and pivot about opposite ends: one is held
/// at its start, the other at its end. An odd step puts that midpoint on a half-nanometre tie,
/// and both legs must round it the same way, so the stub ending there follows instead of the
/// step being refused as a junction asked to follow in two ways.
#[test]
fn opposite_pivots_round_a_shared_midpoint_the_same_way() {
    let (mut s, layer) = fixture();
    let track = |s: &Session, from: Point, to: Point| {
        view(s).tracks().iter().find(|t| (t.start, t.end) == (from, to)).unwrap_or_else(|| panic!("missing track")).id
    };
    let add = |s: &mut Session, from: Point, to: Point| edit(s, ADD_TRACKS, json!({"layer":layer,"width_nm":100_000,"points":[from,to]}));
    // The first leg is held at its start by a vertical rail, the second at its end by a horizontal one.
    add(&mut s, p(30_000_000, 30_000_000), p(30_000_000, 40_000_000));
    add(&mut s, p(32_000_000, 40_000_000), p(38_000_000, 40_000_000));
    let (first_held, first_free) = (p(30_000_000, 35_000_000), p(40_000_000, 35_000_000));
    let (second_free, second_held) = (p(35_000_000, 30_000_000), p(35_000_000, 40_000_000));
    add(&mut s, first_held, first_free);
    add(&mut s, second_free, second_held);
    let (middle, far) = (p(35_000_000, 35_000_000), p(42_000_000, 28_000_000));
    add(&mut s, middle, far);
    let (first, second, stub) = (track(&s, first_held, first_free), track(&s, second_free, second_held), track(&s, middle, far));

    let step = 254_001;
    let mv = params(&s, json!({"tracks":[first, second]}), 0, step);
    let revision = s.revision();
    let applied = s.apply(super::super::MOVE_ITEMS, &mv);
    assert!(applied.is_ok(), "{applied:?}");
    assert_eq!(s.revision(), revision + 1);
    let after = view(&s);
    let ends = |id| after.tracks().iter().find(|t| t.id == id).map(|t| (t.start, t.end)).unwrap_or_else(|| panic!("missing track"));
    assert_eq!(ends(first), (first_held, p(first_free.x_nm, first_free.y_nm + step)));
    assert_eq!(ends(second), (p(second_free.x_nm, second_free.y_nm + step), second_held));
    // Half of 254 001 nm is 127 000.5 nm, which rounds away from zero for both legs.
    assert_eq!(ends(stub), (p(middle.x_nm, middle.y_nm + 127_001), far));
}

/// Off the midpoint, too: two 32 mm legs cross 0.2 mm from their held ends, one held at its
/// start and one at its end, so the crossing moves by 1/160 of the step on both. For 254 000 nm
/// that is a 1587.5 nm tie, which both legs round away from zero in either direction, and the
/// stub ending at the crossing follows instead of the step being refused.
#[test]
fn opposite_pivots_round_an_off_center_shared_point_the_same_way_in_both_directions() {
    for step in [254_000, -254_000] {
        let (mut s, layer) = fixture();
        let track = |s: &Session, from: Point, to: Point| {
            view(s).tracks().iter().find(|t| (t.start, t.end) == (from, to)).unwrap_or_else(|| panic!("missing track")).id
        };
        let add = |s: &mut Session, from: Point, to: Point| edit(s, ADD_TRACKS, json!({"layer":layer,"width_nm":100_000,"points":[from,to]}));
        // A vertical rail holds the first leg's start; a horizontal rail holds the second leg's end.
        add(&mut s, p(30_000_000, 25_000_000), p(30_000_000, 45_000_000));
        add(&mut s, p(20_000_000, 35_200_000), p(31_000_000, 35_200_000));
        let (first_held, first_free) = (p(30_000_000, 35_000_000), p(62_000_000, 35_000_000));
        let (second_free, second_held) = (p(30_200_000, 3_200_000), p(30_200_000, 35_200_000));
        add(&mut s, first_held, first_free);
        add(&mut s, second_free, second_held);
        let (crossing, far) = (p(30_200_000, 35_000_000), p(37_200_000, 28_000_000));
        add(&mut s, crossing, far);
        let (first, second, stub) = (track(&s, first_held, first_free), track(&s, second_free, second_held), track(&s, crossing, far));

        let mv = params(&s, json!({"tracks":[first, second]}), 0, step);
        let revision = s.revision();
        let applied = s.apply(super::super::MOVE_ITEMS, &mv);
        assert!(applied.is_ok(), "{step}: {applied:?}");
        assert_eq!(s.revision(), revision + 1);
        let after = view(&s);
        let ends = |id| after.tracks().iter().find(|t| t.id == id).map(|t| (t.start, t.end)).unwrap_or_else(|| panic!("missing track"));
        assert_eq!(ends(first), (first_held, p(first_free.x_nm, first_free.y_nm + step)));
        assert_eq!(ends(second), (p(second_free.x_nm, second_free.y_nm + step), second_held));
        let tie = if step > 0 { 1_588 } else { -1_588 };
        assert_eq!(ends(stub), (p(crossing.x_nm, crossing.y_nm + tie), far), "{step}");
    }
}
