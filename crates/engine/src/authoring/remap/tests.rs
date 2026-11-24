use super::*;
use crate::{CONNECT_PINS, CommandError, PLACE_SYMBOL, library::sources, persistence};
use std::sync::Arc;

/// Exercise the offline report path unless a test explicitly drives catalog authority.
fn preview(item: &Item, pins: &[PinRemap], revision: Option<u32>, project: Option<&Project>) -> Result<Value, String> {
    super::preview(item, pins, revision, project, None)
}

/// Drive project edits through catalog validation and runtime revision authority.
fn edit(project: &mut crate::Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(project.revision());
    project.execute(command, &params).unwrap();
}

/// Publish an original two-pin symbol, place it twice, wire like terminals, and name both nets.
fn fixture() -> (crate::Session, Session) {
    let original = Item::new(Kind::Symbol).unwrap();
    let manifest = super::super::publishing::new_library("remap-demo", "Remap demo", &original.license).unwrap();
    let prepared = super::super::publishing::prepare(Arc::new(original), &manifest, 0, None, false).unwrap();
    let mut documents = prepared.documents().clone();
    documents.insert("library.json".into(), manifest);
    let item = prepared.item();
    let mut project = crate::Session::default();
    project.execute("project.new", &json!({})).unwrap();
    project.execute(sources::LOAD, &json!({"source":"remap-demo","documents":documents})).unwrap();
    let loaded = project.libraries().symbol(&item.library, &item.id, item.revision).unwrap();
    let digest = sources::symbol_digest(&loaded).unwrap();
    for y in [0, 40_640_000] {
        edit(
            &mut project,
            PLACE_SYMBOL,
            json!({"library":item.library,"definition":item.id,"definition_revision":item.revision,
            "definition_digest":digest,"x_nm":0,"y_nm":y}),
        );
    }
    let symbols = project.project().unwrap().symbols().to_vec();
    for (index, name) in ["INPUT", "OUTPUT"].into_iter().enumerate() {
        edit(&mut project, CONNECT_PINS, json!({"from":symbols[0].terminals()[index].inspect(),"to":symbols[1].terminals()[index].inspect()}));
        let at = symbols[0].pins()[index];
        edit(&mut project, crate::net_labels::ADD, json!({"name":name,"style":"global","x_nm":at.x_nm,"y_nm":at.y_nm}));
    }
    let graph = project.project().unwrap().electrical_connectivity();
    assert_ne!(graph.net_index_for(symbols[0].terminals()[0]), graph.net_index_for(symbols[0].terminals()[1]), "fixture nets must remain separate");
    let mut editor = Session::new(item.as_ref().clone(), true).unwrap();
    editor.bind_published(Some(loaded));
    (project, editor)
}

/// Resolve fixture primitives by explicit terminal metadata, not position or display name.
fn swap(item: &Item) -> Vec<PinRemap> {
    item.objects
        .iter()
        .filter_map(|(object, primitive)| {
            let Primitive::Pin { number, .. } = primitive else {
                return None;
            };
            Some(PinRemap { unit: item.unit.clone(), object: *object, number: if number == "1" { "2" } else { "1" }.into() })
        })
        .collect()
}

/// Swapping two pins previews both current placed nets, changes only the library in one step,
/// and leaves production project bytes identical through apply, undo, redo and offline reopen.
#[test]
fn pin_remap_previews_current_nets_without_adopting_placed_copies() {
    let (mut project, mut editor) = fixture();
    let before = project.snapshot().unwrap();
    let bytes = persistence::encode_documents(before.clone()).unwrap();
    let inspect = project.inspect();
    let original = editor.snapshot();
    let pins = swap(&original);
    let report = project.execute(sources::PREVIEW_PIN_REMAP, &json!({"item":original.as_ref(),"pins":pins,"base_revision":"1"})).unwrap();
    assert_eq!(report["instances"].as_array().unwrap().len(), 2);
    assert_eq!(report["source_revision"], "1");
    assert_eq!(report["changes"][0]["object"], pins[0].object.to_string());
    for instance in report["instances"].as_array().unwrap() {
        assert_eq!(instance["pinned_revision"], "1");
        assert_eq!(instance["pins"][0]["object"], pins[0].object.to_string());
        assert_eq!(instance["pins"][0]["current_nets"], json!(["INPUT"]));
        assert_eq!(instance["pins"][1]["current_nets"], json!(["OUTPUT"]));
        assert_eq!(instance["pins"][0]["before"], "1");
        assert_eq!(instance["pins"][0]["after"], "2");
    }
    assert_eq!(report["saved"], false);
    assert!(!editor.can_undo());
    assert_eq!(project.inspect(), inspect);
    editor.execute(0, Action::RemapPins { pins }).unwrap();
    assert_eq!(json!(editor.snapshot().as_ref()), report["item"]);
    let changed = editor.snapshot();
    assert_eq!(Item::decode(&changed.encode().unwrap()).unwrap(), *changed);
    assert_eq!(changed.symbol().unwrap().pins()[0].number, "2");
    assert_eq!(changed.symbol().unwrap().pins()[0].name, original.symbol().unwrap().pins()[0].name);
    editor.execute(1, Action::Undo).unwrap();
    assert_eq!(editor.snapshot().native().unwrap(), original.native().unwrap());
    assert!(!editor.can_undo());
    editor.execute(2, Action::Redo).unwrap();
    assert_eq!(editor.snapshot(), changed);
    assert!(Arc::ptr_eq(&before, &project.snapshot().unwrap()));
    assert_eq!(persistence::encode_documents(project.snapshot().unwrap()).unwrap(), bytes);
    let reopened = persistence::decode_documents(&bytes).unwrap();
    let offline = preview(&original, &swap(&original), Some(1), Some(&reopened)).unwrap();
    assert_eq!(report["instances"], offline["instances"]);
}

