use super::*;
use crate::{
    authoring::{Item, Kind, Primitive, publishing::new_library},
    board::clipboard::{BoardFragment, BoardSelection},
    clipboard::Fragment,
    library::parts::{LEGACY_PART_FORMAT, PART_FORMAT, Package, PartDefinition, RATED_PART_FORMAT, SupplyRange},
    part_authoring::{Record as Draft, Session as DraftSession},
    schematic::ElectricalType,
};

const LIBRARY: &str = "supply-range-test";
const TERMINAL: &str = "1";
const LICENSE: &str = "MIT OR Apache-2.0";

/// Use ordinary native preparation and loading for synthetic artwork and the package.
fn source(board: bool) -> (Session, sources::Documents, Draft) {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":board})).unwrap();
    let mut documents = sources::Documents::from([("library.json".into(), new_library(LIBRARY, "Synthetic supply ranges", LICENSE).unwrap())]);
    let mut symbol = Item::new(Kind::Symbol).unwrap();
    symbol.id = "regulator".into();
    symbol.name = "Synthetic regulator".into();
    let Some(Primitive::Pin { name, electrical_type, .. }) = symbol.objects.get_mut(&1) else { panic!("fixture needs a pin") };
    *name = Some("VIN".into());
    *electrical_type = ElectricalType::PowerIn;
    let mut footprint = Item::new(Kind::Footprint).unwrap();
    footprint.id = "regulator-package".into();
    for item in [symbol, footprint] {
        let prepared = session.execute(sources::PREPARE_PUBLICATION, &json!({"item":item,"documents":&documents})).unwrap();
        documents.extend(serde_json::from_value::<sources::Documents>(prepared["documents"].clone()).unwrap());
    }
    load(&mut session, &documents);
    let mut draft = Draft::new_part(Reference { library: LIBRARY.into(), id: "regulator".into(), revision: 1 }).unwrap();
    let part = part_mut(&mut draft);
    part.id = "regulator".into();
    part.name = "Synthetic regulator device".into();
    part.packages.insert(
        "test".into(),
        Package {
            name: "Synthetic regulator package".into(),
            footprint: Reference { library: LIBRARY.into(), id: "regulator-package".into(), revision: 1 },
            mapping: BTreeMap::from([("1".into(), vec!["2".into()]), ("2".into(), vec!["1".into()])]),
            unconnected_pads: Vec::new(),
        },
    );
    (session, documents, draft)
}

/// Retrieve the test's reusable generic record, not an orderable override.
fn part_mut(draft: &mut Draft) -> &mut PartDefinition {
    let Draft::Part(part) = draft else { panic!("fixture needs a generic part") };
    part
}

/// Register the exact synthetic library through the shared read command.
fn load(session: &mut Session, documents: &sources::Documents) {
    let list = session.execute(sources::LOAD, &json!({"source":LIBRARY,"documents":documents})).unwrap();
    let loaded = list.as_array().unwrap().iter().find(|source| source["source"] == LIBRARY).unwrap();
    assert_eq!(loaded["problems"], json!([]));
}

/// Prepare without publishing to disk, using the same validator as the GUI publication preview.
fn prepare(session: &mut Session, documents: &sources::Documents, record: &Draft) -> Result<Json, crate::CommandError> {
    session.execute(sources::PREPARE_PART_PUBLICATION, &json!({"record":record,"documents":documents}))
}

/// Register a command-produced record and place it using the resolved digest, on an explicit sheet.
fn place(session: &mut Session, documents: &mut sources::Documents, draft: &Draft, sheet: crate::SheetTarget) -> Json {
    let prepared = prepare(session, documents, draft).unwrap();
    documents.extend(serde_json::from_value::<sources::Documents>(prepared["documents"].clone()).unwrap());
    load(session, documents);
    let id = draft.reference().id;
    let resolved = session.execute(sources::RESOLVE_PART, &json!({"library":LIBRARY,"definition":id,"definition_revision":1})).unwrap();
    let x_nm = session.project().unwrap().components().count() as i64 * 25_400_000;
    apply(
        session,
        PLACE_PART,
        json!({"library":LIBRARY,"definition":id,"definition_revision":1,"definition_digest":resolved["digest"],"sheet":sheet.id(),"x_nm":x_nm,"y_nm":0}),
    );
    resolved
}

