use super::*;
use crate::{CONNECT_PINS, MOVE_SYMBOL, PLACE_SYMBOL, ROTATE_SYMBOL, persistence};

/// Build original command-authored placements; each test supplies its own immutable snapshots.
fn placed(board: bool) -> (Session, Target, Terminal) {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":board})).unwrap();
    s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let p = s.snapshot().unwrap();
    let terminal = p.symbols()[0].terminals()[0];
    let target = p.no_connect_target(terminal).unwrap();
    (s, target, terminal)
}

/// Use the public command contract for both new intent and removal, including current authority.
fn mark(s: &mut Session, target: &Target, marked: bool) {
    s.apply(SET, &json!({"target":target,"marked":marked,"expected_revision":s.revision()})).unwrap();
}

/// Marker edits are atomic, revision-checked and idempotent. Intent never changes connectivity,
/// while later wiring reports a conflict and still exposes unrelated unconnected pins.
#[test]
fn edits_and_checks_preserve_actual_connectivity_and_history() {
    let (mut s, target, terminal) = placed(false);
    let before = s.snapshot().unwrap();
    let bare = s.execute(CHECK, &json!({})).unwrap();
    assert_eq!(bare["no_connect_check"]["total_findings"], 2);
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    mark(&mut s, &target, true);
    let marked = s.snapshot().unwrap();
    let revision = s.revision();
    assert!(marked.has_no_connect(terminal));
    let report = marked.check_no_connects(revision);
    assert!(report.markers[0].issues.is_empty());
    assert_eq!(report.findings.len(), 1);
    assert_eq!(marked.connectivity().unconnected, before.connectivity().unconnected);
    mark(&mut s, &target, true);
    assert_eq!(s.revision(), revision);
    assert!(Arc::ptr_eq(&marked, &s.snapshot().unwrap()));
    for params in [
        json!({"target":target,"marked":false,"expected_revision":revision-1}),
        json!({"target":{"part":target.part,"terminal":"BAD"},"marked":true,"expected_revision":revision}),
        json!({"target":{"part":target.part,"terminal":"BAD"},"marked":false,"expected_revision":revision}),
        json!({"target":target,"marked":"true","expected_revision":revision}),
        json!({"target":{"part":target.part,"terminal":target.terminal,"extra":0},"marked":true,"expected_revision":revision}),
    ] {
        assert!(s.apply(SET, &params).is_err());
        assert!(Arc::ptr_eq(&marked, &s.snapshot().unwrap()));
    }
    s.apply(MOVE_SYMBOL, &json!({"symbol":marked.symbols()[0].id(),"x_nm":12_700_000,"y_nm":5_080_000,"expected_revision":s.revision()})).unwrap();
    for _ in 0..4 {
        let p = s.snapshot().unwrap();
        let report = p.check_no_connects(s.revision());
        assert_eq!(report.markers[0].appearances[0].at, p.terminal_position(terminal).unwrap());
        s.apply(ROTATE_SYMBOL, &json!({"symbol":p.symbols()[0].id(),"expected_revision":s.revision()})).unwrap();
    }
    let p = s.snapshot().unwrap();
    let other = p.symbols()[0].terminals()[1];
    s.apply(CONNECT_PINS, &json!({"from":terminal.inspect(),"to":other.inspect(),"expected_revision":s.revision()})).unwrap();
    assert_eq!(s.snapshot().unwrap().check_no_connects(s.revision()).markers[0].issues, vec!["no_connect_connected"]);
    mark(&mut s, &target, false);
    assert!(!s.snapshot().unwrap().has_no_connect(terminal));
    s.apply("edit.undo", &json!({})).unwrap();
    assert!(s.snapshot().unwrap().has_no_connect(terminal));
    s.apply("edit.redo", &json!({})).unwrap();
    assert!(!s.snapshot().unwrap().has_no_connect(terminal));
}