/// A small match on a larger unrelated graph produces repeatable reports without editing
/// project bytes/history. Unrelated parts are graph inputs, not extra affected report rows;
/// this is not a cold-cache timing measurement.
#[test]
fn pin_remap_small_match_in_unrelated_project_uses_shared_graph() {
    let (mut project, editor) = fixture();
    for index in 0..256 {
        edit(
            &mut project,
            crate::PLACE_RESISTOR,
            json!({
                "x_nm":100_000_000 + (index % 16) * 10_160_000,
                "y_nm":100_000_000 + (index / 16) * 10_160_000
            }),
        );
    }
    let current = project.snapshot().unwrap();
    let state = project.inspect();
    let bytes = persistence::encode_documents(current.clone()).unwrap();
    let item = editor.snapshot();
    let request = json!({"item":item.as_ref(),"pins":swap(&item),"base_revision":"1"});
    let first = project.execute(sources::PREVIEW_PIN_REMAP, &request).unwrap();
    let second = project.execute(sources::PREVIEW_PIN_REMAP, &request).unwrap();
    assert_eq!(first, second);
    // Reset only the derived named-graph cache on a snapshot of the command-generated model.
    // Other model caches can remain warm; this is not a whole-process cold-start benchmark.
    let mut uncached = current.as_ref().clone();
    uncached.named_graph = Arc::default();
    budget::check_graph(&uncached).unwrap();
    assert!(uncached.named_graph.get().is_none());
    let cold = preview(&item, &swap(&item), Some(1), Some(&uncached)).unwrap();
    assert!(uncached.named_graph.get().is_some());
    budget::check_graph(&uncached).unwrap();
    let warm = preview(&item, &swap(&item), Some(1), Some(&uncached)).unwrap();
    assert_eq!(cold, warm);
    assert_eq!(first, cold);
    assert_eq!(first["instances"].as_array().unwrap().len(), 2);
    assert_eq!(first["instances"][0]["pins"][0]["current_nets"], json!(["INPUT"]));
    assert_eq!(first["instances"][0]["pins"][1]["current_nets"], json!(["OUTPUT"]));
    assert_eq!(project.inspect(), state);
    assert!(Arc::ptr_eq(&current, &project.snapshot().unwrap()));
    assert_eq!(persistence::encode_documents(project.snapshot().unwrap()).unwrap(), bytes);
}

/// Place unrelated resistors in command-generated clipboard batches at the same coordinates.
/// There are no markers on the ballast sheet yet, so authoring it does not build dense contacts.
fn add_ballast(project: &mut crate::Session, sheet: crate::SheetTarget, target: usize) {
    if project.project().unwrap().for_sheet(sheet).unwrap().symbols().is_empty() {
        edit(project, crate::PLACE_RESISTOR, json!({"sheet":sheet.id(),"x_nm":0,"y_nm":0}));
    }
    loop {
        let sheet_project = project.project().unwrap().for_sheet(sheet).unwrap();
        let count = sheet_project.symbols().len();
        if count == target {
            break;
        }
        let ids: Vec<_> = sheet_project.symbols().iter().take((target - count).min(count)).map(|symbol| symbol.id()).collect();
        let clipboard = project
            .execute(crate::clipboard::COPY, &json!({"sheet":sheet.id(),"expected_revision":project.revision(),"items":{"symbols":ids,"wires":[]}}))
            .unwrap();
        edit(project, crate::clipboard::PASTE, json!({"sheet":sheet.id(),"text":clipboard["clipboard"],"x_nm":0,"y_nm":0}));
        assert_eq!(project.project().unwrap().for_sheet(sheet).unwrap().symbols().len(), count + ids.len());
    }
}

/// Seed adversarial markers in one disposable snapshot rather than warming the named graph
/// repeatedly through hundreds of intermediate topology commits. Earlier fixtures use real edits.
fn graph_marker_fixture(project: &Project, target: crate::SheetTarget, at: crate::schematic::Point) -> Project {
    // Keep a fixed 256-marker ballast so the direct-contact and route fixtures continue to
    // bracket the remap graph preparation ceiling independently of higher per-sheet label limits.
    const BALLAST_MARKERS: usize = 256;
    let mut next = project.clone();
    let sheet = Arc::make_mut(&mut next.sheets).iter_mut().find(|sheet| sheet.sheet_id == target.id()).unwrap();
    sheet.net_labels = Arc::new(
        (0..BALLAST_MARKERS)
            .map(|index| crate::net_labels::Label {
                id: 20_000 + index as u64,
                name: format!("BALLAST-{index}"),
                style: crate::net_labels::Style::Local,
                at,
                rotation: 0,
                text: None,
            })
            .collect(),
    );
    sheet.connectivity = Arc::default();
    next.named_graph = Arc::default();
    next
}

