use super::*;
use crate::{
    APPLY_UPDATE, ASSIGN_FOOTPRINT, PLACE_RESISTOR, PREVIEW_UPDATE,
    board::definition::basic_components,
    netlist::{
        EXPORT_BOARD_NETLIST, EXPORT_NETLIST,
        tests::{Fixture, apply, fixture},
    },
};
use serde_json::json;
use std::sync::Arc;

/// The exported text of the fixture's project netlist, read through the execute path.
fn exported(s: &mut Session) -> String {
    s.execute(EXPORT_NETLIST, &json!({})).unwrap()["netlist"]["files"][0]["contents"].as_str().unwrap().to_owned()
}

/// Compare a file with the open design through the shared command path, checking that it edits nothing.
fn run(s: &mut Session, text: &str) -> Value {
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    let result = s.execute(COMPARE_NETLIST, &json!({"netlist":text})).unwrap();
    assert_eq!(result["revision"], json!(revision));
    assert_eq!(s.apply(COMPARE_NETLIST, &json!({"netlist":text})).unwrap(), revision);
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    result["comparison"].clone()
}

/// The index of the net listing a terminal.
fn net_of(document: &Value, part: &str, terminal: &str) -> usize {
    document["nets"]
        .as_array()
        .unwrap()
        .iter()
        .position(|net| net["terminals"].as_array().unwrap().iter().any(|t| t["part"] == part && t["terminal"] == terminal))
        .unwrap()
}

/// Move one terminal from its net to the end of another net's list.
fn move_terminal(document: &mut Value, part: &str, terminal: &str, to: usize) {
    let from = net_of(document, part, terminal);
    let list = document["nets"][from]["terminals"].as_array_mut().unwrap();
    let index = list.iter().position(|t| t["part"] == part && t["terminal"] == terminal).unwrap();
    let moved = list.remove(index);
    document["nets"][to]["terminals"].as_array_mut().unwrap().push(moved);
}

/// Read the ordered kind labels from a serialized list of differences or notes.
fn kinds(list: &Value) -> Vec<&str> {
    list.as_array().unwrap().iter().map(|entry| entry["kind"].as_str().unwrap()).collect()
}

/// Done test: a file read back unchanged matches the design with equal counts, and a copy with
/// one terminal moved to another net fails with one difference naming that terminal, the net
/// it is on in the design and the net the file puts it on.
#[test]
fn edited_copy_with_one_terminal_on_another_net_fails_naming_that_terminal() {
    let Fixture { mut session, u1, .. } = fixture();
    let text = exported(&mut session);
    let same = run(&mut session, &text);
    assert_eq!(same["matches"], true);
    assert_eq!(same["identical"], true);
    assert_eq!(same["differences"], json!([]));
    assert_eq!(same["notes"], json!([]));
    assert_eq!(same["expected"], same["found"]);
    assert_eq!(same["expected"], json!({"components":3,"terminals":10,"nets":4,"connections":8,"unconnected":2}));

    let original: Value = serde_json::from_str(&text).unwrap();
    let (from, to) = (net_of(&original, &u1, "2"), net_of(&original, &u1, "1"));
    assert_ne!(from, to);
    let mut edited = original.clone();
    move_terminal(&mut edited, &u1, "2", to);
    let result = run(&mut session, &edited.to_string());
    assert_eq!(result["matches"], false);
    assert_eq!(result["identical"], false);
    assert_eq!(kinds(&result["differences"]), vec!["terminal_moved"]);
    let moved = &result["differences"][0];
    assert_eq!(moved["part"], json!(u1));
    assert_eq!(moved["terminal"], "2");
    let net = |index: usize| json!({"net":{"id":original["nets"][index]["id"],"index":index}});
    assert_eq!(moved["expected"], net(from));
    assert_eq!(moved["found"], net(to));
    assert_eq!(result["notes"], json!([]));
    assert_eq!(result["found"]["connections"], result["expected"]["connections"], "a move keeps the counts");

    // Without persistent IDs the nets pair by shared terminals and the same terminal is named.
    for net in edited["nets"].as_array_mut().unwrap() {
        net["id"] = Value::Null;
    }
    let result = run(&mut session, &edited.to_string());
    assert_eq!(kinds(&result["differences"]), vec!["terminal_moved"]);
    let moved = &result["differences"][0];
    assert_eq!((&moved["part"], &moved["terminal"]), (&json!(u1), &json!("2")));
    assert_eq!(moved["expected"], net(from));
    assert_eq!(moved["found"], json!({"net":{"id":null,"index":to}}));
    assert_eq!(result["notes"][to]["expected_net"], to);
    assert_eq!(result["notes"][to]["found_net"], to);
    assert!(kinds(&result["notes"]).iter().all(|kind| *kind == "net_id_changed"));
    assert_eq!(result["notes"].as_array().unwrap().len(), 4);
}

