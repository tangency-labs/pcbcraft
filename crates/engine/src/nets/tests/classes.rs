//! Classification acceptance exercises the existing net/command fixtures and native codecs.
use super::*;
use crate::{
    net_classes,
    nets::{ClassBinding, ClassTarget, INSPECT_CLASSES, NetAddress, SET_CLASSES},
};
use std::collections::BTreeSet;

/// Author a definition without assigning any current net.
fn class(session: &mut Session, name: &str, rules: Value) -> Uuid {
    apply(session, net_classes::CREATE, json!({"name":name,"rules":rules}));
    session.project().unwrap().net_classes().definitions.iter().find(|class| class.name == name).unwrap().id
}
/// Use the current typed graph key for derived promotion, never a display name.
fn target(session: &Session, pin: &Value) -> ClassTarget {
    let p = session.project().unwrap();
    let graph = graph::Graph::build(p).unwrap();
    ClassTarget::Derived { anchor: graph.target(p.sheet_target(), pin).unwrap() }
}
/// Persistent address inspection/edits are qualified by the current project and owner.
fn address(session: &Session, id: Uuid) -> ClassTarget {
    let p = session.project().unwrap();
    ClassTarget::Persistent { project: p.id, scope: p.net_scope(), net: id }
}
/// Set the entire class set using the same atomic shared command exposed to MCP.
fn assign(session: &mut Session, pin: &Value, classes: &[Uuid]) -> Uuid {
    apply(session, SET_CLASSES, json!({"target":target(session,pin),"classes":classes}));
    id_for(session, pin)
}
/// Read original assignment sources without projecting a second persisted membership graph.
fn bindings(session: &Session, id: Uuid) -> Vec<ClassBinding> {
    session.project().unwrap().net_registry.record(id).unwrap().class_bindings.clone()
}
/// Query through execute, including the payload returned when full inspection is suppressed.
fn inspect(session: &mut Session, id: Uuid) -> Value {
    session.execute(INSPECT_CLASSES, &json!({"target":address(session,id)})).unwrap()["net_assignments"].clone()
}

/// Empty/no-op/invalid requests preserve bytes and history; assignment promotes once and records
/// its source in the same transaction. Removing a set retains the explicitly promoted identity.
#[test]
fn promotion_assignment_inspection_and_removal_are_atomic_and_sparse() {
    let (mut s, pins) = circuit(3, false);
    connect(&mut s, &pins[0], &pins[1]);
    let power = class(&mut s, "Power", json!({"clearance_nm":500000}));
    let snapshot = s.snapshot().unwrap();
    let t = target(&s, &pins[0]);
    let revision = s.revision();
    let report = s.execute(INSPECT_CLASSES, &json!({"target":t})).unwrap();
    assert_eq!(report["net_assignments"]["status"], "derived");
    apply(&mut s, SET_CLASSES, json!({"target":t,"classes":[]}));
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    assert_eq!(s.revision(), revision);
    let id = assign(&mut s, &pins[0], &[power]);
    assert_eq!(s.revision(), revision + 1);
    assert_eq!(s.project().unwrap().net_registry.records.len(), 1);
    assert_eq!(bindings(&s, id), vec![ClassBinding { class: power, origin: id }]);
    assert_eq!(inspect(&mut s, id)["status"], "ready");
    let assigned = s.snapshot().unwrap();
    let saved = encode_documents(assigned.clone()).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&saved[ENTRY_POINT]).unwrap()["format"], "pcbcraft-project/21");
    round_trip(&s);
    let addressed = address(&s, id);
    apply(&mut s, SET_CLASSES, json!({"target":addressed,"classes":[power]}));
    assert!(Arc::ptr_eq(&assigned, &s.snapshot().unwrap()));
    for params in [
        json!({"target":address(&s,id),"classes":[power],"expected_revision":s.revision()-1}),
        json!({"target":address(&s,id),"classes":[Uuid::from_u128(1)],"expected_revision":s.revision()}),
        json!({"target":{"kind":"derived","anchor":{"kind":"terminal","project":s.project().unwrap().id,"scope":s.project().unwrap().net_scope(),"occurrence":Uuid::nil(),"object":s.project().unwrap().symbols().iter().find(|symbol|symbol.id()==pins[2]["symbol"].as_str().unwrap()).unwrap().component,"terminal":"1"}},"classes":[power],"expected_revision":s.revision()}),
        json!({"target":{"kind":"persistent","project":Uuid::from_u128(2),"scope":s.project().unwrap().net_scope(),"net":id},"classes":[power],"expected_revision":s.revision()}),
    ] {
        assert!(s.apply(SET_CLASSES, &params).is_err());
        assert!(Arc::ptr_eq(&assigned, &s.snapshot().unwrap()));
    }
    let addressed = address(&s, id);
    apply(&mut s, SET_CLASSES, json!({"target":addressed,"classes":[]}));
    assert!(bindings(&s, id).is_empty());
    assert_eq!(id_for(&s, &pins[0]), id);
    assert_eq!(
        serde_json::from_slice::<Value>(&encode_documents(s.snapshot().unwrap()).unwrap()[ENTRY_POINT]).unwrap()["format"],
        "pcbcraft-project/20"
    );
    apply(&mut s, "edit.undo", json!({}));
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
    apply(&mut s, "edit.redo", json!({}));
    assert!(bindings(&s, id).is_empty());
    round_trip(&s);
}

