//! Keep the distributed editing demo self-contained and consistent with its walkthrough.

use std::{collections::BTreeSet, fs, path::Path, sync::Arc};

use pcbcraft_engine::{
    MOVE_SYMBOL, Project, Session,
    persistence::{decode_documents, decode_packed, encode_documents, encode_packed},
    schematic::GRID_NM,
};
use serde_json::json;

const ARCHIVE: &[u8] = include_bytes!("../../../examples/sensor-node.pc");

/// The distributed folder and archive contain the same self-contained, byte-stable documents.
#[test]
fn demo_folder_and_archive_migrate_to_the_same_documents() {
    let project = decode_packed(ARCHIVE).unwrap().project;
    let documents = encode_documents(project.clone()).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sensor-node");
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
    assert_eq!(project.symbols().len(), 17);
    assert!(project.wires().len() >= 30);
    assert!(project.electrical_connectivity().unconnected.is_empty());
    let expected: BTreeSet<Vec<String>> = [
        vec!["C1.1", "C2.1", "C3.1", "D1.1", "D2.1", "J1.1", "J2.1", "J3.1", "PWR1.1", "R3.1", "R4.1", "R5.1", "U1.1", "U2.5"],
        vec!["C1.2", "C2.2", "C3.2", "J1.2", "J2.2", "J3.3", "PWR2.1", "U1.8", "U2.2", "U2.4"],
        vec!["J2.3", "R3.2", "U1.4", "U2.6"],
        vec!["J2.4", "R4.2", "U1.5", "U2.1"],
        vec!["J3.2", "U1.6"],
        vec!["R5.2", "U1.7", "U2.3"],
        vec!["R1.1", "U1.2"],
        vec!["R2.1", "U1.3"],
        vec!["D1.2", "R1.2"],
        vec!["D2.2", "R2.2"],
    ]
    .into_iter()
    .map(|net| net.into_iter().map(str::to_owned).collect())
    .collect();
    assert_eq!(named_nets(&project).unwrap(), expected);
    let mut pads = BTreeSet::new();
    for symbol in project.symbols() {
        if project.component(symbol).unwrap().metadata.attributes.exclude_from_board {
            continue;
        }
        for terminal in symbol.terminals() {
            let pad = project.pad_for_terminal(terminal).unwrap();
            assert_eq!(project.terminal_for_pad(pad), Some(terminal));
            assert!(pads.insert(pad.inspect().to_string()));
        }
    }
    assert_eq!(pads.len(), 43);

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
        .electrical_connectivity()
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

/// The MCU/sensor pin maps and dimensions remain pinned offline, including LED polarity.
#[test]
fn sensor_packages_preserve_manufacturer_pin_numbers_and_original_lands() {
    let project = decode_packed(ARCHIVE).unwrap().project;
    let board = project.board_view(project.board_target().unwrap()).unwrap();
    for (r, definition, count) in
        [("U1", "soic8", 8), ("U2", "sot563", 6), ("J1", "header-1x02-254", 2), ("J2", "header-1x04-254", 4), ("J3", "header-1x03-254", 3)]
    {
        let f = board.footprints().find(|f| f.reference == r).unwrap();
        assert_eq!(f.definition, definition);
        assert_eq!(f.pads.len(), count);
        assert!(f.pads.iter().all(|p| p.copper.is_some()));
        assert!(!f.graphics.is_empty());
    }
    for symbol in project.symbols() {
        for terminal in symbol.terminals() {
            if project.component(symbol).unwrap().metadata.attributes.exclude_from_board {
                continue;
            }
            let expected =
                if project.reference(symbol).starts_with('D') { if terminal.number() == "1" { "2" } else { "1" } } else { terminal.number() };
            assert_eq!(project.pad_for_terminal(terminal).unwrap().inspect()["pad"], expected);
        }
    }
    let mcu = board.footprints().find(|f| f.reference == "U1").unwrap();
    let p = |n: &str| mcu.pads.iter().find(|p| p.number == n).unwrap().center;
    assert_eq!(p("8").x_nm - p("1").x_nm, 5_400_000);
    assert_eq!(p("2").y_nm - p("1").y_nm, 1_270_000);
    let sensor = board.footprints().find(|f| f.reference == "U2").unwrap();
    let p = |n: &str| sensor.pads.iter().find(|p| p.number == n).unwrap().center;
    assert_eq!(p("6").x_nm - p("1").x_nm, 1_480_000);
    assert_eq!(p("2").y_nm - p("1").y_nm, 500_000);
}

/// The demo is fully routed on both copper layers with valid via fanouts and a closed outline.
/// Checking the saved example must report zero findings without changing bytes or undo state.
#[test]
fn complete_demo_has_no_unrouted_or_design_rule_findings() {
    let project = decode_packed(ARCHIVE).unwrap().project;
    let target = project.board_target().unwrap();
    let board = project.boards().find(|board| board.target == target).unwrap();
    assert!(board.tracks().len() > 50);
    assert!(board.vias().len() > 4);
    assert_eq!(board.tracks().iter().map(|track| track.layer).collect::<BTreeSet<_>>().len(), 2);
    let ratsnest = project.board_view(target).unwrap().ratsnest().unwrap();
    assert_eq!(ratsnest.terminal_count, 43);
    assert!(ratsnest.missing.is_empty());
    assert!(ratsnest.lines.is_empty());
    let before = encode_documents(project.clone()).unwrap();
    let mut session = Session::default();
    session.open_snapshot(project);
    let revision = session.revision();
    let report = session.check_design(&json!({"board":target.board})).unwrap();
    assert_eq!(report["total"], 0, "{report}");
    assert_eq!(session.revision(), revision);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
}

/// A wire must leave each capacitor along its lead for at least two grid steps before bending.
#[test]
fn capacitor_contacts_have_straight_wire_clearance() {
    use pcbcraft_engine::schematic::WireEndpoint;
    let project = decode_packed(ARCHIVE).unwrap().project;
    for symbol in project.symbols().iter().filter(|s| project.reference(s).starts_with('C')) {
        for terminal in symbol.terminals() {
            let pin = project.terminal_position(terminal).unwrap();
            let outward = (pin.x_nm - symbol.position.x_nm, pin.y_nm - symbol.position.y_nm);
            let wires: Vec<_> = project.wires().iter().filter(|w| w.endpoints().contains(&WireEndpoint::Terminal(terminal))).collect();
            assert!(!wires.is_empty());
            for wire in wires {
                let mut path = project.wire_path(wire);
                if wire.endpoints()[1] == WireEndpoint::Terminal(terminal) {
                    path.reverse();
                }
                let step = (path[1].x_nm - pin.x_nm, path[1].y_nm - pin.y_nm);
                assert_eq!(
                    step.0 * outward.1 - step.1 * outward.0,
                    0,
                    "{} pin {} bends at its contact",
                    project.reference(symbol),
                    terminal.number()
                );
                assert!(step.0 * outward.0 + step.1 * outward.1 > 0);
                assert!(step.0.abs() + step.1.abs() >= 2 * GRID_NM);
            }
        }
    }
}
