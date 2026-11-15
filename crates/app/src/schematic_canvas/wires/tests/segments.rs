use super::*;
use crate::{Tool, ViewId};
use egui::Key;
use std::sync::Arc;

/// A plain click targets a leg in the requested pane, including initial activation.
fn select_segment(h: &mut Harness<'_, PcbApp>, id: u64, point: Point) {
    let pos = view(h, id).camera.screen(point, canvas(h, id));
    h.hover_at(pos);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
}

/// Begin a segment slide, leaving its preview uncommitted for assertions or cancellation.
fn slide_preview(h: &mut Harness<'_, PcbApp>, id: u64, from: Point, to: Point) -> Pos2 {
    let start = view(h, id).camera.screen(from, canvas(h, id));
    let end = view(h, id).camera.screen(to, canvas(h, id));
    h.drag_at(start);
    h.run();
    h.hover_at(end);
    h.run();
    assert!(matches!(view(h, id).schematic.tool, Some(Tool::Reshape { .. })));
    end
}

/// Segment selection paints only that leg blue and Delete removes only that leg. The other
/// pane has a wire highlight, never delete authority, and both panes share one undo history.
#[test]
fn click_and_delete_target_only_the_indicated_segment() {
    for narrow in [false, true] {
        for id in [0, 1] {
            let mut h = split_harness(narrow);
            let before = h.state().session.snapshot().unwrap();
            select_segment(&mut h, id, grid(8, 3));
            assert_eq!(view(&h, id).schematic.selected_leg.as_ref().unwrap().leg, 1);
            assert!(view(&h, 1 - id).schematic.selection.is_empty());
            let ink = pcbcraft_chrome::Tokens::get();
            assert_painted(&h, id, &routes(&h)[0], ink.schematic_wire);
            assert_painted(&h, id, &[grid(8, 0), grid(8, 6)], ink.accent);
            assert_painted(&h, 1 - id, &routes(&h)[0], ink.schematic_highlight);
            focus_tab(&mut h, 1 - id);
            h.key_press(Key::Delete);
            h.run();
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
            focus_tab(&mut h, id);
            h.key_press(Key::Delete);
            h.run();
            assert_eq!(routes(&h), vec![vec![grid(-8, 0), grid(8, 0)]]);
            focus_tab(&mut h, 1 - id);
            click(&mut h, "Undo");
            crate::tests::assert_same_content(&before, &h.state().session.snapshot().unwrap());
        }
    }
}

/// A bent route like the reported alignment example slides one horizontal run into alignment.
/// Both layouts and initial focus states preserve the endpoints, opposite camera and history.
#[test]
fn dragging_a_segment_aligns_adjacent_runs_without_moving_the_whole_wire() {
    for narrow in [false, true] {
        for id in [0, 1] {
            let mut h = split_harness(narrow);
            h.state_mut().run("edit.undo", &Value::Null);
            let revision = h.state().session.revision();
            h.state_mut().run(
                DRAW_WIRE,
                &json!({"from":grid(-12, 8),"to":grid(20, 10),
                "bends":[grid(-8,8),grid(-8,-8),grid(0,-8),grid(0,0),grid(8,0),grid(8,-6),grid(14,-6),grid(14,10)],"expected_revision":revision}),
            );
            h.run();
            let before = h.state().session.snapshot().unwrap();
            let revision = h.state().session.revision();
            let center = view(&h, 1 - id).camera.screen(grid(0, 0), canvas(&h, 1 - id));
            let end = slide_preview(&mut h, id, grid(4, 0), grid(7, -6));
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
            assert_painted(&h, id, &[grid(0, -6), grid(8, -6)], pcbcraft_chrome::Tokens::get().accent);
            h.drop_at(end);
            h.run();
            assert_eq!(h.state().message, None);
            assert_eq!(h.state().session.revision(), revision + 1);
            assert_eq!(
                routes(&h),
                vec![vec![grid(-12, 8), grid(-8, 8), grid(-8, -8), grid(0, -8), grid(0, -6), grid(14, -6), grid(14, 10), grid(20, 10)]]
            );
            let after = h.state().session.snapshot().unwrap();
            assert_eq!(after.wires()[0].endpoints(), before.wires()[0].endpoints());
            assert_eq!(view(&h, 1 - id).camera.screen(grid(0, 0), canvas(&h, 1 - id)), center);
            for pane in [0, 1] {
                let ink = pcbcraft_chrome::Tokens::get();
                assert_painted(&h, pane, &routes(&h)[0], if pane == id { ink.schematic_wire } else { ink.schematic_highlight });
            }
            assert!(view(&h, id).schematic.selected_leg.is_some(), "The reshaped segment stays selected");
            h.key_press(Key::Delete);
            h.run();
            assert_eq!(routes(&h).len(), 2, "The retained selection deletes only the newly aligned segment");
            click(&mut h, "Undo");
            crate::tests::assert_same_content(&after, &h.state().session.snapshot().unwrap());
            focus_tab(&mut h, 1 - id);
            click(&mut h, "Undo");
            crate::tests::assert_same_content(&before, &h.state().session.snapshot().unwrap());
            click(&mut h, "Redo");
            crate::tests::assert_same_content(&after, &h.state().session.snapshot().unwrap());
        }
    }
}