/// Renamed nets and reordered lists leave connectivity alone: the copy matches, is not
/// identical, and the renames and changed value are notes.
#[test]
fn names_and_order_alone_are_notes() {
    let Fixture { mut session, .. } = fixture();
    let text = exported(&mut session);
    let mut edited: Value = serde_json::from_str(&text).unwrap();
    let old = edited["nets"][1]["name"].clone();
    edited["nets"][1]["name"] = json!("RENAMED");
    edited["nets"][1]["names"] = json!(["RENAMED"]);
    edited["nets"].as_array_mut().unwrap().reverse();
    edited["unconnected"].as_array_mut().unwrap().reverse();
    for net in edited["nets"].as_array_mut().unwrap() {
        net["terminals"].as_array_mut().unwrap().reverse();
    }
    edited["components"][0]["value"] = json!("changed");
    let result = run(&mut session, &edited.to_string());
    assert_eq!(result["matches"], true);
    assert_eq!(result["identical"], false);
    assert_eq!(result["differences"], json!([]));
    assert_eq!(kinds(&result["notes"]), vec!["part_changed", "net_renamed", "net_renamed"]);
    assert_eq!(result["notes"][0]["field"], "value");
    assert_eq!((&result["notes"][1]["expected"], &result["notes"][1]["found"]), (&old, &json!("RENAMED")));
    assert_eq!(result["notes"][2]["field"], "names");
}

/// Each schematic annotation row in the format guide is a note, with its own field name;
/// changing annotation alone preserves counts and connectivity.
#[test]
fn schematic_annotation_rows_are_notes() {
    let Fixture { mut session, .. } = fixture();
    let original: Value = serde_json::from_str(&exported(&mut session)).unwrap();
    let electrical_type = if original["components"][0]["terminals"][0]["electrical_type"] == "input" { "output" } else { "input" };
    let revision = original["components"][0]["definition"]["revision"].as_u64().unwrap() + 1;
    for (path, kind, field, value) in [
        ("/project/name", "header", "project.name", json!("Another project name")),
        ("/components/0/reference", "part_changed", "reference", json!("RENAMED")),
        ("/components/0/value", "part_changed", "value", json!("Another value")),
        ("/components/0/kind", "part_changed", "kind", json!("power")),
        ("/components/0/definition/revision", "part_changed", "definition", json!(revision)),
        ("/components/0/units", "part_changed", "units", json!([])),
        ("/components/0/terminals/0/name", "terminal_changed", "name", json!("Another terminal name")),
        ("/components/0/terminals/0/electrical_type", "terminal_changed", "electrical_type", json!(electrical_type)),
    ] {
        let mut file = original.clone();
        let slot = file.pointer_mut(path).unwrap();
        assert_ne!(*slot, value, "{path} exercises a change");
        *slot = value;
        let result = run(&mut session, &file.to_string());
        assert_eq!(result["matches"], true, "{path}: {result}");
        assert_eq!(result["identical"], false, "{path}");
        assert_eq!(result["expected"], result["found"], "{path}");
        assert_eq!(result["differences"], json!([]), "{path}");
        assert_eq!(kinds(&result["notes"]), vec![kind], "{path}");
        assert_eq!(result["notes"][0]["field"], field, "{path}");
    }
}

