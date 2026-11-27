use super::*;
use crate::board_nudge::arrow_delta;
use pcbcraft_engine::{ADD_VIA, SET_COPPER_LOCKED, schematic::Point};

/// Access the model rather than comparing screen pixels or snapped pointer positions.
fn first<'a>(h: &'a Harness<'_, PcbApp>) -> pcbcraft_engine::board::view::FootprintView<'a> {
    h.state()
        .session
        .project()
        .and_then(|p| p.boards().next())
        .and_then(|b| b.footprints().next())
        .unwrap_or_else(|| panic!("expected board footprint"))
}

/// Give keys to the actual board canvas (not its layer panel or the workspace picker).
fn canvas(h: &mut Harness<'_, PcbApp>) {
    let center = h.get_by_label("PCB canvas").rect().center();
    h.hover_at(center);
    h.run();
}

/// Modifiers use exact event state, including the physical Control aliases. Mirrored board
/// views retain screen direction, and an indivisible fine step rounds at model precision.
#[test]
fn grid_steps_and_control_aliases_are_exact() {
    let grid = 254_005;
    for (keys, distance) in [
        (egui::Modifiers::NONE, grid),
        (egui::Modifiers::CTRL, grid),
        (egui::Modifiers::SHIFT, grid * 10),
        (egui::Modifiers::CTRL | egui::Modifiers::SHIFT, grid * 10),
        (egui::Modifiers::ALT, 25_401),
    ] {
        assert_eq!(arrow_delta(Key::ArrowRight, keys, grid, false), Some(Point { x_nm: distance, y_nm: 0 }));
        assert_eq!(arrow_delta(Key::ArrowLeft, keys, grid, true), Some(Point { x_nm: distance, y_nm: 0 }));
        assert_eq!(arrow_delta(Key::ArrowUp, keys, grid, true), Some(Point { x_nm: 0, y_nm: -distance }));
        assert_eq!(arrow_delta(Key::ArrowDown, keys, grid, false), Some(Point { x_nm: 0, y_nm: distance }));
    }
    assert_eq!(arrow_delta(Key::ArrowLeft, egui::Modifiers::ALT | egui::Modifiers::CTRL, grid, false), None);
    assert_eq!(arrow_delta(Key::ArrowLeft, egui::Modifiers::MAC_CMD, grid, false), None);
}

/// Native repeated keydowns move immediately, but each press/release burst is one shared
/// undo entry. Empty selections, pointer outside the board, and dialogs perform no edit.
#[test]
fn held_arrow_groups_undo_and_respects_canvas_ownership() {
    let mut h = board_tracks::routable();
    canvas(&mut h);
    let rev = h.state().session.revision();
    h.key_press(Key::ArrowRight);
    h.run();
    assert_eq!(h.state().session.revision(), rev);
    assert!(h.state().message.is_none(), "an empty selection sends no move: {:?}", h.state().message);
    let id = first(&h).id;
    let old = first(&h).position;
    let step = h.state().views.active.camera.snap_nm();
    h.state_mut().select_board_items(pcbcraft_engine::board::clipboard::BoardSelection { footprints: [id].into(), ..Default::default() });
    canvas(&mut h);
    h.key_down(Key::ArrowRight);
    h.run();
    assert_eq!(first(&h).position.x_nm, old.x_nm + step);
    h.key_down(Key::ArrowRight);
    h.run();
    assert_eq!(first(&h).position.x_nm, old.x_nm + 2 * step);
    h.key_up(Key::ArrowRight);
    h.run();
    h.key_press_modifiers(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, Key::ArrowDown);
    h.run();
    let final_pos = first(&h).position;
    assert_eq!(final_pos, Point { x_nm: old.x_nm + 2 * step, y_nm: old.y_nm + 10 * step });
    h.state_mut().run("edit.undo", &json!({}));
    assert_eq!(first(&h).position.y_nm, old.y_nm);
    h.state_mut().run("edit.undo", &json!({}));
    assert_eq!(first(&h).position, old);
    h.state_mut().run("edit.redo", &json!({}));
    assert_eq!(first(&h).position.x_nm, old.x_nm + 2 * step);
    h.state_mut().run("edit.redo", &json!({}));
    assert_eq!(first(&h).position, final_pos);

    let rev = h.state().session.revision();
    h.hover_at(egui::pos2(2., 2.));
    h.key_press(Key::ArrowRight);
    h.run();
    assert_eq!(h.state().session.revision(), rev);
    canvas(&mut h);
    h.state_mut().ask_name("project.rename");
    h.run();
    h.key_press(Key::ArrowLeft);
    h.run();
    assert_eq!(h.state().session.revision(), rev);
}