/// Split and packed codecs retain exact intent bytes and reject unsupported or malformed
/// extensions. Existing documents without markers remain byte-identical through the codec.
#[test]
fn marker_storage_roundtrips_and_rejects_incompatible_headers() {
    for board in [false, true] {
        let (mut s, target, _) = placed(board);
        let original = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
        let reopened = persistence::decode_documents(&original).unwrap();
        assert_eq!(original, persistence::encode_documents(reopened).unwrap());
        mark(&mut s, &target, true);
        let p = s.snapshot().unwrap();
        assert!(persistence::encode(p.clone()).is_err());
        let docs = persistence::encode_documents(p.clone()).unwrap();
        let value: Value = serde_json::from_slice(&docs["project.json"]).unwrap();
        assert_eq!(value["format"], "pcbcraft-project/19");
        let reopened = persistence::decode_documents(&docs).unwrap();
        assert_eq!(docs, persistence::encode_documents(reopened).unwrap());
        let packed = persistence::encode_packed(p).unwrap();
        let reopened = persistence::decode_packed(&packed).unwrap();
        assert_eq!(docs, reopened.documents);
        for format in ["pcbcraft-project/16", "pcbcraft-project/17"] {
            let mut bad = docs.clone();
            let mut entry = value.clone();
            entry["format"] = json!(format);
            bad.insert("project.json".into(), serde_json::to_vec(&entry).unwrap());
            assert!(persistence::decode_documents(&bad).is_err());
        }
        for records in [
            Value::Null,
            json!([]),
            json!([{"terminal":"1","electrical_type":"passive"},{"terminal":"1","electrical_type":"passive"}]),
            json!([{"terminal":"1","electrical_type":"passive","extra":0}]),
        ] {
            let mut bad = docs.clone();
            let mut entry = value.clone();
            entry["components"][0]["no_connects"] = records;
            bad.insert("project.json".into(), serde_json::to_vec(&entry).unwrap());
            assert!(persistence::decode_documents(&bad).is_err());
        }
    }
}

/// Both mirror axes preserve logical intent at every quarter turn, including undo/redo and
/// reopening project/19 together with sheet/6 before returning to the original pose.
#[test]
fn no_connect_intent_follows_mirrors_and_reopens_with_its_placement() {
    for board in [false, true] {
        for turn in 0..4 {
            for axis in ["horizontal", "vertical"] {
                let (mut s, target, terminal) = placed(board);
                let symbol = s.project().unwrap().symbols()[0].id();
                s.apply(
                    MOVE_SYMBOL,
                    &json!({"symbol":symbol,"x_nm":12_700_000,"y_nm":5_080_000,"rotation":turn*90,"expected_revision":s.revision()}),
                )
                .unwrap();
                mark(&mut s, &target, true);
                let before = s.snapshot().unwrap();
                let at = before.terminal_position(terminal).unwrap();
                let center = before.symbols()[0].position;
                let expected = match axis {
                    "horizontal" => Point { x_nm: 2 * center.x_nm - at.x_nm, y_nm: at.y_nm },
                    _ => Point { x_nm: at.x_nm, y_nm: 2 * center.y_nm - at.y_nm },
                };
                s.apply(crate::MIRROR_SYMBOL, &json!({"symbol":symbol,"axis":axis,"expected_revision":s.revision()})).unwrap();
                let mirrored = s.snapshot().unwrap();
                let report = mirrored.check_no_connects(s.revision());
                assert_eq!(report.total_markers, 1);
                assert_eq!(report.markers[0].target, target);
                assert!(report.markers[0].issues.is_empty());
                assert_eq!(report.markers[0].appearances[0].at, expected);
                assert_eq!(mirrored.terminal_position(terminal), Some(expected));
                assert!(mirrored.has_no_connect(terminal));

                s.apply("edit.undo", &json!({})).unwrap();
                crate::assert_same_content(&before, &s.snapshot().unwrap());
                s.apply("edit.redo", &json!({})).unwrap();
                crate::assert_same_content(&mirrored, &s.snapshot().unwrap());

                let docs = persistence::encode_documents(mirrored.clone()).unwrap();
                let entry: Value = serde_json::from_slice(&docs["project.json"]).unwrap();
                assert_eq!(entry["format"], "pcbcraft-project/19");
                let sheet = docs.iter().find(|(path, _)| path.starts_with("sheets/")).unwrap().1;
                let sheet: Value = serde_json::from_slice(sheet).unwrap();
                assert_eq!(sheet["format"], "pcbcraft-sheet/6");
                let folder = persistence::decode_documents(&docs).unwrap();
                let packed = persistence::encode_packed(mirrored).unwrap();
                let archive = persistence::decode_packed(&packed).unwrap().project;
                for reopened in [folder, archive] {
                    assert_eq!(persistence::encode_documents(reopened.clone()).unwrap(), docs);
                    s.open_snapshot(reopened);
                    let report = s.project().unwrap().check_no_connects(s.revision());
                    assert_eq!(report.markers[0].target, target);
                    assert_eq!(report.markers[0].appearances[0].at, expected);
                    s.apply(crate::MIRROR_SYMBOL, &json!({"symbol":symbol,"axis":axis,"expected_revision":s.revision()})).unwrap();
                    crate::assert_same_content(&before, &s.snapshot().unwrap());
                }
            }
        }
    }
}