/// The actual command must refuse before warming graph caches and must leave publication, history,
/// revision and encoded project bytes unchanged. Successful cold and warm results must be equal.
fn check_graph_command(project: &mut crate::Session, item: &Item, snapshot: Project, refusal: Option<&str>) {
    // Encoding a numbered project validates its net registry, which may warm the named graph.
    // Take the byte baseline first, then give the command a fresh cache on the same authored state.
    let bytes = persistence::encode_documents(Arc::new(snapshot.clone())).unwrap();
    let mut snapshot = snapshot;
    snapshot.named_graph = Arc::default();
    project.project = Some(Arc::new(snapshot));
    let before = project.snapshot().unwrap();
    let revision = project.revision();
    let (undo, redo) = (project.undo.len(), project.redo.len());
    let request = json!({"item":item,"pins":swap(item),"base_revision":"1"});
    assert!(before.named_graph.get().is_none());
    let first = project.execute(sources::PREVIEW_PIN_REMAP, &request);
    if let Some(reason) = refusal {
        let CommandError::InvalidParams(message) = first.unwrap_err() else { panic!("wrong error kind") };
        assert_eq!(message, reason);
        assert!(before.named_graph.get().is_none(), "refusal must precede cold graph work");
    } else {
        let first = first.unwrap();
        assert_eq!(first["instances"].as_array().unwrap().len(), 2, "unrelated sheet must not add report rows");
        assert!(before.named_graph.get().is_some());
        assert_eq!(first, project.execute(sources::PREVIEW_PIN_REMAP, &request).unwrap());
    }
    assert!(Arc::ptr_eq(&before, &project.snapshot().unwrap()));
    assert_eq!(project.revision(), revision);
    assert_eq!((project.undo.len(), project.redo.len()), (undo, redo));
    assert_eq!(bytes, persistence::encode_documents(project.snapshot().unwrap()).unwrap());
}

/// Cross the modeled contact ceiling with unrelated, coincident pins and no wires on their sheet.
/// Keep both snapshots' associated two-part root report far below the row and encoded byte caps.
#[test]
fn pin_remap_cold_graph_budget_counts_unrelated_direct_contacts() {
    let (mut project, editor) = fixture();
    edit(&mut project, crate::sheets::CREATE_SHEET, json!({"name":"Ballast"}));
    let target = project.project().unwrap().sheets().last().unwrap().target;
    for (count, refusal) in [(1800, None), (1870, Some("Pin remap graph preparation budget exceeded."))] {
        add_ballast(&mut project, target, count);
        let current = project.project().unwrap();
        let sheet = current.for_sheet(target).unwrap();
        assert!(sheet.wires().is_empty());
        let at = sheet.symbols()[0].pins()[0];
        assert!(sheet.symbols().iter().all(|symbol| symbol.pins()[0] == at));
        let dense = graph_marker_fixture(current, target, at);
        let original = project.snapshot();
        check_graph_command(&mut project, &editor.snapshot(), dense, refusal);
        project.project = original;
    }
}

/// Add unrelated diagonal routes expanding to 130 physical segments per wire after resolution.
fn graph_route_fixture(project: &Project, target: crate::SheetTarget, count: i64) -> Project {
    use crate::schematic::{Point, Wire, WireEndpoint};
    let mut snapshot = graph_marker_fixture(project, target, Point { x_nm: 0, y_nm: 0 });
    let sheet = Arc::make_mut(&mut snapshot.sheets).last_mut().unwrap();
    assert_eq!(sheet.sheet_id, target.id());
    for index in 0..count {
        let point = |step: i64| Point { x_nm: (step + 1) * 1_270_000, y_nm: (step + 100 * index + 1) * 1_270_000 };
        let start = 21_000 + index as u64 * 3;
        Arc::make_mut(&mut sheet.junctions).extend([(start, point(0)), (start + 1, point(65))]);
        Arc::make_mut(&mut sheet.wires).push(Wire {
            ordinal: start + 2,
            endpoints: [WireEndpoint::Junction(start), WireEndpoint::Junction(start + 1)],
            bends: Some((1..=64).map(point).collect()),
        });
    }
    snapshot
}

/// Exercise resolved intervals, not just a raw wire count or the arithmetic helper.
#[test]
fn pin_remap_cold_graph_budget_counts_unrelated_resolved_routes() {
    let (mut project, editor) = fixture();
    edit(&mut project, crate::sheets::CREATE_SHEET, json!({"name":"Routes"}));
    let target = project.project().unwrap().sheets().last().unwrap().target;
    let original = project.snapshot().unwrap();
    for (count, refusal) in [(28, None), (30, Some("Pin remap graph preparation budget exceeded."))] {
        let snapshot = graph_route_fixture(&original, target, count);
        let sheet = snapshot.for_sheet(target).unwrap();
        assert!(sheet.wires().iter().all(|wire| sheet.wire_path(wire).len() == 131));
        check_graph_command(&mut project, &editor.snapshot(), snapshot, refusal);
    }
}

/// Long escaped occurrence paths can exceed the independent text budget despite few affected
/// rows and modest graph work. Adding one padded occurrence crosses the modeled text ceiling.
#[test]
fn pin_remap_cold_graph_text_budget_counts_unrelated_occurrence_paths() {
    let (mut project, editor) = fixture();
    let mut target = project.project().unwrap().sheet_target();
    for index in 0..8 {
        edit(
            &mut project,
            crate::hierarchy::CREATE_CHILD,
            json!({"sheet":target.id(),"name":format!("{}{index}", "🦆".repeat(199)),"x_nm":0,"y_nm":0}),
        );
        target = project.project().unwrap().sheets().last().unwrap().target;
        if index == 6 {
            add_ballast(&mut project, target, 1820);
        }
        if index >= 6 {
            let snapshot = project.project().unwrap().clone();
            let refusal = (index == 7).then_some("Pin remap graph text preparation budget exceeded.");
            check_graph_command(&mut project, &editor.snapshot(), snapshot, refusal);
        }
    }
}

