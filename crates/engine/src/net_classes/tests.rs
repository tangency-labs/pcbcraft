use super::*;
use crate::{
    CommandError, Session,
    persistence::{self, ENTRY_POINT, decode_documents, decode_packed, encode_documents, encode_packed},
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Start with no physical board, as class authoring must work during schematic capture.
fn schematic() -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    session
}

/// Submit edits through the same revision-checked catalog used by GUI/MCP.
fn edit(session: &mut Session, command: &str, mut params: Value) -> Result<u64, CommandError> {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params)
}

/// Capture a newly allocated identity by comparing catalogs, without relying on vector order/names.
fn create(session: &mut Session, name: &str, rules: Value) -> Uuid {
    let before: BTreeSet<_> = session.project().unwrap().net_classes().definitions.iter().map(|class| class.id).collect();
    edit(session, CREATE, json!({"name":name,"rules":rules})).unwrap();
    session.project().unwrap().net_classes().definitions.iter().find(|class| !before.contains(&class.id)).unwrap().id
}

/// Both native document representations reopen exactly, with IDs and revisions unchanged.
fn round_trip(session: &Session) {
    let project = session.snapshot().unwrap();
    let documents = encode_documents(project.clone()).unwrap();
    let reopened = decode_documents(&documents).unwrap();
    assert_eq!(project.net_classes(), reopened.net_classes());
    assert_eq!(documents, encode_documents(reopened).unwrap());
    let archive = encode_packed(project).unwrap();
    assert_eq!(archive, encode_packed(decode_packed(&archive).unwrap().project).unwrap());
}

/// Definition edits preserve stable identities and reversible revisions without allocating a board.
#[test]
fn catalog_authoring_rename_remove_and_history_survive_reopen() {
    let mut session = schematic();
    let original = encode_documents(session.snapshot().unwrap()).unwrap();
    let power = create(&mut session, "Power", json!({"min_track_width_nm":400_000,"preferred_track_width_nm":500_000}));
    let high_voltage = create(&mut session, "High voltage", json!({"clearance_nm":1_000_000}));
    assert_ne!(power, high_voltage);
    assert!(session.project().unwrap().board_target().is_none());
    assert!(session.project().unwrap().board_setup().is_none());
    assert!(session.project().unwrap().net_registry.records.is_empty());
    round_trip(&session);
    let before_rename = encode_documents(session.snapshot().unwrap()).unwrap();
    edit(&mut session, UPDATE, json!({"class":power,"name":"Supply","rules":{"min_track_width_nm":400_000,"preferred_track_width_nm":500_000}}))
        .unwrap();
    assert_eq!(session.project().unwrap().net_classes().get(power).unwrap().revision, 2);
    assert_eq!(session.project().unwrap().net_classes().get(power).unwrap().name, "Supply");
    round_trip(&session);
    let renamed = encode_documents(session.snapshot().unwrap()).unwrap();
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(before_rename, encode_documents(session.snapshot().unwrap()).unwrap());
    session.apply("edit.redo", &Value::Null).unwrap();
    assert_eq!(renamed, encode_documents(session.snapshot().unwrap()).unwrap());
    edit(&mut session, REMOVE, json!({"class":power})).unwrap();
    assert!(session.project().unwrap().net_classes().get(power).is_none());
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(renamed, encode_documents(session.snapshot().unwrap()).unwrap());
    edit(&mut session, REMOVE, json!({"class":power})).unwrap();
    edit(&mut session, REMOVE, json!({"class":high_voltage})).unwrap();
    assert_eq!(original, encode_documents(session.snapshot().unwrap()).unwrap());
}

