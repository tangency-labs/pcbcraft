use super::*;
use crate::{
    authoring::{Action, Item, Kind, Primitive, Session as AuthoringSession, publishing::new_library},
    board::clipboard::{BoardFragment, BoardSelection},
    clipboard::Fragment,
    library::parts::{LEGACY_PART_FORMAT, MAX_TERMINAL_RATINGS, PART_FORMAT, Package, PartDefinition, RATED_PART_FORMAT, SupplyRange},
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
            let error = decode_documents(&malformed).expect_err("must reject a rated passive terminal");
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

    let mut draft = DraftSession::new(rated.clone(), false).unwrap();
    let mut rename = rated.clone();
    part_mut(&mut rename).name = "Edited before the attempted switch".into();
    draft.replace(1, rename.clone()).unwrap();
    draft.history(2, false).unwrap();
    let before = draft.inspect();
    let mut raw_switch = rated.clone();
    part_mut(&mut raw_switch).symbol = Some(new.clone());
    assert!(draft.replace(3, raw_switch.clone()).unwrap_err().contains("re-enter"));
    for id in ["1", "2"] {
        part_mut(&mut raw_switch).set_supply_range(id, "", "").unwrap();
    }
    assert!(draft.replace(3, raw_switch).unwrap_err().contains("re-enter"));
    assert_eq!(draft.inspect(), before);
    draft.history(3, true).unwrap();
    assert_eq!(draft.record(), &rename);
    draft.history(4, false).unwrap();
    assert_eq!(draft.record(), &rated);

    assert_eq!(part_mut(&mut rated).symbol.as_ref(), Some(&old));
    assert_eq!(part_mut(&mut rated).terminal_ratings.len(), 2);
    for id in ["1", "2"] {
        part_mut(&mut rated).set_supply_range(id, "", "").unwrap();
    }
    let advanced = rated.advance_dependency(SourceKind::Symbol, &old, &new).unwrap();
    let mut picked = rated.select_reference(&ReferenceField::Symbol, Some(new), catalog).unwrap();
    assert_eq!(advanced, picked);
    assert!(part_mut(&mut picked).terminal_ratings.is_empty());
    draft.replace(5, rated).unwrap();
    let unrated = draft.inspect();
    let mut reentered = picked.clone();
    part_mut(&mut reentered).set_supply_range("2", "4.5 V", "36 V").unwrap();
    assert!(draft.replace(6, reentered.clone()).unwrap_err().contains("re-enter"));
    assert_eq!(draft.inspect(), unrated);
    draft.replace(6, picked).unwrap();
    draft.replace(7, reentered.clone()).unwrap();
    draft.record().validate(catalog).unwrap();
    assert!(part_mut(&mut reentered).supply_range("1").is_none());
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

/// A rated part with a designator lock copies as /8 in either scope and pastes with both its
/// supply range and lock intact; the lock-only /3 and NPTH /6 envelopes refuse the rating.
#[test]
fn rated_and_locked_part_round_trips_through_schematic_clipboard_8() {
    for hierarchical in [false, true] {
        let (mut session, mut documents, mut draft) = source(false);
        part_mut(&mut draft).set_supply_range(TERMINAL, "4.5 V", "36 V").unwrap();
        let root = session.project().unwrap().sheet_target();
        let sheet = if hierarchical {
            apply(&mut session, crate::hierarchy::CREATE_CHILD, json!({"sheet":root.id(),"name":"Supply","x_nm":0,"y_nm":0}));
            session.project().unwrap().sheets().find(|sheet| sheet.name == "Supply").unwrap().target
        } else {
            root
        };
        place(&mut session, &mut documents, &draft, sheet);
        let placed = session.project().unwrap().for_sheet(sheet).unwrap();
        let symbol = &placed.symbols()[0];
        let params = json!({"symbol":symbol.id(),"reference":placed.reference(symbol),"value":placed.value(symbol),
            "designator_locked":true,"sheet":sheet.id()});
        apply(&mut session, crate::SET_SYMBOL_FIELDS, params);
        assert!(session.project().unwrap().components().next().unwrap().designator_locked());
        let project = session.project().unwrap().for_sheet(root).unwrap();
        let items = crate::selection::Selection { symbols: project.symbols().iter().map(|symbol| symbol.id()).collect(), ..Default::default() };
        let objects: Vec<_> = project.subsheets.iter().map(|block| format!("sheet-symbol-{}", block.id)).collect();
        let copy = session
            .execute(crate::clipboard::COPY, &json!({"items":items,"objects":objects,"sheet":root.id(),"expected_revision":session.revision()}))
            .unwrap();
        let text = copy["clipboard"].as_str().unwrap();
        let mut envelope: Json = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["format"], "pcbcraft-clipboard/8");
        let fragment = Fragment::from_text(text).unwrap();
        assert_eq!(fragment.has_hierarchy(), hierarchical);
        assert_eq!(fragment.to_text().unwrap(), text);
        for lower in ["pcbcraft-clipboard/1", "pcbcraft-clipboard/2", "pcbcraft-clipboard/3", "pcbcraft-clipboard/6"] {
            envelope["format"] = json!(lower);
            assert!(Fragment::from_text(&envelope.to_string()).is_err(), "{lower} must refuse a rated, locked fragment");
        }
        let mut target = Session::default();
        target.apply("project.new", &json!({"create_board":false})).unwrap();
        let sheet = target.project().unwrap().sheet_target().id();
        apply(&mut target, crate::clipboard::PASTE, json!({"text":text,"sheet":sheet,"x_nm":0,"y_nm":0}));
        let pasted = target.project().unwrap();
        assert!(range(pasted).is_some());
        assert!(pasted.components().next().unwrap().designator_locked());
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

/// A rated device with a slotted package retains both records through schematic and board paste.
#[test]
fn rated_part_and_plated_slot_round_trip_through_both_clipboards() {
    use crate::board::{
        definition::{CopperBinding, PadShape},
        slot::PlatedSlot,
    };

    let (mut session, mut documents, mut draft) = source(true);
    let slot = PlatedSlot::new(400_000, 1_200_000, 90_000).unwrap();
    let mut item = Item::new(Kind::Footprint).unwrap();
    item.id = "slotted-regulator-package".into();
    let mut editor = AuthoringSession::new(item, false).unwrap();
    let Primitive::Pad { mut pad } = editor.snapshot().objects[&1].clone() else { panic!("fixture needs a pad") };
    pad.shape = PadShape::Oval;
    pad.width_nm = 800_000;
    pad.height_nm = 1_700_000;
    pad.corner_radius_nm = 0;
    pad.copper = CopperBinding::AllCopper;
    pad.drill_nm = None;
    pad.plated_slot = Some(slot);
    editor.execute(editor.revision(), Action::Replace { object: 1, primitive: Primitive::Pad { pad } }).unwrap();
    let prepared = session.execute(sources::PREPARE_PUBLICATION, &json!({"item":editor.snapshot(),"documents":&documents})).unwrap();
    let footprint_ref: Reference = serde_json::from_value(prepared["address"].clone()).unwrap();
    documents.extend(serde_json::from_value::<sources::Documents>(prepared["documents"].clone()).unwrap());
    load(&mut session, &documents);

    let part = part_mut(&mut draft);
    part.packages.get_mut("test").unwrap().footprint = footprint_ref;
    part.set_supply_range(TERMINAL, "4.5 V", "36 V").unwrap();
    let root = session.project().unwrap().sheet_target();
    place(&mut session, &mut documents, &draft, root);
    let project = session.snapshot().unwrap();
    let items = crate::selection::Selection::one_symbol(project.symbols()[0].id());
    let copied = session.execute(crate::clipboard::COPY, &json!({"items":items,"sheet":root.id(),"expected_revision":session.revision()})).unwrap();
    let text = copied["clipboard"].as_str().unwrap();
    let mut envelope: Json = serde_json::from_str(text).unwrap();
    assert_eq!(envelope["format"], "pcbcraft-clipboard/10");
    assert_eq!(Fragment::from_text(text).unwrap().to_text().unwrap(), text);
    for earlier in ["pcbcraft-clipboard/7", "pcbcraft-clipboard/8"] {
        envelope["format"] = json!(earlier);
        assert!(Fragment::from_text(&envelope.to_string()).is_err());
    }
    let expected = SupplyRange::parse(TERMINAL, "4.5 V", "36 V").unwrap();
    let mut target = Session::default();
    target.apply("project.new", &json!({})).unwrap();
    let sheet = target.project().unwrap().sheet_target().id();
    apply(&mut target, crate::clipboard::PASTE, json!({"text":text,"sheet":sheet,"x_nm":0,"y_nm":0}));
    let pasted = target.project().unwrap();
    assert_eq!(range(pasted).cloned(), expected);
    let part = pasted.components().next().unwrap().part.as_ref().unwrap();
    assert_eq!(part.resolved.footprints.values().next().unwrap().pads[0].plated_slot, Some(slot));

    let component = project.components().next().unwrap().id();
    apply(&mut session, crate::INITIALIZE_BOARD_SETUP, json!({"copper_layers":2}));
    apply(&mut session, crate::INITIALIZE_UPDATE_BASELINE, json!({}));
    apply(&mut session, crate::PLACE_ASSIGNED_FOOTPRINT, json!({"component":component,"x_nm":0,"y_nm":0}));
    let before = session.snapshot().unwrap();
    let footprint = before.boards().next().unwrap().footprints().next().unwrap().id;
    let items = BoardSelection { footprints: [footprint].into(), ..Default::default() };
    let copied = session.execute(crate::board::clipboard::COPY, &json!({"items":items,"expected_revision":session.revision()})).unwrap();
    let text = copied["clipboard"].as_str().unwrap();
    let mut envelope: Json = serde_json::from_str(text).unwrap();
    assert_eq!(envelope["format"], "pcbcraft-clipboard/11");
    let nested: Json = serde_json::from_str(envelope["circuits"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(nested["format"], "pcbcraft-clipboard/10");
    assert_eq!(BoardFragment::from_text(text).unwrap().to_text().unwrap(), text);
    for earlier in ["pcbcraft-clipboard/8", "pcbcraft-clipboard/9"] {
        envelope["format"] = json!(earlier);
        assert!(BoardFragment::from_text(&envelope.to_string()).is_err());
    }
    apply(
        &mut session,
        crate::board::clipboard::PASTE,
        json!({"text":text,"x_nm":0,"y_nm":10_000_000,
            "schematic_targets":{(root.id().to_string()):{"sheet":root.id(),"at":{"x_nm":0,"y_nm":25_400_000}}}}),
    );
    let pasted = session.project().unwrap();
    let copied = pasted.components().find(|part| part.ordinal > before.highest_id()).unwrap();
    assert_eq!(copied.part.as_ref().unwrap().supply_range(TERMINAL).cloned(), expected);
    let copied_footprint = pasted.boards().next().unwrap().footprints().find(|footprint| footprint.component == copied.id()).unwrap();
    assert!(copied_footprint.pads.iter().any(|pad| pad.plated_slot.is_some_and(|hole| hole.width_nm() == slot.dimensions().0)));
    session.apply("edit.undo", &Json::Null).unwrap();
    crate::assert_same_content(&before, &session.snapshot().unwrap());
}

/// Draft editing, native encoding, native decoding and the part/4 schema all enforce the
/// same [`MAX_TERMINAL_RATINGS`] limit: 128 publishes and reopens, 129 is rejected everywhere.
#[test]
fn supply_range_limit_matches_draft_writer_reader_and_schema() {
    let schema: Json = serde_json::from_str(include_str!("../../../../../docs/schemas/part-v4.schema.json")).unwrap();
    assert_eq!(schema["properties"]["terminal_ratings"]["maxProperties"], json!(MAX_TERMINAL_RATINGS));

    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":false})).unwrap();
    let mut documents = sources::Documents::from([("library.json".into(), new_library(LIBRARY, "Synthetic supply ranges", LICENSE).unwrap())]);
    let mut symbol = Item::new(Kind::Symbol).unwrap();
    symbol.id = "max-supply".into();
    symbol.name = "Max supply symbol".into();
    let mut symbol_editor = AuthoringSession::new(symbol, false).unwrap();
    symbol_editor.execute(0, Action::Delete { objects: vec![1, 3] }).unwrap();
    for index in 1..=MAX_TERMINAL_RATINGS {
        let grid = index as i64 - 1;
        symbol_editor
            .execute(
                symbol_editor.revision(),
                Action::Insert {
                    primitive: Primitive::Pin {
                        number: index.to_string(),
                        name: Some(format!("VIN{index}")),
                        electrical_type: ElectricalType::PowerIn,
                        alternates: Vec::new(),
                        at: crate::schematic::Point { x_nm: (grid % 16 - 8) * 1_270_000, y_nm: (grid / 16 - 4) * 1_270_000 },
                        appearance: None,
                    },
                },
            )
            .unwrap();
    }
    let mut footprint = Item::new(Kind::Footprint).unwrap();
    footprint.id = "max-supply-package".into();
    let Primitive::Pad { pad: template } = footprint.objects[&1].clone() else { panic!("fixture needs a pad") };
    let mut footprint_editor = AuthoringSession::new(footprint, false).unwrap();
    for index in 3..=MAX_TERMINAL_RATINGS {
        let mut pad = template.clone();
        pad.number = index.to_string();
        pad.center = crate::schematic::Point { x_nm: (index as i64 % 16) * 1_270_000, y_nm: (index as i64 / 16 + 1) * 1_270_000 };
        footprint_editor.execute(footprint_editor.revision(), Action::Insert { primitive: Primitive::Pad { pad } }).unwrap();
    }
    let symbol_prepared =
        session.execute(sources::PREPARE_PUBLICATION, &json!({"item":symbol_editor.snapshot().as_ref(),"documents":&documents})).unwrap();
    let symbol_ref: Reference = serde_json::from_value(symbol_prepared["address"].clone()).unwrap();
    documents.extend(serde_json::from_value::<sources::Documents>(symbol_prepared["documents"].clone()).unwrap());
    let footprint_prepared =
        session.execute(sources::PREPARE_PUBLICATION, &json!({"item":footprint_editor.snapshot().as_ref(),"documents":&documents})).unwrap();
    let footprint_ref: Reference = serde_json::from_value(footprint_prepared["address"].clone()).unwrap();
    documents.extend(serde_json::from_value::<sources::Documents>(footprint_prepared["documents"].clone()).unwrap());
    load(&mut session, &documents);

    let mut unrated = Draft::new_part(symbol_ref).unwrap();
    let part = part_mut(&mut unrated);
    part.id = "max-supply-device".into();
    part.name = "Max supply device".into();
    let mapping = (1..=MAX_TERMINAL_RATINGS).map(|index| (index.to_string(), vec![index.to_string()])).collect();
    part.packages
        .insert("test".into(), Package { name: "Max supply package".into(), footprint: footprint_ref, mapping, unconnected_pads: Vec::new() });
    let mut editor = DraftSession::new(unrated.clone(), false).unwrap();
    let mut rated = unrated.clone();
    for index in 1..=MAX_TERMINAL_RATINGS {
        part_mut(&mut rated).set_supply_range(&index.to_string(), "4.5 V", "36 V").unwrap();
    }
    editor.replace(1, rated.clone()).unwrap();
    let encoded = part_mut(&mut rated).encode().unwrap();
    let reopened = PartDefinition::read(&encoded).unwrap();
    assert_eq!(reopened.format, RATED_PART_FORMAT);
    assert_eq!(reopened.terminal_ratings.len(), MAX_TERMINAL_RATINGS);
    let prepared = prepare(&mut session, &documents, editor.record()).unwrap();
    documents.extend(serde_json::from_value::<sources::Documents>(prepared["documents"].clone()).unwrap());
    load(&mut session, &documents);
    let resolved =
        session.execute(sources::RESOLVE_PART, &json!({"library":LIBRARY,"definition":"max-supply-device","definition_revision":1})).unwrap();
    assert_eq!(resolved["part"]["terminal_ratings"].as_object().unwrap().len(), MAX_TERMINAL_RATINGS);

    // Updating an existing rating at the limit stays allowed.
    let mut edited = editor.record().clone();
    part_mut(&mut edited).set_supply_range("1", "3.3 V", "5.5 V").unwrap();
    assert_eq!(part_mut(&mut edited).terminal_ratings.len(), MAX_TERMINAL_RATINGS);

    // Adding a 129th rating fails in draft editing, encoding and native decoding, leaving history intact.
    let extra = (MAX_TERMINAL_RATINGS + 1).to_string();
    let mut over = editor.record().clone();
    let error = part_mut(&mut over).set_supply_range(&extra, "4.5 V", "36 V").unwrap_err();
    assert!(error.contains(&MAX_TERMINAL_RATINGS.to_string()), "{error}");
    assert_eq!(part_mut(&mut over).terminal_ratings.len(), MAX_TERMINAL_RATINGS);
    part_mut(&mut over).terminal_ratings.insert(extra.clone(), SupplyRange::parse(&extra, "4.5 V", "36 V").unwrap().unwrap());
    let revision = editor.revision();
    let before = editor.record().clone();
    assert!(editor.replace(revision, over.clone()).unwrap_err().contains(&MAX_TERMINAL_RATINGS.to_string()));
    assert_eq!(editor.record(), &before);
    assert_eq!(editor.revision(), revision);
    assert!(part_mut(&mut over).encode().unwrap_err().contains(&MAX_TERMINAL_RATINGS.to_string()));
    let mut json_record: Json = serde_json::from_str(&encoded).unwrap();
    json_record["terminal_ratings"][&extra] = json!({"supply_min":{"type":"quantity","value":"4.5","unit":"V"}});
    assert!(PartDefinition::read(&json_record.to_string()).unwrap_err().contains(&MAX_TERMINAL_RATINGS.to_string()));
    editor.history(revision, false).unwrap();
    assert_eq!(editor.record(), &unrated);
}
