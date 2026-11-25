//! Vector bus labels: member expansion, netlist results, refusals, storage and previews.
use super::*;
use crate::net_labels::bus;

/// Eight resistors with local labels D0..D7 on their first pins, plus a bus label D[0..7] on
/// free space below them. Returns the session, its sheet and the bus label's object ID.
fn bus_fixture() -> (Session, SheetTarget, String) {
    let mut s = session();
    let main = s.project().unwrap().sheet_target();
    for i in 0..8 {
        let pin = resistor(&mut s, main, i * 20 * G);
        label(&mut s, main, pin, &format!("D{i}"), "local");
    }
    apply(&mut s, ADD, json!({"sheet":main.id(),"name":"D[0..7]","style":"bus","x_nm":0,"y_nm":-20*G}));
    let bus = s.project().unwrap().for_sheet(main).unwrap().net_labels().last().unwrap().object_id();
    (s, main, bus)
}

/// Net names of the exported netlist, in netlist order.
fn net_names(s: &Session) -> Vec<String> {
    s.project().unwrap().netlist().unwrap().netlist.nets.into_iter().map(|net| net.name).collect()
}

/// The inspected members of one bus label as (member, net) pairs.
fn members_of(s: &Session, id: &str) -> Vec<(String, Option<String>)> {
    let inspected = s.project().unwrap().inspect_schematic();
    let label = inspected["net_labels"].as_array().unwrap().iter().find(|label| label["id"] == id).unwrap().clone();
    label["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|member| (member["name"].as_str().unwrap().to_owned(), member["net"].as_str().map(str::to_owned)))
        .collect()
}

/// A bus D[0..7] over eight labelled pins yields exactly the nets D0..D7, and adds no net of
/// its own. Shrinking it to D[0..3] shrinks its members while D4..D7 keep their nets. Each
/// edit is one undo step.
#[test]
fn bus_members_resolve_to_their_named_nets() {
    let (mut s, main, bus) = bus_fixture();
    let expected: Vec<String> = (0..8).map(|i| format!("D{i}")).collect();
    let mut names = net_names(&s);
    names.sort();
    assert_eq!(names, expected);
    assert!(s.project().unwrap().netlist().unwrap().netlist.nets.iter().all(|net| !net.names.iter().any(|name| name.contains('['))));
    let members = members_of(&s, &bus);
    assert_eq!(members, expected.iter().map(|name| (name.clone(), Some(name.clone()))).collect::<Vec<_>>());

    apply(&mut s, SET, json!({"sheet":main.id(),"id":bus,"name":"D[0..3]","style":"bus","x_nm":0,"y_nm":-20*G}));
    assert_eq!(members_of(&s, &bus).iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["D0", "D1", "D2", "D3"]);
    let mut names = net_names(&s);
    names.sort();
    assert_eq!(names, expected, "D4..D7 still name their own nets");

    s.apply("edit.undo", &Value::Null).unwrap();
    assert_eq!(s.project().unwrap().for_sheet(main).unwrap().net_labels().last().unwrap().name, "D[0..7]");
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(s.project().unwrap().for_sheet(main).unwrap().net_labels().iter().all(|label| label.style != Style::Bus));
}

/// A member with no ordinary label on the sheet has no net, and a descending range lists its
/// members in written order.
#[test]
fn unresolved_and_descending_members() {
    let (mut s, main, bus) = bus_fixture();
    apply(&mut s, SET, json!({"sheet":main.id(),"id":bus,"name":"D[9..6]","style":"bus","x_nm":0,"y_nm":-20*G}));
    assert_eq!(
        members_of(&s, &bus),
        [("D9", None), ("D8", None), ("D7", Some("D7")), ("D6", Some("D6"))].map(|(name, net)| (name.to_owned(), net.map(str::to_owned))).to_vec()
    );
}

/// The bracket spelling has meaning only for the bus style: a local label literally named
/// D[0..7] names one net with that name and joins no member.
#[test]
fn literal_bracket_name_stays_one_net() {
    let mut s = session();
    let main = s.project().unwrap().sheet_target();
    let a = resistor(&mut s, main, 0);
    let b = resistor(&mut s, main, 20 * G);
    let c = resistor(&mut s, main, 40 * G);
    label(&mut s, main, a, "D[0..7]", "local");
    label(&mut s, main, b, "D[0..7]", "local");
    label(&mut s, main, c, "D0", "local");
    let mut names = net_names(&s);
    names.sort();
    assert_eq!(names, ["D0", "D[0..7]"]);
    let graph = s.project().unwrap().electrical_connectivity();
    assert_eq!(graph.net_for(a).unwrap(), &[a, b]);
    assert_eq!(graph.net_for(c).unwrap(), &[c]);
    let inspected = s.project().unwrap().inspect_schematic();
    assert!(inspected["net_labels"].as_array().unwrap().iter().all(|label| label.get("members").is_none()));
}

/// Each grammar limit accepts its last good value and refuses the first bad one.
#[test]
fn bus_grammar_limits() {
    assert_eq!(bus::members("D[0..7]").unwrap(), (0..8).map(|i| format!("D{i}")).collect::<Vec<_>>());
    assert_eq!(bus::members("ADDR_[3..1]").unwrap(), ["ADDR_3", "ADDR_2", "ADDR_1"]);
    assert_eq!(bus::members("D[0..63]").unwrap().len(), bus::MAX_WIDTH);
    assert_eq!(bus::members("D[0..64]").unwrap_err(), "a bus carries at most 64 members");
    assert_eq!(bus::members("D[9998..9999]").unwrap(), ["D9998", "D9999"]);
    assert!(bus::members("D[9999..10000]").unwrap_err().contains("whole number from 0 to 9999"));
    assert!(bus::members("D[0..1]").is_ok());
    assert_eq!(bus::members("D[1..1]").unwrap_err(), "a bus needs at least two members");
    for bad in [
        "D[00..7]", "D[0..07]", "D[-1..7]", "D[+1..7]", "D[0...7]", "D[0..7]x", "D[0..7", "D0..7]", "[0..7]", "A1[0..3]", "D [0..3]", "D[0..3]]",
        "D[[0..3]", "D[]", "D",
    ] {
        assert!(bus::members(bad).unwrap_err().starts_with("a bus "), "{bad}");
    }
}

/// The public command refuses an ill-formed bus name atomically, and the style catalog
/// offers the bus style.
#[test]
fn malformed_bus_names_are_refused_atomically() {
    let mut s = session();
    let before = s.snapshot().unwrap();
    for (name, reason) in [
        ("D[0..64]", "at most 64 members"),
        ("D[01..3]", "without leading zeros"),
        ("A1[0..3]", "cannot end in a digit"),
        ("D", "PREFIX[FIRST..LAST]"),
    ] {
        let params = json!({"name":name,"style":"bus","x_nm":0,"y_nm":0,"expected_revision":s.revision()});
        let error = s.apply(ADD, &params).unwrap_err().to_string();
        assert!(error.contains(reason), "{name}: {error}");
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    let catalog = crate::command(ADD).unwrap().to_json();
    assert_eq!(catalog["params"]["properties"]["style"]["enum"], json!(["local", "global", "bus"]));
    let paste = crate::command(crate::clipboard_text::LABELS).unwrap().to_json();
    assert_eq!(paste["params"]["properties"]["style"]["enum"], json!(["local", "global"]));
}

/// A bus label is no wire endpoint: drawing a wire to or from it, attaching a junction to it,
/// or turning a wired local label into a bus are all refused atomically.
#[test]
fn bus_labels_cannot_end_wires() {
    let (mut s, main, bus) = bus_fixture();
    let before = s.snapshot().unwrap();
    for params in [
        json!({"sheet":main.id(),"from":{"net_label":bus},"to":{"x_nm":10*G,"y_nm":-20*G},"bends":[],"expected_revision":s.revision()}),
        json!({"sheet":main.id(),"from":{"x_nm":10*G,"y_nm":-20*G},"to":{"net_label":bus},"bends":[],"expected_revision":s.revision()}),
    ] {
        assert!(s.apply(DRAW_WIRE, &params).unwrap_err().to_string().contains("a bus label cannot end a wire"));
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    }
    let inspected = s.project().unwrap().inspect_schematic();
    let label = inspected["net_labels"].as_array().unwrap().iter().find(|label| label["id"] == bus).unwrap().clone();
    assert!(label.get("endpoint").is_none());

    apply(&mut s, ADD, json!({"sheet":main.id(),"name":"X","style":"local","x_nm":0,"y_nm":-40*G}));
    let wired = s.project().unwrap().for_sheet(main).unwrap().net_labels().last().unwrap().object_id();
    apply(&mut s, DRAW_WIRE, json!({"sheet":main.id(),"from":{"net_label":wired},"to":{"x_nm":10*G,"y_nm":-40*G},"bends":[]}));
    let free_end = s.project().unwrap().for_sheet(main).unwrap().wires().last().unwrap().endpoints()[1].inspect()["junction"].clone();
    let before = s.snapshot().unwrap();
    let attach = json!({"sheet":main.id(),"junction":free_end,"to":{"net_label":bus},"expected_revision":s.revision()});
    assert!(s.apply(ATTACH_JUNCTION, &attach).unwrap_err().to_string().contains("a bus label cannot end a wire"));
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    let params = json!({"sheet":main.id(),"id":wired,"name":"X[0..1]","style":"bus","x_nm":0,"y_nm":-40*G,"expected_revision":s.revision()});
    assert!(s.apply(SET, &params).unwrap_err().to_string().contains("a bus label cannot end a wire"));
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
}

/// An ordinary marker joins two routes at their crossing without being an endpoint.
/// Refuse replacing that contact with a bus, even when the edit also moves the marker.
#[test]
fn label_at_wire_crossing_cannot_become_bus() {
    let mut s = session();
    for (from, bend, to) in [((0, 8 * G), (0, 10 * G), (20 * G, 10 * G)), ((10 * G, 5 * G), (10 * G, 15 * G), (12 * G, 15 * G))] {
        apply(
            &mut s,
            DRAW_WIRE,
            json!({"from":{"x_nm":from.0,"y_nm":from.1},"to":{"x_nm":to.0,"y_nm":to.1},"bends":[{"x_nm":bend.0,"y_nm":bend.1}]}),
        );
    }
    let crossing = Point { x_nm: 10 * G, y_nm: 10 * G };
    let routes = s.snapshot().unwrap();
    assert_eq!(routes.wires().len(), 2);
    assert!(routes.wires().iter().all(|w| matches!(wire::locate(&routes.wire_path(w), crossing), Ok(wire::OnPath::Leg(_)))));
    let end_a = routes.wires()[0].endpoints()[0];
    let end_b = routes.wires()[1].endpoints()[0];
    let net_pair = |p: &Project| {
        let graph = p.electrical_connectivity();
        (graph.net_index_for_endpoint(end_a).unwrap(), graph.net_index_for_endpoint(end_b).unwrap())
    };
    let (a, b) = net_pair(&routes);
    assert_ne!(a, b, "a geometric crossing alone does not join routes");

    apply(&mut s, ADD, json!({"name":"JOIN","style":"local","x_nm":crossing.x_nm,"y_nm":crossing.y_nm}));
    let before = s.snapshot().unwrap();
    let label = &before.net_labels()[0];
    assert_eq!(before.wires(), routes.wires(), "no wire names the newly placed label as an endpoint");
    let (joined, other) = net_pair(&before);
    assert_eq!(joined, other);
    assert_eq!(before.electrical_connectivity().net_name(joined), "JOIN");
    let revision = s.revision();
    let unchanged = |s: &Session| {
        let after = s.snapshot().unwrap();
        assert_eq!(s.revision(), revision);
        assert!(Arc::ptr_eq(&before, &after));
        assert_eq!(net_pair(&after), (joined, joined));
    };
    let refusal = |error| matches!(error, CommandError::InvalidParams(reason) if reason == "a label touching a wire cannot become a bus label");
    for at in [crossing, Point { x_nm: 30 * G, y_nm: 30 * G }] {
        let mut params = json!({
            "id":label.object_id(),"name":"D[0..7]","style":"bus","x_nm":at.x_nm,"y_nm":at.y_nm,
            "keep_connected":true,"expected_revision":revision
        });
        assert!(s.apply(SET, &params).is_err_and(refusal), "converting to a bus would split JOIN");
        unchanged(&s);
        params["dry_run"] = json!(true);
        assert!(s.execute(SET, &params).is_err_and(refusal));
        unchanged(&s);
        assert!(s.apply(SET, &params).is_err_and(refusal));
        unchanged(&s);
    }

    s.apply("edit.undo", &Value::Null).unwrap();
    let undone = s.snapshot().unwrap();
    assert!(undone.net_labels().is_empty(), "Undo removes the placement, not a refused edit");
    assert_eq!(undone.wires(), routes.wires());
    let (a, b) = net_pair(&undone);
    assert_ne!(a, b);
}

/// Cover the other side of the guard: a bus can overlap wires without making contact,
/// including when a contact-free ordinary label becomes a bus and moves onto a wire.
#[test]
fn bus_labels_can_be_placed_and_moved_over_wires() {
    let mut s = session();
    apply(&mut s, DRAW_WIRE, json!({"from":{"x_nm":0,"y_nm":0},"to":{"x_nm":20*G,"y_nm":0},"bends":[]}));
    let before = s.snapshot().unwrap();
    apply(&mut s, ADD, json!({"name":"D[0..7]","style":"bus","x_nm":5*G,"y_nm":0}));
    let bus = s.project().unwrap().net_labels()[0].object_id();
    apply(&mut s, SET, json!({"id":bus,"name":"D[0..7]","style":"bus","x_nm":10*G,"y_nm":0,"keep_connected":true}));
    apply(&mut s, ADD, json!({"name":"X","style":"local","x_nm":15*G,"y_nm":5*G}));
    let free = s.project().unwrap().net_labels()[1].object_id();
    apply(&mut s, SET, json!({"id":free,"name":"X[0..1]","style":"bus","x_nm":15*G,"y_nm":0,"keep_connected":true}));
    let p = s.project().unwrap();
    assert_eq!(p.wires(), before.wires());
    assert_eq!((p.net_labels()[0].style, p.net_labels()[0].at), (Style::Bus, Point { x_nm: 10 * G, y_nm: 0 }));
    assert_eq!((p.net_labels()[1].style, p.net_labels()[1].at), (Style::Bus, Point { x_nm: 15 * G, y_nm: 0 }));
}

/// A bus label is an optional record in a sheet's net-label array, never a format of its own.
/// Current sheets round-trip unchanged, and reading the oldest sheet format with net labels (/3)
/// preserves the bus when rewriting it in the current format. Removing the bus restores the
/// plain current-format sheet bytes.
#[test]
fn bus_labels_round_trip_in_the_lowest_and_highest_sheet_formats() {
    let mut s = session();
    let main = s.project().unwrap().sheet_target();
    let path = format!("sheets/{}.json", main.id());
    let plain = persistence::encode_documents(s.snapshot().unwrap()).unwrap()[&path].clone();
    apply(&mut s, ADD, json!({"sheet":main.id(),"name":"D[0..7]","style":"bus","x_nm":0,"y_nm":-20*G}));
    let bus = s.project().unwrap().for_sheet(main).unwrap().net_labels().last().unwrap().object_id();
    let documents = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let highest: Value = serde_json::from_slice(&documents[&path]).unwrap();
    let before: Value = serde_json::from_slice(&plain).unwrap();
    assert_eq!(highest["format"], persistence::CURRENT_SHEET_FORMAT);
    assert_eq!(highest["format"], before["format"], "a bus label never selects the sheet format");
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&documents).unwrap()).unwrap(), documents);

    let mut legacy = highest.clone();
    legacy["format"] = json!("pcbcraft-sheet/3");
    legacy.as_object_mut().unwrap().remove("page");
    let mut older = documents.clone();
    older.insert(path.clone(), serde_json::to_vec(&legacy).unwrap());
    let rewritten = persistence::encode_documents(persistence::decode_documents(&older).unwrap()).unwrap();
    let written: Value = serde_json::from_slice(&rewritten[&path]).unwrap();
    assert_eq!(written["format"], persistence::CURRENT_SHEET_FORMAT);
    assert_eq!(written["net_labels"], legacy["net_labels"], "the bus label survives the rewrite from sheet/3");
    assert!(written["net_labels"].as_array().unwrap().iter().any(|label| label["style"] == "bus" && label["name"] == "D[0..7]"));
    assert_eq!(persistence::encode_documents(persistence::decode_documents(&rewritten).unwrap()).unwrap(), rewritten);

    let mut malformed = documents;
    let mut broken = highest;
    let labels = broken["net_labels"].as_array_mut().unwrap();
    let record = labels.iter_mut().find(|label| label["style"] == "bus").unwrap();
    record["name"] = json!("D[0..64]");
    malformed.insert(path.clone(), serde_json::to_vec(&broken).unwrap());
    assert!(persistence::decode_documents(&malformed).unwrap_err().contains("a bus carries at most 64 members"));

    apply(&mut s, DELETE, json!({"sheet":main.id(),"id":bus}));
    assert_eq!(persistence::encode_documents(s.snapshot().unwrap()).unwrap()[&path], plain, "removing the bus label gives the plain sheet bytes");
}

