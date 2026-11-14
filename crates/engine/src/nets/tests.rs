use super::*;
use crate::{
    Session,
    persistence::{ENTRY_POINT, decode_documents, decode_packed, encode_documents, encode_packed},
    selection::Selection,
};
use serde_json::{Value, json};
use std::sync::Arc;

mod codec;

/// All circuit authoring uses the same revision-checked command path as GUI and MCP.
fn apply(session: &mut Session, command: &str, mut params: Value) {
    if !matches!(command, "project.new" | "project.rename" | "edit.undo" | "edit.redo") {
        params["expected_revision"] = json!(session.revision());
    }
    session.apply(command, &params).unwrap();
}

/// Create independent original resistor terminals without naming or promoting their networks.
fn circuit(count: usize, board: bool) -> (Session, Vec<Value>) {
    let mut session = Session::default();
    apply(&mut session, "project.new", json!({"create_board":board}));
    let mut pins = Vec::new();
    for index in 0..count {
        apply(&mut session, crate::PLACE_RESISTOR, json!({"x_nm":index as i64*25_400_000,"y_nm":0}));
        pins.push(json!({"symbol":session.project().unwrap().symbols().last().unwrap().id(),"terminal":"1"}));
    }
    (session, pins)
}

/// Connect two exact terminal appearances, returning the new authored wire ID.
fn connect(session: &mut Session, a: &Value, b: &Value) -> String {
    apply(session, crate::CONNECT_PINS, json!({"from":a,"to":b}));
    session.project().unwrap().wires().last().unwrap().id()
}

/// Resolve a current terminal/label network to its active persistent authority.
fn id_for(session: &Session, anchor: &Value) -> Uuid {
    let project = session.project().unwrap();
    let graph = graph::Graph::build(project).unwrap();
    let anchor = graph.target(project.sheet_target(), anchor).unwrap();
    let index = graph.network(&anchor).unwrap();
    project
        .net_registry
        .records
        .iter()
        .find(|record| matches!(record.state, State::Active {}) && graph.network(&record.primary) == Some(index))
        .unwrap()
        .id
}

/// Promote a supported network and retain its independent UUID for later assertions.
fn promote(session: &mut Session, anchor: &Value) -> Uuid {
    apply(session, PROMOTE, json!({"anchor":anchor}));
    id_for(session, anchor)
}

/// Folder and archive codecs must reopen the exact registry, decisions and deterministic bytes.
fn round_trip(session: &Session) {
    let project = session.snapshot().unwrap();
    let docs = encode_documents(project.clone()).unwrap();
    let reopened = decode_documents(&docs).unwrap();
    assert_eq!(reopened.net_registry, project.net_registry);
    assert_eq!(encode_documents(reopened).unwrap(), docs);
    assert_eq!(decode_packed(&encode_packed(project).unwrap()).unwrap().documents, docs);
}

/// Reading never creates identities; explicit promotion is undoable and idempotent, while the
/// empty-registry project retains project/16 and its exact original document bytes.
#[test]
fn promotion_is_sparse_revision_checked_undoable_and_read_only_inspection_allocates_nothing() {
    let (mut session, pins) = circuit(2, false);
    connect(&mut session, &pins[0], &pins[1]);
    let original = session.snapshot().unwrap();
    let docs = encode_documents(original.clone()).unwrap();
    let revision = session.revision();
    assert_eq!(serde_json::from_slice::<Value>(&docs[ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/16");
    assert_eq!(session.inspect(), session.inspect());
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&original, &session.snapshot().unwrap()));
    assert_eq!(encode_documents(decode_documents(&docs).unwrap()).unwrap(), docs);
    let id = promote(&mut session, &pins[0]);
    let project = session.snapshot().unwrap();
    assert_eq!(project.net_registry.records.len(), 1);
    assert_eq!(project.net_registry.record(id).unwrap().fallbacks.len(), 1);
    let saved = encode_documents(project.clone()).unwrap();
    let entry: Value = serde_json::from_slice(&saved[ENTRY_POINT]).unwrap();
    assert_eq!(entry["format"], "pcbcraft-project/17");
    assert!(entry["schematic_instances"]["net_registry"].is_object());
    assert!(!saved.keys().any(|path| path.ends_with("board.json") || path.ends_with("instances.json")));
    let revision = session.revision();
    assert_eq!(promote(&mut session, &pins[1]), id);
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&project, &session.snapshot().unwrap()));
    let typed = project.net_registry.record(id).unwrap().primary.clone();
    apply(&mut session, PROMOTE, json!({"anchor":typed}));
    assert_eq!(session.revision(), revision);
    assert!(session.apply(PROMOTE, &json!({"anchor":pins[0],"expected_revision":revision-1})).is_err());
    assert!(Arc::ptr_eq(&project, &session.snapshot().unwrap()));
    round_trip(&session);
    apply(&mut session, "edit.undo", json!({}));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), docs);
    apply(&mut session, "edit.redo", json!({}));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), saved);
}

