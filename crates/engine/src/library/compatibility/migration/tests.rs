use super::*;
use crate::{
    Session,
    library::sources,
    persistence::{decode_documents, decode_packed, encode_documents, encode_packed},
};
use serde_json::json;

/// Build historical addresses through the shared placement/assignment commands, with a canonical
/// peer to exercise deduplication. No external assets or hand-authored project documents are used.
fn fixture(board: bool) -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":board})).unwrap();
    let revision = crate::library::compatibility::starter().symbol("connector").unwrap().revision();
    for (index, library) in ["starter", "basic-components"].into_iter().enumerate() {
        let symbol = session.libraries().symbol(library, "connector", revision).unwrap();
        session.apply(crate::PLACE_SYMBOL, &json!({"library":library,"definition":"connector","definition_revision":revision,"definition_digest":sources::symbol_digest(&symbol).unwrap(),"x_nm":index as i64*10_000_000,"y_nm":0,"expected_revision":session.revision()})).unwrap();
        let component = session.project().unwrap().components().last().unwrap().id();
        let footprint = session
            .libraries()
            .source_items(if library == "starter" { "bundled-starter" } else { "bundled-basic-components" })
            .iter()
            .filter(|item| item.key.id == "header-1x02-254")
            .max_by_key(|item| item.key.revision)
            .unwrap()
            .footprint()
            .unwrap();
        let mapping = session.project().unwrap().default_footprint_mapping(&component, &footprint).unwrap();
        session.apply(crate::ASSIGN_FOOTPRINT, &json!({"component":component,"library":library,"definition":footprint.id,"definition_revision":footprint.revision,"mapping":mapping,"expected_revision":session.revision()})).unwrap();
        if board {
            session
                .apply(
                    crate::PLACE_ASSIGNED_FOOTPRINT,
                    &json!({"component":component,"x_nm":index as i64*10_000_000,"y_nm":0,"expected_revision":session.revision()}),
                )
                .unwrap();
        }
    }
    if board {
        session.apply(crate::board::update::INITIALIZE_BASELINE, &json!({"expected_revision":session.revision()})).unwrap();
    }
    session
}

/// Folder and packed opens migrate once, preserve physical intent/history and share equal targets.
#[test]
fn native_open_upgrades_bundled_addresses_once_without_changing_the_original() {
    for board in [false, true] {
        let session = fixture(board);
        let before = session.snapshot().unwrap();
        let documents = encode_documents(before.clone()).unwrap();
        assert!(documents.keys().any(|path| path.contains("/starter/")));
        let opened = project(decode_documents(&documents).unwrap()).unwrap();
        assert!(opened.components().all(|component| component.definition.library == "basic-components"));
        let components: Vec<_> = opened.components().collect();
        assert!(Arc::ptr_eq(&components[0].definition, &components[1].definition));
        for (old, new) in before.symbols().iter().zip(opened.symbols()) {
            assert_eq!((old.id(), old.position, old.rotation, old.mirrored), (new.id(), new.position, new.rotation, new.mirrored));
            assert_eq!(old.definition.units, new.definition.units);
        }
        assert_eq!(before.board.tracks, opened.board.tracks);
        assert_eq!(before.board.vias, opened.board.vias);
        if let (Some(old), Some(new)) = (&before.board.baseline, &opened.board.baseline) {
            assert_eq!(old.source, new.source);
            assert_eq!(old.nets, new.nets);
            assert!(new.components.values().all(|component| component.library == "basic-components"
                && component.package.as_ref().unwrap().library.as_deref() == Some("basic-components")));
        }
        let upgraded = encode_documents(opened.clone()).unwrap();
        assert!(!upgraded.keys().any(|path| path.contains("/starter/")));
        assert_eq!(encode_documents(decode_documents(&upgraded).unwrap()).unwrap(), upgraded);
        let packed = encode_packed(before.clone()).unwrap();
        assert_eq!(encode_documents(project(decode_packed(&packed).unwrap().project).unwrap()).unwrap(), upgraded);
        assert_eq!(encode_documents(before).unwrap(), documents, "opening never mutates its source snapshot");
        assert!(Arc::ptr_eq(&opened, &project(opened.clone()).unwrap()), "already current opens retain their snapshot");
    }
}

/// Original test-only pinned variants exercise historical customization and target collisions.
fn rename_pin(project: &mut Arc<Project>, library: &str) {
    let original = project.components().find(|component| component.definition.library == library).unwrap().definition.clone();
    let mut value = serde_json::to_value(crate::library::saved_copy(&original)).unwrap();
    value["name"] = json!("Custom saved connector");
    let changed = crate::library::read_saved_copy(value).unwrap();
    let project = Arc::make_mut(project);
    for component in Arc::make_mut(&mut project.components).values_mut().filter(|component| component.definition.library == library) {
        component.definition = changed.clone();
    }
    for sheet in Arc::make_mut(&mut project.sheets) {
        for placement in Arc::make_mut(&mut sheet.symbols).iter_mut().filter(|placement| placement.definition.library == library) {
            placement.definition = changed.clone();
        }
    }
}