/// Invalid correspondence, stale edits and unchanged-number collisions are atomic refusals;
/// a same-number request advances neither allocation counters nor Undo. Preparing its
/// candidate and correspondence maps still allocates temporary memory.
#[test]
fn pin_remap_refusals_preserve_history_and_allocation() {
    let (_, mut editor) = fixture();
    let before = editor.inspect();
    let item = editor.snapshot();
    let history = (editor.undo.clone(), editor.redo.clone());
    let allocation = (editor.next_id, editor.next_content_revision);
    let preserved = |editor: &Session| {
        assert_eq!(editor.inspect(), before);
        assert!(Arc::ptr_eq(&editor.snapshot(), &item));
        assert_eq!((&editor.undo, &editor.redo), (&history.0, &history.1));
        assert_eq!((editor.next_id, editor.next_content_revision), allocation);
    };
    let good = swap(&item);
    let mut cases = vec![(vec![], "Choose 1 to 128")];
    let mut duplicate = good.clone();
    duplicate[1].number = duplicate[0].number.clone();
    cases.push((duplicate, "duplicate destination"));
    let mut unknown = good.clone();
    unknown[0].object = 999;
    cases.push((unknown, "Unknown pin"));
    let mut graphic = good.clone();
    graphic[0].object = 2;
    cases.push((graphic, "Unknown pin"));
    let mut bad = good.clone();
    bad[0].number = "!".into();
    cases.push((bad, "Invalid or duplicate"));
    cases.push((vec![good[0].clone()], "collides with an unchanged"));
    cases.push((vec![good[0].clone(), PinRemap { number: "3".into(), ..good[0].clone() }], "duplicate or invalid pin"));
    let mut unit = good.clone();
    unit[0].unit = "absent".into();
    cases.push((unit, "Unknown pin"));
    for (pins, message) in cases {
        let error = editor.execute(0, Action::RemapPins { pins }).unwrap_err();
        assert!(error.contains(message), "{error}");
        preserved(&editor);
    }
    assert!(editor.execute(1, Action::RemapPins { pins: good.clone() }).unwrap_err().contains("changed"));
    preserved(&editor);
    let noop = vec![PinRemap { unit: item.unit.clone(), object: good[0].object, number: "1".into() }];
    assert_eq!(editor.execute(0, Action::RemapPins { pins: noop }).unwrap(), 0);
    preserved(&editor);
    let mut project = crate::Session::default();
    let valid = json!({"item":item.as_ref(),"pins":good});
    project.execute(sources::PREVIEW_PIN_REMAP, &valid).unwrap();
    let project_before = project.inspect();
    let mut guess = valid.clone();
    guess["pins"][0]["guess"] = json!(true);
    let mut path = valid;
    path["path"] = json!("not-an-input");
    for (params, reason) in
        [(guess, "`pins` Each pin remap row requires only unit, object and number."), (path, "`library.preview_pin_remap` has no parameter `path`")]
    {
        let error = project.execute(sources::PREVIEW_PIN_REMAP, &params).unwrap_err();
        let CommandError::InvalidParams(message) = error else { panic!("wrong error kind: {error}") };
        assert_eq!(message, reason);
        assert_eq!(project.inspect(), project_before);
        preserved(&editor);
    }
}

/// Shared terminal appearances and an explicit power role follow one logical correspondence.
#[test]
fn pin_remap_shared_units_keep_names_geometry_and_power_role() {
    let mut item = Item::new(Kind::Symbol).unwrap();
    item.behavior = Some(Behavior::Power { terminal: "1".into() });
    let shared = item.objects[&1].clone();
    let mut editor = Session::new(item, false).unwrap();
    editor.execute(0, Action::AddUnit { unit: "aux".into() }).unwrap();
    editor.execute(1, Action::Insert { primitive: shared }).unwrap();
    let before = editor.snapshot();
    let pins = vec![PinRemap { unit: "main".into(), object: 1, number: "2".into() }, PinRemap { unit: "main".into(), object: 3, number: "1".into() }];
    let shared_id = *before.objects.keys().next().unwrap();
    let duplicate = vec![pins[0].clone(), PinRemap { unit: "aux".into(), object: shared_id, number: "3".into() }];
    assert!(editor.execute(2, Action::RemapPins { pins: duplicate }).unwrap_err().contains("shared logical"));
    editor.execute(2, Action::RemapPins { pins }).unwrap();
    let after = editor.snapshot();
    let Primitive::Pin { number, name, at, .. } = &after.objects[&shared_id] else {
        panic!("pin");
    };
    assert_eq!(number, "2");
    let Primitive::Pin { name: old_name, at: old_at, .. } = &before.objects[&shared_id] else {
        panic!("pin");
    };
    assert_eq!((name, at), (old_name, old_at));
    assert_eq!(after.behavior, Some(Behavior::Power { terminal: "2".into() }));
    assert_eq!(after.symbol().unwrap().units()[0].pins()[0].number, "2");
    assert_eq!(Item::decode(&after.encode().unwrap()).unwrap(), *after);
}

