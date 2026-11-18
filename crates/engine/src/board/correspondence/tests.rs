use super::*;
use crate::{
    DELETE_SYMBOL, PLACE_RESISTOR, PLACE_SYMBOL, PLACE_UNIT,
    board::{
        assignment::{ASSIGN_FOOTPRINT, PLACE_ASSIGNED_FOOTPRINT},
        definition,
        update::{APPLY_UPDATE, INITIALIZE_BASELINE, PREVIEW_UPDATE},
    },
    persistence::{decode_documents, decode_packed, encode_documents, encode_packed},
};
use std::sync::Arc;

/// Original shared-pin symbol and an eight-pad package, loaded and placed with native commands.
fn fixture(board: bool) -> Session {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":board})).unwrap();
    let mut package = definition::basic_components("soic8").unwrap().as_ref().clone();
    package.library = "counterparts".into();
    package.id = "mapped-eight".into();
    for (pad, number) in package.pads.iter_mut().zip(["F", "B_2", "A1", "A2", "C", "D", "E", "NC"]) {
        pad.number = number.into();
    }
    session
        .execute(
            "library.load_source",
            &json!({"source":"counterparts","documents":{
                "library.json":r#"{"format":"pcbcraft-library/1","id":"counterparts","name":"Counterpart fixtures","license":"MIT OR Apache-2.0"}"#,
                "symbols/dual.json":include_str!("../../../../../examples/native-parts/symbols/dual-amplifier.json"),
                "footprints/eight.json":serde_json::to_string(&package).unwrap()
            }}),
        )
        .unwrap();
    let symbol = session.libraries.symbol("counterparts", "dual-amplifier", 1).unwrap();
    apply(
        &mut session,
        PLACE_SYMBOL,
        json!({"library":"counterparts","definition":"dual-amplifier","definition_revision":1,
        "definition_digest":crate::library::sources::symbol_digest(&symbol).unwrap(),"x_nm":0,"y_nm":0}),
    );
    session
}

/// Supply the live revision while exercising the production transaction path.
fn apply(session: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

/// Deliberately nonnumeric permutations, two lands for shared terminal 3, and an explicit NC pad.
fn mapping() -> Value {
    json!({"1":"F","2":"B_2","3":["A1","A2"],"4":"C","5":"D","6":"E"})
}

/// Choose a package without placing it; the exact pinned library revision is inspected.
fn assign(session: &mut Session, mapping: Value) {
    let component = session.project().unwrap().components().next().unwrap().id();
    let definition = definition::basic_components("soic8").unwrap();
    apply(
        session,
        ASSIGN_FOOTPRINT,
        json!({"component":component,"library":"counterparts","definition":"mapped-eight",
        "definition_revision":definition.revision,"mapping":mapping,"unconnected_pads":["NC"]}),
    );
}

/// Place both units across sheets and then their one physical package.
fn placed() -> Session {
    let mut session = fixture(true);
    let component = session.project().unwrap().components().next().unwrap().id();
    apply(&mut session, crate::sheets::CREATE_SHEET, json!({"name":"Second"}));
    let sheet = session.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut session, PLACE_UNIT, json!({"sheet":sheet.id(),"component":component,"unit":"b","x_nm":0,"y_nm":0}));
    assign(&mut session, mapping());
    apply(&mut session, PLACE_ASSIGNED_FOOTPRINT, json!({"component":component,"x_nm":20_000_000,"y_nm":0}));
    session
}

/// Explicit ownership is taken from the first placed sheet, never from a displayed name.
fn terminal(project: &Project, number: &str) -> Source {
    let sheet = project.sheets().next().unwrap().target;
    let projection = project.for_sheet(sheet).unwrap();
    let symbol = &projection.symbols()[0];
    Source::Terminal(TerminalTarget { sheet, terminal: Terminal::parse(&json!({"symbol":symbol.id(),"terminal":number})).unwrap() })
}

/// Resolve the real placement ID before naming a pad.
fn pad(project: &Project, number: &str) -> Source {
    let board = project.board_target().unwrap();
    let footprint = project.board.footprints()[0].id;
    Source::Pad(PadTarget { board, pad: Pad::parse(&format!("footprint-{footprint}"), number).unwrap().on_board(board.board) })
}

