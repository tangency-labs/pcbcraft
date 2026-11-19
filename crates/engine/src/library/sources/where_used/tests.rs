use super::*;
use crate::{
    APPLY_UPDATE, DELETE_SYMBOL, INITIALIZE_UPDATE_BASELINE, PLACE_ASSIGNED_FOOTPRINT, PLACE_SYMBOL, PREVIEW_UPDATE, SET_SYMBOL_FIELDS,
    authoring::{
        Item, Kind as DraftKind,
        publishing::{new_library, prepare},
    },
    library::{
        parts::{CategoryBinding, PartDefinition},
        sources::{LOAD, REMOVE, WHERE_USED, part_digest, symbol_digest},
    },
    part_instances::PLACE_PART,
    persistence::{decode_documents, encode_documents},
};
use serde_json::json;
use std::sync::Arc;

/// Publish original drawings and native part records, then place only through shared commands.
fn fixture(create_board: bool) -> (Session, Key) {
    let manifest = new_library("where-used-demo", "Where-used fixture", "MIT OR Apache-2.0").unwrap();
    let mut drawing = Item::new(DraftKind::Symbol).unwrap();
    drawing.id = "drawing".into();
    let category = |id: &str| Reference { library: "where-used-demo".into(), id: id.into(), revision: 1 };
    drawing.category = Some(CategoryBinding { definition: category("symbol-default"), primary_value: None });
    let mut package = Item::new(DraftKind::Footprint).unwrap();
    package.id = "package".into();
    let drawing = prepare(Arc::new(drawing), &manifest, 0, None, false).unwrap();
    let first = prepare(Arc::new(package), &manifest, 0, None, false).unwrap();
    let mut next = first.item().as_ref().clone();
    next.name = "New package revision".into();
    let later = prepare(Arc::new(next), &manifest, 1, Some(1), false).unwrap();
    let mut documents = drawing.documents().clone();
    documents.extend(first.documents().clone());
    documents.extend(later.documents().clone());
    documents.insert("library.json".into(), manifest);
    for id in ["symbol-default", "native-category"] {
        documents.insert(
            format!("categories/{id}.json"),
            json!({"format":"pcbcraft-category/1","library":"where-used-demo","id":id,"revision":1,"name":id,
                "license":"MIT OR Apache-2.0","fields":{"demo:label":{"name":"Label","value_type":{"type":"text"}}}})
            .to_string(),
        );
    }
    let key = Key { library: first.item().library.clone(), kind: Kind::Footprint, id: first.item().id.clone(), revision: 1 };
    for id in ["alpha", "beta"] {
        let variants = |revision| {
            json!({"name":format!("Package {revision}"),"footprint":{"library":key.library,"id":key.id,"revision":revision},
            "mapping":{"1":["1"],"2":["2"]}})
        };
        let mut record = json!({"format":"pcbcraft-part/1","library":key.library,"id":id,"revision":1,
            "name":id,"license":"MIT OR Apache-2.0","source":{"kind":"original","reference":"Where-used test","locator":"synthetic"},
            "symbol":{"library":key.library,"id":drawing.item().id,"revision":1},"packages":{"first":variants(1),"later":variants(2)}});
        if id == "alpha" {
            record["category"] = json!({"definition": category("native-category")});
        }
        let part = PartDefinition::read(&record.to_string()).unwrap();
        documents.insert(format!("parts/{id}.json"), part.encode().unwrap());
    }
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":create_board})).unwrap();
    session.execute(LOAD, &json!({"source":"demo","documents":documents})).unwrap();
    for (index, (id, package)) in
        [("alpha", Some("first")), ("beta", Some("first")), ("alpha", Some("first")), ("beta", Some("later")), ("beta", None)].into_iter().enumerate()
    {
        let address = Reference { library: key.library.clone(), id: id.into(), revision: 1 };
        let digest = part_digest(session.libraries().resolve_part(&address).unwrap());
        session
            .apply(
                PLACE_PART,
                &json!({"library":key.library,"definition":id,"definition_revision":1,"definition_digest":digest,
            "package":package,"x_nm":index as i64 * 15_000_000,"y_nm":0,"expected_revision":session.revision()}),
            )
            .unwrap();
        if create_board && index < 2 {
            let component = session.project().unwrap().components().last().unwrap().id();
            session
                .apply(
                    PLACE_ASSIGNED_FOOTPRINT,
                    &json!({"component":component,"x_nm":index as i64 * 15_000_000,"y_nm":0,"expected_revision":session.revision()}),
                )
                .unwrap();
        }
    }
    (session, key)
}

