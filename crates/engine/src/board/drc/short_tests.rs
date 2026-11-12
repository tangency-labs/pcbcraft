use super::*;
use crate::{
    ADD_TRACKS, CONNECT_PINS, DELETE_WIRE, SET_FOOTPRINT_TRANSFORM,
    board::{
        track::tests::{copper_layers, routed_fixture, view},
        update::{APPLY_UPDATE, PREVIEW_UPDATE},
    },
};
use std::collections::BTreeSet;

/// Legal copper bridges whose later accepted schematic split creates an existing short.
#[derive(Clone, Copy)]
pub(crate) enum Bridge {
    Track,
    Vias,
    Pads,
}

/// Accept changed schematic membership through the same reviewed operation as GUI and MCP.
fn accept_update(s: &mut Session) {
    let board = view(s).target.board;
    s.apply(PREVIEW_UPDATE, &json!({"board":board,"expected_revision":s.revision()})).unwrap();
    let token = s.cached_board_update().unwrap().plan_token;
    s.apply(APPLY_UPDATE, &json!({"board":board,"plan_token":token,"expected_revision":s.revision()})).unwrap();
}

/// Author a legal bridge, then accept a schematic split while keeping the authored copper.
/// This reproduces an existing short entirely through shared commands, without file surgery.
pub(crate) fn bridged_split(bridge: Bridge) -> Session {
    let mut s = routed_fixture();
    let footprint = view(&s).footprints().find(|f| f.pads.iter().all(|p| p.net.is_some()) && f.pads[0].net != f.pads[1].net).unwrap();
    let component = crate::schematic::wiring::parse_id(&footprint.component, "component-").unwrap();
    let a = footprint.pads[0].center;
    let mut b = footprint.pads[1].center;
    let mut other_component = component;
    let mut terminals = ("2".to_owned(), "1".to_owned()); // The fixture deliberately swaps pad mapping.
    let moved = (footprint.id.clone(), footprint.position, footprint.rotation);
    if matches!(bridge, Bridge::Pads) {
        let other = view(&s)
            .footprints()
            .find(|f| f.id != footprint.id && f.pads.iter().any(|pad| pad.net.is_some() && pad.net != footprint.pads[0].net))
            .unwrap();
        let pad = other.pads.iter().find(|pad| pad.net.is_some() && pad.net != footprint.pads[0].net).unwrap();
        b = pad.center;
        let address = super::super::Pad::parse(&other.id, &pad.number).unwrap();
        let (other, terminal) = s.project().unwrap().board.component_pin_for(address).unwrap();
        other_component = other;
        terminals.1 = terminal.to_owned();
    }
    let symbol = |component| s.project().unwrap().symbols.iter().find(|symbol| symbol.component == component).unwrap().id();
    let (symbol_a, symbol_b) = (symbol(component), symbol(other_component));
    let wires: Vec<_> = s.project().unwrap().wires().iter().map(|wire| wire.id()).collect();
    let board = view(&s).target.board;
    s.apply(
        CONNECT_PINS,
        &json!({"from":{"symbol":symbol_a,"terminal":terminals.0},"to":{"symbol":symbol_b,"terminal":terminals.1},"expected_revision":s.revision()}),
    )
    .unwrap();
    let added: Vec<_> = s.project().unwrap().wires().iter().map(|wire| wire.id()).filter(|id| !wires.contains(id)).collect();
    assert!(!added.is_empty());
    accept_update(&mut s);
    let layers = copper_layers(&s);
    match bridge {
        Bridge::Track => {
            s.apply(ADD_TRACKS, &json!({"board":board,"layer":layers[0],"width_nm":250_000,"points":[a,b],"expected_revision":s.revision()}))
                .unwrap();
        }
        Bridge::Vias => {
            let y = a.y_nm + 2_000_000;
            let points = [a, Point { x_nm: a.x_nm, y_nm: y }, Point { x_nm: b.x_nm, y_nm: y }, b];
            s.apply(
                ADD_TRACKS,
                &json!({"board":board,"layer":layers[0],"width_nm":250_000,"points":points,
                "layer_changes":[{"at":1,"layer":layers[1]},{"at":2,"layer":layers[0]}],
                "via_diameter_nm":600_000,"via_drill_nm":300_000,"expected_revision":s.revision()}),
            )
            .unwrap();
        }
        Bridge::Pads => {
            s.apply(
                SET_FOOTPRINT_TRANSFORM,
                &json!({"board":board,"footprint":moved.0,"x_nm":moved.1.x_nm + b.x_nm - a.x_nm,
                "y_nm":moved.1.y_nm + b.y_nm - a.y_nm,"rotation":moved.2,"expected_revision":s.revision()}),
            )
            .unwrap();
        }
    }
    assert_eq!(shorts(&s, 0).len(), 0, "same intended net is a clean control before the split");
    for wire in added {
        s.apply(DELETE_WIRE, &json!({"wire":wire,"expected_revision":s.revision()})).unwrap();
    }
    assert_eq!(shorts(&s, 0).len(), 0, "unaccepted schematic splits do not change board intent");
    accept_update(&mut s);
    s
}

