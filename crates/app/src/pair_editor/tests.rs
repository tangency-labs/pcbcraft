//! Real Rules controls and live MCP produce the same differential-pair project bytes.

use super::*;
use crate::tests::{click, click_tab};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use pcbcraft_engine::{
    command_api::COMMAND_API_VERSION,
    persistence::{Documents, encode_documents},
};

/// Create a board with three independent, command-authored persistent nets for the two clients.
fn fixture() -> (Harness<'static, PcbApp>, Vec<(Uuid, String)>) {
    let mut app = PcbApp::default();
    app.start_new_project_in(crate::Workspace::Board);
    let sheet = app.session.project().unwrap().sheet_target().id();
    let pins: Vec<_> = (0..6)
        .map(|index| {
            app.run(
                pcbcraft_engine::PLACE_RESISTOR,
                &json!({"sheet":sheet,"x_nm":index*25_400_000,"y_nm":0,"expected_revision":app.session.revision()}),
            );
            assert!(app.message.is_none(), "{:?}", app.message);
            json!({"symbol":app.session.project().unwrap().symbols().last().unwrap().id(),"terminal":"1"})
        })
        .collect();
    for pair in pins.chunks_exact(2) {
        app.run(pcbcraft_engine::CONNECT_PINS, &json!({"sheet":sheet,"from":pair[0],"to":pair[1],"expected_revision":app.session.revision()}));
        assert!(app.message.is_none(), "{:?}", app.message);
        app.run(nets::PROMOTE, &json!({"sheet":sheet,"anchor":pair[0],"expected_revision":app.session.revision()}));
        assert!(app.message.is_none(), "{:?}", app.message);
    }
    let mut h = Harness::builder().with_size(egui::vec2(1200., 900.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    let registry = h.state().session.project().unwrap().inspect_net_registry().unwrap();
    let choices: Vec<_> =
        crate::board_net_choice::net_choices(&registry).into_iter().filter_map(|c| c.id.map(|id| (id, choice_label(&c.label, id)))).collect();
    assert_eq!(choices.len(), 3);
    (h, choices)
}

/// Enter through the board's actual Analyze ribbon and its Rules form.
fn open(h: &mut Harness<'_, PcbApp>) {
    click_tab(h, "Analyze");
    click(h, "Edit net classes, assignments and board defaults.");
    click(h, "Differential pairs…");
}

/// Choose exact UI rows, including polarity and record selection, through shared combo controls.
fn select(h: &mut Harness<'_, PcbApp>, field: &str, row: &str) {
    h.get_by_role_and_label(egui::Role::ComboBox, field).click();
    h.run();
    click(h, row);
}

/// Drive the ordinary text field with focused input rather than modifying the draft directly.
fn name(h: &mut Harness<'_, PcbApp>, text: &str) {
    h.get_by_role_and_label(egui::Role::TextInput, "Pair name").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.get_by_role_and_label(egui::Role::TextInput, "Pair name").type_text(text);
    h.run();
}

/// Deliver the same catalog command through live MCP, with only the live session's own revision.
fn mcp(app: &mut PcbApp, command: &str, mut params: serde_json::Value) -> serde_json::Value {
    if command != "edit.undo" {
        params["expected_revision"] = json!(app.session.revision());
    }
    let request = json!({"method":"engine.execute","params":{"command":command,"params":params,"inspect":false,"version":COMMAND_API_VERSION}});
    let ctx = egui::Context::default();
    let (reply, _) = app.begin_test_live_request(&ctx, request);
    for _ in 0..100 {
        app.poll_remote_pair(&ctx);
        if let Ok(value) = reply.try_recv() {
            assert!(value.get("error").is_none(), "{value}");
            return value["result"].clone();
        }
    }
    panic!("live command did not complete: {command}");
}

/// Test frames advance one deterministic snapshot batch each.
fn wait_for_pair(h: &mut Harness<'_, PcbApp>, expected_revision: u64) {
    for _ in 0..100 {
        if h.state().session.revision() == expected_revision {
            h.run();
            return;
        }
        h.run();
    }
    panic!("pair command did not publish: {:?}", h.state().dialog);
}

/// Compare native working documents, not only a view or a catalog projection.
fn bytes(app: &PcbApp) -> Documents {
    encode_documents(app.session.snapshot().unwrap()).unwrap()
}

/// Create, update and remove through actual GUI controls and the live MCP adapter. Replay the
/// GUI's allocated UUID in MCP to compare identity-bearing bytes after every action, then undo
/// one deletion in each client and recover the exact bytes from immediately before deletion.
#[test]
fn pair_editor_and_live_mcp_match_project_bytes_and_one_undo() {
    let (mut h, choices) = fixture();
    let [(p, positive), (n, negative), (spare, spare_name)] = choices.as_slice() else { panic!("expected three nets") };
    let mut live = PcbApp::default();
    live.session.open_snapshot(h.state().session.snapshot().unwrap());
    let initial = bytes(h.state());
    open(&mut h);
    name(&mut h, "Data");
    select(&mut h, "Positive net", positive);
    select(&mut h, "Negative net", negative);
    let revision = h.state().session.revision();
    click(&mut h, "Save pair");
    wait_for_pair(&mut h, revision + 1);
    let pair = h.state().session.project().unwrap().differential_pairs()[0].id;
    let create = json!({"pair":pair,"name":"Data","positive":p,"negative":n});
    let mut preview = create.clone();
    preview["dry_run"] = json!(true);
    let proposed = mcp(&mut live, nets::CREATE_PAIR, preview);
    assert_eq!(proposed["pair_change"]["after"]["id"], json!(pair));
    assert_eq!(bytes(&live), initial);
    mcp(&mut live, nets::CREATE_PAIR, create);
    assert_eq!(bytes(h.state()), bytes(&live));

    open(&mut h);
    select(&mut h, "Existing pair", &choice_label("Data", pair));
    name(&mut h, "Read data");
    select(&mut h, "Negative net", spare_name);
    let revision = h.state().session.revision();
    click(&mut h, "Save pair");
    wait_for_pair(&mut h, revision + 1);
    mcp(&mut live, nets::UPDATE_PAIR, json!({"pair":pair,"name":"Read data","positive":p,"negative":spare}));
    let edited = bytes(h.state());
    assert_eq!(edited, bytes(&live));

    open(&mut h);
    select(&mut h, "Existing pair", &choice_label("Read data", pair));
    let revision = h.state().session.revision();
    click(&mut h, "Remove pair");
    wait_for_pair(&mut h, revision + 1);
    mcp(&mut live, nets::REMOVE_PAIR, json!({"pair":pair}));
    assert_eq!(bytes(h.state()), initial);
    assert_eq!(bytes(h.state()), bytes(&live));
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run();
    mcp(&mut live, "edit.undo", json!({}));
    assert_eq!(bytes(h.state()), edited);
    assert_eq!(bytes(h.state()), bytes(&live));
}

/// An existing pair adds controls at the top; a short host can still scroll to edit the
/// lower picker or discard the draft while the fixed footer stays visible.
#[test]
fn existing_pair_can_edit_lower_controls_in_short_host() {
    let (mut h, choices) = fixture();
    let pair = nets::new_pair_id().unwrap();
    mcp(h.state_mut(), nets::CREATE_PAIR, json!({"pair":pair,"name":"Data","positive":choices[0].0,"negative":choices[1].0}));
    open(&mut h);
    select(&mut h, "Existing pair", &choice_label("Data", pair));
    h.set_size(egui::vec2(640., 300.));
    h.run();
    let visible = crate::tests::fully_visible;
    assert!(visible(&h, "Save pair"));
    let discard = "Discard pair changes";
    assert!(!visible(&h, discard));
    h.get_by_role_and_label(egui::Role::ComboBox, "Negative net").scroll_to_me();
    h.run();
    select(&mut h, "Negative net", &choices[2].1);
    assert!(!h.get_by_role_and_label(egui::Role::Button, "Save pair").accesskit_node().is_disabled());
    h.get_by_label(discard).scroll_to_me();
    h.run();
    assert!(visible(&h, discard));
    click(&mut h, discard);
    assert!(h.get_by_role_and_label(egui::Role::Button, "Save pair").accesskit_node().is_disabled());
    assert!(visible(&h, "Save pair"));
    assert_eq!(h.state().session.project().unwrap().differential_pairs()[0].negative, choices[1].0);
}

/// A mismatched pin takes precedence over the revision check, including on an
/// inspection-suppressed dry run. Neither refusal changes the session or its bytes.
#[test]
fn pair_preview_checks_mcp_api_version_before_revision() {
    let (h, choices) = fixture();
    let mut live = PcbApp::default();
    live.session.open_snapshot(h.state().session.snapshot().unwrap());
    let before = bytes(&live);
    let revision = live.session.revision();
    let ctx = egui::Context::default();
    let wrong_version = COMMAND_API_VERSION + 1;
    let mut request = json!({"method":"engine.execute","params":{
        "command":nets::CREATE_PAIR,"inspect":false,"version":wrong_version,"params":{
            "dry_run":true,"expected_revision":revision+1,"pair":nets::new_pair_id().unwrap(),
            "name":"Data","positive":choices[0].0,"negative":choices[1].0
        }
    }});
    let mismatch = live.begin_test_live_request(&ctx, request.clone()).0.try_recv().unwrap();
    assert_eq!(mismatch["code"], "version_mismatch");
    assert_eq!(mismatch["details"], json!({"command":nets::CREATE_PAIR,"stated_version":wrong_version,"current_version":COMMAND_API_VERSION}));
    request["params"]["version"] = json!(COMMAND_API_VERSION);
    let stale = live.begin_test_live_request(&ctx, request).0.try_recv().unwrap();
    assert_eq!(stale["code"], "stale_revision");
    assert_eq!(stale["details"], json!({"current_revision":revision}));
    assert_eq!(live.session.revision(), revision);
    assert_eq!(bytes(&live), before);
}

/// A cold first-pair request must yield and cannot publish if the peer disconnects or the
/// project changes before the deferred result reaches its owner.
#[test]
fn pair_editor_live_cold_work_rejects_revoked_or_stale_requests() {
    use std::sync::{atomic::Ordering, mpsc::TryRecvError};
    let (h, choices) = fixture();
    let ctx = egui::Context::default();
    for (disconnect, expected) in [(true, "cancelled"), (false, "stale_revision")] {
        let mut live = PcbApp::default();
        let original = h.state().session.snapshot().unwrap();
        live.session.open_snapshot(original.clone());
        let revision = live.session.revision();
        let request = json!({"method":"engine.execute","params":{"command":nets::CREATE_PAIR,"inspect":false,"params":{
            "pair":nets::new_pair_id().unwrap(),"name":"First pair","positive":choices[0].0,"negative":choices[1].0,"expected_revision":revision
        }}});
        let (reply, connected) = live.begin_test_live_request(&ctx, request);
        assert_eq!(reply.try_recv(), Err(TryRecvError::Empty));
        assert_eq!(live.session.revision(), revision);
        live.poll_remote_pair(&ctx);
        assert_eq!(reply.try_recv(), Err(TryRecvError::Empty));
        if disconnect {
            connected.store(false, Ordering::Release);
        } else {
            live.session.apply("project.rename", &json!({"name":"Changed while measuring"})).unwrap();
        }
        for _ in 0..100 {
            live.poll_remote_pair(&ctx);
        }
        let refusal = reply.try_recv().unwrap();
        assert_eq!(refusal["code"], expected, "{refusal}");
        assert_eq!(live.session.revision(), revision + u64::from(!disconnect));
        assert!(live.session.project().unwrap().differential_pairs().is_empty());
        if !disconnect {
            mcp(&mut live, "edit.undo", json!({}));
        }
        assert_eq!(bytes(&live), encode_documents(original).unwrap());
    }
}

/// Live MCP respects modal ownership. If the session revision changes independently, the open
/// draft retains its fields and cannot submit into the newer revision.
#[test]
fn pair_form_respects_modal_ownership_and_expires_on_revision_change() {
    let (mut h, choices) = fixture();
    open(&mut h);
    name(&mut h, "Pending");
    select(&mut h, "Positive net", &choices[0].1);
    select(&mut h, "Negative net", &choices[1].1);
    let before = h.state().session.revision();
    let request = json!({"method":"engine.execute","params":{"command":"project.rename","params":{"name":"Newer project name"}}});
    let ctx = egui::Context::default();
    let refused = pcbcraft_mcp::live::call_editor(&mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx }, &request).unwrap_err();
    assert_eq!(refused.code(), "unavailable");
    assert!(refused.message().contains("current dialog"));
    assert_eq!(h.state().session.revision(), before);
    h.state_mut().session.apply("project.rename", &json!({"name":"Newer project name"})).unwrap();
    h.run();
    let button = h.get_by_role_and_label(egui::Role::Button, "Save pair");
    assert!(button.accesskit_node().is_disabled());
    assert_eq!(h.state().session.revision(), before + 1);
    assert!(h.state().session.project().unwrap().differential_pairs().is_empty());
}