/// A terminal moving between a net and `unconnected`, a missing or extra component, terminal or
/// net, and a different project are each differences, named by what changed.
#[test]
fn connectivity_changes_are_differences() {
    let Fixture { mut session, u1, r2, .. } = fixture();
    let text = exported(&mut session);
    let original: Value = serde_json::from_str(&text).unwrap();

    // U1.4 is unconnected in the design and on a net in the file.
    let mut joined = original.clone();
    let open = joined["unconnected"].as_array_mut().unwrap();
    let index = open.iter().position(|t| t["part"] == u1.as_str() && t["terminal"] == "4").unwrap();
    let terminal = open.remove(index);
    joined["nets"][0]["terminals"].as_array_mut().unwrap().push(terminal);
    let result = run(&mut session, &joined.to_string());
    assert_eq!(kinds(&result["differences"]), vec!["terminal_moved"]);
    assert_eq!(result["differences"][0]["expected"], "unconnected");
    assert_eq!(result["differences"][0]["found"]["net"]["index"], 0);

    // R2 is absent from the file along with every terminal it listed; its net keeps U1.6 alone.
    let mut without: Value = original.clone();
    without["components"].as_array_mut().unwrap().retain(|component| component["id"] != r2.as_str());
    for net in without["nets"].as_array_mut().unwrap() {
        net["terminals"].as_array_mut().unwrap().retain(|t| t["part"] != r2.as_str());
    }
    without["unconnected"].as_array_mut().unwrap().retain(|t| t["part"] != r2.as_str());
    let result = run(&mut session, &without.to_string());
    assert_eq!(kinds(&result["differences"]), vec!["part_missing"]);
    assert_eq!(result["differences"][0]["part"], json!(r2));
    assert_eq!(result["found"]["components"], 2);

    // An extra part has its own identity even when its terminal numbers already exist elsewhere.
    let mut with_extra = original.clone();
    let mut part = with_extra["components"][0].clone();
    part["id"] = json!("component-99");
    part["units"] = json!([]);
    with_extra["components"].as_array_mut().unwrap().push(part);
    let result = run(&mut session, &with_extra.to_string());
    assert_eq!(kinds(&result["differences"]), vec!["part_extra"]);
    assert_eq!(result["differences"][0]["part"], "component-99");

    // A terminal only the file lists.
    let mut extra = original.clone();
    let component = extra["components"].as_array_mut().unwrap().iter_mut().find(|c| c["id"] == r2.as_str()).unwrap();
    component["terminals"].as_array_mut().unwrap().push(json!({"terminal":"3","name":null,"electrical_type":"passive"}));
    extra["unconnected"].as_array_mut().unwrap().push(json!({"part":r2,"terminal":"3"}));
    let result = run(&mut session, &extra.to_string());
    assert_eq!(kinds(&result["differences"]), vec!["terminal_extra"]);
    assert_eq!((&result["differences"][0]["part"], &result["differences"][0]["terminal"]), (&json!(r2), &json!("3")));

    // A net only the file has: R2.2 joined with nothing else.
    let mut split = original.clone();
    split["unconnected"].as_array_mut().unwrap().retain(|t| !(t["part"] == r2.as_str() && t["terminal"] == "2"));
    split["nets"].as_array_mut().unwrap().push(json!({"id":null,"name":"LONE","names":["LONE"],"terminals":[{"part":r2,"terminal":"2"}]}));
    let result = run(&mut session, &split.to_string());
    assert_eq!(kinds(&result["differences"]), vec!["terminal_moved", "net_extra"]);
    assert_eq!(result["differences"][1]["found_net"], net_of(&split, &r2, "2"));
    assert!(result["differences"][1].get("expected_net").is_none());

    // The same file claimed by another project does not match.
    let mut foreign = original;
    foreign["project"]["id"] = json!(uuid::Uuid::nil());
    let result = run(&mut session, &foreign.to_string());
    assert_eq!(kinds(&result["differences"]), vec!["project"]);
}

