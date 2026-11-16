use super::*;
use crate::{CONNECT_PINS, PLACE_RESISTOR, Session};
use serde_json::json;

/// Preserve allowed bytes and encode punctuation and non-ASCII tokens without ambiguity.
#[test]
fn tokens_use_unambiguous_uppercase_utf8_escaping() {
    assert_eq!(token("U.3/é +_-"), "U%2E3%2F%C3%A9%20+_-");
}

/// Inspection shares the canonical name without changing bytes, revisions or sparse registration.
#[test]
fn project_names_are_read_only_canonical_and_follow_reference_history() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"name":"Names"})).unwrap();
    for x in [0, 20_000_000] {
        s.apply(PLACE_RESISTOR, &json!({"x_nm":x,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    }
    s.apply(
        CONNECT_PINS,
        &json!({"from":{"symbol":"symbol-3","terminal":"1"},"to":{"symbol":"symbol-2","terminal":"2"},"expected_revision":s.revision()}),
    )
    .unwrap();
    let project = s.snapshot().unwrap();
    let project_registry = project.net_registry.clone();
    let before = crate::persistence::encode_documents(project.clone()).unwrap();
    let revision = s.revision();
    assert_eq!(project.connectivity().net_name(0), "Net-0001");
    assert_eq!(project.electrical_connectivity().net_name(0), "Net-0001");
    assert_eq!(project.inspect_net_registry().unwrap()["nets"][0]["name"], "Net-0001");
    assert_eq!(crate::persistence::encode_documents(project).unwrap(), before);
    assert_eq!(s.revision(), revision);
    assert_eq!(s.project().unwrap().net_registry, project_registry);
}

/// Every ambiguous primary gains a path, and colliding digest prefixes extend together.
#[test]
fn equal_scoped_names_qualify_all_members_and_extend_digest_collisions() {
    let names = qualify(
        vec!["Net-(U3-OUT)".into(); 3],
        vec!["Main".into(), "Main".into(), "Power".into()],
        vec![format!("12345678aa{}", "0".repeat(54)), format!("12345678bb{}", "0".repeat(54)), "c".repeat(64)],
        &[false; 3],
    );
    assert_eq!(names, ["Main::Net-(U3-OUT)~12345678aa", "Main::Net-(U3-OUT)~12345678bb", "Power::Net-(U3-OUT)"]);
}

/// Sheet projections retain the qualification decision from the complete project.
#[test]
fn local_name_collisions_are_project_wide_and_explicit_aliases_sort_by_bytes() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"name":"Names"})).unwrap();
    s.apply(crate::sheets::CREATE_SHEET, &json!({"name":"Power.A","expected_revision":s.revision()})).unwrap();
    let sheets: Vec<_> = s.project().unwrap().sheets().map(|sheet| sheet.target).collect();
    for sheet in &sheets {
        s.apply(crate::net_labels::ADD, &json!({"sheet":sheet.id(),"name":"BUS","style":"local","x_nm":0,"y_nm":0,"expected_revision":s.revision()}))
            .unwrap();
    }
    let project = s.project().unwrap();
    let names: BTreeSet<_> = (0..project.electrical_connectivity().nets.len()).map(|i| project.electrical_connectivity().net_name(i)).collect();
    assert_eq!(names, BTreeSet::from(["Main::BUS".into(), "Power%2EA::BUS".into()]));
    for sheet in sheets {
        let own = project.for_sheet(sheet).unwrap();
        assert!(names.contains(&own.connectivity().net_name(0)));
    }
}