/// Bounds and spelling agree for typed and transport callers; no coercion or unknown field is
/// accepted. The public model still uses the existing symbol/terminal limits and pin alphabet.
#[test]
fn pin_remap_transport_bounds_and_wrong_kind() {
    let mut pins: Vec<_> = (1..=128).map(|object| PinRemap { unit: "main".into(), object, number: object.to_string() }).collect();
    assert_eq!(decode(&json!(pins)).unwrap(), pins);
    pins.push(PinRemap { unit: "main".into(), object: 129, number: "129".into() });
    assert!(decode(&json!(pins)).unwrap_err().contains("1 to 128"));
    for number in ["", "123456789", " 1", "1 ", "é", "1.0", "1\n"] {
        assert_eq!(decode(&json!([{"unit":"main","object":"1","number":number}])).unwrap_err(), "Invalid destination terminal number.");
    }
    assert!(decode(&json!([{"unit":"main","object":"1","number":"A_+-1234"}])).is_ok());
    assert!(decode(&json!([{"unit":"a".repeat(64),"object":"1","number":"1"}])).is_ok());
    assert_eq!(decode(&json!([{"unit":"a".repeat(65),"object":"1","number":"1"}])).unwrap_err(), "Invalid pin remap unit.");
    let keys = "Each pin remap row requires only unit, object and number.";
    let object = "Expected a canonical decimal string from 1 through 18446744073709551615.";
    for (entry, reason) in [
        (Value::Null, "Each pin remap row must be an object."),
        (json!({}), keys),
        (json!({"unit":null,"object":"1","number":"1"}), "Invalid pin remap unit."),
        (json!({"unit":"main","number":"1"}), keys),
        (json!({"unit":"main","object":null,"number":"1"}), object),
        (json!({"unit":"main","object":"1"}), keys),
        (json!({"unit":"main","object":"1","number":null}), "Invalid destination terminal number."),
        (json!({"unit":"Main","object":"1","number":"1"}), "Invalid pin remap unit."),
        (json!({"unit":"-main","object":"1","number":"1"}), "Invalid pin remap unit."),
        (json!({"unit":"main","object":"0","number":"1"}), object),
        (json!({"unit":"main","object":1.5,"number":"1"}), object),
        (json!({"unit":"main","object":1,"number":"1"}), object),
        (json!({"unit":"main","object":"1","number":1}), "Invalid destination terminal number."),
        (json!({"unit":"main","object":"1","number":"1","guess":true}), keys),
    ] {
        assert_eq!(decode(&json!([entry])).unwrap_err(), reason);
    }
    for id in [1, (1u64 << 53) + 1, i64::MAX as u64 - 1, i64::MAX as u64, u64::MAX] {
        let decoded = decode(&json!([{"unit":"main","object":id.to_string(),"number":"1"}])).unwrap();
        assert_eq!(decoded[0].object, id);
        assert_eq!(json!(decoded)[0]["object"], id.to_string());
    }
    for spelling in [
        "1",
        "1.0",
        "1e0",
        "4294967295.0",
        "9007199254740993.0",
        "18446744073709551615.0",
        "1.0000000000000001",
        "-1",
        "-0",
        "18446744073709551616",
        "1e30",
    ] {
        let value = serde_json::from_str(&format!(r#"[{{"unit":"main","object":{spelling},"number":"1"}}]"#)).unwrap();
        let error = decode(&value).unwrap_err();
        assert!(error.contains("canonical decimal string"), "{spelling}: {error}");
    }
    let mut footprint = Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    let before = footprint.inspect();
    assert!(
        footprint
            .execute(0, Action::RemapPins { pins: vec![PinRemap { unit: "main".into(), object: 1, number: "2".into() }] })
            .unwrap_err()
            .contains("symbol")
    );
    assert_eq!(footprint.inspect(), before);
}

/// Missing pinned terminals stay unknown, while a known but unplaced terminal becomes current
/// when its unit is placed on another sheet. Neither review changes the pinned project.
#[test]
fn pin_remap_distinguishes_missing_unplaced_and_cross_sheet_terminals() {
    let mut editor = Session::new(Item::new(Kind::Symbol).unwrap(), false).unwrap();
    let mut third = editor.snapshot().objects[&1].clone();
    let Primitive::Pin { number, name, .. } = &mut third else { panic!("pin") };
    *number = "3".into();
    *name = Some("THIRD".into());
    editor.execute(0, Action::AddUnit { unit: "aux".into() }).unwrap();
    editor.execute(1, Action::Insert { primitive: third.clone() }).unwrap();
    let manifest = super::super::publishing::new_library("remap-units", "Remap units", "MIT OR Apache-2.0").unwrap();
    let prepared = super::super::publishing::prepare(editor.snapshot(), &manifest, 0, None, false).unwrap();
    let mut documents = prepared.documents().clone();
    documents.insert("library.json".into(), manifest);
    let item = prepared.item();
    let object = *item.objects.keys().next().unwrap();
    let pins = [PinRemap { unit: "aux".into(), object, number: "4".into() }];
    assert_eq!(preview(item, &pins, Some(item.revision), None).unwrap()["instances"], json!([]));
    let mut project = crate::Session::default();
    project.execute("project.new", &json!({})).unwrap();
    project.execute(sources::LOAD, &json!({"source":"remap-units","documents":documents})).unwrap();
    let digest = sources::symbol_digest(&project.libraries().symbol(&item.library, &item.id, item.revision).unwrap()).unwrap();
    edit(
        &mut project,
        PLACE_SYMBOL,
        json!({"library":item.library,"definition":item.id,"definition_revision":item.revision,
        "definition_digest":digest,"unit":"main","x_nm":0,"y_nm":0}),
    );
    let report = preview(item, &pins, Some(item.revision), project.project()).unwrap();
    assert_eq!(report["instances"][0]["pins"][0]["status"], "unplaced_terminal");
    assert_eq!(report["instances"][0]["pins"][0]["current_name"], "THIRD");
    assert_eq!(report["instances"][0]["pins"][0]["current_nets"], json!([]));
    let part = project.project().unwrap().symbols()[0].component_id();
    edit(&mut project, crate::sheets::CREATE_SHEET, json!({"name":"Remote"}));
    let target = project.project().unwrap().sheets().nth(1).unwrap().target;
    let dense = graph_route_fixture(project.project().unwrap(), target, 30);
    let report = preview(item, &pins, Some(item.revision), Some(&dense)).unwrap();
    assert_eq!(report["instances"][0]["pins"][0]["status"], "unplaced_terminal");
    assert!(dense.named_graph.get().is_none());
    let placed_pin = [PinRemap { unit: "main".into(), object: *item.other_units["main"].keys().next().unwrap(), number: "9".into() }];
    assert_eq!(preview(item, &placed_pin, Some(item.revision), Some(&dense)).unwrap_err(), "Pin remap graph preparation budget exceeded.");
    assert!(dense.named_graph.get().is_none());
    edit(&mut project, crate::PLACE_UNIT, json!({"sheet":target.id(),"component":part,"unit":"aux","x_nm":0,"y_nm":0}));
    let at = project.project().unwrap().for_sheet(target).unwrap().symbols()[0].pins()[0];
    edit(&mut project, crate::net_labels::ADD, json!({"sheet":target.id(),"name":"REMOTE","style":"global","x_nm":at.x_nm,"y_nm":at.y_nm}));
    let before = project.snapshot().unwrap();
    let bytes = persistence::encode_documents(before.clone()).unwrap();
    let report = preview(item, &pins, Some(item.revision), Some(&before)).unwrap();
    assert_eq!(report["instances"][0]["pins"][0]["status"], "current");
    assert_eq!(report["instances"][0]["pins"][0]["current_nets"], json!(["REMOTE"]));
    let mut draft = Session::new(item.as_ref().clone(), true).unwrap();
    let Primitive::Pin { number, at, .. } = &mut third else { panic!("pin") };
    *number = "4".into();
    at.y_nm += 1_270_000;
    draft.execute(0, Action::Insert { primitive: third }).unwrap();
    let changed = draft.snapshot();
    let object = *changed.objects.keys().find(|object| !item.objects.contains_key(object)).unwrap();
    let report = preview(&changed, &[PinRemap { unit: "aux".into(), object, number: "5".into() }], Some(item.revision), Some(&before)).unwrap();
    let row = &report["instances"][0]["pins"][0];
    assert_eq!(row["status"], "missing_terminal");
    assert_eq!(row["current_name"], Value::Null);
    assert_eq!(row["current_pads"], Value::Null);
    assert_eq!(row["current_nets"], json!([]));
    assert!(Arc::ptr_eq(&before, &project.snapshot().unwrap()));
    assert_eq!(persistence::encode_documents(project.snapshot().unwrap()).unwrap(), bytes);
}

/// Generate the supported 128-terminal symbol through authoring commands, not project JSON.
fn bounded_fixture() -> (crate::Session, Item, String) {
    let mut editor = Session::new(Item::new(Kind::Symbol).unwrap(), false).unwrap();
    editor.execute(0, Action::Delete { objects: vec![1, 3] }).unwrap();
    for index in 0..128 {
        editor
            .execute(
                editor.revision(),
                Action::Insert {
                    primitive: Primitive::Pin {
                        number: (index + 1).to_string(),
                        name: Some(format!("P{}", index + 1)),
                        electrical_type: crate::schematic::ElectricalType::Passive,
                        alternates: Vec::new(),
                        at: crate::schematic::Point { x_nm: (index % 16 - 8) * 1_270_000, y_nm: (index / 16 - 4) * 1_270_000 },
                        appearance: None,
                    },
                },
            )
            .unwrap();
    }
    let manifest = super::super::publishing::new_library("bounded-remap", "Bounded remap", "MIT OR Apache-2.0").unwrap();
    let prepared = super::super::publishing::prepare(editor.snapshot(), &manifest, 0, None, false).unwrap();
    let mut documents = prepared.documents().clone();
    documents.insert("library.json".into(), manifest);
    let item = prepared.item().as_ref().clone();
    let mut project = crate::Session::default();
    project.execute("project.new", &json!({})).unwrap();
    project.execute(sources::LOAD, &json!({"source":"bounded-remap","documents":documents})).unwrap();
    let loaded = project.libraries().symbol(&item.library, &item.id, item.revision).unwrap();
    let digest = sources::symbol_digest(&loaded).unwrap();
    (project, item, digest)
}

/// Linux runtime evidence only: process RSS includes allocator reuse and concurrent tests, so
/// its delta is approximate and is not a report-heap or peak-allocation measurement.
fn resident_kib() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/self/status")
            .ok()?
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:")?.split_whitespace().next()?.parse().ok())
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Exact-limit preparation retains all 4096 affected rows; limit+1 and arithmetic overflow
/// refuse without changing production bytes or history. Allocation ordering is source-traced,
/// not measured by this test.
#[test]
fn pin_remap_review_budget_bounds_expanded_rows_before_work() {
    assert!(check_budget(32, 128).is_ok());
    assert!(check_budget(241, 17).unwrap_err().contains("4096 affected rows"));
    assert!(check_budget(usize::MAX, 2).unwrap_err().contains("4096 affected rows"));
    let (mut project, item, digest) = bounded_fixture();
    let pins: Vec<_> = item
        .objects
        .iter()
        .filter_map(|(object, primitive)| {
            let Primitive::Pin { number, .. } = primitive else { return None };
            Some(PinRemap { unit: item.unit.clone(), object: *object, number: format!("A{number}") })
        })
        .collect();
    for index in 0..32 {
        edit(
            &mut project,
            PLACE_SYMBOL,
            json!({"library":item.library,"definition":item.id,"definition_revision":item.revision,"definition_digest":digest,
            "x_nm":(index % 16) * 50_800_000,"y_nm":(index / 16) * 50_800_000}),
        );
        if index == 31 {
            let before = project.snapshot().unwrap();
            let rss_before = resident_kib();
            let start = std::time::Instant::now();
            let report = preview(&item, &pins, Some(item.revision), Some(&before)).unwrap();
            let elapsed = start.elapsed();
            let rows: usize = report["instances"].as_array().unwrap().iter().map(|instance| instance["pins"].as_array().unwrap().len()).sum();
            assert_eq!(rows, MAX_PREVIEW_ROWS);
            eprintln!(
                "pin_remap_budget rows={rows} encoded_bytes={} cpu_prepare_us={} process_rss_before_kib={rss_before:?} process_rss_after_kib={:?}",
                serde_json::to_vec(&report).unwrap().len(),
                elapsed.as_micros(),
                resident_kib()
            );
            assert!(Arc::ptr_eq(&before, &project.snapshot().unwrap()));
        }
    }
    // Production clipboard batches preserve the same 241-part boundary without rebuilding
    // and validating the whole growing project once per individual placement.
    while project.project().unwrap().components().count() < 241 {
        let count = project.project().unwrap().components().count();
        let added = (241 - count).min(count);
        let symbols: Vec<_> = project.project().unwrap().symbols().iter().take(added).map(|symbol| symbol.id()).collect();
        let copied =
            project.execute(crate::clipboard::COPY, &json!({"items":{"symbols":symbols,"wires":[]},"expected_revision":project.revision()})).unwrap();
        edit(&mut project, crate::clipboard::PASTE, json!({"text":copied["clipboard"],"x_nm":0,"y_nm":0}));
        assert_eq!(project.project().unwrap().components().count(), count + added);
    }
    let before = project.snapshot().unwrap();
    let bytes = persistence::encode_documents(before.clone()).unwrap();
    assert!(preview(&item, &pins[..17], Some(item.revision), Some(&before)).unwrap_err().contains("4096 affected rows"));
    assert!(Arc::ptr_eq(&before, &project.snapshot().unwrap()));
    assert_eq!(persistence::encode_documents(project.snapshot().unwrap()).unwrap(), bytes);
}

/// Time only Session::execute: matched item/session setup and route-specific action construction
/// are untimed.
/// This compares two routes at one HEAD, not builds or application/UI event latency.
fn timed_transaction(item: &Item, pins: &[PinRemap], remap: bool) -> (u128, Arc<Item>) {
    let mut editor = Session::new(item.clone(), false).unwrap();
    let action = if remap {
        Action::RemapPins { pins: pins.to_vec() }
    } else {
        let primitives = pins
            .iter()
            .map(|pin| {
                let mut primitive = item.objects[&pin.object].clone();
                let Primitive::Pin { number, .. } = &mut primitive else { panic!("pin") };
                *number = pin.number.clone();
                (pin.object, primitive)
            })
            .collect();
        Action::ReplaceMany { primitives }
    };
    let start = std::time::Instant::now();
    editor.execute(0, action).unwrap();
    (start.elapsed().as_nanos(), editor.snapshot())
}

/// Matched native transaction probe: same HEAD/items and equivalent edits, alternating route order, eight
/// warmups and 64 measured pairs. Absolute new preview cost has its own 4096-row probe above.
#[test]
fn pin_remap_transaction_probe_same_head_not_base() {
    for item in [Item::new(Kind::Symbol).unwrap(), bounded_fixture().1] {
        let pins: Vec<_> = item
            .objects
            .iter()
            .filter_map(|(object, primitive)| {
                let Primitive::Pin { number, .. } = primitive else { return None };
                Some(PinRemap { unit: item.unit.clone(), object: *object, number: format!("A{number}") })
            })
            .collect();
        let mut old_samples = Vec::new();
        let mut new_samples = Vec::new();
        for index in 0..72 {
            let (old, new) = if index % 2 == 0 {
                let old = timed_transaction(&item, &pins, false);
                (old, timed_transaction(&item, &pins, true))
            } else {
                let new = timed_transaction(&item, &pins, true);
                (timed_transaction(&item, &pins, false), new)
            };
            assert_eq!(old.1, new.1);
            if index >= 8 {
                old_samples.push(old.0);
                new_samples.push(new.0);
            }
        }
        old_samples.sort_unstable();
        new_samples.sort_unstable();
        eprintln!(
            "pin_remap_transaction same_head=true not_base=true units=1 pins={} measured_pairs=64 warmup_pairs=8 replace_many_median_ns={} remap_pins_median_ns={}",
            pins.len(),
            old_samples[32],
            new_samples[32]
        );
    }
}

/// Original native source records are placed through the shared command. Preview reads the
/// pinned package map, even offline, without rewriting the native part or physical assignment.
#[test]
fn pin_remap_native_part_maps_remain_current_and_offline() {
    let documents: sources::Documents = [
        ("library.json", include_str!("../../../../../examples/native-parts/library.json")),
        ("symbols/resistor.json", include_str!("../../../../../examples/native-parts/symbols/resistor.json")),
        ("footprints/resistor-0603.json", include_str!("../../../../../examples/native-parts/footprints/resistor-0603.json")),
        ("categories/resistor.json", include_str!("../../../../../examples/native-parts/categories/resistor.json")),
        ("parts/resistor.json", include_str!("../../../../../examples/native-parts/parts/resistor.json")),
    ]
    .into_iter()
    .map(|(path, text)| (path.into(), text.into()))
    .collect();
    let mut project = crate::Session::default();
    project.execute("project.new", &json!({})).unwrap();
    project.execute(sources::LOAD, &json!({"source":"native-remap","documents":documents})).unwrap();
    let reference = crate::library::parts::Reference { library: "parts-demo".into(), id: "resistor".into(), revision: 1 };
    let digest = sources::part_digest(project.libraries().resolve_part(&reference).unwrap());
    edit(
        &mut project,
        crate::part_instances::PLACE_PART,
        json!({"library":"parts-demo","definition":"resistor","definition_revision":1,"definition_digest":digest,"package":"0603","x_nm":0,"y_nm":0}),
    );
    let before = project.snapshot().unwrap();
    let item = Item::edit_symbol(&before.components().next().unwrap().definition).unwrap();
    let report = preview(&item, &swap(&item), Some(1), Some(&before)).unwrap();
    assert_eq!(report["instances"][0]["pins"][0]["current_pads"], json!(["1"]));
    assert_eq!(report["instances"][0]["pins"][1]["current_pads"], json!(["2"]));
    assert_eq!(report["instances"][0]["pins"][0]["after"], "2");
    let bytes = persistence::encode_documents(before.clone()).unwrap();
    let reopened = persistence::decode_documents(&bytes).unwrap();
    assert_eq!(preview(&item, &swap(&item), Some(1), Some(&reopened)).unwrap()["instances"], report["instances"]);
    let mut editor = Session::new(item, true).unwrap();
    editor.execute(0, Action::RemapPins { pins: swap(&editor.snapshot()) }).unwrap();
    assert_eq!(persistence::encode_documents(project.snapshot().unwrap()).unwrap(), bytes);
    assert!(Arc::ptr_eq(&before, &project.snapshot().unwrap()));
}

/// A replacement source cannot lend its published address to older placed content.
#[test]
fn pin_remap_refuses_conflicting_published_base_without_comparing_the_draft() {
    let (mut project, mut editor) = fixture();
    let base = editor.snapshot();
    editor.execute(0, Action::RemapPins { pins: swap(&base) }).unwrap();
    let draft = editor.snapshot();
    let request = json!({"item":draft.as_ref(),"pins":swap(&draft),"base_revision":base.revision.to_string()});
    let preview = |project: &mut crate::Session| project.execute(sources::PREVIEW_PIN_REMAP, &request).unwrap();
    let before = preview(&mut project);
    assert_eq!(before["instances"][0]["pins"][0]["status"], "current");
    project.execute(sources::REMOVE, &json!({"source":"remap-demo"})).unwrap();
    assert_eq!(preview(&mut project), before, "offline pinned evidence remains available");
    let mut replacement = base.export_native().unwrap();
    replacement["name"] = json!("Different symbol at the same address");
    let manifest = super::super::publishing::new_library(&base.library, "Remap demo", &base.license).unwrap();
    project
        .execute(
            sources::LOAD,
            &json!({"source":"replacement","documents":{
                "library.json":manifest, "symbols/replacement.json":replacement.to_string()
            }}),
        )
        .unwrap();
    assert!(project.libraries().symbol(&base.library, &base.id, base.revision).is_some());
    let report = preview(&mut project);
    let instances = report["instances"].as_array().unwrap();
    assert_eq!(instances.len(), 2);
    for instance in instances {
        let pins = instance["pins"].as_array().unwrap();
        assert_eq!(pins.len(), 2);
        for pin in pins {
            assert_eq!(pin["status"], "conflicting_definition");
            assert_eq!(pin["current_name"], Value::Null);
            assert_eq!(pin["current_pads"], Value::Null);
            assert_eq!(pin["current_nets"], json!([]));
        }
    }
}

/// A revision mismatch has an explicit status and no fabricated net match.
#[test]
fn pin_remap_never_guesses_a_different_revision() {
    let (project, editor) = fixture();
    let item = editor.snapshot();
    for source in [None, Some(2)] {
        let report = preview(&item, &swap(&item), source, project.project()).unwrap();
        assert_eq!(report["instances"][0]["pins"][0]["status"], "different_revision");
        assert_eq!(report["instances"][0]["pins"][0]["current_nets"], json!([]));
    }
}
