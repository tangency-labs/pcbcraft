use super::*;
use crate::{CONNECT_PINS, PLACE_RESISTOR, Session};
use serde_json::json;
use std::sync::Arc;

/// Author a connected three-component design through the shared production commands.
pub(crate) fn fixture() -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    for index in 0..3 {
        session.apply(PLACE_RESISTOR, &json!({"x_nm":index * 10_000_000,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    }
    let symbols: Vec<_> = session.project().unwrap().symbols.iter().map(|symbol| (symbol.id(), symbol.component)).collect();
    for (index, (symbol, component)) in symbols.iter().enumerate() {
        let definition = super::super::definition::basic_components("resistor-0402").unwrap();
        session
            .apply(
                super::super::assignment::ASSIGN_FOOTPRINT,
                &json!({"component":format!("component-{component}"),"library":definition.library,"definition":definition.id,
                "definition_revision":definition.revision,"mapping":{"1":"2","2":"1"},"expected_revision":session.revision()}),
            )
            .unwrap();
        session
            .apply(
                super::super::assignment::PLACE_ASSIGNED_FOOTPRINT,
                &json!({"component":format!("component-{component}"),"x_nm":(2-index) as i64 * 8_000_000,"y_nm":2_000_000,
                "expected_revision":session.revision()}),
            )
            .unwrap();
        if index > 0 {
            session
                .apply(
                    CONNECT_PINS,
                    &json!({"from":{"symbol":symbols[0].0,"terminal":"1"},
                "to":{"symbol":symbol,"terminal":"1"},"expected_revision":session.revision()}),
                )
                .unwrap();
        }
    }
    let project = session.project().unwrap();
    let at = project.terminal_position(project.symbols[0].terminals()[0]).unwrap();
    session
        .apply(
            crate::net_labels::ADD,
            &json!({"name":"ACCEPTED","style":"global","x_nm":at.x_nm,"y_nm":at.y_nm,"expected_revision":session.revision()}),
        )
        .unwrap();
    session
}

/// Resolve one addressed view without ever selecting a board by display name or list position.
fn ratsnest(project: &crate::Project) -> Ratsnest {
    project.board_view(project.board_target().unwrap()).unwrap().ratsnest().unwrap()
}

/// No provisional schematic connection leaks onto an unaccepted board; accepted mappings
/// resolve real rotated world coordinates and form a deterministic N-1 spatial chain.
#[test]
fn accepted_geometry_is_board_qualified_deterministic_and_mapping_aware() {
    let mut session = fixture();
    let board = session.project().unwrap().board_target().unwrap();
    assert!(session.project().unwrap().board_view(board).unwrap().ratsnest().is_none());
    session.apply(super::super::update::INITIALIZE_BASELINE, &json!({"expected_revision":session.revision()})).unwrap();
    let project = session.project().unwrap();
    let mut foreign = project.board_view(board).unwrap();
    foreign.target.board = uuid::Uuid::from_u128(99);
    assert!(!foreign.has_accepted_connectivity());
    assert!(foreign.ratsnest().is_none());
    let original = ratsnest(project);
    assert_eq!(original, ratsnest(project));
    assert_eq!((original.net_count, original.terminal_count, original.lines.len()), (1, 3, 2));
    assert!(original.missing.is_empty());
    for line in &original.lines {
        assert!(line.from.position.x_nm <= line.to.position.x_nm);
        for endpoint in [&line.from, &line.to] {
            assert_eq!(endpoint.terminal.board, board);
            assert_eq!(endpoint.pad.inspect()["board"], json!(board.board));
            assert_eq!(endpoint.pad.inspect()["pad"], "2");
            assert_eq!(project.board.pad_position(endpoint.pad), Some(endpoint.position));
        }
    }
    let mut moved = project.clone();
    let footprint = &mut Arc::make_mut(&mut moved.board).footprints[0];
    footprint.x_nm = 30_000_000;
    footprint.y_nm = -4_000_000;
    footprint.rotation = 90;
    let geometry = ratsnest(&moved);
    assert_ne!(geometry.lines, original.lines);
    assert_eq!(geometry.source, original.source);
    for line in &geometry.lines {
        for endpoint in [&line.from, &line.to] {
            assert_eq!(moved.board.pad_position(endpoint.pad), Some(endpoint.position));
        }
    }
    assert_eq!(ratsnest(project), original);
}

/// Proposed splits and renames leave accepted membership intact, and deleting the source
/// preserves the retained footprint endpoint until the board update accepts removal.
#[test]
fn proposed_connections_names_and_removals_cannot_rewire_accepted_airwires() {
    let mut session = fixture();
    session.apply(super::super::update::INITIALIZE_BASELINE, &json!({"expected_revision":session.revision()})).unwrap();
    let before = ratsnest(session.project().unwrap());
    let label = session.project().unwrap().net_labels()[0].clone();
    session.apply(crate::net_labels::SET, &json!({"id":label.object_id(),"name":"PROPOSED","style":"global","x_nm":label.at.x_nm,"y_nm":label.at.y_nm,"expected_revision":session.revision()})).unwrap();
    assert_eq!(ratsnest(session.project().unwrap()), before);
    let wire = session.project().unwrap().wires()[0].id();
    session.apply(crate::DELETE_WIRE, &json!({"wire":wire,"expected_revision":session.revision()})).unwrap();
    assert_eq!(ratsnest(session.project().unwrap()), before);
    let symbol = session.project().unwrap().symbols[0].id();
    session
        .apply(crate::SET_SYMBOL_FIELDS, &json!({"symbol":symbol,"reference":"R99","value":"22k","expected_revision":session.revision()}))
        .unwrap();
    assert_eq!(ratsnest(session.project().unwrap()), before);
    session.apply(crate::DELETE_SYMBOL, &json!({"symbol":symbol,"expected_revision":session.revision()})).unwrap();
    assert_eq!(ratsnest(session.project().unwrap()), before);
    session.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(ratsnest(session.project().unwrap()), before);
    let project = session.project().unwrap();
    let mut next = project.clone();
    let mut proposed = project.board.baseline.as_deref().unwrap().clone();
    proposed.nets[0].terminals.pop();
    Arc::make_mut(&mut next.board).baseline = Some(Arc::new(proposed));
    assert_eq!(ratsnest(&next).lines.len(), 1);
}

/// Missing placement, mapping and physical pad geometry report their terminal addresses
/// separately; no line endpoint is fabricated at the footprint origin or board origin.
#[test]
fn unavailable_accepted_endpoints_are_explicit_without_bogus_coordinates() {
    let mut session = fixture();
    session.apply(super::super::update::INITIALIZE_BASELINE, &json!({"expected_revision":session.revision()})).unwrap();
    let project = session.project().unwrap();
    for reason in [MissingReason::Unplaced, MissingReason::MissingMapping, MissingReason::MissingPad] {
        let mut changed = project.clone();
        let board = Arc::make_mut(&mut changed.board);
        let component = board.footprints[0].component;
        match reason {
            MissingReason::Unplaced => {
                board.by_component.remove(&component);
            }
            MissingReason::MissingMapping => {
                board.footprints[0].mapping.as_mut().unwrap().remove("1");
            }
            MissingReason::MissingPad => {
                board.footprints[0].mapping.as_mut().unwrap().insert("1".into(), "999".into());
            }
        }
        let result = ratsnest(&changed);
        assert_eq!(result.lines.len(), 1);
        assert_eq!(
            result.missing,
            vec![MissingEndpoint { terminal: TerminalAddress { board: result.board, component, terminal: "1".into() }, reason }]
        );
        assert!(result.lines.iter().all(|line| line.from.terminal.component != component && line.to.terminal.component != component));
    }
}
