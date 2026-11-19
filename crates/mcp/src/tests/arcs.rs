use super::*;
use crate::{
    Backend,
    live::{Editor, call_editor},
};
use pcbcraft_engine::{
    board::{arc::SET_ARCS, setup::LayerPurpose},
    persistence::encode_documents,
};

struct OpenEditor(Session);
impl Editor for OpenEditor {
    fn session(&mut self) -> &mut Session {
        &mut self.0
    }
    fn refusal(&self, _command: &str) -> Option<String> {
        None
    }
    fn window_commands(&self) -> Vec<Value> {
        vec![]
    }
    fn inspect(&self) -> Value {
        json!({})
    }
    fn run_window_command(&mut self, _id: &str, _params: &Value) -> Result<(), crate::Failure> {
        Err("not a window command".into())
    }
}

/// Identical UUIDs and revision produce byte-identical authored copper through both adapters.
#[test]
fn circular_arc_diffs_and_native_bytes_match_live_and_headless() {
    let mut setup = Session::default();
    setup.apply("project.new", &json!({"create_board":true})).unwrap();
    setup.apply(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":setup.revision()})).unwrap();
    let board = setup.project().unwrap().board_target().unwrap();
    let layer = setup.project().unwrap().board_view(board).unwrap().layers().into_iter().find(|l| l.purpose == LayerPurpose::Copper).unwrap().id;
    let snapshot = setup.snapshot().unwrap();
    let mut headless = Headless::with_project(snapshot.clone());
    let mut live = Session::default();
    live.open_snapshot(snapshot.clone());
    let mut editor = OpenEditor(live);
    let mut params = json!({"board":board.board,"expected_revision":editor.0.revision(),"dry_run":true,"arcs":[{
        "id":"00000000-0000-0000-0000-000000000001","layer":layer,"width_nm":250000,
        "center":{"x_nm":0,"y_nm":0},"start":{"x_nm":5000000,"y_nm":0},"end":{"x_nm":0,"y_nm":5000000},"direction":"clockwise"}]});
    let version = pcbcraft_engine::command_api::COMMAND_API_VERSION;
    let mismatch = json!({"command":SET_ARCS,"version":version+1,"params":params,"inspect":false});
    let headless_error = headless.call("engine.execute", mismatch.clone()).unwrap_err();
    let live_error = call_editor(&mut editor, &json!({"method":"engine.execute","params":mismatch})).unwrap_err();
    assert_eq!(headless_error, live_error);
    assert_eq!(live_error.code(), "version_mismatch");
    assert_eq!(encode_documents(editor.0.snapshot().unwrap()).unwrap(), encode_documents(snapshot.clone()).unwrap());
    for dry_run in [true, false] {
        params["dry_run"] = json!(dry_run);
        let payload = json!({"command":SET_ARCS,"version":version,"inspect":false,"params":params});
        let h = headless.call("engine.execute", payload.clone()).unwrap();
        let l = call_editor(&mut editor, &json!({"method":"engine.execute","params":payload})).unwrap();
        assert_eq!(h, l);
        assert_eq!(h["arc_tracks"]["after"], params["arcs"]);
        if dry_run {
            assert_eq!(encode_documents(editor.0.snapshot().unwrap()).unwrap(), encode_documents(snapshot.clone()).unwrap());
        }
    }
    let h = headless.call("project.inspect", json!({})).unwrap();
    assert_eq!(h, editor.0.inspect());
    assert_eq!(h["project"]["boards"][0]["arcs"], params["arcs"]);
    let mut replay = Session::default();
    replay.open_snapshot(snapshot);
    replay.apply(SET_ARCS, &params).unwrap();
    assert_eq!(encode_documents(replay.snapshot().unwrap()).unwrap(), encode_documents(editor.0.snapshot().unwrap()).unwrap());
}
