//! Real app input and publication over command-built physical zone intent.
use super::*;
use pcbcraft_engine::{
    board::{
        setup::LayerPurpose,
        zones::{ADD_ZONE, Connection, Options},
    },
    persistence::encode_documents,
    schematic::Point,
};

/// Extend the accepted ratsnest fixture through shared setup/drawing/zone commands.
fn board() -> Harness<'static, PcbApp> {
    let mut h = super::board_ratsnest::board(true);
    let edit = |h: &mut Harness<'_, PcbApp>, id: &str, mut value: Value| {
        value["expected_revision"] = json!(h.state().session.revision());
        h.state_mut().run(id, &value);
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
    };
    edit(&mut h, pcbcraft_engine::INITIALIZE_BOARD_SETUP, json!({"copper_layers":2,"resolve_existing_footprints":true}));
    let (front, outline, target) = {
        let p = h.state().session.project().unwrap();
        let v = p.board_view(p.board_target().unwrap()).unwrap();
        let line = v.ratsnest().unwrap().lines[0].clone();
        (
            v.layers().iter().find(|l| l.purpose == LayerPurpose::Copper && l.side == Some(pcbcraft_engine::board::setup::Side::Front)).unwrap().id,
            v.layers().iter().find(|l| l.purpose == LayerPurpose::Outline).unwrap().id,
            v.net_class_target_for_copper(&format!("{}:{}", line.from.footprint, line.from.pad.inspect()["pad"].as_str().unwrap())).unwrap().unwrap(),
        )
    };
    let points = [[-4_000_000, -4_000_000], [14_000_000, -4_000_000], [14_000_000, 6_000_000], [-4_000_000, 6_000_000]]
        .map(|[x_nm, y_nm]| Point { x_nm, y_nm });
    edit(&mut h, pcbcraft_engine::ADD_DRAWING, json!({"layer":outline,"points":points,"closed":true}));
    edit(
        &mut h,
        ADD_ZONE,
        json!({"target":target,"layer":front,"points":points,"options":Options{connection:Connection::Solid,..Options::default()}}),
    );
    h.run();
    h
}

#[test]
fn automatic_fill_updates_real_canvas_and_ratsnest_without_an_authored_edit() {
    let mut h = board();
    let revision = h.state().session.revision();
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.run();
    let p = h.state().session.project().unwrap();
    let v = p.board_view(p.board_target().unwrap()).unwrap();
    let fill = v.filled_zones().unwrap().unwrap();
    assert!(fill.zones.iter().any(|z| z.islands.iter().any(|i| !i.mesh.triangles.is_empty())));
    assert!(v.ratsnest().unwrap().lines.is_empty());
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    let cached = h.state().views.active.board.ratsnest_cache.as_ref().unwrap();
    assert!(cached.2.is_some());
}

#[test]
fn shared_route_menu_refills_without_dirtying_or_adding_undo() {
    let mut h = board();
    h.run();
    let revision = h.state().session.revision();
    let p = h.state().session.project().unwrap();
    let target = p.board_target().unwrap();
    let before = p.board_view(target).unwrap().filled_zones().unwrap().unwrap();
    click(&mut h, "Layout");
    click(&mut h, "Route menu");
    assert!(!h.get_by_label("Refill zones").accesskit_node().is_disabled());
    click(&mut h, "Refill zones");
    h.run();
    let after = h.state().session.project().unwrap().board_view(target).unwrap().filled_zones().unwrap().unwrap();
    assert!(!std::sync::Arc::ptr_eq(&before, &after));
    assert_eq!(h.state().session.revision(), revision);
}
