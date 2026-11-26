use super::*;
use crate::library::parts::{MAX_KEYWORDS, MAX_LINKS, Package, PartLink};

/// Original bundled connector and header geometry supplies an independent native assembly.
fn fixture() -> (Catalog, Record, String) {
    let catalog = Catalog::bundled();
    let symbol = catalog
        .search("", Some(Kind::Symbol))
        .into_iter()
        .find_map(|(_, item)| (item.key.library == "basic-components" && item.key.id == "connector").then(|| item.symbol().unwrap()))
        .unwrap();
    let footprint = catalog
        .search("", Some(Kind::Footprint))
        .into_iter()
        .find_map(|(_, item)| (item.key.library == "basic-components" && item.key.id == "header-1x02-254").then(|| item.footprint().unwrap()))
        .unwrap();
    let mut record = Record::new_part(Reference { library: symbol.library.clone(), id: symbol.id.clone(), revision: symbol.revision() }).unwrap();
    let Record::Part(part) = &mut record else { panic!() };
    part.name = "Original two-pin connector".into();
    part.packages.insert(
        "header".into(),
        Package {
            name: "Header".into(),
            footprint: Reference { library: footprint.library.clone(), id: footprint.id.clone(), revision: footprint.revision },
            mapping: BTreeMap::from([("1".into(), vec!["1".into()]), ("2".into(), vec!["2".into()])]),
            unconnected_pads: Vec::new(),
        },
    );
    let manifest = publishing::new_library("workshop", "Workshop", "MIT OR Apache-2.0").unwrap();
    (catalog, record, manifest)
}
/// Draft edits may be incomplete, but stale/identity changes and invalid publication are atomic.
#[test]
fn incomplete_mapping_stale_identity_and_publication_validation() {
    let (catalog, record, manifest) = fixture();
    let mut session = Session::new(record.clone(), false).unwrap();
    let prepared = session.prepare(&catalog, &manifest, 0, false, false).unwrap();
    assert_eq!(prepared.reference().revision, 1);
    let mut incomplete = record.clone();
    let Record::Part(part) = &mut incomplete else { panic!() };
    part.packages.get_mut("header").unwrap().mapping.remove("2");
    session.replace(1, incomplete.clone()).unwrap();
    assert!(session.prepare(&catalog, &manifest, 0, false, false).unwrap_err().contains("every symbol terminal"));
    let before = session.inspect();
    assert!(session.replace(1, record.clone()).is_err());
    assert_eq!(session.inspect(), before);
    let mut renamed = incomplete;
    let Record::Part(p) = &mut renamed else { panic!() };
    p.id = "another-item".into();
    assert!(session.replace(2, renamed).is_err());
    assert_eq!(session.inspect(), before);
    session.history(2, false).unwrap();
    assert_eq!(session.record(), &record);
    session.history(3, true).unwrap();
    assert!(session.prepare(&catalog, &manifest, 0, false, false).is_err());
}
/// Successful publication binds identity through history while later input remains unpublished.
#[test]
fn confirmation_preserves_newer_edits_and_rebinds_history() {
    let (catalog, record, manifest) = fixture();
    let mut session = Session::new(record, false).unwrap();
    let prepared = session.prepare(&catalog, &manifest, 0, false, false).unwrap();
    let mut changed = session.record().clone();
    let Record::Part(p) = &mut changed else { panic!() };
    p.name = "Changed while writing".into();
    session.replace(1, changed).unwrap();
    session.confirm(&prepared);
    assert!(session.dirty());
    assert_eq!(session.record().reference(), prepared.reference());
    assert!(session.replace(2, prepared.record().clone()).is_err());
    session.history(session.revision(), false).unwrap();
    assert_eq!(session.record(), prepared.record());
    assert!(!session.dirty());
    session.history(session.revision(), true).unwrap();
    assert!(session.dirty());
    let next = session.prepare(&catalog, &manifest, 1, false, false).unwrap();
    assert_eq!(next.reference().revision, 2);
    session.confirm(&next);
    assert!(!session.dirty());
}
/// Older captures require consent; source-preserving derivatives receive a fresh identity.
#[test]
fn older_base_and_save_as_preserve_exact_relationships() {
    let (catalog, record, manifest) = fixture();
    let first = Session::new(record, false).unwrap().prepare(&catalog, &manifest, 0, false, false).unwrap();
    let mut session = Session::new(first.record().clone(), true).unwrap();
    let mut changed = session.record().clone();
    let Record::Part(p) = &mut changed else { panic!() };
    p.name = "Revised connector".into();
    session.replace(1, changed).unwrap();
    assert!(session.prepare(&catalog, &manifest, 2, false, false).is_err());
    let next = session.prepare(&catalog, &manifest, 2, true, false).unwrap();
    assert_eq!(next.reference().revision, 3);
    let copy = session.prepare(&catalog, &manifest, 2, false, true).unwrap();
    assert_ne!(copy.reference().id, first.reference().id);
    assert_eq!(copy.reference().revision, 1);
    let Record::Part(original) = first.record() else { panic!() };
    let Record::Part(copy) = copy.record() else { panic!() };
    assert_eq!(copy.symbol, original.symbol);
    assert_eq!(copy.packages, original.packages);
    assert_eq!(copy.license, original.license);
    assert!(copy.source.reference.contains(&original.id));
}
/// The native publisher output resolves and places through the production command, then reopens offline.
#[test]
fn publish_generic_and_orderable_place_and_reopen_offline() {
    let (mut catalog, record, manifest) = fixture();
    let prepared = Session::new(record, false).unwrap().prepare(&catalog, &manifest, 0, false, false).unwrap();
    let mut documents = prepared.documents().clone();
    documents.insert("library.json".into(), manifest.clone());
    catalog.install("workshop", Arc::new(NativeSource::load(&documents).unwrap())).unwrap();
    let part = prepared.reference();
    let mut mpn = Record::new_orderable(&catalog, part.clone()).unwrap();
    let Record::Orderable(m) = &mut mpn else { panic!() };
    m.manufacturer = "Synthetic test manufacturer".into();
    m.mpn = "ORIGINAL-2PIN".into();
    m.datasheet = Some("https://example.invalid/test.pdf".into());
    let mpn = Session::new(mpn, false).unwrap().prepare(&catalog, &manifest, 0, false, false).unwrap();
    documents.extend(mpn.documents().clone());
    let source = Arc::new(NativeSource::load(&documents).unwrap());
    let mut project = crate::Session::default();
    project.apply("project.new", &json!({})).unwrap();
    project.install_library_source("workshop", source).unwrap();
    let resolved = project.libraries().resolve_part(&part).unwrap();
    let record = project.libraries().manufacturer_part(&mpn.reference()).unwrap();
    project.apply(crate::part_instances::PLACE_PART,&json!({"expected_revision":project.revision(),"library":part.library,"definition":part.id,"definition_revision":part.revision,"definition_digest":crate::library::sources::part_digest(resolved),"package":"header","manufacturer":record.reference(),"manufacturer_digest":crate::library::sources::manufacturer_digest(&record),"x_nm":0,"y_nm":0})).unwrap();
    let snapshot = project.snapshot().unwrap();
    let documents = crate::persistence::encode_documents(snapshot.clone()).unwrap();
    let packed = crate::persistence::encode_packed(snapshot).unwrap();
    let folder = crate::persistence::decode_documents(&documents).unwrap();
    let archive = crate::persistence::decode_packed(&packed).unwrap();
    assert_eq!(crate::persistence::encode_documents(folder).unwrap(), documents);
    assert_eq!(crate::persistence::encode_documents(archive.project).unwrap(), documents);
}
/// An old placed revision retains its exact dependency closure when a new revision is published.
#[test]
fn publishing_historical_part_preserves_the_offline_project_pin() {
    use crate::{
        library::sources::part_digest,
        part_instances::PLACE_PART,
        persistence::{decode_documents, encode_documents},
    };
    let mut documents = Documents::from([
        ("library.json".into(), include_str!("../../../../examples/native-parts/library.json").into()),
        ("parts/resistor.json".into(), include_str!("../../../../examples/native-parts/parts/resistor.json").into()),
        ("categories/resistor.json".into(), include_str!("../../../../examples/native-parts/categories/resistor.json").into()),
        ("symbols/resistor.json".into(), include_str!("../../../../examples/native-parts/symbols/resistor.json").into()),
        ("footprints/resistor-0603.json".into(), include_str!("../../../../examples/native-parts/footprints/resistor-0603.json").into()),
    ]);
    let mut project = crate::Session::default();
    project.apply("project.new", &json!({"create_board":false})).unwrap();
    project.install_library_source("parts-demo", Arc::new(NativeSource::load(&documents).unwrap())).unwrap();
    let old_bytes = documents["parts/resistor.json"].clone();
    let old = PartDefinition::read(&old_bytes).unwrap();
    let reference = old.reference();
    let resolved = project.libraries().resolve_part(&reference).unwrap();
    project.apply(PLACE_PART, &json!({"library":reference.library,"definition":reference.id,"definition_revision":reference.revision,
        "definition_digest":part_digest(resolved),"package":old.packages.keys().next().unwrap(),"expected_revision":project.revision(),"x_nm":0,"y_nm":0})).unwrap();
    let before = encode_documents(project.snapshot().unwrap()).unwrap();

    let mut editor = Session::new(Record::Part(old.clone()), true).unwrap();
    let mut changed = editor.record().clone();
    let Record::Part(part) = &mut changed else { panic!() };
    part.name = "Updated resistor".into();
    editor.replace(editor.revision(), changed).unwrap();
    let prepared = editor.prepare(project.libraries(), &documents["library.json"], reference.revision, false, false).unwrap();
    documents.extend(prepared.documents().clone());
    project.install_library_source("parts-demo", Arc::new(NativeSource::load(&documents).unwrap())).unwrap();
    let new = project.libraries().resolve_part(&prepared.reference()).unwrap().definition.clone();
    assert_eq!(new.format, CURRENT_PART_FORMAT);
    assert_eq!(new.revision, old.revision + 1);
    assert_eq!(new.symbol, old.symbol);
    assert_eq!(new.category, old.category);
    assert_eq!(new.packages, old.packages);
    assert_eq!(documents["parts/resistor.json"], old_bytes);
    assert_eq!(*project.libraries().resolve_part(&reference).unwrap().definition, old);

    let saved = encode_documents(project.snapshot().unwrap()).unwrap();
    assert_eq!(saved, before);
    let reopened = decode_documents(&saved).unwrap();
    let selection = reopened.components().next().unwrap().part.as_ref().unwrap();
    assert_eq!(*selection.resolved.definition, old);
    assert_eq!(encode_documents(reopened).unwrap(), saved);

    editor.confirm(&prepared);
    editor.history(editor.revision(), false).unwrap();
    let Record::Part(undone) = editor.record() else { panic!() };
    assert_eq!(undone.reference(), new.reference());
    assert_eq!(undone.format, CURRENT_PART_FORMAT);
    assert_eq!(undone.name, old.name);
}