/// Address the item exactly as a client does, including its separate kind and revision.
fn params(key: &Key) -> Value {
    json!({"library":key.library,"kind":key.kind,"definition":key.id,"definition_revision":key.revision})
}

/// Two part definitions select the same footprint three times, including one unplaced
/// assignment. An alternative package and a different pin are not extra appearances.
#[test]
fn where_used_counts_exact_package_instances_across_parts_without_editing() {
    let (mut session, key) = fixture(true);
    session.apply("project.rename", &json!({"name":"Temporary name"})).unwrap();
    session.apply("edit.undo", &json!({})).unwrap();
    let snapshot = session.snapshot().unwrap();
    let bytes = encode_documents(snapshot.clone()).unwrap();
    let before = session.inspect();
    let command = session.execute(WHERE_USED, &params(&key)).unwrap();
    assert_eq!(serde_json::to_value(session.library_where_used(&key).unwrap()).unwrap(), command);
    let ids: Vec<_> = snapshot.components().take(3).map(|component| component.id()).collect();
    let rows = command["instances"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows.iter().map(|row| row["part_instance"].as_str().unwrap()).collect::<Vec<_>>(), ids);
    assert_eq!(rows.iter().map(|row| row["reference"].as_str().unwrap()).collect::<Vec<_>>(), ["U1", "U2", "U3"]);
    assert_eq!(rows.iter().map(|row| row["part"]["id"].as_str().unwrap()).collect::<Vec<_>>(), ["alpha", "beta", "alpha"]);
    assert!(rows.iter().all(|row| row["assigned"] == true && row["board"]["id"] == json!(snapshot.board_target().unwrap().board)));
    assert_eq!(rows[0]["board"]["footprint"], json!(snapshot.boards().next().unwrap().footprint_ids().next().unwrap()));
    assert!(rows[1]["board"]["footprint"].is_string());
    assert!(rows[2]["board"]["footprint"].is_null());
    assert_eq!(command["newest_revision"], false);
    assert_eq!(command["revision_info"]["latest"]["revision"], 2);
    let latest = session.execute(WHERE_USED, &params(&Key { revision: 2, ..key.clone() })).unwrap();
    assert_eq!(latest["newest_revision"], true);
    assert_eq!(latest["instances"].as_array().unwrap().len(), 1);
    for (kind, id, count) in [(Kind::Symbol, "drawing", 5), (Kind::Part, "alpha", 2), (Kind::Symbol, "package", 0)] {
        let result = session.library_where_used(&Key { kind, id: id.into(), ..key.clone() }).unwrap();
        assert_eq!(result.instances.len(), count);
    }
    assert_eq!(session.inspect(), before);
    assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
    assert_eq!(command["catalog_generation"], session.libraries().generation());
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), bytes);
    session.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(session.inspect()["project"]["name"], "Temporary name");
    session.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), bytes);
}

/// Boardless assignments survive source loss, without asserting that their pin is current.
#[test]
fn where_used_keeps_offline_uses_without_manufacturing_a_board() {
    let (mut session, key) = fixture(false);
    session.execute(REMOVE, &json!({"source":"demo"})).unwrap();
    let result = session.execute(WHERE_USED, &params(&key)).unwrap();
    let rows = result["instances"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| row["assigned"] == true && row["board"].is_null()));
    assert!(result["newest_revision"].is_null());
    assert_eq!(result["revision_info"]["status"], "Source unavailable");
}

/// A physical use awaiting removal remains addressable after its schematic owner is gone.
#[test]
fn where_used_reports_the_retained_physical_placement() {
    let (mut session, key) = fixture(true);
    let symbols = session.project().unwrap().symbols();
    let first = symbols[0].id();
    let unassigned = symbols.last().unwrap().id();
    session.apply(DELETE_SYMBOL, &json!({"symbol":unassigned,"expected_revision":session.revision()})).unwrap();
    let before = session.library_where_used(&key).unwrap();
    session.apply(INITIALIZE_UPDATE_BASELINE, &json!({"expected_revision":session.revision()})).unwrap();
    session.apply(DELETE_SYMBOL, &json!({"symbol":first,"expected_revision":session.revision()})).unwrap();
    let now = session.library_where_used(&key).unwrap();
    let retained = &now.instances[0];
    assert_eq!(now.instances.len(), 3);
    assert_eq!(retained.part_instance, before.instances[0].part_instance);
    assert_eq!(retained.reference, before.instances[0].reference);
    assert!(!retained.assigned);
    assert!(retained.part.is_none());
    assert_eq!(retained.board.as_ref().unwrap().footprint, before.instances[0].board.as_ref().unwrap().footprint);
    assert!(retained.board.as_ref().unwrap().pending_removal);
}