/// Dragging the photographed split step consumes the folded old corner in preview and on
/// release, in either pane/layout. Escape restores the original, and undo remains atomic.
#[test]
fn dragging_a_split_step_consumes_the_folded_corner() {
    for narrow in [false, true] {
        for id in [0, 1] {
            let mut h = split_harness(narrow);
            h.state_mut().run("edit.undo", &Value::Null);
            let revision = h.state().session.revision();
            h.state_mut().run(
                DRAW_WIRE,
                &json!({"from":grid(16,0),"to":grid(12,18),
                "bends":[grid(16,6),grid(0,6),grid(0,10),grid(12,10)],"expected_revision":revision}),
            );
            let wire = h.state().session.project().unwrap().wires()[0].id();
            let revision = h.state().session.revision();
            h.state_mut().run(pcbcraft_engine::SLICE_WIRE, &json!({"wire":wire,"at":grid(0,6),"expected_revision":revision}));
            h.run();
            select_segment(&mut h, id, grid(0, 8));
            let before = h.state().session.snapshot().unwrap();
            slide_preview(&mut h, id, grid(0, 8), grid(16, 8));
            let expected = vec![grid(16, 0), grid(16, 10), grid(12, 10), grid(12, 18)];
            assert_painted(&h, id, &expected, pcbcraft_chrome::Tokens::get().schematic_wire);
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
            h.key_press(Key::Escape);
            h.run();
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
            let end = slide_preview(&mut h, id, grid(0, 8), grid(16, 8));
            h.drop_at(end);
            h.run();
            assert_eq!(h.state().message, None);
            assert_eq!(routes(&h), vec![expected]);
            let after = h.state().session.snapshot().unwrap();
            click(&mut h, "Undo");
            crate::tests::assert_same_content(&before, &h.state().session.snapshot().unwrap());
            click(&mut h, "Redo");
            assert!(Arc::ptr_eq(&after, &h.state().session.snapshot().unwrap()));
        }
    }
}

/// Selecting a whole wire explicitly retains the existing group move/Delete behavior. G over
/// an individual leg starts a connection-preserving slide that commits on the next click.
#[test]
fn whole_wire_actions_and_keyboard_segment_drag_remain_distinct() {
    let mut h = split_harness(false);
    right_click_in(&mut h, 1, grid(8, 3));
    click(&mut h, "Select Wire");
    assert!(view(&h, 1).schematic.selected_leg.is_none());
    let before = h.state().session.snapshot().unwrap();
    let from = view(&h, 1).camera.screen(grid(8, 3), canvas(&h, 1));
    let to = from + vec2(20.0, 30.0);
    h.drag_at(from);
    h.run();
    h.hover_at(to);
    h.run();
    assert!(matches!(view(&h, 1).schematic.tool, Some(Tool::MoveItems { .. })));
    h.drop_at(to);
    h.run();
    assert_eq!(routes(&h), vec![vec![grid(-6, 3), grid(10, 3), grid(10, 9)]]);
    click(&mut h, "Undo");
    h.key_press(Key::Delete);
    h.run();
    assert!(routes(&h).is_empty());
    click(&mut h, "Undo");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    let from = view(&h, 1).camera.screen(grid(8, 3), canvas(&h, 1));
    h.key_press(Key::A);
    h.run();
    h.hover_at(from);
    h.run();
    h.key_press(Key::G);
    h.run();
    assert!(view(&h, 1).schematic.tool.is_none(), "Typing in Parts must not start a segment drag");
    click(&mut h, "Cancel");
    h.hover_at(from);
    h.run();
    h.key_press(Key::G);
    h.run();
    assert!(matches!(view(&h, 1).schematic.tool, Some(Tool::Reshape { .. })));
    select_segment(&mut h, 1, grid(10, 3));
    assert_eq!(routes(&h), vec![vec![grid(-8, 0), grid(10, 0), grid(10, 6), grid(8, 6)]]);
}