/// Publication preparation through MCP has no destination paths or project/history mutation.
#[test]
fn headless_preparation_returns_native_bytes_without_project_changes() {
    let (_, record, manifest) = fixture();
    let mut project = crate::Session::default();
    project.apply("project.new", &json!({})).unwrap();
    let before = project.inspect();
    let prepared =
        project.execute(crate::library::sources::PREPARE_PART_PUBLICATION, &json!({"record":record,"documents":{"library.json":manifest}})).unwrap();
    assert_eq!(prepared["saved"], false);
    assert_eq!(prepared["record"]["format"], CURRENT_PART_FORMAT);
    assert!(prepared["documents"].as_object().unwrap().keys().all(|path| path.starts_with("parts/")));
    assert_eq!(project.inspect(), before);
}

/// Choosing the current address preserves authored maps; changing one package clears only
/// that package's map and keeps stable package/part identities for subsequent explicit editing.
#[test]
fn reference_field_choice_preserves_noop_and_unrelated_package_data() {
    let (catalog, mut record, _manifest) = fixture();
    let Record::Part(part) = &mut record else { panic!() };
    part.packages.insert("other".into(), part.packages["header"].clone());
    let current = part.packages["header"].footprint.clone();
    let field = ReferenceField::Footprint { package: "header".into() };
    assert_eq!(record.select_reference(&field, Some(current), &catalog).unwrap(), record);
    let other = catalog.search("", Some(Kind::Footprint)).into_iter().find(|(_, item)| item.key.id != "header-1x02-254").unwrap().1.key.clone();
    let changed =
        record.select_reference(&field, Some(Reference { library: other.library, id: other.id, revision: other.revision }), &catalog).unwrap();
    let Record::Part(before) = &record else { panic!() };
    let Record::Part(after) = &changed else { panic!() };
    assert_eq!(before.reference(), after.reference());
    assert_eq!(before.packages["other"], after.packages["other"]);
    assert!(after.packages["header"].mapping.is_empty());
    assert!(record.select_reference(&ReferenceField::Symbol, None, &catalog).is_ok());
}

