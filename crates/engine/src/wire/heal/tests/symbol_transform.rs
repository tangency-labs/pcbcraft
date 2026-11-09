//! Moving and rotating share one candidate and transaction, including deliberate disconnection.
use super::*;

/// Build two original pin-to-free-end routes through the public command path.
fn wired_symbol() -> Session {
    let mut s = session();
    run(&mut s, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":0,"y_nm":0}));
    let pins = s.project().unwrap().symbols()[0].terminals();
    for (pin, to) in pins.into_iter().zip([p(-20, 0), p(20, 0)]) {
        run(&mut s, DRAW_WIRE, json!({"from":pin.inspect(),"to":to,"bends":[]}));
    }
    s
}

/// Preview, command, undo and persistence agree for connected/disconnected moves and rotations
/// in place. No intermediate rotated symbol or moved wires enter history.
#[test]
fn symbol_transform_preview_and_commit_are_one_atomic_edit() {
    for disconnect in [false, true] {
        for auto_heal in [false, true] {
            for (to, degrees) in [(p(0, 0), 90), (p(0, 12), 270)] {
                let mut s = wired_symbol();
                let before = s.snapshot().unwrap();
                let symbol = before.symbols()[0].clone();
                let paths: Vec<_> = before.wires().iter().map(|wire| before.wire_path(wire)).collect();
                let preview = before.transformed_symbol(&symbol.id(), to, Some((degrees / 90) as u8), disconnect, s.revision(), auto_heal).unwrap();
                assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
                run(
                    &mut s,
                    MOVE_SYMBOL,
                    json!({"symbol":symbol.id(),"x_nm":to.x_nm,"y_nm":to.y_nm,"rotation":degrees,"disconnect":disconnect,"auto_heal":auto_heal}),
                );
                let after = s.snapshot().unwrap();
                crate::assert_same_content(&Arc::new(preview), &after);
                assert_eq!((after.symbols()[0].position, after.symbols()[0].rotation), (to, (degrees / 90) as u8));
                if disconnect {
                    assert_eq!(after.wires().iter().map(|wire| after.wire_path(wire)).collect::<Vec<_>>(), paths);
                    assert!(after.wires().iter().all(|wire| wire.terminals().iter().all(|pin| pin.symbol != symbol.ordinal)));
                } else {
                    assert!(before.same_connectivity(&after));
                    assert_eq!(
                        before.wires().iter().map(|wire| wire.endpoints()).collect::<Vec<_>>(),
                        after.wires().iter().map(|wire| wire.endpoints()).collect::<Vec<_>>()
                    );
                }
                let saved = persistence::encode_documents(after.clone()).unwrap();
                assert_eq!(persistence::encode_documents(persistence::decode_documents(&saved).unwrap()).unwrap(), saved);
                s.apply("edit.undo", &Value::Null).unwrap();
                crate::assert_same_content(&before, &s.snapshot().unwrap());
                s.apply("edit.redo", &Value::Null).unwrap();
                crate::assert_same_content(&after, &s.snapshot().unwrap());
            }
        }
    }
}

/// Omitted orientation retains the current pose, identical poses do not disconnect or add
/// history, and invalid/stale transforms cannot publish partial geometry.
#[test]
fn symbol_transform_defaults_noops_and_rejections_preserve_history() {
    let mut s = wired_symbol();
    let symbol = s.project().unwrap().symbols()[0].id();
    run(&mut s, MOVE_SYMBOL, json!({"symbol":symbol,"x_nm":0,"y_nm":0,"rotation":90}));
    run(&mut s, MOVE_SYMBOL, json!({"symbol":symbol,"x_nm":0,"y_nm":p(0,12).y_nm}));
    assert_eq!(s.project().unwrap().symbols()[0].rotation, 1);
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    run(&mut s, MOVE_SYMBOL, json!({"symbol":symbol,"x_nm":0,"y_nm":p(0,12).y_nm,"rotation":90,"disconnect":true}));
    assert_eq!(s.revision(), revision);
    for params in [
        json!({"symbol":symbol,"x_nm":0,"y_nm":0,"rotation":45,"expected_revision":revision}),
        json!({"symbol":symbol,"x_nm":0,"y_nm":0,"rotation":180,"expected_revision":revision-1}),
        json!({"symbol":symbol,"x_nm":i64::MAX,"y_nm":0,"rotation":180,"expected_revision":revision}),
    ] {
        assert!(s.apply(MOVE_SYMBOL, &params).is_err());
        assert_eq!(s.revision(), revision);
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    assert!(before.transformed_symbol(&symbol, p(0, 0), Some(4), false, revision, true).is_err());
    assert_eq!(crate::command(MOVE_SYMBOL).unwrap().to_json()["params"]["properties"]["rotation"]["enum"], json!([0, 90, 180, 270]));
}
