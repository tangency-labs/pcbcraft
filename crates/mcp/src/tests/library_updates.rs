//! Shared live and headless library revision and lifecycle commands.
use super::{body, call, server};
use crate::{
    Backend, Failure, Headless, Server,
    live::{Editor, call_editor, normalize_request},
};
use pcbcraft_engine::{
    ASSIGN_FOOTPRINT, PLACE_ASSIGNED_FOOTPRINT, PLACE_RESISTOR, PLACE_SYMBOL, Session, SessionScope,
    authoring::{
        Item, Kind, Primitive,
        publishing::{new_library, prepare},
    },
    command_api::COMMAND_API_VERSION,
    error_codes::ErrorCode,
    library::{
        parts::Reference,
        sources::{
            AVAILABLE_REVISIONS, Key, Kind as NativeKind, LOAD, PREVIEW_FOOTPRINT_REVISION, WHERE_USED,
            lifecycle::{self, Assertion, Policy, Provenance},
            symbol_digest,
        },
    },
};
use serde_json::{Value, json};
use std::sync::Arc;

/// MCP publishes the empty parameter contract and returns query data even with inspect:false.
#[test]
fn available_revisions_is_discoverable_and_keeps_query_results() {
    let mut server = server();
    let description = body(&call(&mut server, "describe_command", json!({"command":AVAILABLE_REVISIONS})));
    assert_eq!(description["id"], AVAILABLE_REVISIONS);
    assert_eq!(description["params"]["properties"], json!({}));
    let unavailable = call(&mut server, "execute", json!({"command":AVAILABLE_REVISIONS,"params":{},"inspect":false}));
    assert_eq!(unavailable["isError"], true);
    assert!(unavailable["content"][0]["text"].as_str().unwrap().contains("no project is open"));
    body(&call(&mut server, "execute", json!({"command":"project.new","params":{}})));
    let report = body(&call(&mut server, "execute", json!({"command":AVAILABLE_REVISIONS,"params":{},"inspect":false})));
    assert_eq!(report["instances"], json!([]));
    assert_eq!(report["outdated_instances"], json!([]));
}

/// Where-used rejects bad input and unavailable projects with the same stable codes across adapters.
#[test]
fn where_used_preserves_coded_failures_across_live_and_headless() {
    let mut headless = Headless::default();
    let mut editor = OpenEditor { session: Session::default(), blocked: false };
    let params = json!({"library":"demo","kind":"symbol","definition":"drawing","definition_revision":1});
    let mut invalid = params.clone();
    invalid["definition_revision"] = json!(0);
    let mut unexpected = params.clone();
    unexpected["expected_revision"] = json!(1);
    for (params, expected) in [(params, ErrorCode::Unavailable), (invalid, ErrorCode::InvalidParams), (unexpected, ErrorCode::InvalidParams)] {
        let execute = json!({"command":WHERE_USED,"params":params,"inspect":false});
        let headless_error = headless.call("engine.execute", execute.clone()).unwrap_err();
        let live_error = call_editor(&mut editor, &json!({"method":"engine.execute","params":execute})).unwrap_err();
        assert_eq!(headless_error.code(), expected.as_str());
        assert_eq!(headless_error.details(), &json!({}));
        assert_eq!(live_error, headless_error);
    }
}