/// Optional drawings resolve independently; native publication and inspection retain exact
/// footprints without manufacturing a symbol or electrical map, while schematic placement rejects.
#[test]
fn footprint_only_publication_resolution_and_schematic_rejection_are_atomic() {
    let (mut catalog, _, manifest) = fixture();
    let empty = Record::new_part(None).unwrap();
    empty.validate(&catalog).unwrap();
    let footprint = Reference { library: "basic-components".into(), id: "header-1x02-254".into(), revision: 3 };
    let field = ReferenceField::NewPackage { package: "physical".into() };
    let physical = empty.select_reference(&field, Some(footprint.clone()), &catalog).unwrap();
    physical.validate(&catalog).unwrap();
    assert!(physical.select_reference(&field, Some(footprint), &catalog).is_err());
    let prepared = Session::new(physical, false).unwrap().prepare(&catalog, &manifest, 0, false, false).unwrap();
    let mut documents = prepared.documents().clone();
    documents.insert("library.json".into(), manifest.clone());
    catalog.install("workshop", Arc::new(NativeSource::load(&documents).unwrap())).unwrap();
    let reference = prepared.reference();
    let resolved = catalog.resolve_part(&reference).unwrap();
    assert!(resolved.symbol.is_none());
    assert_eq!(resolved.footprints.len(), 1);
    assert!(resolved.inspect()["symbol"].is_null());
    let mut session = crate::Session::default();
    session.execute("project.new", &json!({})).unwrap();
    session.install_library_source("workshop", Arc::new(NativeSource::load(&documents).unwrap())).unwrap();
    let before = session.inspect();
    let error = session
        .execute(
            crate::part_instances::PLACE_PART,
            &json!({
                "library":reference.library,"definition":reference.id,"definition_revision":reference.revision,
                "definition_digest":crate::library::sources::part_digest(resolved),"expected_revision":session.revision(),"x_nm":0,"y_nm":0
            }),
        )
        .unwrap_err();
    assert!(error.to_string().contains("no symbol"), "{error}");
    assert_eq!(session.inspect(), before);
    let mut orderable = Record::new_orderable(&catalog, reference.clone()).unwrap();
    let Record::Orderable(record) = &mut orderable else { panic!() };
    record.manufacturer = "Original test maker".into();
    record.mpn = "PHYSICAL-TEST".into();
    orderable.validate(&catalog).unwrap();
}