/// A renamed/customized pin and an occupied different destination both retain their exact address.
#[test]
fn customized_pins_and_conflicting_destinations_are_not_replaced() {
    for library in ["starter", "basic-components"] {
        let mut before = fixture(false).snapshot().unwrap();
        rename_pin(&mut before, library);
        let old = before.components().find(|component| component.definition.library == "starter").unwrap().definition.clone();
        let opened = project(decode_documents(&encode_documents(before).unwrap()).unwrap()).unwrap();
        let retained = opened.components().find(|component| component.definition.library == "starter").unwrap();
        assert_eq!(retained.definition, old);
    }
}

/// Physical customization and an occupied canonical footprint address prevent replacement too.
#[test]
fn custom_footprints_and_conflicting_physical_destinations_keep_their_bytes() {
    for library in ["starter", "basic-components"] {
        let mut before = fixture(false).snapshot().unwrap();
        let choices = Arc::make_mut(&mut Arc::make_mut(&mut before).schematic_packages);
        let definition = choices.definitions.iter_mut().find(|(key, _)| key.0 == library).unwrap().1;
        Arc::make_mut(definition).pads[0].width_nm += 100_000;
        let retained = before.schematic_packages.definitions.iter().find(|(key, _)| key.0 == "starter").unwrap().1.clone();
        let opened = project(decode_documents(&encode_documents(before).unwrap()).unwrap()).unwrap();
        let actual = opened.schematic_packages.definitions.iter().find(|(key, _)| key.0 == "starter").unwrap().1;
        assert_eq!(actual, &retained);
    }
}

/// Accepted orphan packages remain offline-resolvable; unavailable historical symbol bodies are
/// not invented merely to change their old logical metadata address.
#[test]
fn retained_board_packages_migrate_without_rewriting_the_capture_or_missing_symbols() {
    let mut session = fixture(true);
    let symbol = session.project().unwrap().symbols()[0].id();
    session.apply(crate::DELETE_SYMBOL, &json!({"symbol":symbol,"expected_revision":session.revision()})).unwrap();
    let before = session.snapshot().unwrap();
    let capture = before.board.baseline.as_ref().unwrap().source.clone();
    let opened = project(decode_documents(&encode_documents(before).unwrap()).unwrap()).unwrap();
    assert_eq!(opened.board.baseline.as_ref().unwrap().source, capture);
    assert!(opened.board.footprints().iter().all(|placement| placement.library.as_deref() == Some("basic-components")));
    assert!(opened.board.definitions.keys().all(|key| key.0 == "basic-components"));
    opened.board.baseline.as_ref().unwrap().validate_assets(&opened).unwrap();
}