/// Revision discovery and exact where-used agree across live and headless dispatch.
/// Both preserve project inspection state; modal ownership is checked before dispatch.
#[test]
fn available_revisions_and_where_used_match_live_and_headless() {
    let manifest = new_library("query-demo", "Query demo", "MIT OR Apache-2.0").unwrap();
    let first = prepare(Arc::new(Item::new(Kind::Symbol).unwrap()), &manifest, 0, None, false).unwrap();
    let mut next = first.item().as_ref().clone();
    next.name = "Next drawing".into();
    let second = prepare(Arc::new(next), &manifest, 1, Some(1), false).unwrap();
    let mut documents = first.documents().clone();
    documents.extend(second.documents().clone());
    documents.insert("library.json".into(), manifest);
    let mut setup = Session::default();
    setup.apply("project.new", &json!({"create_board":true})).unwrap();
    setup.execute(LOAD, &json!({"source":"query-demo","documents":documents})).unwrap();
    for draft in [first.item(), first.item(), second.item()] {
        let symbol = setup.libraries().symbol(&draft.library, &draft.id, draft.revision).unwrap();
        setup
            .apply(
                PLACE_SYMBOL,
                &json!({"library":symbol.library,"definition":symbol.id,"definition_revision":symbol.revision(),
            "definition_digest":symbol_digest(&symbol).unwrap(),"x_nm":0,"y_nm":0,"expected_revision":setup.revision()}),
            )
            .unwrap();
    }
    let snapshot = setup.snapshot().unwrap();
    let mut headless = Headless::with_project(snapshot.clone());
    let mut live = Session::default();
    live.open_snapshot(snapshot.clone());
    let mut editor = OpenEditor { session: live, blocked: false };
    let load = json!({"command":LOAD,"params":{"source":"query-demo","documents":documents},"inspect":false});
    headless.call("engine.execute", load.clone()).unwrap();
    call_editor(&mut editor, &json!({"method":"engine.execute","params":load})).unwrap();
    let before = headless.call("project.inspect", json!({})).unwrap();
    assert_eq!(before, editor.session.inspect());
    let query = json!({"command":AVAILABLE_REVISIONS,"params":{},"inspect":false});
    let headless_report = headless.call("engine.execute", query.clone()).unwrap();
    let live_report = call_editor(&mut editor, &json!({"method":"engine.execute","params":query})).unwrap();
    assert_eq!(headless_report, live_report);
    let ids: Vec<_> = snapshot.components().take(2).map(|instance| instance.id()).collect();
    assert_eq!(live_report["outdated_instances"], json!(ids));
    assert_eq!(live_report["board"], json!(snapshot.board_target().unwrap().board));
    let command = body(&call(&mut server(), "describe_command", json!({"command":WHERE_USED})));
    assert_eq!(command["params"]["required"], json!(["library", "kind", "definition", "definition_revision"]));
    let item = first.item();
    let where_used = json!({"command":WHERE_USED,"inspect":false,"params":{
        "library":item.library,"kind":"symbol","definition":item.id,"definition_revision":item.revision}});
    let live_uses = call_editor(&mut editor, &json!({"method":"engine.execute","params":where_used})).unwrap();
    assert_eq!(headless.call("engine.execute", where_used).unwrap(), live_uses);
    assert_eq!(live_uses["instances"].as_array().unwrap().iter().map(|row| row["part_instance"].as_str().unwrap()).collect::<Vec<_>>(), ids);
    assert_eq!(live_uses["newest_revision"], false);
    assert_eq!(headless.call("project.inspect", json!({})).unwrap(), before);
    assert_eq!(editor.session.inspect(), before);
    assert!(Arc::ptr_eq(&snapshot, &editor.session.snapshot().unwrap()));
    editor.blocked = true;
    assert!(call_editor(&mut editor, &json!({"method":"engine.execute","params":query})).unwrap_err().message().contains("dialog"));
    assert_eq!(editor.session.inspect(), before);
}