/// A net only the design has is missing, a terminal table only the design has is missing, and
/// another basis is a difference; the pure function needs no session.
#[test]
fn missing_nets_terminals_and_basis_compare_without_a_session() {
    let Fixture { session, u1, .. } = fixture();
    let design = session.project().unwrap().netlist().unwrap().netlist;
    let mut file = design.clone();
    let index = file.nets.iter().position(|net| net.terminals.iter().any(|t| t.part == u1 && t.terminal == "6")).unwrap();
    let net = file.nets.remove(index);
    file.unconnected.extend(net.terminals);
    let result = compare(&design, &file).unwrap();
    assert!(!result.matches);
    assert_eq!((result.differences[2].expected_net, result.differences[2].found_net), (Some(index), None));
    assert_eq!(result.differences.iter().map(|d| d.kind).collect::<Vec<_>>(), vec![Kind::TerminalMoved, Kind::TerminalMoved, Kind::NetMissing]);
    assert_eq!(result.found.unconnected, result.expected.unconnected + 2);

    let mut fewer = design.clone();
    let amplifier = fewer.components.iter_mut().find(|component| component.id == u1).unwrap();
    amplifier.terminals.retain(|terminal| terminal.terminal != "4");
    fewer.unconnected.retain(|t| !(t.part == u1 && t.terminal == "4"));
    let result = compare(&design, &fewer).unwrap();
    assert_eq!(result.differences.iter().map(|d| (d.kind, d.terminal)).collect::<Vec<_>>(), vec![(Kind::TerminalMissing, Some("4"))]);

    let mut accepted = design.clone();
    accepted.basis = Basis::Accepted;
    let result = compare(&design, &accepted).unwrap();
    assert_eq!(result.differences.iter().map(|d| d.kind).collect::<Vec<_>>(), vec![Kind::Basis]);
    assert!(compare(&design, &design).unwrap().identical);
}

/// The reader refuses a terminal listed twice or one its component does not have, and a
/// duplicated component or terminal number; one terminal number in two components is fine.
#[test]
fn reader_refuses_terminals_listed_twice_or_unknown() {
    let Fixture { mut session, u1, r1, .. } = fixture();
    let text = exported(&mut session);
    let original: Value = serde_json::from_str(&text).unwrap();
    let refused = |edit: &dyn Fn(&mut Value)| {
        let mut value = original.clone();
        edit(&mut value);
        Netlist::parse(&value.to_string()).unwrap_err()
    };
    let twice = json!({"part":u1,"terminal":"2"});
    assert!(refused(&|v| v["nets"][0]["terminals"].as_array_mut().unwrap().push(twice.clone())).contains("more than once"));
    assert!(refused(&|v| v["unconnected"].as_array_mut().unwrap().push(twice.clone())).contains("more than once"));
    let unknown = json!({"part":u1,"terminal":"7"});
    assert!(refused(&|v| v["unconnected"].as_array_mut().unwrap().push(unknown.clone())).contains("not in its component's terminal table"));
    let nobody = json!({"part":"component-99","terminal":"1"});
    assert!(refused(&|v| v["nets"][0]["terminals"].as_array_mut().unwrap().push(nobody.clone())).contains("terminal table"));
    assert!(
        refused(&|v| {
            let copy = v["components"][0].clone();
            v["components"].as_array_mut().unwrap().push(copy);
        })
        .contains("listed twice")
    );
    assert!(
        refused(&|v| {
            let copy = v["components"][0]["terminals"][0].clone();
            v["components"][0]["terminals"].as_array_mut().unwrap().push(copy);
        })
        .contains("lists terminal")
    );
    // U1 and R1 both have a terminal 1, and the export lists both.
    let parsed = Netlist::parse(&text).unwrap();
    let ones = parsed.nets.iter().flat_map(|net| &net.terminals).filter(|t| t.terminal == "1" && (t.part == u1 || t.part == r1)).count();
    assert_eq!(ones, 2);
}

/// Persistent net identity must be unique regardless of file order; null IDs may repeat.
#[test]
fn read_back_refuses_duplicate_persistent_net_ids() {
    let Fixture { mut session, .. } = fixture();
    let mut file: Value = serde_json::from_str(&exported(&mut session)).unwrap();
    let id = file["nets"][0]["id"].as_str().unwrap().to_owned();
    assert_ne!(file["nets"][0]["id"], file["nets"][1]["id"]);
    file["nets"][1]["id"] = json!(id);
    for _ in 0..2 {
        let params = json!({"netlist":file.to_string()});
        let error = session.execute(COMPARE_NETLIST, &params).unwrap_err();
        assert!(matches!(error, CommandError::InvalidParams(_)), "{error}");
        assert!(error.to_string().contains(&format!("net ID {id} is listed twice")), "{error}");
        file["nets"].as_array_mut().unwrap().reverse();
    }
    for net in file["nets"].as_array_mut().unwrap() {
        net["id"] = Value::Null;
    }
    assert_eq!(run(&mut session, &file.to_string())["matches"], true);
}