/// Unknown, malformed, contradictory and stale requests cannot partially edit data/history.
#[test]
fn invalid_and_stale_edits_and_no_ops_preserve_the_snapshot() {
    let mut session = schematic();
    let id = create(&mut session, "Power", json!({"clearance_nm":500_000}));
    let original = session.snapshot().unwrap();
    let revision = session.revision();
    let valid = json!({"class":id,"name":"Power","rules":{"clearance_nm":500_000}});
    edit(&mut session, UPDATE, valid.clone()).unwrap();
    assert_eq!(revision, session.revision());
    assert!(Arc::ptr_eq(&original, &session.snapshot().unwrap()));
    for rules in [
        Value::Null,
        json!({"unknown":1}),
        json!({"clearance_nm":"0.2 mm"}),
        json!({"clearance_nm":0.5}),
        json!({"clearance_nm":-1}),
        json!({"min_via_drill_nm":i64::MAX}),
        json!({"min_via_diameter_nm":800_000,"max_via_diameter_nm":500_000}),
        json!({"min_track_width_nm":500_000,"preferred_track_width_nm":300_000}),
        json!({"preferred_via_diameter_nm":300_000,"preferred_via_drill_nm":300_000}),
        json!({"min_annular_ring_nm":10_000_000}),
    ] {
        let mut invalid = valid.clone();
        invalid["rules"] = rules;
        assert!(edit(&mut session, UPDATE, invalid).is_err());
        assert!(Arc::ptr_eq(&original, &session.snapshot().unwrap()));
    }
    for name in ["", " Space", "Line\nBreak"] {
        assert!(edit(&mut session, UPDATE, json!({"class":id,"name":name,"rules":{}})).is_err());
    }
    for command in [UPDATE, REMOVE] {
        let mut bad = valid.clone();
        bad["class"] = json!(Uuid::from_u128(7));
        if command == REMOVE {
            bad.as_object_mut().unwrap().remove("name");
            bad.as_object_mut().unwrap().remove("rules");
        }
        assert!(edit(&mut session, command, bad).is_err());
    }
    let mut stale = valid;
    stale["expected_revision"] = json!(revision - 1);
    assert!(session.apply(UPDATE, &stale).is_err());
    assert_eq!(revision, session.revision());
    assert!(Arc::ptr_eq(&original, &session.snapshot().unwrap()));
}

/// Built-ins and explicit board replacement match #242; class minima/maxima can only tighten.
#[test]
fn defaults_and_multiple_classes_resolve_all_dimensions_with_provenance() {
    use Property::*;
    let mut session = schematic();
    let base = session.project().unwrap().resolve_net_classes(&[], None).unwrap();
    assert!(base.conflicts.is_empty());
    for property in Property::ALL {
        assert_eq!(base.values[&property], Some(property.value(&crate::board::rules::Values::default())));
        assert_eq!(base.contributions[&property][0].source, Source::Builtin);
    }
    let power = create(
        &mut session,
        "Power",
        json!({"min_track_width_nm":400_000,"preferred_track_width_nm":600_000,"min_via_diameter_nm":700_000,"preferred_via_diameter_nm":800_000,"min_via_drill_nm":350_000,"preferred_via_drill_nm":400_000}),
    );
    let isolation = create(
        &mut session,
        "High voltage",
        json!({"clearance_nm":1_000_000,"edge_clearance_nm":900_000,"min_annular_ring_nm":150_000,"max_via_diameter_nm":1_000_000,"max_via_drill_nm":500_000}),
    );
    let resolved = session.project().unwrap().resolve_net_classes(&[isolation, power], None).unwrap();
    assert!(resolved.conflicts.is_empty(), "{:?}", resolved.conflicts);
    for (field, value) in [
        (ClearanceNm, 1_000_000),
        (MinTrackWidthNm, 400_000),
        (PreferredTrackWidthNm, 600_000),
        (EdgeClearanceNm, 900_000),
        (MinAnnularRingNm, 150_000),
        (MinViaDiameterNm, 700_000),
        (MaxViaDiameterNm, 1_000_000),
        (MinViaDrillNm, 350_000),
        (MaxViaDrillNm, 500_000),
        (PreferredViaDiameterNm, 800_000),
        (PreferredViaDrillNm, 400_000),
    ] {
        assert_eq!(resolved.values[&field], Some(value));
        assert!(resolved.contributions[&field].iter().any(|c| matches!(c.source, Source::Class { revision: 1, .. })));
    }
    edit(&mut session, crate::board::identity::CREATE_BOARD, json!({"name":"Board"})).unwrap();
    let board = session.project().unwrap().board_target().unwrap();
    let values = crate::board::rules::Values { clearance_nm: 100_000, edge_clearance_nm: 1_100_000, ..Default::default() };
    edit(&mut session, crate::SET_RULES, json!({"board":board.board,"rules":values})).unwrap();
    let project = session.project().unwrap();
    let board_only = project.resolve_net_classes(&[], Some(board)).unwrap();
    assert_eq!(board_only.values[&ClearanceNm], Some(100_000));
    assert!(matches!(board_only.contributions[&ClearanceNm][0].source,Source::Board { rule } if rule == project.board.rules.as_ref().unwrap().id));
    let combined = project.resolve_net_classes(&[power, isolation], Some(board)).unwrap();
    assert_eq!(combined.values[&ClearanceNm], Some(1_000_000));
    assert_eq!(combined.values[&EdgeClearanceNm], Some(1_100_000));
    assert_eq!(project.resolve_net_classes(&[], None).unwrap(), base);
    round_trip(&session);
}