/// A standalone symbol keeps its exact category after reopening offline. Native parts
/// use their own category, including no category, rather than inheriting the drawing default.
#[test]
fn where_used_uses_offline_symbol_category_only_without_native_part() {
    let (mut session, key) = fixture(false);
    let definition = &session.project().unwrap().components().next().unwrap().definition;
    let digest = symbol_digest(definition).unwrap();
    session
        .apply(
            PLACE_SYMBOL,
            &json!({"library":key.library,"definition":"drawing","definition_revision":1,
            "definition_digest":digest,"x_nm":80_000_000,"y_nm":0,"expected_revision":session.revision()}),
        )
        .unwrap();
    let id = session.project().unwrap().components().last().unwrap().id();
    let bytes = encode_documents(session.snapshot().unwrap()).unwrap();
    let mut offline = Session::default();
    offline.open_snapshot(decode_documents(&bytes).unwrap());
    let category = |id: &str, revision| Key { kind: Kind::Category, id: id.into(), revision, ..key.clone() };
    let standalone = offline.library_where_used(&category("symbol-default", 1)).unwrap();
    assert_eq!(standalone.instances.len(), 1);
    assert_eq!(standalone.instances[0].part_instance, id);
    assert!(standalone.instances[0].part.is_none());
    assert_eq!(standalone.newest_revision, None);
    assert!(offline.library_where_used(&category("symbol-default", 2)).unwrap().instances.is_empty());
    let native = offline.library_where_used(&category("native-category", 1)).unwrap();
    assert_eq!(native.instances.len(), 2);
    assert!(native.instances.iter().all(|row| row.part.as_ref().is_some_and(|part| part.id == "alpha")));
}

/// Exclusion preserves authored package intent, but only a real retained placement
/// belongs to the board until the reviewed removal is applied.
#[test]
fn where_used_distinguishes_excluded_assignment_from_retained_board_placement() {
    let (mut session, key) = fixture(true);
    let original = session.library_where_used(&key).unwrap();
    let original_board = original.instances[0].board.as_ref().unwrap();
    session.apply(INITIALIZE_UPDATE_BASELINE, &json!({"expected_revision":session.revision()})).unwrap();
    let project = session.project().unwrap();
    let component = project.components().next().unwrap();
    let symbol = project.symbols()[0].id();
    let mut metadata = serde_json::to_value(&component.metadata).unwrap();
    metadata["attributes"]["exclude_from_board"] = json!(true);
    let fields = json!({"symbol":symbol,"reference":component.reference,"value":component.value,
        "metadata":metadata,"expected_revision":session.revision()});
    session.apply(SET_SYMBOL_FIELDS, &fields).unwrap();
    let before = session.library_where_used(&key).unwrap();
    let retained = &before.instances[0];
    assert!(retained.assigned && retained.assignment_excluded);
    let board = retained.board.as_ref().unwrap();
    assert!(board.pending_removal);
    assert_eq!(board.footprint, original_board.footprint);
    assert_eq!(board.id, original_board.id);
    let plan = session.execute(PREVIEW_UPDATE, &json!({"expected_revision":session.revision()})).unwrap();
    let token = &plan["board_update_preview"]["plan_token"];
    session.apply(APPLY_UPDATE, &json!({"plan_token":token,"expected_revision":session.revision()})).unwrap();
    let after = session.library_where_used(&key).unwrap();
    let excluded = &after.instances[0];
    assert_eq!(excluded.part_instance, retained.part_instance);
    assert!(excluded.assigned && excluded.assignment_excluded);
    assert!(excluded.board.is_none());
    assert!(after.instances[1].board.as_ref().unwrap().footprint.is_some());
}

/// An unknown but valid address returns no uses; malformed addresses and no project are errors.
#[test]
fn where_used_requires_an_open_project_and_valid_exact_address() {
    let mut session = Session::default();
    let query = json!({"library":"demo","kind":"symbol","definition":"unplaced","definition_revision":1});
    assert!(!session.available(WHERE_USED));
    assert!(matches!(session.execute(WHERE_USED, &query), Err(CommandError::Unavailable(_))));
    session.apply("project.new", &json!({})).unwrap();
    assert!(session.available(WHERE_USED));
    assert_eq!(session.execute(WHERE_USED, &query).unwrap()["instances"], json!([]));
    let mut invalid = query.clone();
    invalid["definition_revision"] = json!(0);
    assert!(matches!(session.execute(WHERE_USED, &invalid), Err(CommandError::InvalidParams(_))));
    invalid = query;
    invalid["library"] = json!("not/a/library");
    assert!(matches!(session.execute(WHERE_USED, &invalid), Err(CommandError::InvalidParams(_))));
}