/// Query a pinned supply range without access to a source registration.
fn range(project: &crate::Project) -> Option<&SupplyRange> {
    project.components().next().unwrap().part.as_ref().unwrap().supply_range(TERMINAL)
}

/// The exact decimal voltage and missing bounds survive native folders and portable archives.
/// Unrated /1 and /2 bytes remain stable and the same symbol works for different devices.
#[test]
fn supply_ranges_reopen_offline_without_raising_outer_formats() {
    for (min, max, initial, expected) in [
        ("4500 mV", "36 V", PART_FORMAT, RATED_PART_FORMAT),
        ("-0.000000000000000001 V", "", PART_FORMAT, RATED_PART_FORMAT),
        ("", "0 V", PART_FORMAT, RATED_PART_FORMAT),
        ("", "", LEGACY_PART_FORMAT, LEGACY_PART_FORMAT),
        ("", "", PART_FORMAT, PART_FORMAT),
    ] {
        let (mut session, mut documents, mut draft) = source(false);
        let part = part_mut(&mut draft);
        part.format = initial.into();
        part.set_supply_range(TERMINAL, min, max).unwrap();
        let sheet = session.project().unwrap().sheet_target();
        let resolved = place(&mut session, &mut documents, &draft, sheet);
        let expected_range = SupplyRange::parse(TERMINAL, min, max).unwrap();
        assert_eq!(resolved["part"]["format"], expected);
        if min == "4500 mV" {
            assert_eq!(
                resolved["part"]["terminal_ratings"][TERMINAL],
                json!({
                    "supply_min":{"type":"quantity","value":"4.5","unit":"V"},
                    "supply_max":{"type":"quantity","value":"36","unit":"V"},
                })
            );
        }
        session.execute(sources::REMOVE, &json!({"source":LIBRARY})).unwrap();
        let snapshot = session.snapshot().unwrap();
        let native = encode_documents(snapshot.clone()).unwrap();
        let packed = encode_packed(snapshot).unwrap();
        let reopened = decode_documents(&native).unwrap();
        let unpacked = decode_packed(&packed).unwrap();
        assert_eq!(range(&reopened), expected_range.as_ref());
        assert_eq!(range(&unpacked.project), expected_range.as_ref());
        assert_eq!(unpacked.documents, native);
        assert_eq!(encode_documents(reopened).unwrap(), native);
        assert_eq!(encode_packed(unpacked.project).unwrap(), packed);
        let header: Json = serde_json::from_slice(&native[crate::persistence::ENTRY_POINT]).unwrap();
        assert_eq!(header["format"], "pcbcraft-project/16");
        let path = format!("library/parts/{LIBRARY}/regulator.r1.json");
        let part = PartDefinition::read(std::str::from_utf8(&native[&path]).unwrap()).unwrap();
        assert_eq!(part.format, expected);
        assert_eq!(part.encode().unwrap().as_bytes(), native[&path].as_slice());
        if min == "4500 mV" {
            let mut malformed = native.clone();
            let mut record = json!(part);
            let bounds = record["terminal_ratings"][TERMINAL].take();
            record["terminal_ratings"] = json!({"2":bounds});
            malformed.insert(path, serde_json::to_vec(&record).unwrap());
            let error = decode_documents(&malformed).err().expect("must reject a rated passive terminal");
            assert!(error.contains("Terminal 2") && error.contains("power_in"), "{error}");
        }
    }
}