/// A published historical record keeps its bytes on open. Editing uses the current format,
/// and Undo restores the original symbol and complete terminal map.
#[test]
fn optional_symbol_transition_keeps_published_baseline_and_undo() {
    let (catalog, mut record, _) = fixture();
    let Record::Part(part) = &mut record else { panic!() };
    part.format = crate::library::parts::LEGACY_PART_FORMAT.into();
    let original_bytes = part.encode().unwrap();
    assert_eq!(PartDefinition::read(&original_bytes).unwrap().encode().unwrap(), original_bytes);
    let physical = record.select_reference(&ReferenceField::Symbol, None, &catalog).unwrap();
    let Record::Part(part) = &physical else { panic!() };
    assert_eq!(part.format, CURRENT_PART_FORMAT);
    assert!(part.symbol.is_none());
    assert!(part.packages["header"].mapping.is_empty());
    physical.validate(&catalog).unwrap();
    let mut session = Session::new(record.clone(), true).unwrap();
    assert!(!session.dirty());
    session.replace(1, record.clone()).unwrap();
    assert_eq!(session.revision(), 1);
    session.replace(1, physical.clone()).unwrap();
    session.history(2, false).unwrap();
    assert_eq!(session.record(), &record);
    let logical = physical.select_reference(&ReferenceField::Symbol, record.reference_field(&ReferenceField::Symbol).unwrap(), &catalog).unwrap();
    assert!(logical.validate(&catalog).is_err());
    let mut invalid = part.clone();
    invalid.format = crate::library::parts::LEGACY_PART_FORMAT.into();
    assert!(invalid.encode().is_err());
    let mut value = serde_json::to_value(part).unwrap();
    value["symbol"] = Value::Null;
    assert!(PartDefinition::read(&value.to_string()).is_err());
}