/// Version-pinned lifecycle commands keep their reports with `inspect:false`; version,
/// stale-revision and modal refusals keep distinct codes and never publish an edit.
#[test]
fn versioned_lifecycle_reports_and_typed_refusals_match_live_and_headless() {
    let manifest = new_library("lifecycle-demo", "Lifecycle demo", "MIT OR Apache-2.0").unwrap();
    let native = prepare(Arc::new(Item::new(Kind::Symbol).unwrap()), &manifest, 0, None, false).unwrap();
    let symbol = native.item().symbol().unwrap();
    let assertion = Assertion {
        format: lifecycle::FORMAT.into(),
        address: Reference { library: symbol.library.clone(), id: "drawing-review".into(), revision: 1 },
        target: Key { library: symbol.library.clone(), kind: NativeKind::Symbol, id: symbol.id.clone(), revision: symbol.revision() },
        scheme: Policy::standard().scheme,
        state: "released".into(),
        reason: "Drawing approved".into(),
        provenance: Provenance { publisher: "Lifecycle demo".into() },
    };
    let mut documents = native.documents().clone();
    documents.insert("library.json".into(), manifest);
    documents.insert("lifecycle-assertions/drawing-review.r1.json".into(), assertion.encode().unwrap());

    let mut setup = Session::default();
    setup.apply("project.new", &json!({})).unwrap();
    let snapshot = setup.snapshot().unwrap();
    let mut headless = Headless::with_project(snapshot.clone());
    let mut live = Session::default();
    live.open_snapshot(snapshot.clone());
    let mut editor = OpenEditor { session: live, blocked: false };
    let opening = editor.session.revision();
    let command = |id: &str, params| json!({"command":id,"params":params,"inspect":false,"version":COMMAND_API_VERSION});
    let execute_live = |editor: &mut OpenEditor, params| {
        let mut request = json!({"method":"engine.execute_versioned","params":params});
        normalize_request(&mut request)?;
        call_editor(editor, &request)
    };
    let execute_both = |h: &mut Headless, e: &mut OpenEditor, request: Value| {
        let result = execute_live(e, request.clone()).unwrap();
        let (headless, _) = h.execute(request).unwrap();
        assert_eq!(headless, result);
        result
    };
    execute_both(&mut headless, &mut editor, command(LOAD, json!({"source":"lifecycle-demo","documents":documents})));
    let address = &assertion.address;
    let exact = json!({"library":address.library,"assertion":address.id,"assertion_revision":address.revision});
    let lookup = execute_both(&mut headless, &mut editor, command(lifecycle::LOOKUP, exact.clone()));
    assert_eq!(lookup["assertion"], json!(assertion));
    assert_eq!(lookup["digest"], assertion.digest().unwrap());
    let mut params = exact;
    params["digest"] = lookup["digest"].clone();
    params["expected_revision"] = json!(opening);
    let adopt = command(lifecycle::ADOPT, params.clone());
    params["dry_run"] = json!(true);
    let preview = command(lifecycle::ADOPT, params);
    editor.blocked = true;
    let refused = execute_live(&mut editor, adopt.clone()).unwrap_err();
    assert_eq!(refused.code(), ErrorCode::Unavailable.as_str());
    assert!(Arc::ptr_eq(&snapshot, &editor.session.snapshot().unwrap()));
    editor.blocked = false;
    let mut incompatible = adopt.clone();
    incompatible["version"] = json!(COMMAND_API_VERSION + 1);
    let wrong_version = execute_live(&mut editor, incompatible.clone()).unwrap_err();
    assert_eq!(headless.execute(incompatible).unwrap_err(), wrong_version);
    assert_eq!(wrong_version.code(), ErrorCode::VersionMismatch.as_str());
    assert_eq!(wrong_version.details()["current_version"], COMMAND_API_VERSION);
    assert_eq!(wrong_version.details()["command"], lifecycle::ADOPT);
    assert!(Arc::ptr_eq(&snapshot, &editor.session.snapshot().unwrap()));
    let report = execute_both(&mut headless, &mut editor, preview);
    assert_eq!(report["changed"], true);
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["revision"], opening);
    assert_eq!(report["disposition_after"], "released");
    assert!(Arc::ptr_eq(&snapshot, &editor.session.snapshot().unwrap()));
    let commit = execute_both(&mut headless, &mut editor, adopt.clone());
    assert_eq!(commit["revision"], opening + 1);
    assert_eq!(commit["dry_run"], false);
    let adopted = editor.session.snapshot().unwrap();
    let stale = execute_live(&mut editor, adopt.clone()).unwrap_err();
    assert_eq!(headless.execute(adopt).unwrap_err(), stale);
    assert_eq!(stale.code(), ErrorCode::StaleRevision.as_str());
    assert_eq!(stale.details(), &json!({"current_revision":opening + 1}));
    assert!(Arc::ptr_eq(&adopted, &editor.session.snapshot().unwrap()));
    let inspection = execute_both(&mut headless, &mut editor, command(lifecycle::INSPECT, json!({})));
    assert_eq!(inspection["revision"], opening + 1);
    assert_eq!(inspection["assessments"][0]["assertion"], json!(assertion));
}