/// Cancellation, obstructed release, focus/host changes and external edits discard previews.
/// A stale sub-selection clears instead of silently becoming a whole-wire Delete selection.
#[test]
fn interrupted_segment_drags_and_stale_selections_cannot_commit_or_widen() {
    for interruption in ["escape", "panel", "focus", "collapse", "window", "modal", "edit", "reopen"] {
        let mut h = split_harness(false);
        let mut end = slide_preview(&mut h, 1, grid(8, 3), grid(10, 3));
        match interruption {
            "escape" => h.key_press(Key::Escape),
            "panel" => {
                h.state_mut().browser.open = true;
                h.run();
                end = h.state().browser.rect().center();
                h.hover_at(end);
            }
            "focus" => {
                // Host focus routing can happen independently of the held pointer.
                let app = h.state_mut();
                app.views.active.release_input();
            }
            "collapse" => {
                let ctx = h.ctx.clone();
                h.state_mut().split_editors(&ctx, false);
            }
            "window" => h.event(egui::Event::WindowFocused(false)),
            "modal" => h.state_mut().open_preferences(),
            "edit" => {
                h.state_mut().session.apply("project.rename", &json!({"name":"changed"})).unwrap();
            }
            "reopen" => {
                let snapshot = h.state().session.snapshot().unwrap();
                h.state_mut().session.open_snapshot(snapshot);
            }
            _ => {}
        }
        let before = h.state().session.inspect();
        h.run();
        h.drop_at(end);
        h.run();
        assert_eq!(h.state().session.inspect(), before, "{interruption}");
        assert!(h.state().views.active.schematic.tool.is_none(), "{interruption}");
        if matches!(interruption, "edit" | "reopen") {
            assert!(h.state().views.active.schematic.selection.is_empty());
            h.key_press(Key::Delete);
            h.run();
            assert_eq!(h.state().session.inspect(), before);
        }
    }
}

/// A segment selected before a shared edit is invalidated even without an active drag.
#[test]
fn selected_segments_expire_on_shared_edits_without_whole_wire_fallback() {
    let mut h = split_harness(false);
    select_segment(&mut h, 1, grid(8, 3));
    let app = h.state_mut();
    app.session.apply("project.rename", &json!({"name":"changed"})).unwrap();
    let before = app.session.inspect();
    h.run();
    h.key_press(Key::Delete);
    h.run();
    assert_eq!(h.state().session.inspect(), before);
    assert!(view(&h, 1).schematic.selected_leg.is_none());
    assert!(h.state().views.highlight_for(ViewId(0), &h.state().session).is_empty());
}