/// Native reusable-part pins normalize their dependency addresses as a single closure; the same
/// legacy source remains reusable after migration instead of reporting a spurious immutable conflict.
#[test]
fn part_dependency_addresses_and_source_resolution_migrate_together() {
    let mut session = fixture(false);
    let legacy = session.project().unwrap().components().next().unwrap().definition.clone();
    let footprint = session.project().unwrap().schematic_packages.definitions.iter().find(|(key, _)| key.0 == "starter").unwrap().1.clone();
    let mut part: serde_json::Value = serde_json::from_str(include_str!("../../../../../../examples/native-parts/parts/resistor.json")).unwrap();
    part["symbol"] = json!({"library":"starter","id":legacy.id,"revision":legacy.revision()});
    part["packages"]["0603"]["footprint"] = json!({"library":"starter","id":footprint.id,"revision":footprint.revision});
    let docs = sources::Documents::from([
        ("library.json".into(), include_str!("../../../../../../examples/native-parts/library.json").into()),
        ("parts/resistor.json".into(), part.to_string()),
        ("categories/resistor.json".into(), include_str!("../../../../../../examples/native-parts/categories/resistor.json").into()),
    ]);
    let source = Arc::new(NativeSource::load(&docs).unwrap());
    assert!(source.problems().is_empty(), "{:?}", source.problems());
    let definition = source.items().iter().find_map(|item| item.part()).unwrap();
    let category = source.items().iter().find_map(|item| item.category()).unwrap();
    let resolved =
        Arc::new(ResolvedPart { definition, symbol: legacy, category: Some(category), footprints: BTreeMap::from([("0603".into(), footprint)]) });
    let mut before = session.snapshot().unwrap();
    Arc::make_mut(&mut Arc::make_mut(&mut before).components).values_mut().next().unwrap().part =
        Some(crate::part_instances::Selection { resolved, package: Some("0603".into()), manufacturer: None, requirements: BTreeMap::new() });
    // A source-only resolution may normalize to a project-occupied address, but the ordinary
    // placement validator must reject that conflict atomically rather than publishing either body.
    let mut conflict = before.clone();
    rename_pin(&mut conflict, "basic-components");
    let conflict = project(decode_documents(&encode_documents(conflict).unwrap()).unwrap()).unwrap();
    let original = encode_documents(conflict.clone()).unwrap();
    let mut rejected = Session::default();
    rejected.open_snapshot(conflict.clone());
    rejected.install_library_source("legacy-parts", source.clone()).unwrap();
    let reference = conflict.components().next().unwrap().part.as_ref().unwrap().resolved.definition.reference();
    let resolved = rejected.libraries().resolve_part(&reference).unwrap();
    assert_eq!(resolved.symbol.library, "basic-components");
    let digest = sources::part_digest(resolved);
    let revision = rejected.revision();
    let result = rejected.apply(crate::part_instances::PLACE_PART,&json!({"library":reference.library,"definition":reference.id,"definition_revision":reference.revision,"definition_digest":digest,"x_nm":20_000_000,"y_nm":0,"expected_revision":revision}));
    assert!(result.unwrap_err().to_string().contains("conflicts with its pinned project copy"));
    assert_eq!(rejected.revision(), revision);
    assert!(Arc::ptr_eq(&conflict, &rejected.snapshot().unwrap()));
    assert_eq!(encode_documents(rejected.snapshot().unwrap()).unwrap(), original);
    assert_eq!(encode_documents(decode_documents(&original).unwrap()).unwrap(), original);
    let opened = project(decode_documents(&encode_documents(before).unwrap()).unwrap()).unwrap();
    let selection = opened.components().next().unwrap().part.as_ref().unwrap();
    assert_eq!(selection.resolved.definition.symbol.library, "basic-components");
    assert!(selection.resolved.definition.packages.values().all(|package| package.footprint.library == "basic-components"));
    session.open_snapshot(opened.clone());
    session.install_library_source("legacy-parts", source).unwrap();
    let reference = selection.resolved.definition.reference();
    let resolved = session.libraries().resolve_part(&reference).unwrap();
    assert_eq!(&resolved.definition, &selection.resolved.definition);
    let digest = sources::part_digest(resolved);
    session.apply(crate::part_instances::PLACE_PART,&json!({"library":reference.library,"definition":reference.id,"definition_revision":reference.revision,"definition_digest":digest,"x_nm":20_000_000,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    assert_eq!(session.project().unwrap().components().count(), 3);
    let before = session.snapshot().unwrap();
    let mut conflicting = crate::library::bundled_documents().find(|(id, _)| *id == "basic-components").unwrap().1;
    let revision = selection.resolved.symbol.revision();
    let path = conflicting
        .iter()
        .find(|(path, text)| {
            path.starts_with("symbols/")
                && serde_json::from_str::<serde_json::Value>(text).is_ok_and(|value| value["id"] == "connector" && value["revision"] == revision)
        })
        .unwrap()
        .0
        .clone();
    let mut value: serde_json::Value = serde_json::from_str(&conflicting[&path]).unwrap();
    value["name"] = json!("Conflicting connector");
    conflicting.insert(path, value.to_string());
    session.install_library_source("conflicting", Arc::new(NativeSource::load(&conflicting).unwrap())).unwrap();
    assert!(session.libraries().resolve_part(&reference).is_err(), "normalization must not bypass target-source quarantine");
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
}

/// A malformed in-memory candidate must fail both encoders instead of silently selecting one body.
#[test]
fn encoder_refuses_different_symbol_bodies_at_one_address() {
    let original = project(fixture(false).snapshot().unwrap()).unwrap();
    let documents = encode_documents(original.clone()).unwrap();
    let mut candidate = original.clone();
    let id = *candidate.components.keys().next_back().unwrap();
    let mut changed = serde_json::to_value(crate::library::saved_copy(&candidate.components.get(&id).unwrap().definition)).unwrap();
    changed["name"] = json!("Different pinned content");
    let changed = crate::library::read_saved_copy(changed).unwrap();
    let project = Arc::make_mut(&mut candidate);
    Arc::make_mut(&mut project.components).get_mut(&id).unwrap().definition = changed.clone();
    for sheet in Arc::make_mut(&mut project.sheets) {
        for placement in Arc::make_mut(&mut sheet.symbols).iter_mut().filter(|placement| placement.component == id) {
            placement.definition = changed.clone();
        }
    }
    for error in [encode_documents(candidate.clone()).unwrap_err(), encode_packed(candidate).unwrap_err()] {
        assert!(error.contains("Conflicting pinned symbol basic-components/connector.r"), "{error}");
    }
    assert_eq!(encode_documents(original).unwrap(), documents);
}