/// Object records cannot be substituted with arrays even in the correct positional order.
#[test]
fn read_back_refuses_positional_records() {
    let Fixture { mut session, .. } = fixture();
    let original: Value = serde_json::from_str(&exported(&mut session)).unwrap();
    for (path, keys) in [
        ("/project", &["id", "name"][..]),
        ("/components/0/definition", &["library", "id", "revision"]),
        ("/nets/0/terminals/0", &["part", "terminal"]),
        ("/unconnected/0", &["part", "terminal"]),
    ] {
        let mut file = original.clone();
        let slot = file.pointer_mut(path).unwrap();
        *slot = Value::Array(keys.iter().map(|key| slot[*key].clone()).collect());
        let error = session.execute(COMPARE_NETLIST, &json!({"netlist":file.to_string()})).unwrap_err();
        assert!(matches!(error, CommandError::InvalidParams(_)), "{path}: {error}");
        assert!(error.to_string().contains("sequence, expected an object"), "{path}: {error}");
    }
}

/// The command refuses a missing, null, non-string, oversized or invalid `netlist` and unknown
/// parameters the same way through execute and apply; text of exactly the size limit is read.
#[test]
fn command_parameters_are_checked_at_their_edges() {
    let mut empty = Session::default();
    let error = empty.execute(COMPARE_NETLIST, &json!({"netlist":"{}"})).unwrap_err();
    assert!(matches!(error, CommandError::Unavailable(_)), "{error}");
    assert!(error.to_string().contains("no project is open"));
    let Fixture { mut session, .. } = fixture();
    let text = exported(&mut session);
    let before = session.snapshot().unwrap();
    let oversized = format!("{text}{}", " ".repeat(MAX_TEXT_BYTES + 1 - text.len()));
    let mut malformed: Value = serde_json::from_str(&text).unwrap();
    let duplicate = malformed["nets"][0]["terminals"][0].clone();
    malformed["unconnected"].as_array_mut().unwrap().push(duplicate);
    for (params, says) in [
        (json!({}), "needs `netlist`"),
        (json!({"netlist":null}), "at most 8 MiB"),
        (json!({"netlist":1}), "at most 8 MiB"),
        (json!({"netlist":true}), "at most 8 MiB"),
        (json!({"netlist":[]}), "at most 8 MiB"),
        (json!({"netlist":{}}), "at most 8 MiB"),
        (json!({"netlist":oversized}), "at most 8 MiB"),
        (json!({"netlist":""}), "invalid netlist"),
        (json!({"netlist":"{}"}), "invalid netlist"),
        (json!({"netlist":malformed.to_string()}), "more than once"),
        (json!({"netlist":text,"extra":true}), "no parameter `extra`"),
    ] {
        let applied = session.apply(COMPARE_NETLIST, &params).unwrap_err();
        assert!(matches!(applied, CommandError::InvalidParams(_)), "{applied}");
        assert!(applied.to_string().contains(says), "{applied} for {params}");
        assert_eq!(session.execute(COMPARE_NETLIST, &params).unwrap_err(), applied);
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    }
    let largest = format!("{text}{}", " ".repeat(MAX_TEXT_BYTES - text.len()));
    assert_eq!(largest.len(), MAX_TEXT_BYTES);
    assert_eq!(run(&mut session, &largest)["matches"], true);
}

/// UTF-8 byte length, rather than character count, bounds input: a valid multibyte document
/// of exactly 8 MiB is accepted, and adding one ASCII whitespace byte is refused before parsing.
#[test]
fn multibyte_text_is_bounded_by_encoded_bytes() {
    let Fixture { mut session, .. } = fixture();
    let mut file: Value = serde_json::from_str(&exported(&mut session)).unwrap();
    file["project"]["name"] = json!("");
    let room = MAX_TEXT_BYTES - file.to_string().len();
    file["project"]["name"] = json!(format!("{}{}", "é".repeat(room / 2), " ".repeat(room % 2)));
    let mut text = file.to_string();
    assert_eq!(text.len(), MAX_TEXT_BYTES);
    assert!(text.chars().count() < MAX_TEXT_BYTES);
    assert_eq!(run(&mut session, &text)["matches"], true);
    text.push(' ');
    assert_eq!(text.len(), MAX_TEXT_BYTES + 1);
    assert!(text.chars().count() < MAX_TEXT_BYTES);
    let before = session.snapshot().unwrap();
    let params = json!({"netlist":text});
    let error = session.execute(COMPARE_NETLIST, &params).unwrap_err();
    assert!(matches!(error, CommandError::InvalidParams(_)), "{error}");
    assert!(error.to_string().contains("at most 8 MiB"), "{error}");
    assert_eq!(session.apply(COMPARE_NETLIST, &params).unwrap_err(), error);
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
}