/// Select shorts without hiding unrelated DRC diagnostics; zero clearance still detects contact.
fn shorts(s: &Session, clearance_nm: i64) -> Vec<Violation> {
    view(s).check_design(Rules { clearance_nm, ..Rules::default() }).violations.into_iter().filter(|v| v.kind == Kind::Short).collect()
}

/// Preserve exact history snapshot identities rather than checking only whether undo is possible.
fn history(s: &Session) -> [Vec<Option<*const crate::Project>>; 2] {
    [&s.undo, &s.redo].map(|stack| stack.iter().map(|entry| entry.as_ref().map(std::sync::Arc::as_ptr)).collect())
}

/// Connected copper must not hide the distinct intended nets joined by an existing bridge.
#[test]
fn track_bridge_between_intended_nets_reports_a_short() {
    let s = bridged_split(Bridge::Track);
    for clearance in [0, DEFAULT_CLEARANCE_NM] {
        let violations = shorts(&s, clearance);
        assert_eq!(violations.len(), 1, "{violations:#?}");
        let short = &violations[0];
        assert_eq!(short.nets.len(), 2);
        assert!(short.nets.windows(2).all(|pair| pair[0].net < pair[1].net));
        assert_eq!(short.nets.iter().map(|net| net.terminals.len()).sum::<usize>(), 2);
        let track = view(&s).tracks()[0].id;
        assert!(short.items.contains(&track.to_string()));
        assert_eq!(short.items.len(), 3, "both pads and their bridge are actionable");
        assert!(short.items.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(short.nets.iter().all(|net| short.message.contains(&net.name)));
    }
}

/// Cross-layer contacts must report the whole shorted island, including both through vias.
#[test]
fn via_mediated_shorts_include_tracks_vias_and_terminal_memberships() {
    let s = bridged_split(Bridge::Vias);
    assert_eq!(view(&s).vias().len(), 2);
    let violations = shorts(&s, 0);
    assert_eq!(violations.len(), 1, "{violations:#?}");
    let items = &violations[0].items;
    for id in view(&s).tracks().iter().map(|track| track.id).chain(view(&s).vias().iter().map(|via| via.id)) {
        assert!(items.contains(&id.to_string()));
    }
    assert_eq!(items.len(), 7, "two pads, three tracks and two vias");
}

/// Direct pad contact has no track/via node to trigger the old clearance/island check.
#[test]
fn direct_pad_contact_reports_a_short_without_tracks_or_vias() {
    let s = bridged_split(Bridge::Pads);
    assert!(view(&s).tracks().is_empty() && view(&s).vias().is_empty());
    let violations = shorts(&s, 0);
    assert_eq!(violations.len(), 1, "{violations:#?}");
    assert_eq!(violations[0].items.len(), 2);
    assert_eq!(violations[0].nets.len(), 2);
    assert_eq!(violations[0].nets.iter().map(|net| net.terminals.len()).sum::<usize>(), 2);
}

/// Existing shorts stay diagnosable without blocking unrelated edits. Malformed paths, stale
/// authority and new contacts into the bad island preserve bytes, revision and both history stacks.
#[test]
fn existing_shorts_preserve_scoped_edit_rejection_and_atomic_history() {
    let mut s = bridged_split(Bridge::Track);
    let front = copper_layers(&s)[0];
    let bad_track = view(&s).tracks()[0].clone();
    let before = crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let reported = shorts(&s, 0);
    let start = Point { x_nm: 0, y_nm: 30_000_000 };
    let end = Point { x_nm: 5_000_000, y_nm: 30_000_000 };
    let board = view(&s).target.board;
    s.apply(ADD_TRACKS, &json!({"board":board,"layer":front,"width_nm":250_000,"points":[start,end],"expected_revision":s.revision()})).unwrap();
    assert_eq!(shorts(&s, 0), reported);
    let after = crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), before);
    let revision = s.revision();
    let stacks = history(&s);
    for params in [
        json!({"board":board,"layer":front,"points":[start],"width_nm":250_000,"expected_revision":revision}),
        json!({"board":board,"layer":front,"points":[start,end],"width_nm":250_000,"expected_revision":revision-1}),
        json!({"board":Uuid::from_u128(5),"layer":front,"points":[start,end],"width_nm":250_000,"expected_revision":revision}),
        json!({"board":board,"layer":front,"points":[bad_track.start,Point { x_nm:bad_track.start.x_nm,y_nm:bad_track.start.y_nm+2_000_000 }],"width_nm":250_000,"expected_revision":revision}),
    ] {
        assert!(s.apply(ADD_TRACKS, &params).is_err());
        assert_eq!(s.revision(), revision);
        assert_eq!(history(&s), stacks);
        assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), before);
    }
    s.apply("edit.redo", &json!({})).unwrap();
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), after);
}