/// Unresolved intent remains addressed rather than being discarded or suppressing an
/// unrelated pin's warning; explicit removal can clear a retained incompatible record.
#[test]
fn retained_intent_reports_missing_and_changed_terminal_meanings() {
    let (mut s, target, _) = placed(false);
    mark(&mut s, &target, true);
    let mut docs = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let mut entry: Value = serde_json::from_slice(&docs["project.json"]).unwrap();
    entry["components"][0]["no_connects"] = json!([{"terminal":"1","electrical_type":"output"},{"terminal":"OLD","electrical_type":"input"}]);
    docs.insert("project.json".into(), serde_json::to_vec(&entry).unwrap());
    let p = persistence::decode_documents(&docs).unwrap();
    let report = p.check_no_connects(7);
    assert_eq!(report.markers[0].issues, vec!["no_connect_incompatible_terminal"]);
    assert_eq!(report.markers[1].issues, vec!["no_connect_missing_target"]);
    assert!(report.findings.iter().any(|f| f.rule == "unconnected_pin" && f.target.terminal == "2"));
    let copied = crate::clipboard::Fragment::capture(&p, &crate::selection::Selection::one_symbol(p.symbols()[0].id())).unwrap().to_text().unwrap();
    let restored = crate::clipboard::Fragment::from_text(&copied).unwrap();
    assert_eq!(restored.project.check_no_connects(7).markers.len(), 2);
    s.open_snapshot(p);
    mark(&mut s, &Target { part: target.part, terminal: "OLD".into() }, false);
    assert_eq!(s.snapshot().unwrap().check_no_connects(s.revision()).markers.len(), 1);
}

