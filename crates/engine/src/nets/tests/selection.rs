//! Context selections prove membership from the current graph without authored promotion.
use super::*;
use crate::{Terminal, WireEndpoint};

/// Attached free junctions resolve the same typed target as terminals; unsupported and foreign
/// endpoints cannot allocate a registry record, alter history or change saved documents.
#[test]
fn attached_junction_selection_is_read_only_and_identity_based() {
    let (mut s, pins) = circuit(3, false);
    connect(&mut s, &pins[0], &pins[1]);
    let at = s.project().unwrap().terminal_position(Terminal::parse(&pins[1]).unwrap()).unwrap();
    apply(&mut s, crate::DRAW_WIRE, json!({"from":pins[1],"to":{"x_nm":at.x_nm,"y_nm":at.y_nm+12_700_000},"bends":[]}));
    let project = s.snapshot().unwrap();
    let sheet = project.sheet_target();
    let revision = s.revision();
    let saved = encode_documents(project.clone()).unwrap();
    let pin = WireEndpoint::Terminal(Terminal::parse(&pins[0]).unwrap());
    let target = project.net_class_target_for_endpoint(sheet, pin).unwrap().unwrap();
    let junction = project.wires().last().unwrap().endpoints()[1];
    assert!(matches!(junction, WireEndpoint::Junction(_)));
    assert_eq!(project.net_class_target_for_endpoint(sheet, junction).unwrap(), Some(target));
    let bare = WireEndpoint::Terminal(Terminal::parse(&pins[2]).unwrap());
    assert_eq!(project.net_class_target_for_endpoint(sheet, bare).unwrap(), None);
    assert!(project.net_class_target_for_endpoint(sheet, WireEndpoint::Junction(u64::MAX)).is_err());
    assert_eq!(revision, s.revision());
    assert!(Arc::ptr_eq(&project, &s.snapshot().unwrap()));
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
    assert!(project.net_registry.records.is_empty());
}

/// Independent meaningful networks retain different exact targets, even before persistence.
#[test]
fn selected_networks_never_share_authority_through_an_unnamed_display() {
    let (mut s, pins) = circuit(4, false);
    connect(&mut s, &pins[0], &pins[1]);
    connect(&mut s, &pins[2], &pins[3]);
    let project = s.project().unwrap();
    let read = |i| project.net_class_target_for_endpoint(project.sheet_target(), WireEndpoint::Terminal(Terminal::parse(&pins[i]).unwrap())).unwrap();
    assert_eq!(read(0), read(1));
    assert_eq!(read(2), read(3));
    assert_ne!(read(0), read(2));
}