/// Bare pins and dangling one-pin stubs do not compete with the remaining meaningful net,
/// and reconnecting a previously bare pin is continuation rather than a fictitious merge.
#[test]
fn bare_and_stub_disconnects_and_reconnects_keep_the_id_without_ambiguity() {
    for stub in [false, true] {
        let (mut session, pins) = circuit(3, false);
        connect(&mut session, &pins[0], &pins[1]);
        let branch = connect(&mut session, &pins[1], &pins[2]);
        if stub {
            let at = session.project().unwrap().terminal_position(crate::Terminal::parse(&pins[2]).unwrap()).unwrap();
            apply(&mut session, crate::DRAW_WIRE, json!({"from":pins[2],"to":{"x_nm":at.x_nm,"y_nm":at.y_nm+12_700_000},"bends":[]}));
        }
        let id = promote(&mut session, &pins[0]);
        apply(&mut session, crate::DELETE_WIRE, json!({"wire":branch}));
        assert_eq!(id_for(&session, &pins[0]), id);
        assert_eq!(session.project().unwrap().net_registry.records.len(), 1);
        connect(&mut session, &pins[1], &pins[2]);
        assert_eq!(id_for(&session, &pins[2]), id);
        assert!(matches!(session.project().unwrap().net_registry.record(id).unwrap().state, State::Active {}));
        round_trip(&session);
    }
}

/// Persisted fallback order does not refill when members arrive or a fallback vanishes. With
/// no saved anchor remaining on the sole continuation, canonical C/D replaces A/B in one edit.
#[test]
fn fallback_retention_and_canonical_primary_replacement_survive_history_and_reopen() {
    let (mut session, pins) = circuit(4, false);
    connect(&mut session, &pins[0], &pins[1]);
    let id = promote(&mut session, &pins[0]);
    let original = session.project().unwrap().net_registry.record(id).unwrap().clone();
    let detach = connect(&mut session, &pins[0], &pins[2]);
    connect(&mut session, &pins[2], &pins[3]);
    assert_eq!(session.project().unwrap().net_registry.record(id).unwrap(), &original);
    apply(&mut session, crate::DELETE_SYMBOL, json!({"symbol":pins[1]["symbol"]}));
    assert!(session.project().unwrap().net_registry.record(id).unwrap().fallbacks.is_empty());
    let before = session.project().unwrap().net_registry.clone();
    apply(&mut session, crate::DELETE_WIRE, json!({"wire":detach}));
    assert_eq!(id_for(&session, &pins[2]), id);
    let after = session.project().unwrap().net_registry.clone();
    let replacement = after.record(id).unwrap();
    assert_ne!(replacement.primary, original.primary);
    assert_eq!(replacement.fallbacks.len(), 1);
    assert_eq!(after.records.len(), 1);
    round_trip(&session);
    apply(&mut session, "edit.undo", json!({}));
    assert_eq!(session.project().unwrap().net_registry, before);
    apply(&mut session, "edit.redo", json!({}));
    assert_eq!(session.project().unwrap().net_registry, after);
}

/// Deleting a primary retains its identity on the surviving fallback rather than a new UUID.
#[test]
fn deleting_a_primary_adopts_a_surviving_fallback() {
    let (mut session, pins) = circuit(3, false);
    connect(&mut session, &pins[0], &pins[1]);
    let id = promote(&mut session, &pins[0]);
    connect(&mut session, &pins[1], &pins[2]);
    let fallback = session.project().unwrap().net_registry.record(id).unwrap().fallbacks[0].clone();
    apply(&mut session, crate::DELETE_SYMBOL, json!({"symbol":pins[0]["symbol"]}));
    assert_eq!(session.project().unwrap().net_registry.record(id).unwrap().primary, fallback);
    assert_eq!(id_for(&session, &pins[1]), id);
    round_trip(&session);
}