/// Shared supply appearances are one intent. Independent duplication remaps the part while
/// partial-unit capture excludes the other unit's flags; removing one drawing retains diagnosis.
#[test]
fn multiunit_intent_and_partial_clipboard_use_logical_terminal_identity() {
    use crate::{clipboard, library, selection::Selection};
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let docs: BTreeMap<String, String> = [
        ("library.json".into(), include_str!("../../../../examples/native-parts/library.json").into()),
        ("symbols/dual-amplifier.json".into(), include_str!("../../../../examples/native-parts/symbols/dual-amplifier.json").into()),
    ]
    .into_iter()
    .collect();
    s.apply(library::sources::LOAD, &json!({"source":"native","documents":docs})).unwrap();
    let definition = s.libraries().latest_symbol("parts-demo", "dual-amplifier").unwrap();
    s.apply(PLACE_SYMBOL,&json!({"library":"parts-demo","definition":"dual-amplifier","definition_revision":1,"definition_digest":library::sources::symbol_digest(&definition).unwrap(),"unit":"a","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let first = s.snapshot().unwrap().symbols()[0].clone();
    let supply = Target { part: first.component_id(), terminal: "3".into() };
    mark(&mut s, &supply, true);
    s.apply(crate::PLACE_UNIT, &json!({"component":first.component_id(),"unit":"b","x_nm":25_400_000,"y_nm":0,"expected_revision":s.revision()}))
        .unwrap();
    let second = s.snapshot().unwrap().symbols()[1].clone();
    let input_b = Target { part: first.component_id(), terminal: "5".into() };
    mark(&mut s, &input_b, true);
    let report = s.snapshot().unwrap().check_no_connects(s.revision());
    assert_eq!(report.markers[0].appearances.len(), 2);
    assert!(report.markers.iter().all(|marker| marker.issues.is_empty()));
    let p = s.snapshot().unwrap();
    let mut unresolved = (*p).clone();
    Arc::make_mut(&mut unresolved.components).get_mut(&first.component).unwrap().no_connects.insert("OLD".into(), ElectricalType::Input);
    let error = clipboard::Fragment::capture(&unresolved, &Selection::one_symbol(first.id())).unwrap_err();
    assert!(error.to_string().contains("copy all placed units"));
    let text = clipboard::Fragment::capture(&p, &Selection::one_symbol(first.id())).unwrap().to_text().unwrap();
    s.apply(clipboard::PASTE, &json!({"text":text,"x_nm":50_800_000,"y_nm":0,"rotation":90,"expected_revision":s.revision()})).unwrap();
    let p = s.snapshot().unwrap();
    let copy = p.symbols().last().unwrap();
    assert_ne!(copy.component_id(), first.component_id());
    assert_eq!(p.component(copy).unwrap().no_connects.keys().collect::<Vec<_>>(), vec!["3"]);
    assert!(p.has_no_connect(copy.terminals().into_iter().find(|pin| pin.number() == "3").unwrap()));
    s.apply(crate::DELETE_SYMBOL, &json!({"symbol":second.id(),"expected_revision":s.revision()})).unwrap();
    let report = s.snapshot().unwrap().check_no_connects(s.revision());
    assert!(report.markers.iter().any(|marker| marker.target == input_b && marker.issues == vec!["no_connect_missing_target"]));
    s.apply("edit.undo", &json!({})).unwrap();
    assert!(s.snapshot().unwrap().check_no_connects(s.revision()).markers.iter().all(|marker| marker.issues.is_empty()));
}

/// Authored named contacts are connections even without a wire. Moving the label away and
/// Undo re-evaluate the conflict without changing or losing the marker's logical identity.
#[test]
fn named_contacts_are_checked_against_intent() {
    let (mut s, target, terminal) = placed(false);
    mark(&mut s, &target, true);
    let at = s.snapshot().unwrap().terminal_position(terminal).unwrap();
    s.apply(crate::net_labels::ADD, &json!({"name":"NC_CONFLICT","style":"global","x_nm":at.x_nm,"y_nm":at.y_nm,"expected_revision":s.revision()}))
        .unwrap();
    assert_eq!(s.snapshot().unwrap().check_no_connects(s.revision()).markers[0].issues, vec!["no_connect_connected"]);
    let id = s.snapshot().unwrap().net_labels()[0].object_id();
    s.apply(
        crate::net_labels::SET,
        &json!({"id":id,"name":"NC_CONFLICT","style":"global","x_nm":25_400_000,"y_nm":0,"expected_revision":s.revision()}),
    )
    .unwrap();
    assert!(s.snapshot().unwrap().check_no_connects(s.revision()).markers[0].issues.is_empty());
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(s.snapshot().unwrap().check_no_connects(s.revision()).markers[0].issues, vec!["no_connect_connected"]);
}

/// Retained invalid intent is syntactically valid authored data. A full-budget source provides
/// an exhaustive count oracle and verifies that compound duplication cannot bypass edit limits.
#[test]
fn detailed_report_is_bounded_and_over_budget_paste_is_atomic() {
    let (mut s, _, _) = placed(false);
    for index in 1..33 {
        s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":index*12_700_000,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    }
    let mut p = (*s.snapshot().unwrap()).clone();
    for part in Arc::make_mut(&mut p.components).values_mut().take(32) {
        part.no_connects = (0..128).map(|i| (format!("P{i}"), ElectricalType::Passive)).collect();
    }
    let documents = persistence::encode_documents(Arc::new(p)).unwrap();
    let p = persistence::decode_documents(&documents).unwrap();
    let report = p.check_no_connects(s.revision());
    assert_eq!(report.total_markers, MAX_MARKERS);
    assert_eq!(report.total_findings, MAX_MARKERS + 66);
    assert_eq!(report.markers.len(), MAX_FINDINGS);
    assert_eq!(report.findings.len(), MAX_FINDINGS);
    assert!(report.truncated);
    let text = crate::clipboard::Fragment::capture(&p, &crate::selection::Selection::one_symbol(p.symbols()[0].id())).unwrap().to_text().unwrap();
    s.open_snapshot(p);
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    let error = s.apply(crate::clipboard::PASTE, &json!({"text":text,"x_nm":500_000_000,"y_nm":0,"expected_revision":revision})).unwrap_err();
    assert!(error.to_string().contains("No-connect intent"), "{error}");
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert_eq!(s.revision(), revision);
}

/// Native hierarchical fragments preserve marker intent while independently remapping the
/// child sheet and part identity. Undo/redo and reopening restore the whole typed relationship.
#[test]
fn hierarchical_clipboard_remaps_terminal_intent_with_its_owner() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let root = s.project().unwrap().sheet_target();
    s.apply(crate::hierarchy::CREATE_CHILD, &json!({"sheet":root.id(),"name":"Child","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let child = s.project().unwrap().sheets().find(|sheet| sheet.name == "Child").unwrap().target;
    s.apply(PLACE_SYMBOL, &json!({"sheet":child.id(),"definition":"header-4","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let p = s.project().unwrap().for_sheet(child).unwrap();
    let target = p.no_connect_target(p.symbols()[0].terminals()[0]).unwrap();
    mark(&mut s, &target, true);
    let block = s.project().unwrap().subsheets[0].id;
    let copied=s.execute(crate::clipboard::COPY,&json!({"sheet":root.id(),"items":crate::selection::Selection::default(),"objects":[format!("sheet-symbol-{block}")],"expected_revision":s.revision()})).unwrap();
    s.apply(
        crate::clipboard::PASTE,
        &json!({"sheet":root.id(),"text":copied["clipboard"],"x_nm":25_400_000,"y_nm":25_400_000,"expected_revision":s.revision()}),
    )
    .unwrap();
    let p = s.snapshot().unwrap();
    let report = p.check_no_connects(s.revision());
    assert_eq!(report.total_markers, 2);
    let queried = s.execute(CHECK, &json!({})).unwrap();
    assert_eq!(queried["no_connect_check"]["total_markers"], 2);
    let copy = report.markers.iter().find(|marker| marker.target.part != target.part).unwrap();
    assert_eq!(copy.target.terminal, target.terminal);
    assert_ne!(copy.appearances[0].sheet, child);
    assert!(copy.issues.is_empty());
    let docs = persistence::encode_documents(p).unwrap();
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&docs).unwrap()).unwrap(), docs);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().unwrap().check_no_connects(s.revision()).total_markers, 1);
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(s.project().unwrap().check_no_connects(s.revision()).total_markers, 2);
}

/// A copied subtree owns all intent for a multi-unit part spread across descendant sheets,
/// including unresolved records, regardless of which unit the remapper encounters first.
#[test]
fn hierarchical_clipboard_preserves_no_connects_from_every_unit() {
    for unresolved in [false, true] {
        let mut s = Session::default();
        s.apply("project.new", &json!({})).unwrap();
        let root = s.project().unwrap().sheet_target();
        s.apply(crate::hierarchy::CREATE_CHILD, &json!({"sheet":root.id(),"name":"Child","x_nm":0,"y_nm":0,"expected_revision":s.revision()}))
            .unwrap();
        let child = s.project().unwrap().sheets().find(|sheet| sheet.name == "Child").unwrap().target;
        s.apply(crate::hierarchy::CREATE_CHILD, &json!({"sheet":child.id(),"name":"Grandchild","x_nm":0,"y_nm":0,"expected_revision":s.revision()}))
            .unwrap();
        let grandchild = s.project().unwrap().sheets().find(|sheet| sheet.name == "Grandchild").unwrap().target;
        s.apply(
            crate::library::sources::LOAD,
            &json!({"source":"units","documents":{
                "library.json":include_str!("../../../../examples/native-parts/library.json"),
                "symbols/dual.json":include_str!("../../../../examples/native-parts/symbols/dual-amplifier.json"),
            }}),
        )
        .unwrap();
        let definition = s.libraries().latest_symbol("parts-demo", "dual-amplifier").unwrap();
        s.apply(PLACE_SYMBOL, &json!({"sheet":child.id(),"library":"parts-demo","definition":"dual-amplifier","definition_revision":1,
            "definition_digest":crate::library::sources::symbol_digest(&definition).unwrap(),"unit":"a","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
        let original = s.project().unwrap().for_sheet(child).unwrap().symbols()[0].clone();
        s.apply(
            crate::PLACE_UNIT,
            &json!({"sheet":grandchild.id(),"component":original.component_id(),"unit":"b","x_nm":0,"y_nm":0,"expected_revision":s.revision()}),
        )
        .unwrap();
        for terminal in ["1", "3", "5"] {
            mark(&mut s, &Target { part: original.component_id(), terminal: terminal.into() }, true);
        }
        if unresolved {
            let mut p = (*s.snapshot().unwrap()).clone();
            Arc::make_mut(&mut p.components).get_mut(&original.component).unwrap().no_connects.insert("OLD".into(), ElectricalType::Input);
            s.open_snapshot(Arc::new(p));
        }
        let before = s.snapshot().unwrap();
        let expected = before.component(&original).unwrap().no_connects.clone();
        let block = before.subsheets[0].id;
        let copied = s
            .execute(
                crate::clipboard::COPY,
                &json!({"sheet":root.id(),"items":crate::selection::Selection::default(),
            "objects":[format!("sheet-symbol-{block}")],"expected_revision":s.revision()}),
            )
            .unwrap();
        s.apply(
            crate::clipboard::PASTE,
            &json!({"sheet":root.id(),"text":copied["clipboard"],"x_nm":25_400_000,"y_nm":25_400_000,"expected_revision":s.revision()}),
        )
        .unwrap();
        let after = s.snapshot().unwrap();
        let copy = after.components.values().find(|part| part.ordinal != original.component).unwrap();
        assert_eq!(copy.no_connects, expected);
        assert_eq!(after.check_no_connects(s.revision()).total_markers, expected.len() * 2);
        let docs = persistence::encode_documents(after.clone()).unwrap();
        assert_eq!(persistence::encode_documents(persistence::decode_documents(&docs).unwrap()).unwrap(), docs);
        let packed = persistence::encode_packed(after.clone()).unwrap();
        assert_eq!(persistence::encode_documents(persistence::decode_packed(&packed).unwrap().project).unwrap(), docs);
        s.apply("edit.undo", &json!({})).unwrap();
        crate::assert_same_content(&before, &s.snapshot().unwrap());
        s.apply("edit.redo", &json!({})).unwrap();
        crate::assert_same_content(&after, &s.snapshot().unwrap());
    }
}