/// Bare/stub disconnects do not inherit intent or compete with a continuing classified net.
/// Lost primary/fallback anchors are replaced without losing the binding or allocating a new ID.
#[test]
fn ordinary_disconnects_and_lost_anchors_keep_only_the_meaningful_assignment() {
    for stub in [false, true] {
        let (mut s, pins) = circuit(4, false);
        connect(&mut s, &pins[0], &pins[1]);
        let power = class(&mut s, "Power", json!({}));
        let id = assign(&mut s, &pins[0], &[power]);
        let branch = connect(&mut s, &pins[0], &pins[2]);
        connect(&mut s, &pins[2], &pins[3]);
        if stub {
            let at = s.project().unwrap().terminal_position(crate::Terminal::parse(&pins[0]).unwrap()).unwrap();
            apply(&mut s, crate::DRAW_WIRE, json!({"from":pins[0],"to":{"x_nm":at.x_nm,"y_nm":at.y_nm+12700000},"bends":[]}));
        }
        apply(&mut s, crate::DELETE_SYMBOL, json!({"symbol":pins[1]["symbol"]}));
        apply(&mut s, crate::DELETE_WIRE, json!({"wire":branch}));
        assert_eq!(id_for(&s, &pins[2]), id);
        assert_eq!(s.project().unwrap().net_registry.records.len(), 1);
        assert_eq!(bindings(&s, id), vec![ClassBinding { class: power, origin: id }]);
        round_trip(&s);
        connect(&mut s, &pins[0], &pins[2]);
        assert_eq!(id_for(&s, &pins[0]), id);
        apply(&mut s, crate::DELETE_SYMBOL, json!({"symbol":pins[0]["symbol"]}));
        assert_eq!(id_for(&s, &pins[2]), id);
        assert_eq!(bindings(&s, id), vec![ClassBinding { class: power, origin: id }]);
        round_trip(&s);
    }
}