/// List order alone is not a connectivity change or annotation note, but changes field equality.
#[test]
fn list_order_alone_leaves_no_differences_or_notes() {
    let Fixture { mut session, .. } = fixture();
    let mut file: Value = serde_json::from_str(&exported(&mut session)).unwrap();
    file["components"].as_array_mut().unwrap().reverse();
    for part in file["components"].as_array_mut().unwrap() {
        part["terminals"].as_array_mut().unwrap().reverse();
    }
    file["nets"].as_array_mut().unwrap().reverse();
    for net in file["nets"].as_array_mut().unwrap() {
        net["terminals"].as_array_mut().unwrap().reverse();
    }
    file["unconnected"].as_array_mut().unwrap().reverse();
    let result = run(&mut session, &file.to_string());
    assert_eq!(result["matches"], true);
    assert_eq!(result["identical"], false);
    assert_eq!(result["differences"], json!([]));
    assert_eq!(result["notes"], json!([]));
}

/// A board's file is compared with that board's accepted netlist: it matches as exported, a
/// changed pad mapping is a difference, and a board the project does not have is refused.
#[test]
fn board_files_compare_with_the_accepted_netlist() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    let root = s.project().unwrap().sheet_target();
    apply(&mut s, PLACE_RESISTOR, json!({"sheet":root.id(),"x_nm":0,"y_nm":0}));
    let part = s.project().unwrap().components().next().unwrap().id();
    let definition = basic_components("resistor-0402").unwrap();
    apply(
        &mut s,
        ASSIGN_FOOTPRINT,
        json!({"component":part,"library":definition.library,"definition":definition.id,
            "definition_revision":definition.revision,"mapping":{"1":"1","2":"2"}}),
    );
    let preview = s.execute(PREVIEW_UPDATE, &json!({"expected_revision":s.revision()})).unwrap()["board_update_preview"].clone();
    s.apply(APPLY_UPDATE, &json!({"plan_token":preview["plan_token"],"expected_revision":s.revision()})).unwrap();
    let text = s.execute(EXPORT_BOARD_NETLIST, &json!({})).unwrap()["netlist"]["files"][0]["contents"].as_str().unwrap().to_owned();
    let same = run(&mut s, &text);
    assert_eq!((&same["matches"], &same["identical"]), (&json!(true), &json!(true)));
    assert_eq!(same["expected"]["terminals"], 2);

    let original: Value = serde_json::from_str(&text).unwrap();
    assert!(original["board"]["source"].is_object());
    let mut positional = original.clone();
    let source = &mut positional["board"]["source"];
    *source = json!([source["capture_id"], source["content_digest"]]);
    let error = s.execute(COMPARE_NETLIST, &json!({"netlist":positional.to_string()})).unwrap_err();
    assert!(error.to_string().contains("sequence, expected an object"), "{error}");

    let revision = original["components"][0]["package"]["revision"].as_u64().unwrap() + 1;
    for (path, kind, field, value) in [
        ("/board/name", "header", "board.name", json!("Another board name")),
        ("/board/source", "header", "board.source", Value::Null),
        ("/components/0/kind", "part_changed", "kind", json!("unknown")),
        ("/components/0/package/revision", "part_changed", "package", json!(revision)),
    ] {
        let mut file = original.clone();
        let slot = file.pointer_mut(path).unwrap();
        assert_ne!(*slot, value, "{path} exercises a change");
        *slot = value;
        let result = run(&mut s, &file.to_string());
        assert_eq!(result["matches"], true, "{path}: {result}");
        assert_eq!(result["identical"], false, "{path}");
        assert_eq!(kinds(&result["notes"]), vec![kind], "{path}");
        assert_eq!(result["notes"][0]["field"], field, "{path}");
        assert_eq!(result["differences"], json!([]), "{path}");
    }
    let accepted = Netlist::parse(&text).unwrap();
    let mut other_board = accepted.clone();
    other_board.board.as_mut().unwrap().id = uuid::Uuid::nil();
    let compared = compare(&accepted, &other_board).unwrap();
    assert_eq!(compared.differences.iter().map(|entry| entry.kind).collect::<Vec<_>>(), vec![Kind::Board]);

    let mut swapped = original.clone();
    swapped["components"][0]["terminals"][0]["pads"] = json!(["2"]);
    swapped["components"][0]["terminals"][1]["pads"] = json!(["1"]);
    let result = run(&mut s, &swapped.to_string());
    assert_eq!(kinds(&result["differences"]), vec!["pads_changed", "pads_changed"]);
    assert_eq!((&result["differences"][0]["expected"], &result["differences"][0]["found"]), (&json!(["1"]), &json!(["2"])));

    let mut elsewhere = original;
    elsewhere["board"]["id"] = json!(uuid::Uuid::nil());
    let error = s.execute(COMPARE_NETLIST, &json!({"netlist":elsewhere.to_string()})).unwrap_err();
    assert!(matches!(error, CommandError::InvalidParams(_)), "{error}");
    assert!(error.to_string().contains("not in the open project"), "{error}");

    // The schematic file of the same project is compared with the schematic, not the board.
    let schematic = exported(&mut s);
    assert_eq!(run(&mut s, &schematic)["matches"], true);
}