/// An anchored split creates only the additional meaningful identity; a later actual merge
/// retires both source IDs and stores a fresh ID with exact source-capture lineage.
#[test]
fn anchored_split_and_merge_preserve_lineage_and_never_reassign_retired_ids() {
    let (mut session, pins) = circuit(4, true);
    connect(&mut session, &pins[0], &pins[1]);
    let middle = connect(&mut session, &pins[1], &pins[2]);
    connect(&mut session, &pins[2], &pins[3]);
    let original = promote(&mut session, &pins[0]);
    apply(&mut session, crate::DELETE_WIRE, json!({"wire":middle}));
    assert_eq!(id_for(&session, &pins[0]), original);
    let child = id_for(&session, &pins[2]);
    assert_ne!(child, original);
    let record = session.project().unwrap().net_registry.record(child).unwrap();
    assert_eq!(record.lineage.len(), 1);
    assert_eq!(record.lineage[0].net, Some(original));
    record.lineage[0].snapshot.validate().unwrap();
    round_trip(&session);
    connect(&mut session, &pins[1], &pins[2]);
    let merged = id_for(&session, &pins[0]);
    assert!(![original, child].contains(&merged));
    let registry = session.project().unwrap().net_registry.clone();
    assert_eq!(
        registry.record(merged).unwrap().lineage.iter().filter_map(|s| s.net).collect::<std::collections::BTreeSet<_>>(),
        [original, child].into()
    );
    for id in [original, child] {
        assert!(matches!(&registry.record(id).unwrap().state,State::Retired {reason:Retirement::Merged,successors,..} if successors==&vec![merged]));
    }
    round_trip(&session);
    apply(&mut session, "edit.undo", json!({}));
    apply(&mut session, "edit.redo", json!({}));
    assert_eq!(session.project().unwrap().net_registry, registry);
    apply(&mut session, "edit.undo", json!({}));
    connect(&mut session, &pins[1], &pins[2]);
    assert!(![original, child, merged].contains(&id_for(&session, &pins[0])));
}

/// A lost-anchor split is represented explicitly, then one chosen branch retains the source
/// identity through undo/redo, reopening and provisional-identity retirement.
#[test]
fn ambiguous_split_has_explicit_persisted_revision_checked_resolution() {
    let (mut session, pins) = circuit(6, false);
    connect(&mut session, &pins[0], &pins[1]);
    let id = promote(&mut session, &pins[0]);
    connect(&mut session, &pins[0], &pins[2]);
    connect(&mut session, &pins[2], &pins[3]);
    connect(&mut session, &pins[0], &pins[4]);
    connect(&mut session, &pins[4], &pins[5]);
    let selected = Selection {
        symbols: [pins[0]["symbol"].as_str().unwrap().into(), pins[1]["symbol"].as_str().unwrap().into()].into(),
        ..Selection::default()
    };
    apply(&mut session, crate::DELETE_ITEMS, json!({"items":selected}));
    let ambiguous = session.project().unwrap().net_registry.clone();
    assert!(matches!(&ambiguous.record(id).unwrap().state,State::Ambiguous {branches,..} if branches.len()==2));
    let provisional = id_for(&session, &pins[2]);
    let other = id_for(&session, &pins[4]);
    assert_ne!(provisional, other);
    round_trip(&session);
    let before = session.snapshot().unwrap();
    assert!(session.apply(RESOLVE_AMBIGUITY, &json!({"net":id,"anchor":pins[2],"expected_revision":session.revision()-1})).is_err());
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    apply(&mut session, RESOLVE_AMBIGUITY, json!({"net":id,"anchor":pins[2]}));
    assert_eq!(id_for(&session, &pins[2]), id);
    assert_eq!(id_for(&session, &pins[4]), other);
    assert!(matches!(
        session.project().unwrap().net_registry.record(provisional).unwrap().state,
        State::Retired { reason: Retirement::Resolution, .. }
    ));
    let resolved = session.project().unwrap().net_registry.clone();
    round_trip(&session);
    apply(&mut session, "edit.undo", json!({}));
    assert_eq!(session.project().unwrap().net_registry, ambiguous);
    apply(&mut session, "edit.redo", json!({}));
    assert_eq!(session.project().unwrap().net_registry, resolved);
    apply(&mut session, "edit.undo", json!({}));
    connect(&mut session, &pins[3], &pins[4]);
    let merged = id_for(&session, &pins[2]);
    assert!(matches!(&session.project().unwrap().net_registry.record(id).unwrap().state,
        State::Ambiguous {branches,..} if branches == &vec![merged]));
    round_trip(&session);
    apply(&mut session, RESOLVE_AMBIGUITY, json!({"net":id,"anchor":pins[2]}));
    assert_eq!(id_for(&session, &pins[4]), id);
    round_trip(&session);
}