/// A metric grid in one pane controls both mouse and G reshaping without borrowing the
/// other pane's coarser grid or snapping the wire's existing off-grid offset away.
#[test]
fn segment_reshape_uses_its_own_panes_custom_grid() {
    for narrow in [false, true] {
        for id in [0, 1] {
            let mut h = split_harness(narrow);
            h.state_mut().set_view_grid(ViewId(id), 500_000);
            h.state_mut().set_view_grid(ViewId(1 - id), 2_540_000);
            h.run();
            let before = h.state().session.snapshot().unwrap();
            let revision = h.state().session.revision();
            let from = grid(8, 3);
            let to = Point { x_nm: 11_160_000, y_nm: from.y_nm };
            if id == 0 {
                let end = slide_preview(&mut h, id, from, to);
                assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
                h.drop_at(end);
                h.run();
            } else {
                select_segment(&mut h, id, from);
                h.key_press(Key::G);
                h.run();
                assert!(matches!(view(&h, id).schematic.tool, Some(Tool::Reshape { .. })));
                select_segment(&mut h, id, to);
            }
            assert_eq!(h.state().message, None);
            assert_eq!(h.state().session.revision(), revision + 1);
            assert_eq!(
                routes(&h),
                vec![vec![grid(-8, 0), Point { x_nm: 11_160_000, y_nm: 0 }, Point { x_nm: 11_160_000, y_nm: 7_620_000 }, grid(8, 6)]]
            );
            assert_eq!(view(&h, id).camera.snap_nm(), 500_000);
            assert_eq!(view(&h, 1 - id).camera.snap_nm(), 2_540_000);
            assert_eq!(h.state().session.project().unwrap().wires()[0].endpoints(), before.wires()[0].endpoints());
            click(&mut h, "Undo");
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        }
    }
}

/// #105's corner and shared-junction handles take priority over segment dragging, even after
/// a segment click. Both split layouts/panes preserve every connected route and shared undo.
#[test]
fn corner_and_branched_endpoint_handles_survive_segment_selection_in_both_panes() {
    for narrow in [false, true] {
        for id in [0, 1] {
            let mut h = split_harness(narrow);
            let revision = h.state().session.revision();
            h.state_mut().run(DRAW_WIRE, &json!({"from":{"junction":"junction-3"},"to":grid(16,6),"bends":[],"expected_revision":revision}));
            let revision = h.state().session.revision();
            h.state_mut().run(DRAW_WIRE, &json!({"from":{"junction":"junction-3"},"to":grid(8,12),"bends":[],"expected_revision":revision}));
            h.run();
            let before = h.state().session.snapshot().unwrap();
            let endpoints: Vec<_> = before.wires().iter().map(|wire| wire.endpoints()).collect();
            for (from, to, expected) in [
                (
                    grid(8, 0),
                    grid(4, 3),
                    vec![
                        vec![grid(-8, 0), grid(-8, 3), grid(4, 3), grid(4, 6), grid(8, 6)],
                        vec![grid(8, 6), grid(16, 6)],
                        vec![grid(8, 6), grid(8, 12)],
                    ],
                ),
                (
                    grid(8, 6),
                    grid(10, 9),
                    vec![
                        vec![grid(-8, 0), grid(10, 0), grid(10, 9)],
                        vec![grid(10, 9), grid(16, 9), grid(16, 6)],
                        vec![grid(10, 9), grid(10, 12), grid(8, 12)],
                    ],
                ),
            ] {
                select_segment(&mut h, id, grid(8, 3));
                let end = slide_preview(&mut h, id, from, to);
                let Some(Tool::Reshape { drag, .. }) = &view(&h, id).schematic.tool else { panic!("handle did not start a reshape") };
                assert!(!matches!(drag.edit, pcbcraft_engine::wire::Reshape::Segment { .. }));
                assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
                h.drop_at(end);
                h.run();
                assert_eq!(h.state().message, None);
                assert_eq!(routes(&h), expected);
                assert_eq!(h.state().session.project().unwrap().wires().iter().map(|wire| wire.endpoints()).collect::<Vec<_>>(), endpoints);
                assert_eq!(view(&h, id).schematic.selection.single_wire(), Some("wire-4"));
                let after = h.state().session.snapshot().unwrap();
                for pane in [0, 1] {
                    assert_painted(&h, pane, &expected[1], pcbcraft_chrome::Tokens::get().schematic_wire);
                }
                focus_tab(&mut h, 1 - id);
                click(&mut h, "Undo");
                assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
                click(&mut h, "Redo");
                assert!(Arc::ptr_eq(&after, &h.state().session.snapshot().unwrap()));
                click(&mut h, "Undo");
            }
        }
    }
}