/// Among equal overlaps the pair with fewer unshared terminals wins: U1.2 moving from a
/// two-terminal net into a one-terminal net is that one move, whatever order the nets are in.
#[test]
fn equal_overlaps_pair_the_closest_nets() {
    let Fixture { session, u1, r1, r2, .. } = fixture();
    let mut design = session.project().unwrap().netlist().unwrap().netlist;
    let has = |net: &Net, part: &str, terminal: &str| net.terminals.iter().any(|t| t.part == part && t.terminal == terminal);
    let lone = design.nets.iter().position(|net| has(net, &r2, "1")).unwrap();
    assert_eq!(design.nets[lone].terminals.len(), 2);
    assert!(has(&design.nets[lone], &u1, "6"));
    let index = design.nets[lone].terminals.iter().position(|t| t.part == r2 && t.terminal == "1").unwrap();
    let open = design.nets[lone].terminals.remove(index);
    design.unconnected.push(open);
    let pair = design.nets.iter().position(|net| has(net, &u1, "2")).unwrap();
    assert_eq!(design.nets[pair].terminals.len(), 2);
    assert!(has(&design.nets[pair], &r1, "1"));

    let mut found = design.clone();
    let index = found.nets[pair].terminals.iter().position(|t| t.part == u1 && t.terminal == "2").unwrap();
    let moved = found.nets[pair].terminals.remove(index);
    found.nets[lone].terminals.push(moved);
    for net in &mut found.nets {
        net.id = None;
    }
    for order in [false, true] {
        if order {
            found.nets.reverse();
        }
        let result = compare(&design, &found).unwrap();
        let named: Vec<_> = result.differences.iter().map(|d| (d.kind, d.part, d.terminal)).collect();
        assert_eq!(named, vec![(Kind::TerminalMoved, Some(u1.as_str()), Some("2"))], "reversed: {order}");
    }
}

/// The format guide names the current difference and note labels in their respective categories.
#[test]
fn format_guide_lists_every_kind() {
    let guide = include_str!("../../../../../docs/netlist-format.md");
    let all = [
        Kind::Project,
        Kind::Basis,
        Kind::Board,
        Kind::PartMissing,
        Kind::PartExtra,
        Kind::TerminalMissing,
        Kind::TerminalExtra,
        Kind::TerminalMoved,
        Kind::PadsChanged,
        Kind::NetMissing,
        Kind::NetExtra,
        Kind::NetRenamed,
        Kind::NetIdChanged,
        Kind::Header,
        Kind::PartChanged,
        Kind::TerminalChanged,
    ];
    let (_, kinds) = guide.split_once("The difference kinds are ").unwrap();
    let (differences, notes) = kinds.split_once("The note kinds\nare ").unwrap();
    let (notes, _) = notes.split_once("A file whose nets").unwrap();
    for kind in all {
        let section = match kind {
            Kind::Project
            | Kind::Basis
            | Kind::Board
            | Kind::PartMissing
            | Kind::PartExtra
            | Kind::TerminalMissing
            | Kind::TerminalExtra
            | Kind::TerminalMoved
            | Kind::PadsChanged
            | Kind::NetMissing
            | Kind::NetExtra => differences,
            Kind::NetRenamed | Kind::NetIdChanged | Kind::Header | Kind::PartChanged | Kind::TerminalChanged => notes,
        };
        let name = serde_json::to_value(kind).unwrap();
        let name = name.as_str().unwrap();
        assert!(section.contains(&format!("`{name}`")), "docs/netlist-format.md names `{name}` in the right category");
    }
}