/// Allocation refusal at the canonical transaction boundary cannot publish a partial named edit.
#[test]
fn exhausted_registry_preserves_the_project_revision_history_and_update_preview() {
    let (mut session, pins) = circuit(2, true);
    connect(&mut session, &pins[0], &pins[1]);
    let id = promote(&mut session, &pins[0]);
    let mut project = session.project().unwrap().clone();
    let mut template = project.net_registry.record(id).unwrap().clone();
    template.state =
        State::Retired { snapshot: crate::board::update::capture_source(&project).unwrap(), reason: Retirement::Removed, successors: Vec::new() };
    project.net_registry =
        Arc::new(Registry { records: (1..=MAX_RECORDS as u128).map(|id| Record { id: Uuid::from_u128(id), ..template.clone() }).collect() });
    project.net_registry.validate(&project).unwrap();
    session.open_snapshot(Arc::new(project));
    apply(&mut session, crate::board::update::PREVIEW_UPDATE, json!({}));
    let before = session.snapshot().unwrap();
    let revision = session.revision();
    let report = session.board_update_report();
    let undo = session.undo.len();
    let redo = session.redo.len();
    let result =
        session.apply(crate::net_labels::ADD, &json!({"name":"REFUSED","style":"local","x_nm":0,"y_nm":25_400_000,"expected_revision":revision}));
    assert!(result.is_err());
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), revision);
    assert_eq!((session.undo.len(), session.redo.len()), (undo, redo));
    assert_eq!(session.board_update_report(), report);
}

/// Named singleton records arise on authoring, survive renames, and become retired context when
/// deleted; no orphan wire geometry receives an identity in their place.
#[test]
fn named_singletons_and_power_terminals_are_registered_without_promoting_orphan_wires() {
    let (mut session, _) = circuit(0, false);
    apply(&mut session, crate::net_labels::ADD, json!({"name":"SIGNAL","style":"local","x_nm":0,"y_nm":0}));
    let label = session.project().unwrap().net_labels()[0].id;
    let target = json!({"net_label":format!("net-label-{label}")});
    let id = id_for(&session, &target);
    assert_eq!(session.project().unwrap().net_registry.record(id).unwrap().primary.kind, AnchorKind::Label);
    apply(&mut session, crate::net_labels::SET, json!({"id":format!("net-label-{label}"),"name":"RENAMED","style":"local","x_nm":0,"y_nm":0}));
    assert_eq!(id_for(&session, &target), id);
    round_trip(&session);
    apply(&mut session, crate::net_labels::DELETE, json!({"id":format!("net-label-{label}")}));
    assert!(matches!(session.project().unwrap().net_registry.record(id).unwrap().state, State::Retired { reason: Retirement::Removed, .. }));
    apply(&mut session, crate::DRAW_WIRE, json!({"from":{"x_nm":0,"y_nm":0},"to":{"x_nm":0,"y_nm":10_160_000},"bends":[]}));
    assert_eq!(session.project().unwrap().net_registry.records.len(), 1);
    round_trip(&session);
}