/// GUI's typed check and the shared command report identical ordered findings without mutation.
/// Reopening production-encoded bytes preserves the report, including accepted terminal keys.
#[test]
fn short_reports_are_deterministic_read_only_and_survive_reopen() {
    let mut s = bridged_split(Bridge::Vias);
    let board = view(&s).target.board;
    let before = crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap();
    let report = view(&s).check_design(Rules::default());
    let revision = s.revision();
    let stacks = history(&s);
    let response = s.execute(CHECK_DESIGN, &json!({"board":board})).unwrap();
    assert_eq!(response["design_check"], serde_json::to_value(&report).unwrap());
    assert_eq!(response["design_check"]["counts"]["short"], 1);
    assert_eq!(s.revision(), revision);
    assert_eq!(history(&s), stacks);
    assert_eq!(crate::persistence::encode_documents(s.snapshot().unwrap()).unwrap(), before);
    let reopened = crate::persistence::decode_documents(&before).unwrap();
    assert_eq!(reopened.board_view(view(&s).target).unwrap().check_design(Rules::default()), report);
    assert_eq!(view(&s).check_design(Rules::default()), report);
    let terminal_keys: BTreeSet<_> =
        report.violations.iter().filter(|v| v.kind == Kind::Short).flat_map(|v| &v.nets).flat_map(|net| &net.terminals).collect();
    assert_eq!(terminal_keys.len(), 2);
}

/// Place overlapping native footprints before naming their isolated logical pins. Identical local
/// names on different sheets remain distinct nets; identical global names join them deliberately.
pub(crate) fn coincident_named_pads(count: usize, style: &str, opposite_side: bool) -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({"create_board":true})).unwrap();
    let board = s.project().unwrap().board_target().unwrap().board;
    s.apply(crate::INITIALIZE_BOARD_SETUP, &json!({"board":board,"copper_layers":2,"expected_revision":s.revision()})).unwrap();
    let mut pins = Vec::new();
    for index in 0..count {
        if index > 0 {
            s.apply(crate::sheets::CREATE_SHEET, &json!({"name":format!("Remote {index}"),"expected_revision":s.revision()})).unwrap();
        }
        let sheet = s.project().unwrap().sheets().last().unwrap().target;
        s.apply(crate::PLACE_RESISTOR, &json!({"sheet":sheet.id(),"x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
        let projection = s.project().unwrap().for_sheet(sheet).unwrap();
        let symbol = &projection.symbols()[0];
        let at = projection.terminal_position(symbol.terminals()[0]).unwrap();
        let symbol = symbol.id();
        s.apply(
            crate::PLACE_FOOTPRINT,
            &json!({"board":board,"sheet":sheet.id(),"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"1","2":"2"},
            "x_nm":0,"y_nm":0,"expected_revision":s.revision()}),
        )
        .unwrap();
        if index > 0 && opposite_side {
            let footprint = view(&s).footprints().last().unwrap();
            let x = 2 * footprint.pads[0].center.x_nm;
            s.apply(
                SET_FOOTPRINT_TRANSFORM,
                &json!({"board":board,"footprint":footprint.id,"x_nm":x,"y_nm":0,"rotation":0,"side":"back","expected_revision":s.revision()}),
            )
            .unwrap();
        }
        pins.push((sheet, at));
    }
    for (sheet, at) in pins {
        s.apply(
            crate::net_labels::ADD,
            &json!({"sheet":sheet.id(),"name":"LOCAL","style":style,"x_nm":at.x_nm,"y_nm":at.y_nm,"expected_revision":s.revision()}),
        )
        .unwrap();
    }
    s
}

/// Equal display names cannot conceal a short, and three conflicting nets produce one island
/// finding with every terminal. Both unaccepted legacy intent and accepted history are covered.
#[test]
fn snapshot_memberships_distinguish_equal_names_and_report_all_conflicting_nets() {
    let mut s = coincident_named_pads(3, "local", false);
    for accepted in [false, true] {
        if accepted {
            let board = view(&s).target.board;
            s.apply(crate::board::update::INITIALIZE_BASELINE, &json!({"board":board,"expected_revision":s.revision()})).unwrap();
        }
        let violations = shorts(&s, 0);
        assert_eq!(violations.len(), 1, "{violations:#?}");
        assert_eq!(violations[0].nets.len(), 3);
        assert!(violations[0].nets.iter().all(|net| net.name == "LOCAL" && net.terminals.len() == 1));
        assert!(violations[0].nets.windows(2).all(|pair| pair[0].net < pair[1].net));
        assert_eq!(violations[0].items.len(), 3);
    }
}

/// A common global net is intentional contact, while opposite-side SMD pads have no contact.
#[test]
fn same_net_and_separate_copper_layers_do_not_report_shorts() {
    for (style, opposite_side) in [("global", false), ("local", true)] {
        let mut s = coincident_named_pads(2, style, opposite_side);
        let netted: Vec<_> = view(&s).footprints().flat_map(|f| f.pads).filter(|pad| pad.net.is_some()).collect();
        assert_eq!(netted.len(), 2);
        assert_eq!(netted[0].center, netted[1].center, "the control must test actual geometric overlap");
        assert!(shorts(&s, 0).is_empty());
        let board = view(&s).target.board;
        s.apply(crate::board::update::INITIALIZE_BASELINE, &json!({"board":board,"expected_revision":s.revision()})).unwrap();
        assert!(shorts(&s, 0).is_empty());
    }
}
