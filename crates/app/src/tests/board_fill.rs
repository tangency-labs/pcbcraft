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
pub(crate) fn board() -> Harness<'static, PcbApp> {
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
fn board_ribbon_refills_without_dirtying_or_adding_undo() {
    let mut h = board();
    h.set_size(egui::vec2(1400., 780.));
    h.run();
    let revision = h.state().session.revision();
    let p = h.state().session.project().unwrap();
    let target = p.board_target().unwrap();
    let before = p.board_view(target).unwrap().filled_zones().unwrap().unwrap();
    click(&mut h, "Layout");
    for label in ["Fill zone outlines with copper", "Remove poured copper and keep zone outlines"] {
        let button = h.get_by_label(label).rect();
        assert!(button.right() < h.get_by_label("Active layer").rect().left());
    }
    assert!(!h.get_by_label("Fill zone outlines with copper").accesskit_node().is_disabled());
    click(&mut h, "Fill zone outlines with copper");
    h.run();
    let after = h.state().session.project().unwrap().board_view(target).unwrap().filled_zones().unwrap().unwrap();
    assert!(!std::sync::Arc::ptr_eq(&before, &after));
    assert_eq!(h.state().session.revision(), revision);
}

#[test]
fn cancel_pour_dialog_retires_pending_material_and_allows_retry() {
    let mut h = board();
    h.run();
    let revision = h.state().session.revision();
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let target = h.state().session.project().unwrap().board_target().unwrap();
    for escape in [false, true] {
        h.state_mut().views.active.board.fills.pause_queue = true;
        h.key_press(Key::B);
        h.run_steps(2);
        assert!(matches!(h.state().dialog, crate::Dialog::CopperPour(_)));
        let _ = h.get_by_label("0 of 1 zones · 0:00 elapsed");
        h.input_mut().time = Some(h.ctx.input(|input| input.time) + 3.1);
        h.run();
        let _ = h.get_by_label("0 of 1 zones · 0:03 elapsed");
        if escape {
            h.key_press(Key::Escape);
            h.run();
        } else {
            click(&mut h, "Cancel");
        }
        assert!(matches!(h.state().dialog, crate::Dialog::None));
        h.state_mut().views.active.board.fills.pause_queue = false;
        h.run();
        let status = h.state().session.zone_fill_status(target).unwrap();
        assert_eq!(status["zone_fill"]["result"]["state"], "unfilled");
        assert_eq!(poured_triangles(&h), 0);
        assert_eq!(h.state().session.revision(), revision);
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
        h.key_press(Key::B);
        h.run();
        assert!(poured_triangles(&h) > 0);
    }
}

/// Count the actual canvas fill triangles, independently from pads, traces and chrome.
fn poured_triangles(h: &Harness<'_, PcbApp>) -> usize {
    let canvas = h.get_by_label("PCB canvas").rect();
    let project = h.state().session.project().unwrap();
    let view = project.board_view(project.board_target().unwrap()).unwrap();
    let layer = view.layers().into_iter().find(|layer| layer.id == view.zones()[0].layer).unwrap();
    let ink = h.state().board_appearance.color(view, &layer).gamma_multiply(0.32);
    h.output()
        .shapes
        .iter()
        .filter(|shape| shape.clip_rect == canvas)
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Mesh(mesh) if mesh.vertices.iter().all(|vertex| vertex.color == ink) => Some(mesh.indices.len() / 3),
            _ => None,
        })
        .sum()
}

/// Real menu actions remove physical copper/airwire proofs and restore it, keeping outlines,
/// layer visibility, native bytes and authored history. Pending automatic work cannot repour.
#[test]
fn ribbon_pour_and_remove_change_physical_copper_without_deleting_zones() {
    for (width, theme) in [(1400., crate::preferences::ThemeChoice::Dark), (760., crate::preferences::ThemeChoice::Light)] {
        let mut h = board();
        h.set_size(egui::vec2(width, 750.));
        h.state_mut().preferences.theme = theme;
        h.run();
        let revision = h.state().session.revision();
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let hidden = h.state().views.active.board.hidden_layers.clone();
        assert!(poured_triangles(&h) > 0);
        click(&mut h, "Layout");
        click(&mut h, "Remove poured copper and keep zone outlines");
        h.run();
        assert_eq!(poured_triangles(&h), 0);
        let project = h.state().session.project().unwrap();
        let view = project.board_view(project.board_target().unwrap()).unwrap();
        assert_eq!(view.zones().len(), 1);
        assert!(!view.ratsnest().unwrap().lines.is_empty());
        assert!(!view.zone_fill_enabled());
        assert_eq!(h.state().views.active.board.hidden_layers, hidden);
        assert!(h.get_by_label("Remove poured copper and keep zone outlines").accesskit_node().is_disabled());
        assert!(!h.get_by_label("Fill zone outlines with copper").accesskit_node().is_disabled());
        click(&mut h, "Fill zone outlines with copper");
        h.run();
        assert!(poured_triangles(&h) > 0);
        let project = h.state().session.project().unwrap();
        let view = project.board_view(project.board_target().unwrap()).unwrap();
        assert!(view.ratsnest().unwrap().lines.is_empty());
        assert_eq!(h.state().session.revision(), revision);
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    }
}

/// Keyboard and live MCP take the same board-wide derived command path as the ribbon.
#[test]
fn pour_shortcuts_and_live_commands_share_removal_and_resume() {
    let mut h = board();
    h.run();
    let revision = h.state().session.revision();
    h.key_press_modifiers(Modifiers::COMMAND, Key::B);
    h.run();
    assert_eq!(poured_triangles(&h), 0);
    h.key_press(Key::B);
    h.run();
    assert!(poured_triangles(&h) > 0);
    let ctx = h.ctx.clone();
    let remove = json!({"method":"engine.execute","params":{"command":pcbcraft_engine::UNFILL_ZONES,"params":{},"inspect":false}});
    let answer = pcbcraft_mcp::live::call_editor(&mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx }, &remove).unwrap();
    assert_eq!(answer["zone_fill"]["result"]["state"], "unfilled");
    h.run();
    assert_eq!(poured_triangles(&h), 0);
    let pour = json!({"method":"engine.execute","params":{"command":pcbcraft_engine::REFILL_ZONES,"params":{},"inspect":false}});
    let answer = pcbcraft_mcp::live::call_editor(&mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx }, &pour).unwrap();
    assert_eq!(answer["zone_fill"]["result"]["state"], "pending");
    h.run();
    assert!(poured_triangles(&h) > 0);
    assert_eq!(h.state().session.revision(), revision);
}
