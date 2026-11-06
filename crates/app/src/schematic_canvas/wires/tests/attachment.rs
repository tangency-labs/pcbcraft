use super::*;
use pcbcraft_engine::{PLACE_SYMBOL, schematic::WireEndpoint};
use std::sync::Arc;

/// Add a pin or junction target through engine commands, returning its exact identity/position.
fn add_target(h: &mut Harness<'_, PcbApp>, pin: bool) -> (WireEndpoint, Point) {
    let session = &mut h.state_mut().session;
    let (command, mut params) = if pin {
        (PLACE_SYMBOL, json!({"definition":"resistor","x_nm":14 * GRID_NM,"y_nm":10 * GRID_NM}))
    } else {
        (DRAW_WIRE, json!({"from":grid(12,10),"to":grid(18,10),"bends":[]}))
    };
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
    let project = session.project().unwrap();
    let endpoint = if pin { WireEndpoint::Terminal(project.symbols()[0].terminals()[0]) } else { project.wires()[1].endpoints()[0] };
    let point = project.endpoint_position(endpoint).unwrap();
    h.run();
    (endpoint, point)
}

/// Release near a pin/explicit junction attaches to its exact coordinate rather than the
/// active grid. The preview is disposable and undo restores the original graph in one step.
#[test]
fn endpoint_drags_attach_to_pins_and_junctions() {
    for pin in [false, true] {
        let mut h = harness();
        let (target, point) = add_target(&mut h, pin);
        h.state_mut().set_view_grid(crate::ViewId(0), 500_000);
        select_wire(&mut h);
        let before = h.state().session.snapshot().unwrap();
        let revision = h.state().session.revision();
        let end = screen(&h, point) + vec2(2.0, 1.0);
        drag(&mut h, grid(8, 6), end);
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        let pos = screen(&h, point);
        assert!(
            h.output().shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Circle(circle) if circle.center == pos && circle.stroke.color == pcbcraft_chrome::Tokens::get().accent)),
            "the exact attachment target has a visible ring"
        );
        drop(&mut h, end);
        assert_eq!(h.state().message, None);
        let after = h.state().session.snapshot().unwrap();
        assert_eq!(after.wires()[0].endpoints()[1], target);
        assert_eq!(after.endpoint_position(target), Some(point));
        assert_eq!(h.state().session.revision(), revision + 1);
        click(&mut h, "Undo");
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        click(&mut h, "Redo");
        assert!(Arc::ptr_eq(&after, &h.state().session.snapshot().unwrap()));
    }
}

/// A gesture in either split pane uses that pane's camera, updates the shared graph, and keeps
/// the opposite pane's camera and edit selection unchanged.
#[test]
fn attachments_work_in_either_split_pane() {
    for narrow in [false, true] {
        for id in [0, 1] {
            let mut h = split_harness(narrow);
            let (target, point) = add_target(&mut h, false);
            focus_tab(&mut h, id);
            select_wire(&mut h);
            let before = h.state().session.snapshot().unwrap();
            let other_center = view(&h, 1 - id).camera.screen(grid(0, 0), canvas(&h, 1 - id));
            let start = view(&h, id).camera.screen(grid(8, 6), canvas(&h, id));
            let end = view(&h, id).camera.screen(point, canvas(&h, id));
            h.drag_at(start);
            h.run();
            h.hover_at(end);
            h.run();
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
            drop(&mut h, end);
            assert_eq!(h.state().message, None);
            assert_eq!(h.state().session.project().unwrap().wires()[0].endpoints()[1], target);
            assert!(view(&h, 1 - id).selection.is_empty());
            assert_eq!(view(&h, 1 - id).camera.screen(grid(0, 0), canvas(&h, 1 - id)), other_center);
            focus_tab(&mut h, 1 - id);
            click(&mut h, "Undo");
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        }
    }
}

/// Escape, a release over UI, focus loss, concurrent edits, and replacing the project discard
/// an attachment preview. The gesture never gets to connect against a newer snapshot.
#[test]
fn cancelled_or_stale_attachments_do_not_connect() {
    for case in 0..5 {
        let mut h = harness();
        let (_, point) = add_target(&mut h, true);
        select_wire(&mut h);
        let end = screen(&h, point);
        drag(&mut h, grid(8, 6), end);
        let mut release = end;
        match case {
            0 => h.key_press(egui::Key::Escape),
            1 => {
                release = h.get_by_label("Capture").rect().center();
                h.hover_at(release);
            }
            2 => h.event(egui::Event::WindowFocused(false)),
            3 => {
                h.state_mut().session.apply("project.rename", &json!({"name":"Changed"})).unwrap();
            }
            _ => {
                let snapshot = h.state().session.snapshot().unwrap();
                h.state_mut().session.open_snapshot(snapshot);
            }
        }
        h.run();
        let before = h.state().session.snapshot().unwrap();
        let revision = h.state().session.revision();
        drop(&mut h, release);
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()), "case {case}");
        assert_eq!(h.state().session.revision(), revision);
    }
}

/// Two coincident identities are ambiguous, and joining the opposite end of the same route
/// would collapse it. Neither release commits, while an ordinary wire crossing stays separate.
#[test]
fn ambiguous_or_invalid_attachment_targets_cancel_the_drop() {
    let mut h = harness();
    let (_, point) = add_target(&mut h, false);
    let revision = h.state().session.revision();
    h.state_mut().session.apply(DRAW_WIRE, &json!({"from":point,"to":grid(12,16),"bends":[],"expected_revision":revision})).unwrap();
    h.run();
    for target in [point, grid(-8, 0)] {
        select_wire(&mut h);
        let before = h.state().session.snapshot().unwrap();
        let end = screen(&h, target);
        drag(&mut h, grid(8, 6), end);
        drop(&mut h, end);
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
    select_wire(&mut h);
    let ends = h.state().session.project().unwrap().wires()[0].endpoints();
    let nets = h.state().session.project().unwrap().connectivity().nets.len();
    let end = screen(&h, grid(15, 10));
    drag(&mut h, grid(8, 6), end);
    drop(&mut h, end);
    assert_eq!(h.state().session.project().unwrap().wires()[0].endpoints(), ends);
    assert_eq!(h.state().session.project().unwrap().connectivity().nets.len(), nets);
}