/// Shared preparation rejects wrong dimensions, a reversed exact interval and wrong logical IDs.
#[test]
fn supply_range_refusals_name_bounds_and_exact_terminal() {
    let (mut session, documents, original) = source(false);
    for (terminal, min, max, messages) in [
        (TERMINAL, json!({"type":"quantity","value":"40","unit":"V"}), "36", ["Terminal 1", "40 V", "36 V"]),
        (TERMINAL, json!({"type":"quantity","value":"4.5","unit":"A"}), "36", ["Terminal 1", "supply_min", "unit V, got A"]),
        ("2", json!({"type":"quantity","value":"4.5","unit":"V"}), "36", ["Terminal 2", "power_in", "supply range"]),
        ("VIN", json!({"type":"quantity","value":"4.5","unit":"V"}), "36", ["Unknown", "terminal VIN", "exact symbol"]),
    ] {
        let mut record: Json = json!(original);
        record["terminal_ratings"] = json!({ (terminal): {"supply_min":min,"supply_max":{"type":"quantity","value":max,"unit":"V"}} });
        let draft: Draft = serde_json::from_value(record).unwrap();
        let before = session.snapshot().unwrap();
        let error = prepare(&mut session, &documents, &draft).unwrap_err().to_string();
        assert!(messages.iter().all(|msg| error.contains(msg)), "{error}");
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    }
    let mut symbol_less = original.clone();
    let p = part_mut(&mut symbol_less);
    p.symbol = None;
    p.packages.clear();
    p.set_supply_range(TERMINAL, "4.5", "36").unwrap();
    let error = prepare(&mut session, &documents, &symbol_less).unwrap_err().to_string();
    assert!(error.contains("terminal 1") && error.contains("exact symbol"), "{error}");

    // IEEE floats cannot distinguish these neighbors; exact signed intervals still can.
    assert!(SupplyRange::parse(TERMINAL, "1.000000000000000002", "1.000000000000000001").is_err());
    for (min, max) in [("-1.000000000000000002", "-1.000000000000000001"), ("-0.1", "0.1"), ("9.9", "10"), ("4.5", "4.5")] {
        assert!(SupplyRange::parse(TERMINAL, min, max).is_ok());
    }
}

/// Swapping two power-input numbers in a real drawing edit cannot transfer either
/// rating implicitly through dependency advancement or through the exact picker.
#[test]
fn supply_ranges_require_reentry_after_a_same_symbol_pin_swap() {
    use crate::authoring::{
        Action, Session as DrawingSession,
        publishing::{self, Publication},
        remap::PinRemap,
    };
    use crate::{library::sources::Kind as SourceKind, part_authoring::ReferenceField};

    let mut session = Session::default();
    let manifest = new_library(LIBRARY, "Synthetic supply ranges", LICENSE).unwrap();
    let mut symbol = Item::new(Kind::Symbol).unwrap();
    symbol.library = LIBRARY.into();
    symbol.id = "dual-input".into();
    for (object, label) in [(1, "VIN"), (3, "VIO")] {
        let Some(Primitive::Pin { name, electrical_type, .. }) = symbol.objects.get_mut(&object) else { panic!("fixture needs a pin") };
        *name = Some(label.into());
        *electrical_type = ElectricalType::PowerIn;
    }
    let first = publishing::prepare(Arc::new(symbol), &manifest, 0, None, false).unwrap();
    let item = first.item().as_ref().clone();
    let unit = item.unit.clone();
    let mut drawing = DrawingSession::new(item, true).unwrap();
    let pins = vec![PinRemap { unit: unit.clone(), object: 1, number: "2".into() }, PinRemap { unit, object: 3, number: "1".into() }];
    drawing.execute(0, Action::RemapPins { pins }).unwrap();
    let second = publishing::prepare(drawing.snapshot(), &manifest, 1, Some(1), false).unwrap();
    let mut documents = sources::Documents::from([("library.json".into(), manifest)]);
    documents.extend(first.documents().clone());
    documents.extend(second.documents().clone());
    load(&mut session, &documents);

    let old = first.reference();
    let new = second.reference();
    for (reference, terminal_one) in [(&old, "VIN"), (&new, "VIO")] {
        let exact = session.libraries().symbol(&reference.library, &reference.id, reference.revision).unwrap();
        let pin = &exact.pins()[exact.pin_index("1").unwrap()];
        assert_eq!(pin.name.as_deref(), Some(terminal_one));
        assert_eq!(pin.electrical_type, ElectricalType::PowerIn);
        assert_eq!(exact.pins()[exact.pin_index("2").unwrap()].electrical_type, ElectricalType::PowerIn);
    }
    let mut rated = Draft::new_part(old.clone()).unwrap();
    part_mut(&mut rated).set_supply_range("1", "4.5 V", "36 V").unwrap();
    part_mut(&mut rated).set_supply_range("2", "1.8 V", "3.3 V").unwrap();
    let catalog = session.libraries();
    rated.validate(catalog).unwrap();
    assert_eq!(rated.select_reference(&ReferenceField::Symbol, Some(old.clone()), catalog).unwrap(), rated);
    for error in [
        rated.advance_dependency(SourceKind::Symbol, &old, &new).unwrap_err(),
        rated.select_reference(&ReferenceField::Symbol, Some(new.clone()), catalog).unwrap_err(),
    ] {
        assert!(error.contains("re-enter"), "{error}");
    }
    assert_eq!(part_mut(&mut rated).symbol.as_ref(), Some(&old));
    assert_eq!(part_mut(&mut rated).terminal_ratings.len(), 2);
    for id in ["1", "2"] {
        part_mut(&mut rated).set_supply_range(id, "", "").unwrap();
    }
    let advanced = rated.advance_dependency(SourceKind::Symbol, &old, &new).unwrap();
    let mut picked = rated.select_reference(&ReferenceField::Symbol, Some(new), catalog).unwrap();
    assert_eq!(advanced, picked);
    assert!(part_mut(&mut picked).terminal_ratings.is_empty());
    part_mut(&mut picked).set_supply_range("2", "4.5 V", "36 V").unwrap();
    picked.validate(catalog).unwrap();
    assert!(part_mut(&mut picked).supply_range("1").is_none());
}