/// A dry run of place, edit or delete returns the changed documents and edits nothing.
#[test]
fn label_dry_runs_preview_without_editing() {
    let (mut s, main, bus) = bus_fixture();
    let path = format!("sheets/{}.json", main.id());
    let calls = [
        (ADD, json!({"sheet":main.id(),"name":"A[0..1]","style":"bus","x_nm":0,"y_nm":-40*G})),
        (SET, json!({"sheet":main.id(),"id":bus,"name":"D[0..3]","style":"bus","x_nm":0,"y_nm":-20*G})),
        (DELETE, json!({"sheet":main.id(),"id":bus})),
    ];
    for (command, mut params) in calls {
        let before = s.snapshot().unwrap();
        let revision = s.revision();
        params["expected_revision"] = json!(revision);
        params["dry_run"] = json!(true);
        let preview = s.execute(command, &params).unwrap();
        assert_eq!(preview["revision"], revision);
        assert_eq!(preview["provisional"], true);
        assert!(preview["changes"].as_array().unwrap().iter().any(|change| change["path"] == path), "{command}");
        assert_eq!(s.apply(command, &params).unwrap(), revision);
        assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
        assert_eq!(s.revision(), revision);
    }
    let after: Value = {
        let mut params = json!({"sheet":main.id(),"id":bus,"dry_run":true,"expected_revision":s.revision()});
        let preview = s.execute(DELETE, &params).unwrap();
        params["dry_run"] = json!(false);
        s.apply(DELETE, &params).unwrap();
        preview["changes"].as_array().unwrap().iter().find(|change| change["path"] == path).unwrap()["after"].clone()
    };
    let saved = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(after, std::str::from_utf8(&saved[&path]).unwrap());
    assert!(
        String::from_utf8(saved[&path].clone()).unwrap().contains(persistence::CURRENT_SHEET_FORMAT),
        "removing the bus keeps the current sheet format"
    );
}

/// Dragging a bus label previews its moved and rotated pose without failing on missing wire
/// contact or carrying wires that pass through its anchor.
#[test]
fn dragging_a_bus_label_previews_its_pose_without_carrying_wires() {
    let (mut s, main, _) = bus_fixture();
    apply(&mut s, DRAW_WIRE, json!({"sheet":main.id(),"from":{"x_nm":-10*G,"y_nm":-20*G},"to":{"x_nm":10*G,"y_nm":-20*G},"bends":[]}));
    let before = s.project().unwrap().for_sheet(main).unwrap();
    let bus_id = before.net_labels().iter().find(|label| label.style == Style::Bus).unwrap().id;
    let to = Point { x_nm: 4 * G, y_nm: -24 * G };
    let preview = before.dragged_net_label(bus_id, to, 90, s.revision(), true).unwrap();
    let moved = preview.net_labels().iter().find(|label| label.id == bus_id).unwrap();
    assert_eq!((moved.at, moved.rotation), (to, 90));
    assert_eq!(preview.wires(), before.wires(), "a bus label carries no wires");
    assert!(before.dragged_net_label(bus_id, to, 45, s.revision(), true).is_err());
}