/// Incompatible hard constraints stay visible; preferences cannot silently loosen them or win ties.
#[test]
fn conflicts_report_sources_and_never_select_a_preference_by_order() {
    use Property::*;
    let mut session = schematic();
    let a = create(&mut session, "A", json!({"min_via_diameter_nm":800_000,"min_track_width_nm":400_000,"preferred_track_width_nm":500_000}));
    let b = create(&mut session, "B", json!({"max_via_diameter_nm":700_000,"preferred_track_width_nm":600_000}));
    let resolution = session.project().unwrap().resolve_net_classes(&[a, b], None).unwrap();
    assert_eq!(resolution.values[&PreferredTrackWidthNm], None);
    assert_eq!(resolution.values[&MinViaDiameterNm], Some(800_000));
    assert_eq!(resolution.values[&MaxViaDiameterNm], Some(700_000));
    let conflict = resolution.conflicts.iter().find(|c| c.properties == [PreferredTrackWidthNm]).unwrap();
    let contributing: BTreeSet<_> = conflict.contributions[&PreferredTrackWidthNm]
        .iter()
        .filter_map(|c| if let Source::Class { rule, .. } = c.source { Some(rule) } else { None })
        .collect();
    assert_eq!(contributing, BTreeSet::from([a, b]));
    assert!(resolution.conflicts.iter().any(|c| c.properties == [MinViaDiameterNm, MaxViaDiameterNm]));
    let min_only = create(&mut session, "Hard minimum", json!({"min_track_width_nm":800_000}));
    let minimum = session.project().unwrap().resolve_net_classes(&[min_only], None).unwrap();
    assert_eq!(minimum.values[&MinTrackWidthNm], Some(800_000));
    assert_eq!(minimum.values[&PreferredTrackWidthNm], None);
    assert!(minimum.conflicts.iter().any(|c| c.properties.contains(&MinTrackWidthNm)));
    let ring = create(&mut session, "Ring", json!({"min_annular_ring_nm":400_000}));
    let small = create(&mut session, "Small", json!({"max_via_diameter_nm":700_000}));
    let impossible = session.project().unwrap().resolve_net_classes(&[ring, small], None).unwrap();
    assert!(impossible.conflicts.iter().any(|c| c.properties == [MinViaDrillNm, MinAnnularRingNm, MaxViaDiameterNm]));
}

/// Pair clearance, provenance and conflicts are symmetric, independent of names and file order.
#[test]
fn pair_resolution_is_symmetric_including_preferences_and_board_overrides() {
    let mut session = schematic();
    let a = create(&mut session, "Power", json!({"clearance_nm":400_000,"preferred_track_width_nm":300_000}));
    let b = create(&mut session, "High voltage", json!({"clearance_nm":900_000,"preferred_track_width_nm":400_000}));
    let mut project = session.project().unwrap().clone();
    let original = project.resolve_class_clearance(&[a], &[b], None).unwrap();
    assert_eq!(original.clearance_nm, 900_000);
    assert!(original.conflicts.is_empty());
    assert!(original.objects.iter().any(|o| o.values[&Property::PreferredTrackWidthNm] == Some(300_000)));
    assert!(original.objects.iter().any(|o| o.values[&Property::PreferredTrackWidthNm] == Some(400_000)));
    assert_eq!(original, project.resolve_class_clearance(&[b], &[a], None).unwrap());
    assert_eq!(project.resolve_class_clearance(&[b, a], &[a, b], None).unwrap(), project.resolve_class_clearance(&[a, b], &[b, a], None).unwrap());
    let conflicting = project.resolve_class_clearance(&[a, b], &[b], None).unwrap();
    assert!(!conflicting.conflicts.is_empty());
    assert_eq!(conflicting, project.resolve_class_clearance(&[b], &[a, b], None).unwrap());
    let catalog = Arc::make_mut(&mut project.net_classes);
    catalog.definitions.reverse();
    for class in &mut catalog.definitions {
        class.name = "Same name".into();
    }
    assert_eq!(original, project.resolve_class_clearance(&[a], &[b], None).unwrap());
    edit(&mut session, crate::board::identity::CREATE_BOARD, json!({"name":"Board"})).unwrap();
    let board = session.project().unwrap().board_target().unwrap();
    edit(
        &mut session,
        crate::SET_RULES,
        json!({"board":board.board,"rules":crate::board::rules::Values { clearance_nm:1_100_000,..Default::default() }}),
    )
    .unwrap();
    let project = session.project().unwrap();
    let pair = project.resolve_class_clearance(&[a], &[b], Some(board)).unwrap();
    assert_eq!(pair.clearance_nm, 1_100_000);
    assert_eq!(pair, project.resolve_class_clearance(&[b], &[a], Some(board)).unwrap());
}