/// Author one bounded set of metadata on the current draft.
fn with_metadata(record: &mut Record, description: &str) {
    let Record::Part(part) = record else { panic!("fixture needs a generic part") };
    part.description = Some(description.into());
    part.keywords = vec!["Precision".into(), "low-noise".into()];
    part.links = vec![PartLink { label: "Datasheet".into(), url: "https://example.invalid/ds.pdf".into() }];
}

/// Publication uses the current envelope with and without descriptive metadata.
#[test]
fn metadata_edits_and_publication_keep_the_current_format() {
    let (mut catalog, record, manifest) = fixture();
    let mut session = Session::new(record, false).unwrap();
    let mut edited = session.record().clone();
    with_metadata(&mut edited, "Precision connector");
    let Record::Part(part) = &edited else { panic!() };
    assert_eq!(part.format, CURRENT_PART_FORMAT);
    session.replace(1, edited).unwrap();
    let Record::Part(part) = session.record() else { panic!() };
    assert_eq!(part.format, CURRENT_PART_FORMAT);
    assert_eq!(part.search_text(), "precision connector precision low-noise");

    let prepared = session.prepare(&catalog, &manifest, 0, false, false).unwrap();
    let Record::Part(published) = prepared.record() else { panic!() };
    assert_eq!(published.format, CURRENT_PART_FORMAT);
    assert_eq!(published.description.as_deref(), Some("Precision connector"));
    let mut documents = prepared.documents().clone();
    let text = documents.iter().find(|(path, _)| path.starts_with("parts/")).unwrap().1;
    assert_eq!(PartDefinition::read(text).unwrap(), *published);
    documents.insert("library.json".into(), manifest.clone());
    catalog.install("workshop", Arc::new(NativeSource::load(&documents).unwrap())).unwrap();
    let resolved = catalog.resolve_part(&prepared.reference()).unwrap();
    assert_eq!(resolved.definition.description.as_deref(), Some("Precision connector"));
    assert_eq!(resolved.definition.links.len(), 1);

    session.confirm(&prepared);
    let mut cleared = session.record().clone();
    let Record::Part(part) = &mut cleared else { panic!() };
    part.description = None;
    part.keywords.clear();
    part.links.clear();
    session.replace(session.revision(), cleared).unwrap();
    let next = session.prepare(&catalog, &manifest, 1, false, false).unwrap();
    let bytes = next.documents().values().next().unwrap();
    let reopened = PartDefinition::read(bytes).unwrap();
    assert_eq!(reopened.format, CURRENT_PART_FORMAT);
    assert_eq!(reopened.revision, 2);
    assert_eq!(reopened.packages, published.packages);
    assert!(reopened.description.is_none() && reopened.keywords.is_empty() && reopened.links.is_empty());
}

