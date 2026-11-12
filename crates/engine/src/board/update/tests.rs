use super::*;
use crate::{CONNECT_PINS, DELETE_SYMBOL, PLACE_RESISTOR, Session, persistence::encode_documents};
use serde_json::json;

/// Capture IDs deliberately differ while content digests can stay equal.
fn source(id: u128) -> SourceCapture {
    SourceCapture { capture_id: Uuid::from_u128(id), content_digest: "a".repeat(64) }
}

/// The serializer's sink refuses overflow before extending its retained byte buffer.
#[test]
fn baseline_serialization_enforces_its_limit_during_writes() {
    use std::io::Write;
    let mut writer = BaselineWriter { bytes: Vec::new() };
    let chunk = vec![b'x'; 64 * 1024];
    for _ in 0..MAX_BASELINE_BYTES / chunk.len() {
        writer.write_all(&chunk).unwrap();
    }
    assert_eq!(writer.bytes.len(), MAX_BASELINE_BYTES);
    assert!(writer.write_all(b"x").is_err());
    assert_eq!(writer.bytes.len(), MAX_BASELINE_BYTES);
}

/// Build all schematic and physical state through the existing shared commands.
pub(super) fn fixture() -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    for x_nm in [0, 10_000_000] {
        session.apply(PLACE_RESISTOR, &json!({"x_nm":x_nm,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    }
    let ids: Vec<_> = session.project().unwrap().components.keys().copied().collect();
    for (index, id) in ids.into_iter().enumerate() {
        let name = if index == 0 { "resistor-0402" } else { "resistor-0603" };
        let definition = super::super::definition::starter(name).unwrap();
        session
            .apply(
                super::super::assignment::ASSIGN_FOOTPRINT,
                &json!({"component":format!("component-{id}"),"library":definition.library,
                    "definition":definition.id,"definition_revision":definition.revision,
                    "mapping":{"1":"1","2":"2"},"expected_revision":session.revision()}),
            )
            .unwrap();
        session
            .apply(
                super::super::assignment::PLACE_ASSIGNED_FOOTPRINT,
                &json!({"component":format!("component-{id}"),"x_nm":index as i64 * 8_000_000,
                    "y_nm":2_000_000,"expected_revision":session.revision()}),
            )
            .unwrap();
    }
    session
}

/// A baseline uses stable logical terminals and never consumes the live model.
fn capture(project: &Project, id: u128) -> Baseline {
    Baseline::proposed(project, &Baseline::current_choices(project), source(id)).unwrap()
}

/// Separate captures of identical content remain distinct historical addresses,
/// but do not force a no-op update, package rewrite or fresh accepted capture.
#[test]
fn identical_capture_is_noop_and_history_addresses_remain_distinct() {
    let session = fixture();
    let project = session.project().unwrap();
    let accepted = capture(project, 1);
    let proposed = capture(project, 2);
    assert_eq!(accepted.source.content_digest, proposed.source.content_digest);
    assert_ne!(accepted.source.capture_id, proposed.source.capture_id);
    assert!(preview(&accepted, &proposed).unwrap().is_noop());
    let bytes = encode_documents(session.snapshot().unwrap()).unwrap();
    let (next, baseline) = apply_candidate(project, &accepted, &proposed).unwrap();
    assert_eq!(baseline, accepted);
    assert_eq!(encode_documents(Arc::new(next)).unwrap(), bytes);
    let encoded = accepted.encode().unwrap();
    assert_eq!(Baseline::decode(&encoded).unwrap(), accepted);
}

/// Added membership and names are resolved through the complete electrical graph,
/// while accepted physical positions and pinned packages stay unchanged.
#[test]
fn connection_changes_are_proposed_without_replacing_board_geometry() {
    let mut session = fixture();
    let accepted = capture(session.project().unwrap(), 1);
    let symbols: Vec<_> = session.project().unwrap().symbols.iter().map(|symbol| symbol.id()).collect();
    let before_board = encode_documents(session.snapshot().unwrap()).unwrap()[&session.project().unwrap().board_document_path().unwrap()].clone();
    session
        .apply(
            CONNECT_PINS,
            &json!({"from":{"symbol":symbols[0],"terminal":"1"},
                "to":{"symbol":symbols[1],"terminal":"2"},"expected_revision":session.revision()}),
        )
        .unwrap();
    let proposed = capture(session.project().unwrap(), 2);
    let plan = preview(&accepted, &proposed).unwrap();
    assert!(plan.components.is_empty());
    assert!(plan.removed_nets.is_empty());
    assert_eq!(plan.added_nets.len(), 1);
    let ids: Vec<_> = proposed.components.keys().copied().collect();
    assert_eq!(
        proposed.nets[0].terminals,
        vec![TerminalKey { component: ids[0], terminal: "1".into() }, TerminalKey { component: ids[1], terminal: "2".into() }]
    );
    let (next, adopted) = apply_candidate(session.project().unwrap(), &accepted, &proposed).unwrap();
    assert_eq!(adopted, proposed);
    let documents = encode_documents(Arc::new(next)).unwrap();
    let before: serde_json::Value = serde_json::from_slice(&before_board).unwrap();
    let after: serde_json::Value = serde_json::from_slice(&documents[&crate::persistence::test_board_path(&documents)]).unwrap();
    for field in ["footprints", "assignments", "setup", "graphics", "outline", "silkscreen"] {
        assert_eq!(after[field], before[field], "physical field {field}");
    }
    let reopened = crate::persistence::decode_documents(&documents).unwrap();
    assert_eq!(reopened.board.baseline.as_deref(), Some(&proposed));
    assert!(accepted.nets.is_empty());
}

/// Global labels across independent flat sheets must become one normalized
/// board connection even though each sheet inspection shows only its own pin.
#[test]
fn capture_uses_project_connectivity_instead_of_the_active_sheet_projection() {
    let mut session = fixture();
    let main = session.project().unwrap().sheet_target();
    session.apply(crate::sheets::CREATE_SHEET, &json!({"name":"Remote","expected_revision":session.revision()})).unwrap();
    let remote = session.project().unwrap().sheets().nth(1).unwrap().target;
    session.apply(PLACE_RESISTOR, &json!({"sheet":remote.id(),"x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let mut expected = Vec::new();
    for sheet in [main, remote] {
        let projection = session.project().unwrap().for_sheet(sheet).unwrap();
        let symbol = &projection.symbols()[0];
        let terminal = symbol.terminals()[0];
        expected.push(TerminalKey { component: symbol.component, terminal: terminal.number().into() });
        let at = projection.terminal_position(terminal).unwrap();
        session
            .apply(
                crate::net_labels::ADD,
                &json!({"sheet":sheet.id(),"name":"SHARED","style":"global","x_nm":at.x_nm,"y_nm":at.y_nm,"expected_revision":session.revision()}),
            )
            .unwrap();
    }
    expected.sort();
    let proposed = capture(session.project().unwrap(), 1);
    assert_eq!(proposed.nets, vec![NetState { terminals: expected, names: vec!["SHARED".into()] }]);
    assert_eq!(session.project().unwrap().for_sheet(remote).unwrap().connectivity().nets[0].len(), 1);
}

/// Explicit replacements and remaps retain physical placement identities and do
/// not infer matching pin numbers from the new package's label.
#[test]
fn package_and_mapping_updates_preserve_placement_identity_and_pose() {
    let session = fixture();
    let project = session.project().unwrap();
    let accepted = capture(project, 1);
    let ids: Vec<_> = accepted.components.keys().copied().collect();
    let old = &project.board.footprints()[0];
    let expected_pose = (old.id, old.x_nm, old.y_nm, old.rotation);
    let mut choices = Baseline::current_choices(project);
    let replacement = choices[&ids[1]].clone();
    choices.insert(ids[0], replacement);
    let proposed = Baseline::proposed(project, &choices, source(2)).unwrap();
    assert!(preview(&accepted, &proposed).unwrap().components.contains(&ComponentChange::Package { component: ids[0] }));
    let (next, replaced) = apply_candidate(project, &accepted, &proposed).unwrap();
    let changed = &next.board.footprints()[0];
    assert_eq!((changed.id, changed.x_nm, changed.y_nm, changed.rotation), expected_pose);
    assert_eq!(changed.definition, "resistor-0603");
    choices.get_mut(&ids[0]).unwrap().mapping = BTreeMap::from([("1".into(), "2".into()), ("2".into(), "1".into())]);
    let proposed = Baseline::proposed(&next, &choices, source(3)).unwrap();
    assert!(preview(&replaced, &proposed).unwrap().components.contains(&ComponentChange::Mapping { component: ids[0] }));
    let (remapped, _) = apply_candidate(&next, &replaced, &proposed).unwrap();
    assert_eq!(remapped.board.footprints()[0].mapping.as_ref().unwrap()["1"], "2");
    assert_eq!(project.board.footprints()[0].definition, "resistor-0402");
}

/// A removed live component remains a valid historical record. The future deletion
/// command supplies this retained board snapshot; Apply removes only its approved
/// placement and package data, and source/current snapshots remain unchanged.
#[test]
fn retained_removal_does_not_require_a_live_schematic_component() {
    let mut session = fixture();
    let old_project = session.project().unwrap().clone();
    let accepted = capture(&old_project, 1);
    let symbol = old_project.symbols[0].id();
    session.apply(DELETE_SYMBOL, &json!({"symbol":symbol,"expected_revision":session.revision()})).unwrap();
    let mut pending = session.project().unwrap().clone();
    pending.board = old_project.board.clone();
    let proposed = capture(&pending, 2);
    let removed = old_project.symbols[0].component;
    assert!(preview(&accepted, &proposed).unwrap().components.contains(&ComponentChange::Removed { component: removed }));
    accepted.validate_assets(&pending).unwrap();
    assert_eq!(Baseline::decode(&accepted.encode().unwrap()).unwrap(), accepted);
    let (next, adopted) = apply_candidate(&pending, &accepted, &proposed).unwrap();
    assert_eq!(pending.board.footprints().len(), 2);
    assert_eq!(next.board.footprints().len(), 1);
    assert!(!adopted.components.contains_key(&removed));
    assert!(!next.board.assignments.contains_key(&removed));
    assert!(encode_documents(Arc::new(next)).is_ok());
}

/// Missing intent, invalid mappings, unavailable pinned definitions and stale
/// schematic inputs all fail before a cloned candidate can be published.
#[test]
fn invalid_and_stale_updates_leave_the_input_unchanged() {
    let mut session = fixture();
    let accepted = capture(session.project().unwrap(), 1);
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    let mut choices = Baseline::current_choices(session.project().unwrap());
    let id = *choices.keys().next().unwrap();
    choices.remove(&id);
    let missing = Baseline::proposed(session.project().unwrap(), &choices, source(2)).unwrap();
    assert_eq!(preview(&accepted, &missing).unwrap().blockers, vec![Blocker::MissingPackage { component: id }]);
    assert!(apply_candidate(session.project().unwrap(), &accepted, &missing).is_err());
    choices = Baseline::current_choices(session.project().unwrap());
    choices.get_mut(&id).unwrap().mapping.insert("1".into(), "2".into());
    assert!(Baseline::proposed(session.project().unwrap(), &choices, source(2)).is_err());
    let proposed = capture(session.project().unwrap(), 2);
    let mut unavailable = session.project().unwrap().clone();
    Arc::make_mut(&mut unavailable.board).definitions.clear();
    assert!(apply_candidate(&unavailable, &accepted, &proposed).is_err());
    let symbol = session.project().unwrap().symbols[0].id();
    let reference = session.project().unwrap().components[&id].reference.clone();
    session
        .apply(crate::SET_SYMBOL_FIELDS, &json!({"symbol":symbol,"reference":reference,"value":"22k","expected_revision":session.revision()}))
        .unwrap();
    assert!(apply_candidate(session.project().unwrap(), &accepted, &proposed).is_err());
    let unchanged = before;
    let original = crate::persistence::decode_documents(&unchanged).unwrap();
    assert_eq!(capture(&original, 1), accepted);
}

/// Native inputs reject duplicate IDs, unknown source terminals, owner mismatches,
/// duplicate membership, invalid capture digests and oversized payloads explicitly.
#[test]
fn malformed_baselines_are_rejected_without_aliasing_history() {
    let session = fixture();
    let baseline = capture(session.project().unwrap(), 1);
    let id = *baseline.components.keys().next().unwrap();
    let bytes = baseline.encode().unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let record = format!("\"{id}\":{}", serde_json::to_string(&baseline.components[&id]).unwrap());
    let duplicated = text.replacen(&record, &format!("{record},{record}"), 1);
    assert!(Baseline::decode(duplicated.as_bytes()).is_err());
    let noncanonical = text.replacen(&record, &record.replacen(&format!("\"{id}\":"), &format!("\"0{id}\":"), 1), 1);
    assert!(Baseline::decode(noncanonical.as_bytes()).is_err());
    let mut invalid = baseline.clone();
    invalid.source.content_digest = "A".repeat(64);
    assert!(invalid.validate().is_err());
    invalid = baseline.clone();
    invalid.nets.push(NetState { terminals: vec![TerminalKey { component: id, terminal: "1".into() }; 2], names: Vec::new() });
    assert!(invalid.validate().is_err());
    invalid = baseline.clone();
    invalid.board = Uuid::from_u128(999);
    assert!(preview(&baseline, &invalid).is_err());
    assert!(Baseline::decode(&vec![b' '; MAX_BASELINE_BYTES + 1]).is_err());
}

/// Repeating a mapping key cannot be hidden by a valid final overwritten value;
/// oversized maps are refused during deserialization before baseline adoption.
#[test]
fn duplicate_or_excessive_package_mappings_are_rejected_while_decoding() {
    let session = fixture();
    let baseline = capture(session.project().unwrap(), 1);
    let package = baseline.components.values().find_map(|component| component.package.as_ref()).unwrap();
    let duplicate = duplicate_mapping(&baseline.encode().unwrap(), &package.mapping);
    assert!(Baseline::decode(&duplicate).is_err());

    let mut excessive = serde_json::to_value(package).unwrap();
    excessive["mapping"] = json!((1..=129).map(|number| (number.to_string(), number.to_string())).collect::<BTreeMap<_, _>>());
    assert!(serde_json::from_value::<Package>(excessive).is_err());
}

/// Corrupt only a negative test payload by duplicating a mapping key with the
/// same value, so ordinary last-key-wins JSON decoding would conceal the error.
pub(super) fn duplicate_mapping(bytes: &[u8], mapping: &BTreeMap<String, crate::library::parts::PadIds>) -> Vec<u8> {
    let text = std::str::from_utf8(bytes).unwrap();
    let encoded = serde_json::to_string(mapping).unwrap();
    let (terminal, pad) = mapping.first_key_value().unwrap();
    let duplicate = format!("{{{}:{},{}", serde_json::to_string(terminal).unwrap(), serde_json::to_string(pad).unwrap(), &encoded[1..]);
    let corrupted = text.replacen(&format!("\"mapping\":{encoded}"), &format!("\"mapping\":{duplicate}"), 1);
    assert_ne!(corrupted, text);
    corrupted.into_bytes()
}