/// Explicit requests reject unknown owners/identities and inspection does not mutate or promote nets.
#[test]
fn read_only_commands_require_explicit_valid_targets_and_preserve_bytes() {
    let mut session = schematic();
    let id = create(&mut session, "Class", json!({}));
    let original = session.snapshot().unwrap();
    let bytes = encode_documents(original.clone()).unwrap();
    let revision = session.revision();
    for params in [json!({"classes":[id]}), json!({"classes":[],"other_classes":[id]})] {
        assert!(session.execute(RESOLVE, &params).unwrap()["resolution"]["conflicts"].as_array().unwrap().is_empty());
    }
    assert_eq!(session.execute(INSPECT, &json!({})).unwrap()["net_classes"], json!(original.net_classes()));
    for params in [
        json!({"classes":[Uuid::from_u128(8)]}),
        json!({"classes":[id,id]}),
        json!({"classes":[Uuid::nil()]}),
        json!({"classes":[],"board":Uuid::from_u128(9)}),
        json!({"classes":[],"board":null}),
        json!({"classes":[],"other_classes":[Uuid::from_u128(7)]}),
    ] {
        assert!(session.execute(RESOLVE, &params).is_err());
    }
    assert!(original.resolve_net_classes(&[], Some(crate::board::view::BoardTarget { project: original.id(), board: Uuid::from_u128(4) })).is_err());
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&original, &session.snapshot().unwrap()));
    assert_eq!(bytes, encode_documents(session.snapshot().unwrap()).unwrap());
}

/// Format adoption is explicit and minimal; malformed or downgraded headers cannot lose class intent.
#[test]
fn native_format_boundary_rejects_invalid_catalogs_and_preserves_other_documents() {
    let mut session = schematic();
    let original = encode_documents(session.snapshot().unwrap()).unwrap();
    create(&mut session, "Power", json!({"clearance_nm":500_000}));
    let documents = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(documents.len(), original.len());
    for (path, bytes) in &original {
        if path != ENTRY_POINT {
            assert_eq!(bytes, &documents[path]);
        }
    }
    let entry: Value = serde_json::from_slice(&documents[ENTRY_POINT]).unwrap();
    assert_eq!(entry["format"], "pcbcraft-project/20");
    for format in ["pcbcraft-project/16", "pcbcraft-project/17", "pcbcraft-project/19"] {
        let mut value = entry.clone();
        value["format"] = json!(format);
        let mut bad = documents.clone();
        bad.insert(ENTRY_POINT.into(), serde_json::to_vec(&value).unwrap());
        assert!(decode_documents(&bad).is_err());
    }
    let good = entry["net_classes"].clone();
    let mut catalogs = vec![Value::Null, json!({}), json!({"format":"pcbcraft-net-classes/1","definitions":[]})];
    for (path, value) in [("format", json!("pcbcraft-net-classes/2")), ("extra", json!(1))] {
        let mut catalog = good.clone();
        catalog[path] = value;
        catalogs.push(catalog);
    }
    for (path, value) in [
        ("id", json!(Uuid::nil())),
        ("revision", json!(0)),
        ("name", json!("")),
        ("rules", json!({"clearance_nm":-1})),
        ("rules", json!({"surprise":1})),
    ] {
        let mut catalog = good.clone();
        catalog["definitions"][0][path] = value;
        catalogs.push(catalog);
    }
    let mut duplicate = good.clone();
    let class = duplicate["definitions"][0].clone();
    duplicate["definitions"].as_array_mut().unwrap().push(class);
    catalogs.push(duplicate);
    for catalog in catalogs {
        let mut value = entry.clone();
        value["net_classes"] = catalog;
        let mut bad = documents.clone();
        bad.insert(ENTRY_POINT.into(), serde_json::to_vec(&value).unwrap());
        assert!(decode_documents(&bad).is_err());
    }
    let mut missing = entry.clone();
    missing.as_object_mut().unwrap().remove("net_classes");
    let mut bad = documents.clone();
    bad.insert(ENTRY_POINT.into(), serde_json::to_vec(&missing).unwrap());
    assert!(decode_documents(&bad).is_err());
    let raw = String::from_utf8(documents[ENTRY_POINT].clone()).unwrap();
    let repeated = raw.replace("\"clearance_nm\":500000", "\"clearance_nm\":500000,\"clearance_nm\":100000");
    assert_ne!(raw, repeated);
    bad.insert(ENTRY_POINT.into(), repeated.into_bytes());
    assert!(decode_documents(&bad).is_err());
    assert!(persistence::encode(session.snapshot().unwrap()).is_err());
    round_trip(&session);
}