/// A native power symbol names its singleton using its logical component terminal, with no
/// synthetic label or occurrence-qualified unit identity.
#[test]
fn power_singleton_uses_a_component_terminal_anchor() {
    let (mut session, _) = circuit(0, false);
    let definition = session.libraries.symbol("power", "gnd", 1).unwrap();
    let digest = crate::library::sources::symbol_digest(&definition).unwrap();
    apply(
        &mut session,
        crate::PLACE_SYMBOL,
        json!({"library":"power","definition":"gnd","definition_revision":1,"definition_digest":digest,"value":"RETURN","x_nm":0,"y_nm":0}),
    );
    let project = session.project().unwrap();
    let record = &project.net_registry.records[0];
    assert_eq!(record.primary.kind, AnchorKind::Terminal);
    assert!(record.primary.occurrence.is_nil());
    assert!(record.fallbacks.is_empty());
    assert_eq!(record.primary.object, project.symbols()[0].component);
    round_trip(&session);
}

/// First-board creation changes only current owner qualification and storage home. Historical
/// source anchors retain their captured schematic scope, and Undo restores exact schematic bytes.
#[test]
fn first_board_creation_moves_registry_and_preserves_historical_scope() {
    let (mut session, pins) = circuit(4, false);
    connect(&mut session, &pins[0], &pins[1]);
    let middle = connect(&mut session, &pins[1], &pins[2]);
    connect(&mut session, &pins[2], &pins[3]);
    let id = promote(&mut session, &pins[0]);
    apply(&mut session, crate::DELETE_WIRE, json!({"wire":middle}));
    connect(&mut session, &pins[1], &pins[2]);
    let merged = id_for(&session, &pins[0]);
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    let old = session.project().unwrap().net_registry.clone();
    apply(&mut session, crate::board::identity::CREATE_BOARD, json!({}));
    let project = session.project().unwrap();
    let scope = project.net_scope();
    assert_eq!(scope.kind, ScopeKind::Board);
    assert_eq!(id_for(&session, &pins[0]), merged);
    for record in &project.net_registry.records {
        if matches!(record.state, State::Active {}) {
            assert!(record.anchors().all(|a| a.scope == scope));
        } else {
            assert!(record.anchors().all(|a| a.scope.kind == ScopeKind::Schematic));
        }
        assert!(record.lineage.iter().all(|source| source.anchor.scope.kind == ScopeKind::Schematic));
    }
    assert!(matches!(project.net_registry.record(id).unwrap().state, State::Retired { .. }));
    let after = encode_documents(session.snapshot().unwrap()).unwrap();
    let entry: Value = serde_json::from_slice(&after[ENTRY_POINT]).unwrap();
    assert!(entry.get("schematic_instances").is_none());
    let path = project.instances_document_path().unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&after[&path]).unwrap()["format"], "pcbcraft-instances/2");
    round_trip(&session);
    apply(&mut session, "edit.undo", json!({}));
    assert_eq!(session.project().unwrap().net_registry, old);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
    apply(&mut session, "edit.redo", json!({}));
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), after);
}

/// Joining a meaningful derived network is a real merge with source-qualified derived lineage;
/// merely adding a previously bare pin to the same net keeps the resulting UUID.
#[test]
fn registered_and_meaningful_derived_sources_merge_but_bare_pins_continue() {
    let (mut session, pins) = circuit(5, false);
    connect(&mut session, &pins[0], &pins[1]);
    let old = promote(&mut session, &pins[0]);
    connect(&mut session, &pins[2], &pins[3]);
    assert_eq!(session.project().unwrap().net_registry.records.len(), 1);
    connect(&mut session, &pins[1], &pins[2]);
    let merged = id_for(&session, &pins[0]);
    assert_ne!(old, merged);
    let lineage = &session.project().unwrap().net_registry.record(merged).unwrap().lineage;
    assert_eq!(lineage.len(), 2);
    assert!(lineage.iter().any(|source| source.net == Some(old)));
    assert!(lineage.iter().any(|source| source.net.is_none()));
    connect(&mut session, &pins[3], &pins[4]);
    assert_eq!(id_for(&session, &pins[4]), merged);
    round_trip(&session);
}

