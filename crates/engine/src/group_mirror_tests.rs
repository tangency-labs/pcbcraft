//! A selected group reflects about its grid-snapped center as one revision-checked edit.
use super::*;
use crate::{
    persistence::encode_documents,
    schematic::{GRID_NM, MirrorAxis, Point, WireEndpoint},
    selection::Selection,
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Keep commands on the same validated GUI/MCP path while supplying the current revision.
fn apply(session: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

/// A point in whole grid steps.
fn grid(x: i64, y: i64) -> Point {
    Point { x_nm: x * GRID_NM, y_nm: y * GRID_NM }
}

/// Two resistors joined by a bent wire, and a local label wired to the first resistor's free pin.
fn fixture() -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name": "Group mirror"})).unwrap();
    for at in [grid(0, 0), grid(20, 6)] {
        apply(&mut session, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":at.x_nm,"y_nm":at.y_nm}));
    }
    let [first, second] = [0, 1].map(|index| session.project().unwrap().symbols()[index].id());
    apply(
        &mut session,
        DRAW_WIRE,
        json!({"from":{"symbol":first,"terminal":"2"},"to":{"symbol":second,"terminal":"1"},"bends":[grid(10, 0),grid(10, 6)]}),
    );
    apply(&mut session, net_labels::ADD, json!({"name":"SIG","style":"local","x_nm":-10 * GRID_NM,"y_nm":0}));
    let label = WireEndpoint::NetLabel(session.project().unwrap().net_labels()[0].id).inspect();
    apply(&mut session, DRAW_WIRE, json!({"from":{"symbol":first,"terminal":"1"},"to":label,"bends":[]}));
    session
}

/// Select every authored object using identities from inspection.
fn all(project: &Project) -> Selection {
    Selection {
        symbols: project.symbols().iter().map(|symbol| symbol.id()).collect(),
        wires: project.wires().iter().map(|wire| wire.id()).collect(),
        net_labels: project.net_labels().iter().map(|label| label.object_id()).collect(),
        ..Selection::default()
    }
}

/// Both axes reflect every selected position about the snapped selection center, compose each
/// symbol's pose like Mirror Symbol, keep the netlist, and take exactly one undo step. Mirroring
/// again on the same axis restores the exact saved documents.
#[test]
fn group_mirror_reflects_about_the_snapped_center_in_one_undo_step() {
    // Anchors span x -10..20 and y 0..6 grid steps, so the doubled pivots are 10 and 6 steps.
    for (axis, reflect, label_rotation) in [
        (MirrorAxis::Horizontal, (|p: Point| Point { x_nm: 10 * GRID_NM - p.x_nm, y_nm: p.y_nm }) as fn(Point) -> Point, 180),
        (MirrorAxis::Vertical, |p: Point| Point { x_nm: p.x_nm, y_nm: 6 * GRID_NM - p.y_nm }, 0),
    ] {
        let mut session = fixture();
        let before = session.snapshot().unwrap();
        let documents = encode_documents(before.clone()).unwrap();
        let revision = session.revision();
        session.apply(MIRROR_ITEMS, &json!({"items":all(&before),"axis":axis.as_str(),"expected_revision":revision})).unwrap();
        assert_eq!(session.revision(), revision + 1);
        let after = session.snapshot().unwrap();

        for (old, new) in before.symbols().iter().zip(after.symbols()) {
            assert_eq!(new.id(), old.id());
            assert_eq!(new.position, reflect(old.position), "{axis:?}");
            assert_eq!((new.turns(), new.is_mirrored()), schematic::mirror_pose(old.turns(), old.is_mirrored(), axis));
            assert!(new.pins().iter().all(|pin| pin.x_nm % GRID_NM == 0 && pin.y_nm % GRID_NM == 0), "{axis:?} keeps pins on grid");
        }
        let (old, new) = (&before.net_labels()[0], &after.net_labels()[0]);
        assert_eq!((new.at, new.rotation), (reflect(old.at), label_rotation), "{axis:?}");
        for (old, new) in before.wires().iter().zip(after.wires()) {
            assert_eq!(new.id(), old.id());
            let path: Vec<_> = before.wire_path(old).into_iter().map(reflect).collect();
            assert_eq!(after.wire_path(new), path, "{axis:?}");
        }
        assert_eq!(before.inspect_schematic()["netlist"], after.inspect_schematic()["netlist"], "{axis:?}");

        session.apply("edit.undo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        session.apply("edit.redo", &Value::Null).unwrap();
        assert!(Arc::ptr_eq(&after, &session.snapshot().unwrap()));

        apply(&mut session, MIRROR_ITEMS, json!({"items":all(&after),"axis":axis.as_str()}));
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), documents, "{axis:?} twice restores the saved bytes");
    }
}

/// An odd span keeps its bounds: the doubled pivot is exact, so two parts three steps apart
/// swap places instead of shifting the group by a grid step.
#[test]
fn odd_sized_groups_keep_their_bounds() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name": "Odd group"})).unwrap();
    for x in [0, 3] {
        apply(&mut session, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":x * GRID_NM,"y_nm":0}));
    }
    let items = all(session.project().unwrap());
    apply(&mut session, MIRROR_ITEMS, json!({"items":items,"axis":"horizontal"}));
    let positions: Vec<_> = session.project().unwrap().symbols().iter().map(|symbol| symbol.position).collect();
    assert_eq!(positions, [grid(3, 0), grid(0, 0)]);
}

/// A selection of one symbol gives exactly the documents of Mirror Symbol, attached wires included.
#[test]
fn a_single_symbol_selection_matches_mirror_symbol() {
    for axis in ["horizontal", "vertical"] {
        let mut session = fixture();
        let symbol = session.project().unwrap().symbols()[0].id();
        apply(&mut session, MIRROR_SYMBOL, json!({"symbol":symbol,"axis":axis}));
        let single = encode_documents(session.snapshot().unwrap()).unwrap();
        session.apply("edit.undo", &Value::Null).unwrap();
        apply(&mut session, MIRROR_ITEMS, json!({"items":Selection::one_symbol(symbol),"axis":axis}));
        assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), single, "{axis}");
    }
}