/// Both directions return every supported land and shared unit appearance, with exact ownership.
#[test]
fn complete_sets_include_multiple_pads_and_unit_appearances() {
    let session = placed();
    let project = session.project().unwrap();
    let source = terminal(project, "3");
    let result = project.resolve_counterparts(source);
    assert_eq!(result.status, Status::Resolved);
    assert_eq!(result.terminals.len(), 2);
    assert_ne!(result.terminals[0].sheet, result.terminals[1].sheet);
    assert_eq!(result.pads.iter().map(|target| target.pad.number.as_str()).collect::<Vec<_>>(), ["A1", "A2"]);
    for endpoint in result.pads.iter().copied().map(Source::Pad).chain(result.terminals.iter().copied().map(Source::Terminal)) {
        let reverse = project.resolve_counterparts(endpoint);
        assert_eq!(reverse.terminals, result.terminals);
        assert_eq!(reverse.pads, result.pads);
        assert_eq!(Source::parse(&endpoint.inspect()).unwrap(), endpoint);
    }
    let single = project.resolve_counterparts(terminal(project, "1"));
    assert_eq!(single.pads[0].pad.number.as_str(), "F");
    assert_eq!(single.terminals.len(), 1);
    assert_eq!(project.pad_for_terminal(result.terminals[0].terminal), Some(result.pads[0].pad), "legacy representative lookup is preserved");
}

/// Unavailable choices are distinct; no owner, matching pad number or coordinate is substituted.
#[test]
fn missing_assignment_placement_nc_and_foreign_targets_are_distinct() {
    let mut session = fixture(false);
    let source = terminal(session.project().unwrap(), "3");
    assert_eq!(session.resolve_counterparts(source, session.revision()).status, Status::NoBoard);
    session = fixture(true);
    let source = terminal(session.project().unwrap(), "3");
    assert_eq!(session.resolve_counterparts(source, session.revision()).status, Status::Unassigned);
    assign(&mut session, mapping());
    assert_eq!(session.resolve_counterparts(source, session.revision()).status, Status::UnplacedPackage);
    let session = placed();
    let project = session.project().unwrap();
    let nc = project.resolve_counterparts(pad(project, "NC"));
    assert_eq!(nc.status, Status::UnconnectedPad);
    assert!(nc.terminals.is_empty());
    assert_eq!(nc.pads.len(), 1);
    assert_eq!(project.resolve_counterparts(pad(project, "Z9")).status, Status::MissingTarget);
    let Source::Terminal(mut wrong) = terminal(project, "3") else { panic!("expected the fixture endpoint kind") };
    wrong.sheet.project = Uuid::from_u128(1);
    assert_eq!(project.resolve_counterparts(Source::Terminal(wrong)).status, Status::ForeignProject);
    wrong.sheet.project = project.id();
    wrong.sheet.sheet = Uuid::from_u128(1);
    assert_eq!(project.resolve_counterparts(Source::Terminal(wrong)).status, Status::ForeignSheet);
    let Source::Pad(mut wrong) = pad(project, "A1") else { panic!("expected the fixture endpoint kind") };
    wrong.board.board = Uuid::from_u128(1);
    assert_eq!(project.resolve_counterparts(Source::Pad(wrong)).status, Status::ForeignBoard);
    let Source::Terminal(mut absent) = terminal(project, "3") else { panic!("expected the fixture endpoint kind") };
    absent.terminal = Terminal::parse(&json!({"symbol":"symbol-999999","terminal":"3"})).unwrap();
    assert_eq!(project.resolve_counterparts(Source::Terminal(absent)).status, Status::MissingTarget);
    assert_eq!(Session::default().resolve_counterparts(source, 0).status, Status::NoProject);
}

/// Inspection follows accepted placement mappings even while a different assignment awaits review.
#[test]
fn pending_package_choices_do_not_replace_accepted_correspondence() {
    let mut session = placed();
    apply(&mut session, crate::board::update::INITIALIZE_BASELINE, json!({}));
    let source = terminal(session.project().unwrap(), "3");
    let accepted = session.project().unwrap().resolve_counterparts(source);
    let mut changed = mapping();
    changed["1"] = json!(["A1", "A2"]);
    changed["3"] = json!("F");
    assign(&mut session, changed);
    assert_eq!(session.project().unwrap().resolve_counterparts(source), accepted);
}