/// Meaningful branches inherit declared dimensional intent and origin. Compatible merge unions
/// classifications without duplicating shared sources or manufacturing a reconciliation task.
#[test]
fn split_and_compatible_merge_retain_origins_and_class_sets() {
    let (mut s, pins) = circuit(4, false);
    connect(&mut s, &pins[0], &pins[1]);
    let middle = connect(&mut s, &pins[1], &pins[2]);
    connect(&mut s, &pins[2], &pins[3]);
    let defaults = crate::board::rules::Values::default();
    let mut rules = serde_json::Map::new();
    for property in net_classes::Property::ALL {
        assert!(property.shareable_on_split());
        rules.insert(property.as_str().into(), json!(property.value(&defaults)));
    }
    rules.insert("min_track_width_nm".into(), json!(400000));
    rules.insert("preferred_track_width_nm".into(), json!(500000));
    let power = class(&mut s, "Power", Value::Object(rules));
    let high = class(&mut s, "High voltage", json!({"clearance_nm":500000}));
    let original = assign(&mut s, &pins[0], &[power]);
    apply(&mut s, crate::DELETE_WIRE, json!({"wire":middle}));
    let child = id_for(&s, &pins[2]);
    assert_ne!(child, original);
    assert_eq!(bindings(&s, child), bindings(&s, original));
    assert_eq!(inspect(&mut s, child)["status"], "ready");
    let result = inspect(&mut s, child);
    for property in net_classes::Property::ALL {
        assert!(!result["resolution"]["values"][property.as_str()].is_null());
    }
    round_trip(&s);
    assign(&mut s, &pins[2], &[power, high]);
    connect(&mut s, &pins[1], &pins[2]);
    let merged = id_for(&s, &pins[0]);
    assert!(![original, child].contains(&merged));
    let actual: BTreeSet<_> = bindings(&s, merged).into_iter().collect();
    assert_eq!(actual, BTreeSet::from([ClassBinding { class: power, origin: original }, ClassBinding { class: high, origin: child }]));
    let report = inspect(&mut s, merged);
    assert_eq!(report["status"], "ready");
    assert_eq!(report["resolution"]["values"]["clearance_nm"], 500000);
    round_trip(&s);
    let merged_registry = s.project().unwrap().net_registry.clone();
    apply(&mut s, "edit.undo", json!({}));
    apply(&mut s, "edit.redo", json!({}));
    assert_eq!(s.project().unwrap().net_registry, merged_registry);
}