/// A read-only headless MCP session answers where-used through the ordinary execute tool in
/// both return modes; project inspection, revision and history stay as opened.
#[test]
fn read_only_headless_where_used_keeps_project_and_history() {
    let mut setup = Session::default();
    setup.apply("project.new", &json!({"create_board":true})).unwrap();
    setup.apply(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":setup.revision()})).unwrap();
    let snapshot = setup.snapshot().unwrap();
    let component = snapshot.components().next().unwrap();
    let symbol = &component.definition;
    let params = json!({"library":symbol.library,"kind":"symbol","definition":symbol.id,"definition_revision":symbol.revision()});
    let mut s = Server::new(Box::new(Headless::with_scope(SessionScope::ReadOnly, Some(snapshot.clone()))));
    let before = body(&call(&mut s, "inspect_project", json!({})));
    assert_eq!(before["scope"], "read_only");
    let mut reports = Vec::new();
    for inspect in [true, false] {
        let result = call(&mut s, "execute", json!({"command":WHERE_USED,"params":params,"inspect":inspect}));
        assert_ne!(result["isError"], true, "{result}");
        reports.push(body(&result));
    }
    assert_eq!(reports[0], reports[1]);
    assert_eq!(reports[0]["instances"].as_array().map(Vec::len), Some(1));
    assert_eq!(reports[0]["instances"][0]["part_instance"], component.id());
    assert_eq!(reports[0]["instances"][0]["reference"], component.reference());
    assert_eq!(reports[0]["revision"], before["revision"]);
    assert_eq!(body(&call(&mut s, "inspect_project", json!({}))), before);
}

/// The catalog exposes the preview, and live and headless clients receive the same geometry
/// rather than a revision-only response when they suppress ordinary project inspection.
/// The same successful result is available through the normal read-only headless MCP tool.
#[test]
fn footprint_revision_preview_is_discoverable_and_retained_for_both_backends() {
    let mut server = server();
    let described = body(&call(&mut server, "describe_command", json!({"command":PREVIEW_FOOTPRINT_REVISION})));
    assert_eq!(described["id"], PREVIEW_FOOTPRINT_REVISION);
    assert_eq!(described["params"]["required"], json!(["board", "part_instance"]));

    let manifest = new_library("pad-query", "Pad query", "MIT OR Apache-2.0").unwrap();
    let symbol = prepare(Arc::new(Item::new(Kind::Symbol).unwrap()), &manifest, 0, None, false).unwrap();
    let first = prepare(Arc::new(Item::new(Kind::Footprint).unwrap()), &manifest, 0, None, false).unwrap();
    let mut next = first.item().as_ref().clone();
    let Primitive::Pad { pad } = next.objects.get_mut(&2).unwrap() else { panic!("expected authored pad 2") };
    pad.center.x_nm += 200_000;
    let second = prepare(Arc::new(next), &manifest, 1, Some(1), false).unwrap();
    let mut documents = symbol.documents().clone();
    documents.extend(first.documents().clone());
    documents.extend(second.documents().clone());
    documents.insert("library.json".into(), manifest);
    let load = json!({"command":LOAD,"params":{"source":"test","documents":&documents},"inspect":false});
    let mut setup = Session::default();
    setup.apply("project.new", &json!({"create_board":true})).unwrap();
    setup.execute(LOAD, &load["params"]).unwrap();
    let draft = symbol.item();
    let symbol = setup.libraries().symbol(&draft.library, &draft.id, draft.revision).unwrap();
    setup
        .apply(
            PLACE_SYMBOL,
            &json!({"library":symbol.library,"definition":symbol.id,"definition_revision":1,
        "definition_digest":symbol_digest(&symbol).unwrap(),"x_nm":0,"y_nm":0,"expected_revision":setup.revision()}),
        )
        .unwrap();
    let part = setup.project().unwrap().components().next().unwrap().id();
    let footprint = first.item().footprint().unwrap();
    setup
        .apply(
            ASSIGN_FOOTPRINT,
            &json!({"component":part,"library":footprint.library,"definition":footprint.id,
        "definition_revision":1,"mapping":{"1":"1","2":"2"},"expected_revision":setup.revision()}),
        )
        .unwrap();
    setup.apply(PLACE_ASSIGNED_FOOTPRINT, &json!({"component":part,"x_nm":0,"y_nm":0,"expected_revision":setup.revision()})).unwrap();
    let snapshot = setup.snapshot().unwrap();
    let mut headless = Headless::with_project(snapshot.clone());
    headless.call("engine.execute", load.clone()).unwrap();
    let mut editor = OpenEditor { session: Session::default(), blocked: false };
    editor.session.open_snapshot(snapshot.clone());
    call_editor(&mut editor, &json!({"method":"engine.execute","params":load})).unwrap();
    let before = editor.session.inspect();
    let query =
        json!({"command":PREVIEW_FOOTPRINT_REVISION,"params":{"board":snapshot.board_target().unwrap().board,"part_instance":part},"inspect":false});
    let headless_report = headless.call("engine.execute", query.clone()).unwrap();
    let live_report = call_editor(&mut editor, &json!({"method":"engine.execute","params":query})).unwrap();
    assert_eq!(headless_report, live_report);
    assert_eq!(live_report["delta"]["pads"][0]["center_shift_nm"], 200_000.0);

    let mut read_only = Headless::with_scope(SessionScope::ReadOnly, Some(snapshot.clone()));
    read_only.install_fixture_source("test", &documents);
    let mut read_only = Server::new(Box::new(read_only));
    let read_only_before = body(&call(&mut read_only, "inspect_project", json!({})));
    assert_eq!(read_only_before["scope"], "read_only");
    assert_eq!(read_only_before["can_undo"], false);
    assert_eq!(read_only_before["can_redo"], false);
    let read_only_report = body(&call(&mut read_only, "execute", query.clone()));
    assert_eq!(read_only_report, headless_report);
    assert_eq!(body(&call(&mut read_only, "inspect_project", json!({}))), read_only_before);

    let mut unknown = query.clone();
    unknown["params"]["part_instance"] = json!("component-9999");
    let headless_error = headless.call("engine.execute", unknown.clone()).unwrap_err();
    let live_error = call_editor(&mut editor, &json!({"method":"engine.execute","params":unknown})).unwrap_err();
    assert_eq!(headless_error.code(), ErrorCode::NotFound.as_str());
    assert_eq!(live_error, headless_error);
    assert_eq!(editor.session.inspect(), before);
    assert!(Arc::ptr_eq(&snapshot, &editor.session.snapshot().unwrap()));
}

/// Exercise the ordinary live ownership adapter without opening an OS window.
struct OpenEditor {
    session: Session,
    blocked: bool,
}
impl Editor for OpenEditor {
    /// All document operations share this session.
    fn session(&mut self) -> &mut Session {
        &mut self.session
    }
    /// Modal UI ownership is respected before query dispatch.
    fn refusal(&self, _: &str) -> Option<String> {
        self.blocked.then(|| "finish the dialog".into())
    }
    /// The fixture registers no window-only commands.
    fn window_commands(&self) -> Vec<Value> {
        vec![]
    }
    /// No window state is needed for document revision discovery.
    fn inspect(&self) -> Value {
        json!({})
    }
    /// Window-only operations remain outside this query.
    fn run_window_command(&mut self, _: &str, _: &Value) -> Result<(), Failure> {
        Err(Failure::new(ErrorCode::Unavailable, "not a window command"))
    }
}
