//! Keep the distributed editing demo self-contained and consistent with its walkthrough.

use std::{collections::BTreeSet, fs, path::Path, sync::Arc};

use pcbcraft_engine::{
    MOVE_SYMBOL, Project, Session,
    persistence::{decode_documents, decode_packed, encode_documents, encode_packed},
    schematic::GRID_NM,
};
use serde_json::json;

const ARCHIVE: &[u8] = include_bytes!("../../../examples/dual-led.pc");

/// The distributed version-5 folder and archive migrate to the same canonical documents.
#[test]
fn demo_folder_and_archive_migrate_to_the_same_documents() {
    let project = decode_packed(ARCHIVE).unwrap().project;
    let documents = encode_documents(project.clone()).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/dual-led");
    let saved = documents.keys().map(|path| (path.clone(), fs::read(root.join(path)).unwrap())).collect();
    assert_eq!(documents, encode_documents(decode_documents(&saved).unwrap()).unwrap());
    assert_eq!(decode_packed(ARCHIVE).unwrap().documents, saved);
    let encoded = encode_packed(project).unwrap();
    assert_eq!(encode_documents(decode_packed(&encoded).unwrap().project).unwrap(), documents);
}

/// Check the intended branches and both directions of every pin/pad mapping, including after a move.
#[test]
fn demo_connectivity_and_mappings_survive_an_undoable_edit() {
    let project = decode_packed(ARCHIVE).unwrap().project;
    assert_eq!(project.symbols().len(), 7);
    assert_eq!(project.wires().len(), 12);
    assert_eq!(project.junctions().count(), 4);
    assert_eq!(project.connectivity().unconnected.len(), 3);
    let q = project.symbols().iter().find(|symbol| project.reference(symbol) == "Q1").unwrap();
    assert!(q.terminals().iter().all(|pin| project.connectivity().unconnected.contains(pin)));
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
    assert_eq!(pads.len(), 15);

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

/// The distributed demo carries actual passive/LED copper rectangles and a three-pad SOT23;
/// LED cathodes map to the marked physical pad, independently of symbol numbering.
#[test]
fn demo_packages_have_pinned_geometry_and_correct_polarity_mapping() {
    let project = decode_packed(ARCHIVE).unwrap().project;
    let board = project.board_view(project.board_target().unwrap()).unwrap();
    let expected = [
        ("J1", "header-1x02-254", 2),
        ("R1", "resistor-0402", 2),
        ("R2", "resistor-0603", 2),
        ("C1", "capacitor-0603", 2),
        ("D1", "led-0402", 2),
        ("D2", "led-0402", 2),
        ("Q1", "sot23", 3),
    ];
    for (reference, definition, count) in expected {
        let footprint = board.footprints().find(|f| f.reference == reference).unwrap();
        assert_eq!(footprint.definition, definition);
        assert_eq!(footprint.pads.len(), count);
        assert!(footprint.pads.iter().all(|pad| pad.copper.is_some()));
        assert!(!footprint.graphics.is_empty());
    }
    for symbol in project.symbols().iter().filter(|s| project.reference(s).starts_with('D')) {
        let terminal = symbol.terminals()[1];
        assert_eq!(terminal.number(), "2");
        assert_eq!(project.pad_for_terminal(terminal).unwrap().inspect()["pad"], "1");
    }
    let pads = board.footprints().find(|f| f.reference == "Q1").unwrap().pads;
    assert_eq!(pads[0].center.y_nm, pads[1].center.y_nm);
    assert!(pads[2].center.y_nm < pads[0].center.y_nm);
}