/// A complete Apply is one draft undo step, and removing the last range restores the unrated envelope.
#[test]
fn supply_range_edit_uses_one_draft_revision_and_native_format_is_strict() {
    let (_, _, original) = source(false);
    let mut draft = DraftSession::new(original.clone(), false).unwrap();
    let mut rated = original.clone();
    part_mut(&mut rated).set_supply_range(TERMINAL, "4.5 V", "36 V").unwrap();
    draft.replace(1, rated.clone()).unwrap();
    assert_eq!(draft.revision(), 2);
    assert_eq!(draft.record(), &rated);
    assert!(draft.replace(1, original.clone()).is_err());
    draft.history(2, false).unwrap();
    assert_eq!(draft.record(), &original);
    assert_eq!(draft.inspect()["can_undo"], false);
    draft.history(3, true).unwrap();
    let mut unrated = rated.clone();
    part_mut(&mut unrated).set_supply_range(TERMINAL, "", "").unwrap();
    assert_eq!(unrated, original);
    draft.replace(4, unrated).unwrap();
    assert_eq!(draft.record(), &original);

    // Native codecs do not repair malformed input's claimed format or shape.
    for (field, value) in [
        ("format", json!(PART_FORMAT)),
        ("format", json!("pcbcraft-part/3")),
        ("terminal_ratings", json!({})),
        ("terminal_ratings", json!({"1":{}})),
        ("terminal_ratings", json!({"1":{"supply_min":null}})),
    ] {
        let mut object = json!(rated);
        object[field] = value;
        assert!(PartDefinition::read(&object.to_string()).is_err(), "{object}");
    }
}

/// Schematic /8 supports both flat and rooted payloads; the header also requires rated content.
#[test]
fn supply_range_schematic_clipboard_is_content_gated_and_portable() {
    for (rated, hierarchical, format) in [(false, false, 1), (false, true, 2), (true, false, 8), (true, true, 8)] {
        let (mut session, mut documents, mut draft) = source(false);
        if rated {
            part_mut(&mut draft).set_supply_range(TERMINAL, "4.5 V", "36 V").unwrap();
        }
        let root = session.project().unwrap().sheet_target();
        let sheet = if hierarchical {
            apply(&mut session, crate::hierarchy::CREATE_CHILD, json!({"sheet":root.id(),"name":"Supply","x_nm":0,"y_nm":0}));
            session.project().unwrap().sheets().find(|sheet| sheet.name == "Supply").unwrap().target
        } else {
            root
        };
        place(&mut session, &mut documents, &draft, sheet);
        let project = session.project().unwrap().for_sheet(root).unwrap();
        let items = crate::selection::Selection { symbols: project.symbols().iter().map(|symbol| symbol.id()).collect(), ..Default::default() };
        let objects: Vec<_> = project.subsheets.iter().map(|block| format!("sheet-symbol-{}", block.id)).collect();
        let copy = session
            .execute(crate::clipboard::COPY, &json!({"items":items,"objects":objects,"sheet":root.id(),"expected_revision":session.revision()}))
            .unwrap();
        let text = copy["clipboard"].as_str().unwrap();
        let mut envelope: Json = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["format"], format!("pcbcraft-clipboard/{format}"));
        let fragment = Fragment::from_text(text).unwrap();
        assert_eq!(fragment.has_hierarchy(), hierarchical);
        assert_eq!(fragment.to_text().unwrap(), text);
        envelope["format"] =
            json!(if rated { if hierarchical { "pcbcraft-clipboard/2" } else { "pcbcraft-clipboard/1" } } else { "pcbcraft-clipboard/8" });
        assert!(Fragment::from_text(&envelope.to_string()).is_err());
        let mut target = Session::default();
        target.apply("project.new", &json!({"create_board":false})).unwrap();
        let sheet = target.project().unwrap().sheet_target().id();
        apply(&mut target, crate::clipboard::PASTE, json!({"text":text,"sheet":sheet,"x_nm":0,"y_nm":0}));
        assert_eq!(range(target.project().unwrap()).is_some(), rated);
    }
}