/// Bounded metadata is edited atomically with one revision, undo/redo and stale-capture checks.
#[test]
fn metadata_edits_are_bounded_and_join_the_draft_history() {
    let (_, original, _) = fixture();
    let mut session = Session::new(original.clone(), false).unwrap();
    let mut valid = original.clone();
    with_metadata(&mut valid, "Revised connector");
    session.replace(1, valid.clone()).unwrap();
    assert_eq!(session.revision(), 2);
    assert_eq!(session.record(), &valid);
    session.history(2, false).unwrap();
    assert_eq!(session.record(), &original);
    session.history(3, true).unwrap();
    assert_eq!(session.record(), &valid);
    assert!(session.replace(3, original.clone()).is_err());
    assert_eq!(session.record(), &valid);
    assert_eq!(session.revision(), 4);

    type Edit = (fn(&mut PartDefinition), &'static str);
    let edits: [Edit; 3] = [
        (|p: &mut PartDefinition| p.description = Some("x".repeat(513)), "description"),
        (|p: &mut PartDefinition| p.keywords = (0..MAX_KEYWORDS + 1).map(|i| format!("k{i}")).collect(), "keywords"),
        (
            |p: &mut PartDefinition| {
                p.links = (0..MAX_LINKS + 1).map(|i| PartLink { label: format!("L{i}"), url: "https://example.invalid".into() }).collect()
            },
            "links",
        ),
    ];
    for (edit, message) in edits {
        let mut over = valid.clone();
        let Record::Part(part) = &mut over else { panic!() };
        edit(part);
        assert!(session.replace(4, over).unwrap_err().contains(message), "{message}");
        assert_eq!(session.record(), &valid);
        assert_eq!(session.revision(), 4);
    }
}

/// Removing ratings or metadata leaves the native format current and preserves the other content.
#[test]
fn removing_ratings_and_metadata_keeps_the_current_format() {
    let (catalog, record, _) = fixture();
    let mut session = Session::new(record.clone(), false).unwrap();
    let mut both = record.clone();
    with_metadata(&mut both, "Rated connector");
    let Record::Part(part) = &mut both else { panic!() };
    part.set_supply_range("1", "4.5 V", "36 V").unwrap();
    assert_eq!(part.format, CURRENT_PART_FORMAT);
    session.replace(1, both.clone()).unwrap();
    assert_eq!(session.record(), &both);

    let physical = session.record().select_reference(&ReferenceField::Symbol, None, &catalog).unwrap();
    let Record::Part(part) = &physical else { panic!() };
    assert_eq!(part.format, CURRENT_PART_FORMAT);
    assert!(part.terminal_ratings.is_empty());
    assert!(part.symbol.is_none());
    assert_eq!(part.description.as_deref(), Some("Rated connector"));
    assert!(part.packages.values().all(|package| package.mapping.is_empty()));
    let mut clear = both.clone();
    let Record::Part(part) = &mut clear else { panic!() };
    part.description = None;
    part.keywords.clear();
    part.links.clear();
    assert_eq!(part.format, CURRENT_PART_FORMAT);
    part.set_supply_range("1", "", "").unwrap();
    let plain = PartDefinition::read(&part.encode().unwrap()).unwrap();
    assert_eq!(plain.format, CURRENT_PART_FORMAT);
    assert!(plain.terminal_ratings.is_empty());
    assert!(plain.description.is_none() && plain.keywords.is_empty() && plain.links.is_empty());
}
