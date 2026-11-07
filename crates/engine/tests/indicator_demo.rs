//! Keep the distributed editing demo self-contained and consistent with its walkthrough.

use std::{collections::BTreeSet, fs, path::Path, sync::Arc};

use pcbcraft_engine::{
    MOVE_SYMBOL, Project, Session,
    persistence::{decode_documents, decode_packed, encode_documents, encode_packed},
    schematic::GRID_NM,
};
use serde_json::json;

const ARCHIVE: &[u8] = include_bytes!("../../../examples/dual-led.pc.zip");

/// The distributed version-5 folder and archive migrate to the same canonical documents.
#[test]
fn demo_folder_and_archive_migrate_to_the_same_documents() {
    let project = decode_packed(ARCHIVE).unwrap().project;
    let documents = encode_documents(project.clone()).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/dual-led");
    let saved = documents.keys().map(|path| (path.clone(), fs::read(root.join(path)).unwrap())).collect();
    assert_eq!(documents, encode_documents(decode_documents(&saved).unwrap()).unwrap());
    assert_eq!(decode_packed(ARCHIVE).unwrap().documents.unwrap(), saved);
    let encoded = encode_packed(project).unwrap();
    assert_eq!(encode_documents(decode_packed(&encoded).unwrap().project).unwrap(), documents);
}

/// Check the intended branches and both directions of every pin/pad mapping, including after a move.
#[test]
fn demo_connectivity_and_mappings_survive_an_undoable_edit() {
    let project = decode_packed(ARCHIVE).unwrap().project;
    assert_eq!(project.symbols().len(), 6);
    assert_eq!(project.wires().len(), 12);
    assert_eq!(project.junctions().count(), 4);
    assert!(project.connectivity().unconnected.is_empty());
    let expected: BTreeSet<Vec<String>> =
        [vec!["C1.1", "J1.1", "R1.1", "R2.1"], vec!["C1.2", "D1.2", "D2.2", "J1.2"], vec!["D1.1", "R1.2"], vec!["D2.1", "R2.2"]]
            .into_iter()
            .map(|net| net.into_iter().map(str::to_owned).collect())
            .collect();
    assert_eq!(named_nets(&project).unwrap(), expected);
    let mut pads = BTreeSet::new();
    for symbol in project.symbols() {
        for terminal in symbol.terminals() {
            let pad = project.pad_for_terminal(terminal).unwrap();
            assert_eq!(project.terminal_for_pad(pad), Some(terminal));
            assert!(pads.insert(pad.inspect().to_string()));
        }
    }
    assert_eq!(pads.len(), 12);

    let r1 = project.symbols().iter().find(|symbol| project.reference(symbol) == "R1").unwrap();
    let mut session = Session::default();
    session.open_snapshot(Arc::clone(&project));
    let params = json!({"symbol": r1.id(), "x_nm": r1.position.x_nm, "y_nm": r1.position.y_nm - 2 * GRID_NM,
        "expected_revision": session.revision()});
    session.apply(MOVE_SYMBOL, &params).unwrap();
    let moved = session.snapshot().unwrap();
    assert_ne!(encode_documents(moved.clone()).unwrap(), encode_documents(project.clone()).unwrap());
    assert_eq!(named_nets(&moved).unwrap(), expected);
    for terminal in r1.terminals() {
        assert_eq!(moved.pad_for_terminal(terminal), project.pad_for_terminal(terminal));
    }
    assert!(session.apply(MOVE_SYMBOL, &params).is_err());
    session.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), encode_documents(project).unwrap());
    session.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), encode_documents(moved).unwrap());
}

/// Name nets independently of IDs and traversal order, rejecting unresolved terminals.
fn named_nets(project: &Project) -> Option<BTreeSet<Vec<String>>> {
    project
        .connectivity()
        .nets
        .iter()
        .map(|net| {
            let mut pins = net
                .iter()
                .map(|terminal| Some(format!("{}.{}", project.reference(project.terminal_symbol(*terminal)?), terminal.number())))
                .collect::<Option<Vec<_>>>()?;
            pins.sort();
            Some(pins)
        })
        .collect()
}