/// The file's net names are not copied once per terminal: a file putting every terminal on one
/// net whose name fills the 8 MiB limit gives a comparison that holds the name once and is
/// less than 16 KiB larger than the file. Naming the file's net in each moved terminal's
/// message and `found`, as the comparison once did, would copy it 16 times for these 8 moves.
#[test]
fn a_long_net_name_in_the_file_is_reported_once() {
    let Fixture { mut session, .. } = fixture();
    let original: Value = serde_json::from_str(&exported(&mut session)).unwrap();
    let mut merged = original.clone();
    let mut all: Vec<Value> = Vec::new();
    for net in merged["nets"].as_array().unwrap() {
        all.extend(net["terminals"].as_array().unwrap().iter().cloned());
    }
    all.extend(merged["unconnected"].as_array().unwrap().iter().cloned());
    merged["unconnected"] = json!([]);
    let mut net = merged["nets"][0].clone();
    net["terminals"] = Value::Array(all);
    net["name"] = json!("");
    merged["nets"] = json!([net]);
    let room = MAX_TEXT_BYTES - merged.to_string().len();
    merged["nets"][0]["name"] = json!("N".repeat(room));
    let text = merged.to_string();
    assert_eq!(text.len(), MAX_TEXT_BYTES);

    let result = run(&mut session, &text);
    let moved = result["differences"].as_array().unwrap().iter().filter(|d| d["kind"] == "terminal_moved").count();
    assert_eq!(moved, 8, "every terminal off the first net moved: {}", kinds(&result["differences"]).join(","));
    let serialized = result.to_string();
    let name = "N".repeat(room);
    assert_eq!(serialized.matches(name.as_str()).count(), 1, "only the net_renamed note names it");
    assert!(serialized.len() < text.len() + 16 * 1024, "{} bytes for a {} byte file", serialized.len(), text.len());
}

/// Compact design references fit under a small report budget even when many rows refer to a
/// design net with a large display name. No move payload needs the name itself.
#[test]
fn design_net_display_names_do_not_expand_moves() {
    let Fixture { session, .. } = fixture();
    let mut design = session.project().unwrap().netlist().unwrap().netlist;
    design.nets[0].name = "design net ".repeat(1_024);
    let mut found = design.clone();
    let moved = std::mem::take(&mut found.nets[0].terminals);
    let moves = moved.len();
    assert!(moves > 1);
    found.nets[1].terminals.extend(moved);
    let result = compare_with_budget(&design, &found, 4_096, 32).unwrap();
    let report = serde_json::to_value(result).unwrap();
    assert_eq!(report["differences"].as_array().unwrap().len(), moves);
    for row in report["differences"].as_array().unwrap() {
        assert_eq!(row["expected"], json!({"net":{"id":design.nets[0].id,"index":0}}));
        assert_eq!(row["found"], json!({"net":{"id":found.nets[1].id,"index":1}}));
    }
    assert!(!report.to_string().contains(design.nets[0].name.as_str()));
}

/// Enforce one shared row budget and count actually escaped bytes before retaining a row.
#[test]
fn report_budget_counts_escaping_and_shares_rows() {
    let Fixture { session, .. } = fixture();
    let design = session.project().unwrap().netlist().unwrap().netlist;
    let mut found = design.clone();
    found.nets[0].name = "\n".repeat(512);
    let report = compare_with_budget(&design, &found, 4_096, 1).unwrap();
    let row_cost = serde_json::to_vec(&report.notes[0]).unwrap().len() + 1;
    assert!(row_cost > found.nets[0].name.len() * 2);
    assert!(compare_with_budget(&design, &found, row_cost, 1).is_ok());
    assert!(matches!(compare_with_budget(&design, &found, row_cost - 1, 1), Err(CommandError::Unavailable(_))));

    found.project.id = uuid::Uuid::nil();
    assert!(compare_with_budget(&design, &found, 4_096, 2).is_ok());
    assert!(matches!(compare_with_budget(&design, &found, 4_096, 1), Err(CommandError::Unavailable(_))));
}
