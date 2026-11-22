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

/// The decoder's refusal of a bus label under an older format or of format 16 without one.
const FORMAT_REFUSAL: &str = "bus labels require sheet format 16, and sheet format 16 requires a bus label";

/// Only a sheet holding a bus label is written as sheet format 16; it round-trips exactly.
/// Format 16 without a bus, and a bus under an older format, are both refused.
#[test]
fn bus_sheets_use_format_16() {
    let (mut s, main, _) = bus_fixture();
    apply(&mut s, sheets::CREATE_SHEET, json!({"name":"Plain"}));
    let plain = s.project().unwrap().sheets().nth(1).unwrap().target;
    let documents = persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let path = format!("sheets/{}.json", main.id());
    let plain_path = format!("sheets/{}.json", plain.id());
    let sheet: Value = serde_json::from_slice(&documents[&path]).unwrap();
    assert_eq!(sheet["format"], "pcbcraft-sheet/16");
    let other: Value = serde_json::from_slice(&documents[&plain_path]).unwrap();
    assert_eq!(other["format"], "pcbcraft-sheet/9");
    let reopened = persistence::decode_documents(&documents).unwrap();
    assert_eq!(persistence::encode_documents(reopened).unwrap(), documents);

    let mut downgraded = documents.clone();
    let mut older = sheet.clone();
    older["format"] = json!("pcbcraft-sheet/9");
    downgraded.insert(path.clone(), serde_json::to_vec(&older).unwrap());
    assert!(persistence::decode_documents(&downgraded).unwrap_err().contains(FORMAT_REFUSAL));

    let mut upgraded = documents.clone();
    let mut newer = other.clone();
    newer["format"] = json!("pcbcraft-sheet/16");
    upgraded.insert(plain_path, serde_json::to_vec(&newer).unwrap());
    assert!(persistence::decode_documents(&upgraded).unwrap_err().contains(FORMAT_REFUSAL));

    let mut paperless = documents.clone();
    let mut bare = sheet.clone();
    bare.as_object_mut().unwrap().remove("page");
    paperless.insert(path.clone(), serde_json::to_vec(&bare).unwrap());
    assert!(persistence::decode_documents(&paperless).is_ok(), "paper stays optional in format 16");

    let mut malformed = documents;
    let mut broken = sheet;
    let labels = broken["net_labels"].as_array_mut().unwrap();
    let bus = labels.iter_mut().find(|label| label["style"] == "bus").unwrap();
    bus["name"] = json!("D[0..64]");
    malformed.insert(path, serde_json::to_vec(&broken).unwrap());
    assert!(persistence::decode_documents(&malformed).unwrap_err().contains("a bus carries at most 64 members"));
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
    assert!(String::from_utf8(saved[&path].clone()).unwrap().contains("pcbcraft-sheet/9"), "removing the last bus returns to format 9");
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