/// Anchor priority and explicit selection operate on canonical typed keys and enforce the
/// four-anchor cap independently from the number of terminals in a transient graph.
#[test]
fn canonical_creation_priority_explicit_selection_and_four_anchor_cap() {
    let project = Uuid::from_u128(1);
    let scope = Scope { kind: ScopeKind::Schematic, id: project };
    let anchors: std::collections::BTreeSet<_> = (1..=8)
        .map(|object| Anchor {
            kind: match object {
                1 => AnchorKind::Port,
                2 => AnchorKind::Label,
                _ => AnchorKind::Terminal,
            },
            project,
            scope,
            occurrence: if object <= 2 { Uuid::from_u128(2) } else { Uuid::nil() },
            object,
            terminal: if object > 2 { "1".into() } else { String::new() },
        })
        .collect();
    let network = graph::Network { anchors: anchors.clone(), names: Default::default() };
    let record = network.record(Uuid::from_u128(3), None).unwrap();
    assert_eq!(record.primary.kind, AnchorKind::Label);
    assert_eq!(record.fallbacks.len(), 3);
    let selected = anchors.iter().find(|anchor| anchor.object == 8).unwrap();
    let explicit = network.record(Uuid::from_u128(4), Some(selected)).unwrap();
    assert_eq!(&explicit.primary, selected);
    assert_eq!(explicit.fallbacks.first().unwrap().kind, AnchorKind::Label);
    let mut reserved = (1..=MAX_RECORDS as u128).map(Uuid::from_u128).collect();
    assert!(reconcile::allocate(&mut reserved).is_err());
    assert_eq!(reserved.len(), MAX_RECORDS);
}

/// Non-anchor geometry edits change only their authored sheet; accepted baseline membership
/// never causes sparse identity promotion, including on a physically assigned board.
#[test]
fn non_anchor_wire_edits_leave_registry_bytes_and_baseline_nets_derived() {
    let (mut session, pins) = circuit(2, true);
    connect(&mut session, &pins[0], &pins[1]);
    let components: Vec<_> = session.project().unwrap().components().map(|component| component.id()).collect();
    for component in components {
        apply(
            &mut session,
            crate::ASSIGN_FOOTPRINT,
            json!({"component":component,"library":"basic-components","definition":"resistor-0402","definition_revision":4,"mapping":{"1":"1","2":"2"}}),
        );
    }
    apply(&mut session, crate::board::update::INITIALIZE_BASELINE, json!({}));
    assert!(session.project().unwrap().net_registry.records.is_empty());
    let baseline = session.project().unwrap().board.baseline.clone().unwrap();
    promote(&mut session, &pins[0]);
    let project = session.project().unwrap();
    let proposed = crate::board::update::Baseline::proposed(
        project,
        &crate::board::update::Baseline::current_choices(project),
        crate::board::update::capture_source(project).unwrap(),
    )
    .unwrap();
    assert!(crate::board::update::preview(&baseline, &proposed).unwrap().is_noop());
    assert_eq!(project.board.baseline.as_ref(), Some(&baseline));
    apply(
        &mut session,
        crate::DRAW_WIRE,
        json!({"from":{"x_nm":0,"y_nm":25_400_000},"to":{"x_nm":25_400_000,"y_nm":50_800_000},"bends":[{"x_nm":25_400_000,"y_nm":25_400_000}]}),
    );
    let wire = session.project().unwrap().wires().last().unwrap().id();
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    let registry = session.project().unwrap().net_registry.clone();
    apply(
        &mut session,
        crate::MOVE_WIRE_SEGMENT,
        json!({"wire":wire,"at":{"x_nm":25_400_000,"y_nm":38_100_000},"to":{"x_nm":38_100_000,"y_nm":38_100_000}}),
    );
    let after = encode_documents(session.snapshot().unwrap()).unwrap();
    let changed: Vec<_> = before.iter().filter(|(path, bytes)| after.get(*path) != Some(*bytes)).map(|(path, _)| path.as_str()).collect();
    assert_eq!(changed, vec![format!("sheets/{}.json", session.project().unwrap().sheet_target().id())]);
    assert_eq!(session.project().unwrap().net_registry, registry);
    round_trip(&session);
}

/// Recreating removed connectivity cannot reactivate a retired identity, even with identical anchors.
#[test]
fn retired_identity_is_not_reactivated_by_recreating_identical_connectivity() {
    let (mut session, pins) = circuit(2, false);
    let wire = connect(&mut session, &pins[0], &pins[1]);
    let old = promote(&mut session, &pins[0]);
    apply(&mut session, crate::DELETE_WIRE, json!({"wire":wire}));
    assert!(matches!(session.project().unwrap().net_registry.record(old).unwrap().state, State::Retired { reason: Retirement::Removed, .. }));
    connect(&mut session, &pins[0], &pins[1]);
    assert_eq!(session.project().unwrap().net_registry.records.len(), 1);
    assert_ne!(promote(&mut session, &pins[0]), old);
    round_trip(&session);
}

