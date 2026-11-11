use super::*;
use crate::{ATTACH_JUNCTION, MOVE_JUNCTION};

/// Extract the free junction identity used by these command-built fixtures.
fn junction(endpoint: WireEndpoint) -> u64 {
    let WireEndpoint::Junction(id) = endpoint else { panic!("expected a junction") };
    id
}

/// Add a pin target away from the original route, using the shared placement command.
fn pin_target(session: &mut Session) -> WireEndpoint {
    run(session, PLACE_SYMBOL, json!({"definition":"resistor","x_nm":14 * GRID_NM,"y_nm":14 * GRID_NM})).unwrap();
    WireEndpoint::Terminal(session.project().unwrap().symbols()[0].terminals()[0])
}

/// An explicit drop connects every branch, preserves wire/far-end identities, and previews
/// exactly what is committed. Undo, redo and both file encodings retain that topology.
#[test]
fn attaching_a_branched_junction_to_a_pin_is_one_persistent_edit() {
    let (mut session, _) = free_wire();
    let source = session.project().unwrap().wires()[0].endpoints()[1];
    run(&mut session, DRAW_WIRE, json!({"from":source.inspect(),"to":at(20,10),"bends":[]})).unwrap();
    let target = pin_target(&mut session);
    run(&mut session, DRAW_WIRE, json!({"from":target.inspect(),"to":at(10,25),"bends":[]})).unwrap();
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    assert_eq!(before.connectivity().nets.len(), 2);
    let preview = before.attached(junction(source), super::super::super::Anchor::Existing(target), revision, true).unwrap();
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    run(&mut session, ATTACH_JUNCTION, json!({"junction":source.inspect()["junction"],"to":target.inspect()})).unwrap();
    let after = session.snapshot().unwrap();
    assert_eq!(session.revision(), revision + 1);
    assert_eq!(after.wires, preview.wires);
    assert_eq!(after.junctions, preview.junctions);
    assert_eq!(after.endpoint_position(source), None);
    assert_eq!(after.endpoint_position(target), before.endpoint_position(target));
    assert_eq!(after.connectivity().nets.len(), 1);
    assert_eq!(after.wire_path(&after.wires()[2]), before.wire_path(&before.wires()[2]), "the target's existing route stays fixed");
    for (old, new) in before.wires().iter().zip(after.wires()) {
        assert_eq!(old.id(), new.id());
        assert_eq!(new.endpoints(), old.endpoints().map(|end| if end == source { target } else { end }));
        assert!(after.wire_path(new).windows(2).all(|p| p[0] != p[1] && (p[0].x_nm == p[1].x_nm || p[0].y_nm == p[1].y_nm)));
    }
    for encoding in [Encoding::Json, Encoding::Zip] {
        let bytes = persistence::encode_file(after.clone(), encoding).unwrap();
        let (loaded, _) = persistence::decode_file(&bytes).unwrap();
        assert_eq!(loaded.wires, after.wires);
        assert_eq!(loaded.junctions, after.junctions);
        assert_eq!(loaded.connectivity().nets.len(), 1);
        assert_eq!(persistence::encode_file(loaded, encoding).unwrap(), bytes, "a no-op save is byte stable");
    }
    let documents = persistence::encode_documents(after.clone()).unwrap();
    let loaded = persistence::decode_documents(&documents).unwrap();
    assert_eq!(loaded.wires, after.wires);
    assert_eq!(loaded.junctions, after.junctions);
    assert_eq!(persistence::encode_documents(loaded).unwrap(), documents);
    let packed = persistence::encode_packed(after.clone()).unwrap();
    let loaded = persistence::decode_packed(&packed).unwrap();
    assert_eq!(&loaded.documents, &documents);
    assert_eq!(persistence::encode_packed(loaded.project).unwrap(), packed);
    session.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    session.apply("edit.redo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&after, &session.snapshot().unwrap()));
}

/// Geometry alone never connects, even at exactly the same coordinates. An explicit
/// attachment to one of two coincident endpoints joins only the chosen identity.
#[test]
fn coincident_endpoints_require_an_explicit_identity() {
    let (mut session, _) = free_wire();
    let source = session.project().unwrap().wires()[0].endpoints()[1];
    for end in [at(20, 14), at(14, 20)] {
        run(&mut session, DRAW_WIRE, json!({"from":at(14,14),"to":end,"bends":[]})).unwrap();
    }
    let target = session.project().unwrap().wires()[1].endpoints()[0];
    let separate = session.project().unwrap().wires()[2].endpoints()[0];
    run(&mut session, MOVE_JUNCTION, json!({"junction":source.inspect()["junction"],"to":at(14,14)})).unwrap();
    assert_eq!(session.project().unwrap().connectivity().nets.len(), 3);
    let routes_before = routes(&session);
    run(&mut session, ATTACH_JUNCTION, json!({"junction":source.inspect()["junction"],"to":target.inspect()})).unwrap();
    let project = session.project().unwrap();
    assert_eq!(project.connectivity().nets.len(), 2);
    assert_eq!(routes(&session), routes_before);
    assert_eq!(project.wires()[0].endpoints()[1], target);
    assert_eq!(project.wires()[2].endpoints()[0], separate);
    assert_eq!(project.junctions().count(), 5);
}

/// Invalid, self-looping, duplicate or stale attachments leave history and all project data
/// unchanged. A raw coordinate is not an attachment identity.
#[test]
fn invalid_attachments_are_atomic() {
    let (mut session, _) = free_wire();
    let [source, far] = session.project().unwrap().wires()[0].endpoints();
    run(&mut session, DRAW_WIRE, json!({"from":far.inspect(),"to":at(20,10),"bends":[]})).unwrap();
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    for to in [
        source.inspect(),
        far.inspect(),
        json!({"junction":"junction-999"}),
        json!({"symbol":"symbol-999","terminal":"1"}),
        at(20, 10),
        json!({"junction":"junction-2","extra":true}),
    ] {
        assert!(run(&mut session, ATTACH_JUNCTION, json!({"junction":source.inspect()["junction"],"to":to})).is_err());
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        assert_eq!(session.revision(), revision);
    }
    assert!(run(&mut session, ATTACH_JUNCTION, json!({"junction":"junction-999","to":far.inspect()})).is_err());
    assert!(
        session.apply(ATTACH_JUNCTION, &json!({"junction":source.inspect()["junction"],"to":far.inspect(),"expected_revision":revision-1})).is_err()
    );
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(session.project().unwrap().wires().len(), 1, "failed edits added no history");
}