/// Reference edits and authored aliases change presentation while retaining typed net authority.
#[test]
fn reference_edits_and_explicit_name_adoption_keep_typed_identity() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    for x in [0, 20_000_000] {
        s.apply(PLACE_RESISTOR, &json!({"x_nm":x,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    }
    s.apply(
        CONNECT_PINS,
        &json!({"from":{"symbol":"symbol-2","terminal":"2"},"to":{"symbol":"symbol-3","terminal":"1"},"expected_revision":s.revision()}),
    )
    .unwrap();
    s.apply(crate::nets::PROMOTE, &json!({"anchor":{"symbol":"symbol-2","terminal":"2"},"expected_revision":s.revision()})).unwrap();
    let before = s.snapshot().unwrap();
    let id = before.net_registry.records[0].id;
    s.apply(crate::SET_SYMBOL_FIELDS, &json!({"symbol":"symbol-2","reference":"Z99","value":"R","expected_revision":s.revision()})).unwrap();
    assert_eq!(s.project().unwrap().connectivity().net_name(0), "Net-0001");
    assert_eq!(s.project().unwrap().net_registry.records[0].id, id);
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().unwrap().connectivity().net_name(0), "Net-0001");
    s.apply("edit.redo", &json!({})).unwrap();
    let at = s.project().unwrap().symbols()[0].pins()[1];
    for (name, style) in [("ZZZ", "global"), ("AAA", "local")] {
        s.apply(crate::net_labels::ADD, &json!({"name":name,"style":style,"x_nm":at.x_nm,"y_nm":at.y_nm,"expected_revision":s.revision()})).unwrap();
    }
    let project = s.snapshot().unwrap();
    assert_eq!(project.connectivity().net_name(0), "AAA");
    assert_eq!(project.connectivity().net_names(0), ["AAA", "ZZZ"]);
    assert_eq!(project.net_registry.records.iter().filter(|r| matches!(r.state, crate::nets::State::Active {})).count(), 1);
    let docs = crate::persistence::encode_documents(project).unwrap();
    let reopened = crate::persistence::decode_documents(&docs).unwrap();
    assert_eq!(reopened.connectivity().net_name(0), "AAA");
}

/// Splits and merges follow persistent identity policy; deleting an anchor preserves the
/// surviving branch number rather than substituting another component reference.
#[test]
fn topology_changes_preserve_surviving_numbers_and_undo_exactly() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    for i in 0..6 {
        s.apply(PLACE_RESISTOR, &json!({"x_nm":i*20_000_000,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    }
    let endpoint = |id| json!({"symbol":format!("symbol-{id}"),"terminal":"1"});
    for (a, b) in [(2, 3), (3, 4), (5, 6), (6, 7)] {
        s.apply(CONNECT_PINS, &json!({"from":endpoint(b),"to":endpoint(a),"expected_revision":s.revision()})).unwrap();
    }
    let names = |s: &Session| {
        let graph = s.project().unwrap().electrical_connectivity();
        (0..graph.nets.len()).map(|i| graph.net_name(i)).collect::<BTreeSet<_>>()
    };
    assert_eq!(names(&s), BTreeSet::from(["Net-0001".into(), "Net-0002".into()]));
    s.apply(CONNECT_PINS, &json!({"from":endpoint(4),"to":endpoint(5),"expected_revision":s.revision()})).unwrap();
    let bridge = s.project().unwrap().wires().last().unwrap().id();
    assert_eq!(names(&s), BTreeSet::from(["Net-0003".into()]));
    s.apply(crate::DELETE_WIRE, &json!({"wire":bridge,"expected_revision":s.revision()})).unwrap();
    assert_eq!(names(&s), BTreeSet::from(["Net-0003".into(), "Net-0004".into()]));
    s.apply(crate::DELETE_SYMBOL, &json!({"symbol":"symbol-2","expected_revision":s.revision()})).unwrap();
    assert_eq!(names(&s), BTreeSet::from(["Net-0003".into(), "Net-0004".into()]));
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(names(&s), BTreeSet::from(["Net-0003".into(), "Net-0004".into()]));
}

/// Reserved-looking authored names remain ordinary electrical text; orphan geometry is not
/// promoted or given a fabricated signal name, and absent targets remain explicitly unresolved.
#[test]
fn orphan_groups_and_reserved_looking_authored_names_are_distinct() {
    let names = qualify(
        vec!["Unconnected".into(); 3],
        vec!["A".into(), "B".into(), "C".into()],
        vec!["a".repeat(64), "b".repeat(64), "c".repeat(64)],
        &[false, false, true],
    );
    assert_eq!(names, ["A::Unconnected", "B::Unconnected", "Unconnected"]);
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    s.apply(crate::DRAW_WIRE, &json!({"from":{"x_nm":0,"y_nm":0},"to":{"x_nm":10_000_000,"y_nm":0},"bends":[],"expected_revision":s.revision()}))
        .unwrap();
    let p = s.project().unwrap();
    assert_eq!(p.connectivity().net_name(0), "Unconnected");
    assert_eq!(p.connectivity().net_name(1), "Unresolved net");
    assert!(p.net_registry.records.is_empty());
    assert!(p.inspect_net_registry().unwrap()["nets"].as_array().unwrap().is_empty());
}

/// Parent pins and child ports share the child's occurrence-qualified identity and path.
#[test]
fn hierarchical_port_aliases_use_the_child_occurrence_path() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let root = s.project().unwrap().sheet_target();
    s.apply(
        crate::hierarchy::CREATE_CHILD,
        &json!({"sheet":root.id(),"name":"Power","x_nm":50_800_000,"y_nm":25_400_000,"expected_revision":s.revision()}),
    )
    .unwrap();
    let child = s.project().unwrap().sheets().nth(1).unwrap().target;
    s.apply(
        crate::hierarchy::ADD_LABEL,
        &json!({"sheet":child.id(),"name":"BUS","direction":"input","x_nm":0,"y_nm":0,"expected_revision":s.revision()}),
    )
    .unwrap();
    s.apply(
        crate::net_labels::ADD,
        &json!({"sheet":root.id(),"name":"BUS","style":"local","x_nm":200_000_000,"y_nm":0,"expected_revision":s.revision()}),
    )
    .unwrap();
    s.apply(PLACE_RESISTOR, &json!({"sheet":root.id(),"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let project = s.project().unwrap();
    let own = project.for_sheet(root).unwrap();
    let parent_pin = own.sheet_pins(&own.sheet_symbols()[0]).next().unwrap();
    let pin = parent_pin.label.id;
    let bend = crate::schematic::Point { x_nm: own.symbols()[0].pins()[0].x_nm, y_nm: parent_pin.at.y_nm };
    let symbol = own.symbols()[0].id();
    s.apply(crate::DRAW_WIRE,&json!({"sheet":root.id(),"from":{"symbol":symbol,"terminal":"1"},"to":{"sheet_pin":format!("label-{pin}")},"bends":[bend],"expected_revision":s.revision()})).unwrap();
    let graph = s.project().unwrap().electrical_connectivity();
    assert_eq!(
        (0..graph.nets.len()).map(|i| graph.net_name(i)).collect::<BTreeSet<_>>(),
        BTreeSet::from(["Main::BUS".into(), "Main/Power::BUS".into()])
    );
}

/// Physical edits with unchanged schematic inputs reuse the immutable naming result.
#[test]
fn physical_edits_retain_the_project_naming_cache() {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    for x in [0, 20_000_000] {
        s.apply(PLACE_RESISTOR, &json!({"x_nm":x,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    }
    s.apply(
        CONNECT_PINS,
        &json!({"from":{"symbol":"symbol-2","terminal":"2"},"to":{"symbol":"symbol-3","terminal":"1"},"expected_revision":s.revision()}),
    )
    .unwrap();
    s.apply(
        crate::PLACE_FOOTPRINT,
        &json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":0,"y_nm":0,"expected_revision":s.revision()}),
    )
    .unwrap();
    let before = s.snapshot().unwrap();
    assert_eq!(before.electrical_connectivity().net_name(0), "Net-0001");
    let footprint = before.boards().next().unwrap().footprints().next().unwrap().id;
    s.apply(crate::SET_FOOTPRINT_TRANSFORM, &json!({"footprint":footprint,"x_nm":10_000_000,"y_nm":0,"rotation":0,"expected_revision":s.revision()}))
        .unwrap();
    let after = s.snapshot().unwrap();
    assert!(std::sync::Arc::ptr_eq(&before.named_graph, &after.named_graph));
    assert_eq!(after.electrical_connectivity().net_name(0), "Net-0001");
}

/// A first physical owner changes canonical scoped addresses, so cached sheet qualification expires.
#[test]
fn first_board_creation_invalidates_sheet_name_qualification() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    for x in [0, 20_000_000] {
        s.apply(PLACE_RESISTOR, &json!({"x_nm":x,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    }
    s.apply(
        CONNECT_PINS,
        &json!({"from":{"symbol":"symbol-2","terminal":"2"},"to":{"symbol":"symbol-3","terminal":"1"},"expected_revision":s.revision()}),
    )
    .unwrap();
    let before = s.snapshot().unwrap();
    assert_eq!(before.connectivity().net_name(0), "Net-0001");
    s.apply(crate::board::identity::CREATE_BOARD, &json!({"expected_revision":s.revision()})).unwrap();
    let after = s.snapshot().unwrap();
    assert!(!before.same_connectivity(&after));
    assert_eq!(after.connectivity().net_name(0), after.electrical_connectivity().net_name(0));
}

/// Prefixes are shortest for each net, and an authored literal cannot impersonate an 8-byte suffix.
#[test]
fn digest_prefix_lengths_are_individual_and_avoid_literal_display_names() {
    let names = qualify(
        vec!["X".into(); 3],
        vec!["Main".into(); 3],
        vec![format!("12345678aa{}", "0".repeat(54)), format!("12345678bb{}", "0".repeat(54)), "c".repeat(64)],
        &[false; 3],
    );
    assert_eq!(names, ["Main::X~12345678aa", "Main::X~12345678bb", "Main::X~cccccccc"]);
    let names = qualify(
        vec!["X".into(), "X".into(), "Main::X~aaaaaaaa".into()],
        vec!["Main".into(); 3],
        vec!["a".repeat(64), "b".repeat(64), "c".repeat(64)],
        &[false; 3],
    );
    assert_eq!(names, ["Main::X~aaaaaaaaaa", "Main::X~bbbbbbbb", "Main::X~aaaaaaaa"]);
}