/// Genuine equal-precedence conflicts retain both source assignments until explicit set replacement.
/// Definition removal must refuse dangling current/historical references or explicitly rebind all.
#[test]
fn conflicting_merge_is_inspectable_resolvable_and_class_removal_rebinds_atomically() {
    let (mut s, pins) = circuit(4, false);
    connect(&mut s, &pins[0], &pins[1]);
    connect(&mut s, &pins[2], &pins[3]);
    let a = class(&mut s, "Power", json!({"preferred_track_width_nm":400000}));
    let b = class(&mut s, "Signal", json!({"preferred_track_width_nm":500000}));
    let left = assign(&mut s, &pins[0], &[a]);
    let right = assign(&mut s, &pins[2], &[b]);
    connect(&mut s, &pins[1], &pins[2]);
    let merged = id_for(&s, &pins[0]);
    assert_eq!(inspect(&mut s, merged)["status"], "conflicted");
    assert_eq!(bindings(&s, left), vec![ClassBinding { class: a, origin: left }]);
    assert_eq!(bindings(&s, right), vec![ClassBinding { class: b, origin: right }]);
    round_trip(&s);
    let conflicted = s.snapshot().unwrap();
    assign(&mut s, &pins[0], &[a]);
    assert_eq!(inspect(&mut s, merged)["status"], "ready");
    apply(&mut s, "edit.undo", json!({}));
    assert_eq!(s.project().unwrap().net_registry, conflicted.net_registry);
    round_trip(&s);
    apply(&mut s, "edit.redo", json!({}));
    let before = s.snapshot().unwrap();
    assert!(s.apply(net_classes::REMOVE, &json!({"class":b,"expected_revision":s.revision()})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert!(s.apply(SET_CLASSES, &json!({"target":address(&s,right),"classes":[],"expected_revision":s.revision()})).is_err());
    apply(&mut s, net_classes::REMOVE, json!({"class":b,"rebind":a}));
    assert!(s.project().unwrap().net_classes().get(b).is_none());
    assert!(s.project().unwrap().net_registry.records.iter().all(|r| r.class_bindings.iter().all(|binding| binding.class != b)));
    round_trip(&s);
    apply(&mut s, "edit.undo", json!({}));
    assert_eq!(s.project().unwrap().net_registry, before.net_registry);
    assert!(s.project().unwrap().net_classes().get(b).is_some());
}

/// An unanchored split retains an identity choice while shareable class intent follows both
/// meaningful branches. Later class choices on the selected branch survive identity resolution.
#[test]
fn ambiguous_split_and_explicit_resolution_keep_current_branch_assignments() {
    let (mut s, pins) = circuit(6, false);
    connect(&mut s, &pins[0], &pins[1]);
    let a = class(&mut s, "Power", json!({}));
    let b = class(&mut s, "Signal", json!({}));
    let id = assign(&mut s, &pins[0], &[a]);
    connect(&mut s, &pins[0], &pins[2]);
    connect(&mut s, &pins[2], &pins[3]);
    connect(&mut s, &pins[0], &pins[4]);
    connect(&mut s, &pins[4], &pins[5]);
    let selected = Selection {
        symbols: [pins[0]["symbol"].as_str().unwrap().into(), pins[1]["symbol"].as_str().unwrap().into()].into(),
        ..Selection::default()
    };
    apply(&mut s, crate::DELETE_ITEMS, json!({"items":selected}));
    assert_eq!(inspect(&mut s, id)["status"], "identity_ambiguous");
    for pin in [&pins[2], &pins[4]] {
        assert_eq!(bindings(&s, id_for(&s, pin)), vec![ClassBinding { class: a, origin: id }]);
    }
    round_trip(&s);
    assert!(s.apply(SET_CLASSES, &json!({"target":address(&s,id),"classes":[b],"expected_revision":s.revision()})).is_err());
    let chosen = assign(&mut s, &pins[2], &[b]);
    let other = id_for(&s, &pins[4]);
    let before = s.project().unwrap().net_registry.clone();
    apply(&mut s, RESOLVE_AMBIGUITY, json!({"net":id,"anchor":pins[2]}));
    assert_eq!(id_for(&s, &pins[2]), id);
    assert_eq!(bindings(&s, id), vec![ClassBinding { class: b, origin: chosen }]);
    assert_eq!(bindings(&s, other), vec![ClassBinding { class: a, origin: id }]);
    round_trip(&s);
    apply(&mut s, "edit.undo", json!({}));
    assert_eq!(s.project().unwrap().net_registry, before);
    apply(&mut s, "edit.redo", json!({}));
    round_trip(&s);
}

/// First-board creation transfers the same registry home and IDs; unsupported consumers refuse
/// required assigned dimensions, while direct checker reports cannot claim a default-only pass.
#[test]
fn first_board_and_unsupported_consumers_preserve_identity_and_intent() {
    let (mut s, pins) = circuit(2, false);
    connect(&mut s, &pins[0], &pins[1]);
    let power = class(&mut s, "Power", json!({"clearance_nm":500000}));
    let id = assign(&mut s, &pins[0], &[power]);
    let before = s.snapshot().unwrap();
    let schematic = s.project().unwrap().net_scope();
    round_trip(&s);
    apply(&mut s, crate::board::identity::CREATE_BOARD, json!({}));
    let p = s.project().unwrap();
    assert_ne!(p.net_scope(), schematic);
    assert_eq!(id_for(&s, &pins[0]), id);
    assert_eq!(bindings(&s, id), vec![ClassBinding { class: power, origin: id }]);
    let board = p.board_target().unwrap();
    let scope = p.net_scope();
    let docs = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&docs[&p.instances_document_path().unwrap()]).unwrap()["format"], "pcbcraft-instances/3");
    round_trip(&s);
    assert!(p.resolve_net_assignment(NetAddress { project: p.id, scope: schematic, net: id }).is_err());
    assert!(p.resolve_net_assignment(NetAddress { project: p.id, scope, net: id }).is_ok());
    assert!(s.check_design(&json!({"board":board.board})).unwrap_err().to_string().contains("assigned net-class"));
    assert!(s.design_check_job(&json!({"board":board.board})).is_err());
    assert!(s.clearance_query(board, 0).is_err());
    let report = p.board_view(board).unwrap().check_design(crate::board::drc::Rules::default());
    assert_eq!(report.total, 1);
    assert_eq!(report.violations[0].kind, crate::board::drc::Kind::UnsupportedNetClasses);
    assert!(s.prepare_copper_edit(crate::ADD_TRACKS,&json!({"board":board.board,"expected_revision":s.revision(),"layer":Uuid::from_u128(55),"points":[{"x_nm":0,"y_nm":0},{"x_nm":1000000,"y_nm":0}]})).err().unwrap().to_string().contains("assigned net-class"));
    apply(&mut s, "edit.undo", json!({}));
    assert_eq!(s.project().unwrap().net_registry, before.net_registry);
    apply(&mut s, "edit.redo", json!({}));
    round_trip(&s);
}