/// Both clipboard ladders advance for copied rated parts, not for an unrelated part in the source.
#[test]
fn supply_range_board_clipboard_keeps_rated_and_unrated_footprints_separate() {
    let (mut session, mut documents, mut draft) = source(true);
    let root = session.project().unwrap().sheet_target();
    place(&mut session, &mut documents, &draft, root);
    let part = part_mut(&mut draft);
    part.id = "rated-regulator".into();
    part.set_supply_range(TERMINAL, "4.5", "36").unwrap();
    place(&mut session, &mut documents, &draft, root);
    let components: Vec<_> = session
        .project()
        .unwrap()
        .components()
        .map(|component| (component.id(), component.part.as_ref().unwrap().supply_range(TERMINAL).is_some()))
        .collect();
    apply(&mut session, crate::INITIALIZE_BOARD_SETUP, json!({"copper_layers":2}));
    apply(&mut session, crate::INITIALIZE_UPDATE_BASELINE, json!({}));
    for (index, (component, _)) in components.iter().enumerate() {
        apply(&mut session, crate::PLACE_ASSIGNED_FOOTPRINT, json!({"component":component,"x_nm":index as i64 * 10_000_000,"y_nm":0}));
    }
    let original = session.snapshot().unwrap();
    for (owner, rated) in components {
        let target = original.board_target().unwrap();
        let footprint = original.board_view(target).unwrap().footprints().find(|f| f.component == owner).unwrap().id;
        let items = BoardSelection { footprints: [footprint].into(), ..Default::default() };
        let result = session.execute(crate::board::clipboard::COPY, &json!({"items":items,"expected_revision":session.revision()})).unwrap();
        let text = result["clipboard"].as_str().unwrap();
        let mut envelope: Json = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["format"], if rated { "pcbcraft-clipboard/9" } else { "pcbcraft-clipboard/1" });
        let nested: Json = serde_json::from_str(envelope["circuits"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(nested["format"], if rated { "pcbcraft-clipboard/8" } else { "pcbcraft-clipboard/1" });
        assert_eq!(BoardFragment::from_text(text).unwrap().to_text().unwrap(), text);
        envelope["format"] = json!(if rated { "pcbcraft-clipboard/1" } else { "pcbcraft-clipboard/9" });
        assert!(BoardFragment::from_text(&envelope.to_string()).is_err());
        if rated {
            apply(
                &mut session,
                crate::board::clipboard::PASTE,
                json!({
                    "text":text,"x_nm":0,"y_nm":10_000_000,
                    "schematic_targets":{ (root.id().to_string()):{"sheet":root.id(),"at":{"x_nm":0,"y_nm":25_400_000}} }
                }),
            );
            let copied = session.project().unwrap().components().find(|part| part.ordinal > original.highest_id()).unwrap();
            assert_eq!(copied.part.as_ref().unwrap().supply_range(TERMINAL).cloned(), SupplyRange::parse(TERMINAL, "4.5", "36").unwrap());
            session.apply("edit.undo", &Json::Null).unwrap();
            crate::assert_same_content(&original, &session.snapshot().unwrap());
        }
    }
}