/// A desired package added after baseline adoption requires review, not just placement.
#[test]
fn unplaced_counterparts_report_when_update_pcb_is_required() {
    let mut session = fixture(true);
    assign(&mut session, mapping());
    apply(&mut session, INITIALIZE_BASELINE, json!({}));
    let accepted = terminal(session.project().unwrap(), "3");
    let result = session.resolve_counterparts(accepted, session.revision());
    assert_eq!(result.status, Status::UnplacedPackage);
    assert!(result.inspect().get("pending_update").is_none());

    apply(&mut session, PLACE_RESISTOR, json!({"x_nm":10_000_000,"y_nm":0}));
    let project = session.project().unwrap();
    let sheet = project.sheets().next().unwrap().target;
    let projection = project.for_sheet(sheet).unwrap();
    let resistor = projection.symbols().last().unwrap();
    let component = format!("component-{}", resistor.component);
    let source = Source::Terminal(TerminalTarget { sheet, terminal: Terminal::parse(&json!({"symbol":resistor.id(),"terminal":"1"})).unwrap() });
    let result = session.resolve_counterparts(source, session.revision());
    assert_eq!(result.status, Status::Unassigned);
    assert!(result.inspect().get("pending_update").is_none());

    let definition = definition::basic_components("resistor-0402").unwrap();
    apply(
        &mut session,
        ASSIGN_FOOTPRINT,
        json!({"component":component,"library":definition.library,"definition":definition.id,
            "definition_revision":definition.revision,"mapping":{"1":"1","2":"2"}}),
    );
    let before = session.snapshot().unwrap();
    let result = session.resolve_counterparts(source, session.revision());
    assert_eq!(result.status, Status::UnplacedPackage);
    assert_eq!(result.inspect()["pending_update"], true);
    assert_eq!(result.terminals.len(), 1);
    assert!(result.pads.is_empty());
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    let revision = session.revision();
    let placement = json!({"component":component,"x_nm":10_000_000,"y_nm":0,"expected_revision":revision});
    assert!(matches!(
        session.apply(PLACE_ASSIGNED_FOOTPRINT, &placement),
        Err(crate::CommandError::Unavailable(reason)) if reason.contains("reviewed package update")
    ));
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));

    let review = session.execute(PREVIEW_UPDATE, &json!({"expected_revision":revision})).unwrap();
    apply(&mut session, APPLY_UPDATE, json!({"plan_token":review["board_update_preview"]["plan_token"]}));
    let result = session.resolve_counterparts(source, session.revision());
    assert_eq!(result.status, Status::UnplacedPackage);
    assert!(result.inspect().get("pending_update").is_none());
    assert!(result.pads.is_empty());
    apply(&mut session, PLACE_ASSIGNED_FOOTPRINT, placement);
    let result = session.resolve_counterparts(source, session.revision());
    assert_eq!(result.status, Status::Resolved);
    assert_eq!(result.pads.len(), 1);
    assert!(result.inspect().get("pending_update").is_none());
}

/// Read-only queries preserve bytes, snapshot identity and history across edits and offline reopen.
#[test]
fn counterpart_queries_follow_stable_identity_and_reject_stale_revisions() {
    let mut session = placed();
    let source = terminal(session.project().unwrap(), "3");
    let original = session.project().unwrap().resolve_counterparts(source);
    let before = session.snapshot().unwrap();
    let bytes = encode_documents(before.clone()).unwrap();
    let inspection = session.inspect();
    for _ in 0..20 {
        assert_eq!(session.resolve_counterparts(source, session.revision()), original);
    }
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(session.inspect(), inspection);
    assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), bytes);
    let revision = session.revision();
    let symbol = original.terminals[0].terminal.inspect()["symbol"].clone();
    apply(
        &mut session,
        crate::SET_SYMBOL_FIELDS,
        json!({"sheet":original.terminals[0].sheet.id(),"symbol":symbol,"reference":"U77","value":"Changed"}),
    );
    let footprint = original.pads[0].pad.inspect()["footprint"].clone();
    apply(&mut session, crate::SET_FOOTPRINT_TRANSFORM, json!({"footprint":footprint,"x_nm":1_000_000,"y_nm":2_000_000,"rotation":90}));
    assert_eq!(session.resolve_counterparts(source, revision).status, Status::Stale);
    assert_eq!(session.resolve_counterparts(source, session.revision()), original);
    for _ in 0..2 {
        session.apply("edit.undo", &Value::Null).unwrap();
    }
    assert_eq!(session.resolve_counterparts(source, session.revision()), original);
    for _ in 0..2 {
        session.apply("edit.redo", &Value::Null).unwrap();
    }
    let saved = encode_documents(session.snapshot().unwrap()).unwrap();
    let reopened = decode_documents(&saved).unwrap();
    assert_eq!(reopened.resolve_counterparts(source), original);
    assert_eq!(encode_documents(reopened).unwrap(), saved);
    assert_eq!(decode_packed(&encode_packed(session.snapshot().unwrap()).unwrap()).unwrap().project.resolve_counterparts(source), original);
    let revision = session.revision();
    session.open_snapshot(session.snapshot().unwrap());
    assert_eq!(session.resolve_counterparts(source, revision).status, Status::Stale);
}

