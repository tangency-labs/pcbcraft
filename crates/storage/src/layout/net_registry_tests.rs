use super::*;
use pcbcraft_engine::{
    CONNECT_PINS, PLACE_RESISTOR, Session, nets,
    persistence::{decode_documents, encode_documents},
};
use serde_json::{Value, json};

/// Run document edits through the ordinary shared command path with current revision authority.
fn apply(session: &mut Session, command: &str, mut params: Value) {
    if !matches!(command, "project.new" | "edit.undo" | "edit.redo") {
        params["expected_revision"] = json!(session.revision());
    }
    session.apply(command, &params).unwrap();
}

/// Build a small unnamed network with an explicit board or schematic owner.
fn circuit(board: bool) -> Session {
    let mut session = Session::default();
    apply(&mut session, "project.new", json!({"create_board":board}));
    for x in [0, 25_400_000] {
        apply(&mut session, PLACE_RESISTOR, json!({"x_nm":x,"y_nm":0}));
    }
    apply(&mut session, CONNECT_PINS, json!({"from":{"symbol":"symbol-2","terminal":"1"},"to":{"symbol":"symbol-3","terminal":"1"}}));
    session
}

/// Every publication checkpoint must recover the complete old or new identity/graph bundle;
/// a subsequent unchanged save cannot stage or change bytes.
fn interruption_matrix(before: &Documents, after: &Documents) {
    let probe = tempfile::tempdir().unwrap();
    let root = probe.path().join("probe");
    crate::save(&root, None, before, &mut |_| Ok(())).unwrap();
    let mut steps = Vec::new();
    crate::save(&root, Some(before), after, &mut |step| {
        steps.push(step.to_owned());
        Ok(())
    })
    .unwrap();
    for (failed, step) in steps.iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        crate::save(&root, None, before, &mut |_| Ok(())).unwrap();
        let mut index = 0;
        assert!(
            crate::save(&root, Some(before), after, &mut |_| {
                let stop = index == failed;
                index += 1;
                if stop { Err("interrupted".into()) } else { Ok(()) }
            })
            .is_err()
        );
        let committed = root.join(".pcbcraft-save").exists() || step == "completed";
        let loaded = crate::load(&root).unwrap();
        assert_eq!(&loaded, if committed { after } else { before }, "{step}");
        decode_documents(&loaded).unwrap();
        crate::save(&root, Some(&loaded), &loaded, &mut |_| Err("no-op staged bytes".into())).unwrap();
    }
}

/// First registry creation upgrades only metadata in either ownership home, and interruption
/// cannot expose a new header paired with old/missing logical identity records.
#[test]
fn first_registry_record_and_undo_recover_at_every_checkpoint() {
    for board in [false, true] {
        let mut session = circuit(board);
        let before = encode_documents(session.snapshot().unwrap()).unwrap();
        apply(&mut session, nets::PROMOTE, json!({"anchor":{"symbol":"symbol-2","terminal":"1"}}));
        let after = encode_documents(session.snapshot().unwrap()).unwrap();
        interruption_matrix(&before, &after);
        interruption_matrix(&after, &before);
        if board {
            let path = session.project().unwrap().board_document_path().unwrap();
            assert_eq!(after[&path], before[&path]);
        }
    }
}

/// First-board creation and its undo move the same registry in the existing atomic folder save.
#[test]
fn first_board_registry_scope_transfer_recovers_at_every_checkpoint() {
    let mut session = circuit(false);
    apply(&mut session, nets::PROMOTE, json!({"anchor":{"symbol":"symbol-2","terminal":"1"}}));
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    apply(&mut session, "pcb.create_board", json!({}));
    let after = encode_documents(session.snapshot().unwrap()).unwrap();
    interruption_matrix(&before, &after);
    interruption_matrix(&after, &before);
    apply(&mut session, "edit.undo", json!({}));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
}

/// First class bindings version both metadata homes through the existing journal, including
/// interruption at every checkpoint and exact Undo publication; physical baseline bytes stay fixed.
#[test]
fn first_class_assignment_and_first_board_transfer_recover_at_every_checkpoint() {
    for board in [false, true] {
        let mut session = circuit(board);
        apply(&mut session, pcbcraft_engine::net_classes::CREATE, json!({"name":"Power","rules":{"clearance_nm":500000}}));
        let class = session.project().unwrap().net_classes().definitions[0].id;
        let target = json!({"kind":"derived","anchor":session.project().unwrap().inspect_net_registry().unwrap()["nets"][0]["anchor"]});
        let before = encode_documents(session.snapshot().unwrap()).unwrap();
        apply(&mut session, nets::SET_CLASSES, json!({"target":target,"classes":[class]}));
        let after = encode_documents(session.snapshot().unwrap()).unwrap();
        interruption_matrix(&before, &after);
        interruption_matrix(&after, &before);
        if board {
            let path = session.project().unwrap().board_document_path().unwrap();
            assert_eq!(before[&path], after[&path]);
        } else {
            apply(&mut session, "pcb.create_board", json!({}));
            let transferred = encode_documents(session.snapshot().unwrap()).unwrap();
            interruption_matrix(&after, &transferred);
            interruption_matrix(&transferred, &after);
        }
    }
}