/// A mixed selection moves as one group; locking its via prevents any partial movement.
/// Independently selected reference text moves without moving its footprint.
#[test]
fn group_lock_and_reference_text_are_respected() {
    let mut h = board_tracks::routable();
    let positions = |h: &Harness<'_, PcbApp>| {
        h.state()
            .session
            .project()
            .and_then(|p| p.boards().next())
            .map(|b| b.footprints().map(|f| (f.id, f.position)).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let before = positions(&h);
    let id = before[0].0.clone();
    let board = |h: &Harness<'_, PcbApp>| h.state().session.project().and_then(|p| p.board_target()).unwrap_or_else(|| panic!("missing board")).board;
    let board = board(&h);
    let rev = h.state().session.revision();
    h.state_mut().run(ADD_VIA, &json!({"board":board,"at":{"x_nm":3_000_000,"y_nm":4_000_000},"expected_revision":rev}));
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    let via = h.state().session.project().and_then(|p| p.boards().next()).unwrap_or_else(|| panic!("missing board")).vias()[0].id;
    let items = pcbcraft_engine::board::clipboard::BoardSelection {
        footprints: before.iter().map(|(id, _)| id.clone()).collect(),
        vias: [via].into(),
        ..Default::default()
    };
    let step = h.state().views.active.camera.snap_nm();
    h.state_mut().select_board_items(items.clone());
    canvas(&mut h);
    h.key_press(Key::ArrowRight);
    h.run();
    let moved = positions(&h);
    assert_eq!(moved, before.iter().map(|(id, p)| (id.clone(), Point { x_nm: p.x_nm + step, y_nm: p.y_nm })).collect::<Vec<_>>());
    let via_at = |h: &Harness<'_, PcbApp>| h.state().session.project().and_then(|p| p.boards().next()).map(|b| b.vias()[0].at);
    assert_eq!(via_at(&h), Some(Point { x_nm: 3_000_000 + step, y_nm: 4_000_000 }), "the unlocked via moves with its group");
    let rev = h.state().session.revision();
    h.state_mut().run(SET_COPPER_LOCKED, &json!({"board":board,"vias":[via],"locked":true,"expected_revision":rev}));
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    h.state_mut().views.active.filters.include_locked = true;
    h.state_mut().select_board_items(items);
    canvas(&mut h);
    let rev = h.state().session.revision();
    h.key_press(Key::ArrowRight);
    h.run();
    assert_eq!(h.state().session.revision(), rev);
    assert_eq!(positions(&h), moved);
    assert_eq!(via_at(&h), Some(Point { x_nm: 3_000_000 + step, y_nm: 4_000_000 }), "a locked via refuses the whole group step");
    h.state_mut().run("edit.undo", &json!({}));
    let app = h.state_mut();
    app.views.select_board_reference(id, &app.session, app.preferences.selection_follow);
    canvas(&mut h);
    let offset = first(&h).reference_offset;
    h.key_press_modifiers(egui::Modifiers::ALT, Key::ArrowDown);
    h.run();
    assert_eq!(positions(&h), moved);
    assert_eq!(first(&h).reference_offset, Point { x_nm: offset.x_nm, y_nm: offset.y_nm + step / 10 });
}

/// An armed board tool and a focused widget each keep the arrow keys: the selection, the
/// revision and the message stay as they were while the pointer is over the canvas.
#[test]
fn focused_widgets_and_active_tools_keep_the_arrows() {
    let mut h = board_tracks::routable();
    let id = first(&h).id;
    let old = first(&h).position;
    h.state_mut().select_board_items(pcbcraft_engine::board::clipboard::BoardSelection { footprints: [id].into(), ..Default::default() });
    canvas(&mut h);
    let rev = h.state().session.revision();
    h.state_mut().views.active.board.measure.tool = true;
    assert!(crate::board_clipboard::busy(&h.state().views.active.board));
    h.key_press(Key::ArrowRight);
    h.run();
    assert_eq!(h.state().session.revision(), rev, "the ruler keeps the arrows");
    assert_eq!(first(&h).position, old);
    h.state_mut().views.active.board.measure.tool = false;

    h.get_by_label("Show layer Top").focus();
    h.run();
    canvas(&mut h);
    assert!(h.ctx.egui_wants_keyboard_input(), "the layer eye keeps keyboard focus");
    h.key_press(Key::ArrowRight);
    h.run();
    assert_eq!(h.state().session.revision(), rev, "a focused widget keeps the arrows");
    assert_eq!(first(&h).position, old);
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
}

/// Reference text follows the screen direction of each arrow whatever its footprint's pose:
/// a half turn reverses both axes and the back side mirrors X. The footprint stays put.
#[test]
fn reference_text_nudges_in_screen_direction_on_turned_and_back_footprints() {
    let mut h = board_tracks::routable();
    let step = h.state().views.active.camera.snap_nm();
    for (rotation, side, along_x, along_y) in [(180, "front", -1, -1), (0, "back", -1, 1)] {
        let id = first(&h).id;
        let at = first(&h).position;
        let revision = h.state().session.revision();
        h.state_mut().run(
            pcbcraft_engine::SET_FOOTPRINT_TRANSFORM,
            &json!({"footprint":id,"x_nm":at.x_nm,"y_nm":at.y_nm,"rotation":rotation,"side":side,"expected_revision":revision}),
        );
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        let position = first(&h).position;
        let app = h.state_mut();
        app.views.select_board_reference(id.clone(), &app.session, app.preferences.selection_follow);
        canvas(&mut h);
        let offset = first(&h).reference_offset;
        h.key_press(Key::ArrowRight);
        h.run();
        h.key_press(Key::ArrowDown);
        h.run();
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        assert_eq!(
            first(&h).reference_offset,
            Point { x_nm: offset.x_nm + along_x * step, y_nm: offset.y_nm + along_y * step },
            "{rotation} degrees on the {side}"
        );
        assert_eq!(first(&h).position, position, "{rotation} degrees on the {side}");
    }
}