/// Repeated appearances of the same component terminal have one zero-occurrence key and one
/// persistent identity, even when their symbol units are placed on different flat sheets.
#[test]
fn multiunit_terminal_keys_span_sheets_and_promote_once() {
    let (mut session, pins) = circuit(1, false);
    let root = session.project().unwrap().sheet_target();
    session
        .execute(
            "library.load_source",
            &json!({"source":"units","documents":{
                "library.json":include_str!("../../../../examples/native-parts/library.json"),
                "symbols/dual-amplifier.json":include_str!("../../../../examples/native-parts/symbols/dual-amplifier.json")
            }}),
        )
        .unwrap();
    let definition = session.libraries.symbol("parts-demo", "dual-amplifier", 1).unwrap();
    let digest = crate::library::sources::symbol_digest(&definition).unwrap();
    apply(
        &mut session,
        crate::PLACE_SYMBOL,
        json!({"library":"parts-demo","definition":"dual-amplifier","definition_revision":1,"definition_digest":digest,"x_nm":0,"y_nm":25_400_000}),
    );
    let symbol = session.project().unwrap().symbols().last().unwrap().clone();
    let part = symbol.component_id();
    let a = json!({"symbol":symbol.id(),"terminal":"3"});
    apply(&mut session, crate::sheets::CREATE_SHEET, json!({"name":"Other unit"}));
    let other = session.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut session, crate::PLACE_UNIT, json!({"sheet":other.id(),"component":part,"unit":"b","x_nm":0,"y_nm":0}));
    let b = json!({"symbol":session.project().unwrap().for_sheet(other).unwrap().symbols()[0].id(),"terminal":"3"});
    apply(&mut session, crate::CONNECT_PINS, json!({"sheet":root.id(),"from":a,"to":pins[0]}));
    apply(&mut session, PROMOTE, json!({"sheet":root.id(),"anchor":a}));
    let id = id_for(&session, &a);
    let revision = session.revision();
    apply(&mut session, PROMOTE, json!({"sheet":other.id(),"anchor":b}));
    assert_eq!(session.revision(), revision);
    let project = session.project().unwrap();
    let graph = graph::Graph::build(project).unwrap();
    let a = graph.target(root, &a).unwrap();
    let b = graph.target(other, &b).unwrap();
    assert_eq!(a.canonical_key().unwrap(), b.canonical_key().unwrap());
    assert!(a.occurrence.is_nil());
    assert_eq!(project.net_registry.record(id).unwrap().primary, a);
    assert_eq!(project.net_registry.records.len(), 1);
    round_trip(&session);
}

/// Legacy intentionally named nets remain unpromoted through Open, inspection and non-electrical
/// edits. Their first authored graph change creates records without changing existing objects.
#[test]
fn opening_a_named_project_sixteen_does_not_promote_or_change_its_bytes() {
    let (mut session, _) = circuit(0, false);
    apply(&mut session, crate::net_labels::ADD, json!({"name":"OLD","style":"local","x_nm":0,"y_nm":0}));
    let mut historical = session.project().unwrap().clone();
    historical.net_registry = Arc::default();
    let documents = encode_documents(Arc::new(historical)).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&documents[ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/16");
    let a = decode_documents(&documents).unwrap();
    let b = decode_documents(&documents).unwrap();
    assert!(a.net_registry.records.is_empty());
    assert_eq!(encode_documents(b).unwrap(), documents);
    session.open_snapshot(a);
    let before = session.snapshot().unwrap();
    session.inspect();
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    apply(&mut session, "project.rename", json!({"name":"Still compatible"}));
    assert!(session.project().unwrap().net_registry.records.is_empty());
    apply(&mut session, crate::net_labels::ADD, json!({"name":"NEW","style":"local","x_nm":25_400_000,"y_nm":0}));
    assert_eq!(session.project().unwrap().net_registry.records.len(), 2);
    round_trip(&session);
}

mod classes;