/// Deleted appearances and accepted physical remnants remain explicit unavailable results.
#[test]
fn unplaced_and_deleted_logical_counterparts_do_not_fall_back_to_an_owner() {
    let mut session = placed();
    apply(&mut session, crate::board::update::INITIALIZE_BASELINE, json!({}));
    let source = pad(session.project().unwrap(), "F");
    let first = session.project().unwrap().sheets().next().unwrap().target;
    let symbol = session.project().unwrap().for_sheet(first).unwrap().symbols()[0].id();
    apply(&mut session, DELETE_SYMBOL, json!({"sheet":first.id(),"symbol":symbol}));
    assert_eq!(session.project().unwrap().resolve_counterparts(source).status, Status::UnplacedSymbol);
    let second = session.project().unwrap().sheets().nth(1).unwrap().target;
    let symbol = session.project().unwrap().for_sheet(second).unwrap().symbols()[0].id();
    apply(&mut session, DELETE_SYMBOL, json!({"sheet":second.id(),"symbol":symbol}));
    assert_eq!(session.project().unwrap().resolve_counterparts(source).status, Status::UnresolvedMapping);
}

/// A complete result is not truncated when one terminal maps to nearly all supported package pads.
#[test]
fn complete_sets_reach_the_supported_pad_limit() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    apply(&mut session, crate::PLACE_RESISTOR, json!({"x_nm":0,"y_nm":0}));
    let mut package = definition::basic_components("soic8").unwrap().as_ref().clone();
    package.library = "pad-bound".into();
    package.id = "many-lands".into();
    let prototype = package.pads[0].clone();
    package.pads = (0..128)
        .map(|index| {
            let mut pad = prototype.clone();
            pad.number = format!("P{index:03}");
            pad.center.x_nm = index * 100_000;
            pad
        })
        .collect();
    package.validate().unwrap();
    session
        .execute(
            "library.load_source",
            &json!({"source":"pad-bound","documents":{
                "library.json":r#"{"format":"pcbcraft-library/1","id":"pad-bound","name":"Original capacity fixture","license":"MIT OR Apache-2.0"}"#,
                "footprints/many.json":serde_json::to_string(&package).unwrap()
            }}),
        )
        .unwrap();
    let component = session.project().unwrap().components().next().unwrap().id();
    apply(
        &mut session,
        ASSIGN_FOOTPRINT,
        json!({"component":component,"library":package.library,"definition":package.id,
        "definition_revision":package.revision,"mapping":{"1":(0..127).map(|index|format!("P{index:03}")).collect::<Vec<_>>(),"2":"P127"}}),
    );
    apply(&mut session, PLACE_ASSIGNED_FOOTPRINT, json!({"component":component,"x_nm":0,"y_nm":0}));
    let project = session.project().unwrap();
    let result = project.resolve_counterparts(terminal(project, "1"));
    assert_eq!(result.status, Status::Resolved);
    assert_eq!(result.pads.len(), 127);
    assert_eq!(result.pads[0].pad.number.as_str(), "P000");
    assert_eq!(result.pads[126].pad.number.as_str(), "P126");
    assert_eq!(project.resolve_counterparts(pad(project, "P126")).terminals, result.terminals);
    let reopened = decode_documents(&encode_documents(session.snapshot().unwrap()).unwrap()).unwrap();
    assert_eq!(reopened.resolve_counterparts(result.source), result);
}