/// Stale revisions, unknown or empty selections, bad axes and items on another sheet are refused
/// before any change, leaving the snapshot and revision untouched.
#[test]
fn invalid_group_mirrors_change_nothing() {
    let mut session = fixture();
    let items = all(session.project().unwrap());
    apply(&mut session, crate::sheets::CREATE_SHEET, json!({"name":"Second"}));
    let [main, second] = [0, 1].map(|index| session.project().unwrap().sheets().nth(index).unwrap().target.id());
    apply(&mut session, PLACE_SYMBOL, json!({"sheet":second,"definition":"resistor","x_nm":0,"y_nm":0}));
    let remote = {
        let project = session.project().unwrap();
        let target = project.sheets().nth(1).unwrap().target;
        project.for_sheet(target).unwrap().symbols()[0].id()
    };
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    for params in [
        json!({"sheet":main,"items":items,"axis":"horizontal","expected_revision":revision - 1}),
        json!({"sheet":main,"items":items,"axis":"horizontal","expected_revision":revision + 1}),
        json!({"sheet":main,"items":items,"axis":"diagonal","expected_revision":revision}),
        json!({"sheet":main,"items":items,"expected_revision":revision}),
        json!({"sheet":main,"items":{"symbols":["symbol-999"],"wires":[]},"axis":"vertical","expected_revision":revision}),
        json!({"sheet":main,"items":{"symbols":[],"wires":[]},"axis":"vertical","expected_revision":revision}),
        json!({"sheet":main,"items":{"symbols":[remote],"wires":[]},"axis":"vertical","expected_revision":revision}),
    ] {
        assert!(session.apply(MIRROR_ITEMS, &params).is_err(), "{params}");
        assert_eq!(session.revision(), revision);
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()), "{params}");
    }
}