/// Display names and geometry do not become assignment authority; classification rename retains
/// IDs while inspection exposes the new definition revision without changing the project.
#[test]
fn named_net_and_class_rename_and_geometry_moves_retain_assignment() {
    let (mut s, _) = circuit(0, false);
    apply(&mut s, crate::net_labels::ADD, json!({"name":"SIGNAL","style":"local","x_nm":0,"y_nm":0}));
    let label = s.project().unwrap().net_labels()[0].id;
    let pin = json!({"net_label":format!("net-label-{label}")});
    let class = class(&mut s, "Power", json!({"clearance_nm":500000}));
    let id = assign(&mut s, &pin, &[class]);
    let original = bindings(&s, id);
    apply(&mut s, crate::net_labels::SET, json!({"id":format!("net-label-{label}"),"name":"RENAMED","style":"local","x_nm":25400000,"y_nm":0}));
    assert_eq!(id_for(&s, &pin), id);
    assert_eq!(bindings(&s, id), original);
    apply(&mut s, net_classes::UPDATE, json!({"class":class,"name":"Renamed class","rules":{"clearance_nm":500000}}));
    assert_eq!(s.project().unwrap().net_classes().get(class).unwrap().revision, 2);
    assert_eq!(bindings(&s, id), original);
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    assert_eq!(inspect(&mut s, id)["status"], "ready");
    assert_eq!(s.revision(), revision);
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    round_trip(&s);
}

/// Assignment changes preserve the accepted physical baseline and invalidate stale pending
/// review authority atomically. Retired targets are historical, while later edits reopen normally.
#[test]
fn pending_board_update_and_baseline_keep_bound_intent_across_reopen() {
    let (mut s, pins) = circuit(3, true);
    connect(&mut s, &pins[0], &pins[1]);
    let components: Vec<_> = s.project().unwrap().components().map(|component| component.id()).collect();
    for component in components {
        apply(
            &mut s,
            crate::ASSIGN_FOOTPRINT,
            json!({"component":component,"library":"basic-components","definition":"resistor-0402","definition_revision":4,"mapping":{"1":"1","2":"2"}}),
        );
    }
    apply(&mut s, crate::INITIALIZE_UPDATE_BASELINE, json!({}));
    let board = s.project().unwrap().board_target().unwrap();
    let before_board = encode_documents(s.snapshot().unwrap()).unwrap()[&s.project().unwrap().board_document_path().unwrap()].clone();
    let power = class(&mut s, "Power", json!({"clearance_nm":500000}));
    apply(&mut s, crate::PREVIEW_UPDATE, json!({"board":board.board}));
    let before = s.snapshot().unwrap();
    let review = s.board_update_report();
    let invalid = Uuid::from_u128(989);
    assert!(s.apply(SET_CLASSES, &json!({"target":target(&s,&pins[0]),"classes":[invalid],"expected_revision":s.revision()})).is_err());
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert_eq!(s.board_update_report(), review);
    let id = assign(&mut s, &pins[0], &[power]);
    let current = s.snapshot().unwrap();
    let docs = encode_documents(current.clone()).unwrap();
    assert_eq!(docs[&current.board_document_path().unwrap()], before_board);
    connect(&mut s, &pins[1], &pins[2]);
    assert_eq!(id_for(&s, &pins[2]), id);
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let mut reopened = Session::default();
    reopened.open_snapshot(decode_documents(&saved).unwrap());
    assert_eq!(bindings(&reopened, id), vec![ClassBinding { class: power, origin: id }]);
    apply(&mut reopened, crate::PREVIEW_UPDATE, json!({"board":board.board}));
    assert!(reopened.board_update_report().is_some());
    round_trip(&reopened);
}

