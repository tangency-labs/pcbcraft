//! Available-revision discovery is one shared, non-editing engine query.
use super::{body, call, server};
use crate::{
    Backend, Failure, Headless,
    live::{Editor, call_editor},
};
use pcbcraft_engine::{
    PLACE_SYMBOL, Session,
    authoring::{
        Item, Kind,
        publishing::{new_library, prepare},
    },
    error_codes::ErrorCode,
    library::sources::{AVAILABLE_REVISIONS, LOAD, symbol_digest},
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

/// Loaded revisions produce the same two-of-three report through live and headless execution.
/// Modal ownership is checked before dispatch; both paths preserve project inspection state.
#[test]
fn available_revisions_has_identical_live_and_headless_results() {
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
    assert_eq!(headless.call("project.inspect", json!({})).unwrap(), before);
    assert_eq!(editor.session.inspect(), before);
    assert!(Arc::ptr_eq(&snapshot, &editor.session.snapshot().unwrap()));
    editor.blocked = true;
    assert!(call_editor(&mut editor, &json!({"method":"engine.execute","params":query})).unwrap_err().message().contains("dialog"));
    assert_eq!(editor.session.inspect(), before);
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