/// Class metadata coexists with the separately versioned registry and no-connect vocabularies.
#[test]
fn classes_round_trip_with_board_rules_net_registry_and_no_connects() {
    let mut session = crate::board::track::tests::routed_fixture();
    let board = session.project().unwrap().board_target().unwrap();
    edit(&mut session, crate::SET_RULES, json!({"board":board.board,"rules":crate::board::rules::Values::default()})).unwrap();
    edit(&mut session, crate::nets::PROMOTE, json!({"anchor":{"symbol":"symbol-2","terminal":"1"}})).unwrap();
    create(&mut session, "Power", json!({"clearance_nm":500_000}));
    round_trip(&session);
    edit(&mut session, crate::no_connect::SET, json!({"target":{"part":"component-2","terminal":"2"},"marked":true})).unwrap();
    round_trip(&session);
    let documents = encode_documents(session.snapshot().unwrap()).unwrap();
    assert!(String::from_utf8(documents[ENTRY_POINT].clone()).unwrap().contains("pcbcraft-project/20"));
}

/// Geometry vocabulary/ranges stay aligned with board defaults, with no new unit convention.
#[test]
fn property_vocabulary_matches_board_rules() {
    let fields: BTreeSet<_> = Property::ALL.into_iter().map(Property::as_str).collect();
    let board_fields: BTreeSet<_> = crate::board::rules::LIMIT_FIELDS.iter().chain(crate::board::rules::PREFERENCE_FIELDS).copied().collect();
    assert_eq!(fields, board_fields);
    let schema = super::rules_schema();
    let board_schema = crate::board::rules::schema();
    for field in fields {
        assert_eq!(schema["properties"][field], board_schema["oneOf"][1]["properties"][field]);
    }
}

/// The catalog is available on legacy snapshots, but adoption is an authored, undoable change.
#[test]
fn legacy_adoption_and_definition_revision_exhaustion_are_atomic() {
    let mut session = schematic();
    let legacy = persistence::diagnostic_snapshot(session.snapshot().unwrap());
    let original = encode_documents(legacy.clone()).unwrap();
    session.open_snapshot(legacy);
    session.execute(INSPECT, &json!({})).unwrap();
    session.execute(RESOLVE, &json!({"classes":[]})).unwrap();
    assert_eq!(original, encode_documents(session.snapshot().unwrap()).unwrap());
    let id = create(&mut session, "Legacy class", json!({}));
    round_trip(&session);
    session.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(original, encode_documents(session.snapshot().unwrap()).unwrap());
    session.apply("edit.redo", &Value::Null).unwrap();
    let mut exhausted = session.project().unwrap().clone();
    Arc::make_mut(&mut exhausted.net_classes).definitions.iter_mut().find(|c| c.id == id).unwrap().revision = u32::MAX;
    session.open_snapshot(Arc::new(exhausted));
    let snapshot = session.snapshot().unwrap();
    edit(&mut session, UPDATE, json!({"class":id,"name":"Legacy class","rules":{}})).unwrap();
    assert!(edit(&mut session, UPDATE, json!({"class":id,"name":"Renamed","rules":{}})).is_err());
    assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
}

/// The supported limit is exercised through native commands; resolution stays bounded and equal
/// preferences agree. Limit failures and oversized requests preserve all existing definitions.
#[test]
fn full_catalog_resolves_and_rejects_overflow_without_losing_definitions() {
    let mut session = schematic();
    let ids: Vec<_> = (0..MAX_CLASSES)
        .map(|index| create(&mut session, &format!("Class {index}"), json!({"clearance_nm":200_000+index,"preferred_track_width_nm":300_000})))
        .collect();
    let snapshot = session.snapshot().unwrap();
    let resolved = snapshot.resolve_net_classes(&ids, None).unwrap();
    assert!(resolved.conflicts.is_empty());
    assert_eq!(resolved.values[&Property::ClearanceNm], Some(200_000 + MAX_CLASSES as i64 - 1));
    assert_eq!(resolved.values[&Property::PreferredTrackWidthNm], Some(300_000));
    assert_eq!(resolved.contributions[&Property::ClearanceNm].len(), MAX_CLASSES + 1);
    assert!(edit(&mut session, CREATE, json!({"name":"Overflow","rules":{}})).is_err());
    assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
    assert!(snapshot.resolve_net_classes(&vec![ids[0]; MAX_CLASSES + 1], None).is_err());
    assert!(snapshot.resolve_class_clearance(&ids, &vec![ids[0]; MAX_CLASSES + 1], None).is_err());
    round_trip(&session);
}