/// The versioned envelope prevents old headers or malformed bindings from silently dropping
/// saved intent. Instance ownership, class identities and origin identities are validated together.
#[test]
fn binding_codec_refuses_old_headers_empty_unknown_and_dangling_assignments() {
    for board in [false, true] {
        let (mut s, pins) = circuit(2, board);
        connect(&mut s, &pins[0], &pins[1]);
        let power = class(&mut s, "Power", json!({}));
        assign(&mut s, &pins[0], &[power]);
        let p = s.snapshot().unwrap();
        let docs = encode_documents(p.clone()).unwrap();
        let path = p.instances_document_path().unwrap_or_else(|| ENTRY_POINT.into());
        let original: Value = serde_json::from_slice(&docs[&path]).unwrap();
        for case in 0..7 {
            let mut changed = docs.clone();
            let mut value = original.clone();
            let record =
                if board { &mut value["net_registry"]["records"][0] } else { &mut value["schematic_instances"]["net_registry"]["records"][0] };
            let bindings = &mut record["class_bindings"];
            match case {
                0 => *bindings = json!([]),
                1 => bindings[0]["class"] = json!(Uuid::from_u128(999)),
                2 => bindings[0]["origin"] = json!(Uuid::from_u128(999)),
                3 => {
                    let duplicate = bindings[0].clone();
                    bindings.as_array_mut().unwrap().push(duplicate);
                }
                4 => bindings[0]["unknown"] = json!(true),
                5 => *bindings = Value::Null,
                _ => {
                    record.as_object_mut().unwrap().remove("class_bindings");
                }
            }
            changed.insert(path.clone(), serde_json::to_vec(&value).unwrap());
            assert!(decode_documents(&changed).is_err(), "accepted {case} board {board}");
        }
        let mut entry: Value = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
        entry["format"] = json!("pcbcraft-project/20");
        let mut changed = docs.clone();
        changed.insert(ENTRY_POINT.into(), serde_json::to_vec(&entry).unwrap());
        assert!(decode_documents(&changed).is_err());
        if board {
            let mut value = original;
            value["format"] = json!("pcbcraft-instances/2");
            let mut changed = docs.clone();
            changed.insert(path, serde_json::to_vec(&value).unwrap());
            assert!(decode_documents(&changed).is_err());
        }
        assert!(crate::persistence::encode(p.clone()).is_err());
        round_trip(&s);
    }
}

/// Single-use hierarchical sheets keep assignments in the existing project/instance home;
/// renaming a sheet does not rewrite component/net identity or invent a board-owned copy.
#[test]
fn single_use_child_sheet_assignment_survives_rename_and_native_reopen() {
    let (mut s, _) = circuit(0, false);
    let root = s.project().unwrap().sheet_target();
    apply(&mut s, crate::hierarchy::CREATE_CHILD, json!({"sheet":root.id(),"name":"Child","x_nm":0,"y_nm":0}));
    let child = s.project().unwrap().sheets().find(|sheet| sheet.name == "Child").unwrap().target;
    for x in [0, 25400000] {
        apply(&mut s, crate::PLACE_RESISTOR, json!({"sheet":child.id(),"x_nm":x,"y_nm":0}));
    }
    let pins: Vec<_> =
        s.project().unwrap().for_sheet(child).unwrap().symbols().iter().map(|symbol| json!({"symbol":symbol.id(),"terminal":"1"})).collect();
    apply(&mut s, crate::CONNECT_PINS, json!({"sheet":child.id(),"from":pins[0],"to":pins[1]}));
    let power = class(&mut s, "Power", json!({"clearance_nm":500000}));
    let graph = graph::Graph::build(s.project().unwrap()).unwrap();
    let target = ClassTarget::Derived { anchor: graph.target(child, &pins[0]).unwrap() };
    apply(&mut s, SET_CLASSES, json!({"target":target,"classes":[power]}));
    let records = s.project().unwrap().net_registry.clone();
    apply(&mut s, crate::sheets::RENAME_SHEET, json!({"sheet":child.id(),"name":"Renamed child"}));
    assert_eq!(s.project().unwrap().net_registry, records);
    assert!(s.project().unwrap().board_target().is_none());
    round_trip(&s);
}
