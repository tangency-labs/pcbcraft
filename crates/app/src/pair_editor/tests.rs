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

/// The board harness and its persistent net choices as (ID, chooser label).
type Fixture = (Harness<'static, PcbApp>, Vec<(Uuid, String)>);

/// Create a board with three independent, command-authored persistent nets for the two clients.
fn fixture() -> Result<Fixture, String> {
    let mut app = PcbApp::default();
    app.start_new_project_in(crate::Workspace::Board);
    let sheet = app.session.project().ok_or("new project is open")?.sheet_target().id();
    let pins = (0..6)
        .map(|index| -> Result<serde_json::Value, String> {
            app.run(
                pcbcraft_engine::PLACE_RESISTOR,
                &json!({"sheet":sheet,"x_nm":index*25_400_000,"y_nm":0,"expected_revision":app.session.revision()}),
            );
            assert!(app.message.is_none(), "{:?}", app.message);
            let symbol = app.session.project().and_then(|project| project.symbols().last()).ok_or("placed resistor")?.id();
            Ok(json!({"symbol":symbol,"terminal":"1"}))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for pair in pins.as_chunks::<2>().0 {
        app.run(pcbcraft_engine::CONNECT_PINS, &json!({"sheet":sheet,"from":pair[0],"to":pair[1],"expected_revision":app.session.revision()}));
        assert!(app.message.is_none(), "{:?}", app.message);
        app.run(nets::PROMOTE, &json!({"sheet":sheet,"anchor":pair[0],"expected_revision":app.session.revision()}));
        assert!(app.message.is_none(), "{:?}", app.message);
    }
    // A published pair repaints every editor window and the main window, which can take more
    // than the default four steps to settle; other whole-app harnesses use the same bound.
    let mut h = Harness::builder().with_max_steps(64).with_size(egui::vec2(1200., 900.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    let registry = h.state().session.project().ok_or("project stays open")?.inspect_net_registry()?;
    let choices: Vec<_> =
        crate::board_net_choice::net_choices(&registry).into_iter().filter_map(|c| c.id.map(|id| (id, choice_label(&c.label, id)))).collect();
    assert_eq!(choices.len(), 3);
    Ok((h, choices))
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
fn mcp(app: &mut PcbApp, command: &str, mut params: serde_json::Value) -> Result<serde_json::Value, String> {
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
            return Ok(value["result"].clone());
        }
    }
    Err(format!("live command did not complete: {command}"))
}

/// Test frames advance one deterministic snapshot batch each.
fn wait_for_pair(h: &mut Harness<'_, PcbApp>, expected_revision: u64) -> Result<(), String> {
    for _ in 0..100 {
        if h.state().session.revision() == expected_revision {
            h.run();
            return Ok(());
        }
        h.run();
    }
    Err(format!("pair command did not publish: {:?}", h.state().dialog))
}

/// Compare native working documents, not only a view or a catalog projection.
fn bytes(app: &PcbApp) -> Result<Documents, String> {
    encode_documents(app.session.snapshot().ok_or("project snapshot")?)
}

/// Create, update and remove through actual GUI controls and the live MCP adapter. Replay the
/// GUI's allocated UUID in MCP to compare identity-bearing bytes after every action, then undo
/// one deletion in each client and recover the exact bytes from immediately before deletion.
#[test]
fn pair_editor_and_live_mcp_match_project_bytes_and_one_undo() {
    let (mut h, choices) = fixture().unwrap();
    let [(p, positive), (n, negative), (spare, spare_name)] = choices.as_slice() else { panic!("expected three nets") };
    let mut live = PcbApp::default();
    live.session.open_snapshot(h.state().session.snapshot().unwrap());
    let initial = bytes(h.state()).unwrap();
    open(&mut h);
    name(&mut h, "Data");
    select(&mut h, "Positive net", positive);
    select(&mut h, "Negative net", negative);
    let revision = h.state().session.revision();
    click(&mut h, "Save pair");
    wait_for_pair(&mut h, revision + 1).unwrap();
    let pair = h.state().session.project().unwrap().differential_pairs()[0].id;
    let create = json!({"pair":pair,"name":"Data","positive":p,"negative":n});
    let mut preview = create.clone();
    preview["dry_run"] = json!(true);
    let proposed = mcp(&mut live, nets::CREATE_PAIR, preview).unwrap();
    assert_eq!(proposed["pair_change"]["after"]["id"], json!(pair));
    assert_eq!(bytes(&live).unwrap(), initial);
    mcp(&mut live, nets::CREATE_PAIR, create).unwrap();
    assert_eq!(bytes(h.state()).unwrap(), bytes(&live).unwrap());

    open(&mut h);
    select(&mut h, "Existing pair", &choice_label("Data", pair));
    name(&mut h, "Read data");
    select(&mut h, "Negative net", spare_name);
    let revision = h.state().session.revision();
    click(&mut h, "Save pair");
    wait_for_pair(&mut h, revision + 1).unwrap();
    mcp(&mut live, nets::UPDATE_PAIR, json!({"pair":pair,"name":"Read data","positive":p,"negative":spare})).unwrap();
    let edited = bytes(h.state()).unwrap();
    assert_eq!(edited, bytes(&live).unwrap());

    open(&mut h);
    select(&mut h, "Existing pair", &choice_label("Read data", pair));
    let revision = h.state().session.revision();
    click(&mut h, "Remove pair");
    wait_for_pair(&mut h, revision + 1).unwrap();
    mcp(&mut live, nets::REMOVE_PAIR, json!({"pair":pair})).unwrap();
    assert_eq!(bytes(h.state()).unwrap(), initial);
    assert_eq!(bytes(h.state()).unwrap(), bytes(&live).unwrap());
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run();
    mcp(&mut live, "edit.undo", json!({})).unwrap();
    assert_eq!(bytes(h.state()).unwrap(), edited);
    assert_eq!(bytes(h.state()).unwrap(), bytes(&live).unwrap());
}

/// Submit a cold pair via the editor's browser queue. Observe actual native writer work for
/// several frame budgets on a large opened project, then revoke it while Save is still pending.
#[test]
fn large_browser_pair_capacity_yields_and_cancels_during_native_save() {
    use crate::board_drc::jobs::{Queue, Request, Work};
    use pcbcraft_engine::persistence::{ENTRY_POINT, decode_documents};

    const EXTRA_PARTS: u64 = 1024;
    let (h, choices) = fixture().unwrap();
    let mut docs = bytes(h.state()).unwrap();
    let mut project: serde_json::Value = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
    let sheet_path = docs.keys().find(|path| path.starts_with("sheets/")).unwrap().clone();
    let mut sheet: serde_json::Value = serde_json::from_slice(&docs[&sheet_path]).unwrap();
    let part = project["components"][0].clone();
    let symbol = sheet["symbols"][0].clone();
    for i in 0..EXTRA_PARTS {
        let mut component = part.clone();
        let instance = 10_000 + 2 * i;
        component["id"] = json!(instance);
        component["reference_number"] = json!(100 + i);
        component["reference"] = json!(format!("R{}", 100 + i));
        project["components"].as_array_mut().unwrap().push(component);
        let mut placement = symbol.clone();
        placement["id"] = json!(instance + 1);
        placement["component"] = json!(instance);
        placement["x_nm"] = json!((i % 32) * 25_400_000);
        placement["y_nm"] = json!((i / 32 + 1) * 25_400_000);
        sheet["symbols"].as_array_mut().unwrap().push(placement);
    }
    project["numeric_id_high_water"] = json!(10_000 + 2 * EXTRA_PARTS);
    docs.insert(ENTRY_POINT.into(), serde_json::to_vec(&project).unwrap());
    docs.insert(sheet_path, serde_json::to_vec(&sheet).unwrap());
    let mut session = pcbcraft_engine::Session::default();
    session.open_snapshot(decode_documents(&docs).unwrap());
    let original = session.snapshot().unwrap();
    assert_eq!(original.components().count(), 6 + EXTRA_PARTS as usize);
    let revision = session.revision();
    let pair = nets::new_pair_id().unwrap();
    let params = json!({"pair":pair,"name":"Cold pair","positive":choices[0].0,"negative":choices[1].0,"expected_revision":revision});
    let mut queue = Queue::default();
    queue.submit(Request { id: 1, job: session.prepare_pair_edit(nets::CREATE_PAIR, &params).unwrap() });
    let per_poll = <nets::PairEditJob as Work>::BROWSER_BATCH;
    let mut measured = 0;
    for _ in 0..64 {
        assert!(queue.poll().is_none());
        let progress = queue.progress().unwrap();
        if progress.stage == "Encoding native pair documents" {
            assert!(progress.completed - measured <= per_poll);
            measured = progress.completed;
        }
        if measured >= per_poll * 16 {
            break;
        }
    }
    assert_eq!(measured, per_poll * 16);
    assert!(measured < EXTRA_PARTS as usize);
    queue.cancel();
    let revoked = queue.poll().expect("cancellation returns on the next frame");
    assert_eq!(revoked.id, 1);
    assert!(revoked.report.unwrap().is_none());
    assert_eq!(session.revision(), revision);
    assert!(Arc::ptr_eq(&original, &session.snapshot().unwrap()));
    assert!(session.project().unwrap().differential_pairs().is_empty());

    queue.submit(Request { id: 2, job: session.prepare_pair_edit(nets::CREATE_PAIR, &params).unwrap() });
    let complete = (0..4096).find_map(|_| queue.poll()).expect("fresh cold work finishes");
    let prepared = complete.report.unwrap().unwrap().unwrap();
    assert!(Arc::ptr_eq(&original, &session.snapshot().unwrap()));
    session.complete_pair_edit(prepared, false).unwrap();
    assert_eq!(session.revision(), revision + 1);
    let final_docs = encode_documents(session.snapshot().unwrap()).unwrap();
    assert_eq!(decode_documents(&final_docs).unwrap().differential_pairs()[0].id, pair);
}

/// An existing pair adds controls at the top; a short host can still scroll to edit the
/// lower picker or discard the draft while the fixed footer stays visible.
#[test]
fn existing_pair_can_edit_lower_controls_in_short_host() {
    let (mut h, choices) = fixture().unwrap();
    let pair = nets::new_pair_id().unwrap();
    mcp(h.state_mut(), nets::CREATE_PAIR, json!({"pair":pair,"name":"Data","positive":choices[0].0,"negative":choices[1].0})).unwrap();
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
    let (h, choices) = fixture().unwrap();
    let mut live = PcbApp::default();
    live.session.open_snapshot(h.state().session.snapshot().unwrap());
    let before = bytes(&live).unwrap();
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
    assert_eq!(bytes(&live).unwrap(), before);
}

/// A cold first-pair request must yield and cannot publish if the peer disconnects or the
/// project changes before the deferred result reaches its owner.
#[test]
fn pair_editor_live_cold_work_rejects_revoked_or_stale_requests() {
    use std::sync::{atomic::Ordering, mpsc::TryRecvError};
    let (h, choices) = fixture().unwrap();
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
            mcp(&mut live, "edit.undo", json!({})).unwrap();
        }
        assert_eq!(bytes(&live).unwrap(), encode_documents(original).unwrap());
    }
}

/// Live MCP respects modal ownership. If the session revision changes independently, the open
/// draft retains its fields and cannot submit into the newer revision.
#[test]
fn pair_form_respects_modal_ownership_and_expires_on_revision_change() {
    let (mut h, choices) = fixture().unwrap();
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

/// The Rules dialog renders the NeedsReview conflict banner with an undoable Mark reviewed
/// action (enabled only while both endpoints are live), keeps retained endpoints selectable
/// on the selected pair, and filters retired and already-paired nets out of New pair choosers.
#[test]
fn pair_editor_needs_review_mark_reviewed_and_chooser_filtering() {
    let (mut h, choices) = fixture().unwrap();
    let pair = nets::new_pair_id().unwrap();
    let (positive, negative, spare) = (choices[0].0, choices[1].0, choices[2].0);
    mcp(h.state_mut(), nets::CREATE_PAIR, json!({"pair":pair,"name":"Data","positive":positive,"negative":negative})).unwrap();

    // Add a third pin to `positive`: both members stay Active, while endpoint membership changes -> NeedsReview.
    let (sheet, sym0, wire1) = {
        let project = h.state().session.project().unwrap();
        (project.sheet_target().id(), project.symbols()[0].id(), project.wires()[1].id())
    };
    let rev = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::PLACE_RESISTOR, &json!({"sheet":sheet,"x_nm":152_400_000,"y_nm":0,"expected_revision":rev}));
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    let extra = h.state().session.project().unwrap().symbols().last().unwrap().id();
    let rev = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::CONNECT_PINS,
        &json!({"sheet":sheet,"from":{"symbol":sym0,"terminal":"1"},"to":{"symbol":extra,"terminal":"1"},"expected_revision":rev}),
    );
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    let live_reason = match &h.state().session.project().unwrap().differential_pairs()[0].state {
        nets::PairState::NeedsReview { reason } => reason.clone(),
        other => panic!("expected NeedsReview after endpoint change, got {other:?}"),
    };
    h.run();

    // Open Rules -> Differential pairs and select "Data": conflict banner visible, Mark reviewed enabled.
    let pair_label = choice_label("Data", pair);
    open(&mut h);
    select(&mut h, "Existing pair", &pair_label);
    h.get_by_label(&format!("Needs review: {live_reason}"));
    assert!(!h.get_by_role_and_label(egui::Role::Button, "Mark reviewed").accesskit_node().is_disabled());

    // Clicking Mark reviewed clears NeedsReview -> Active through the shared undoable edit path.
    let before_review = h.state().session.revision();
    click(&mut h, "Mark reviewed");
    wait_for_pair(&mut h, before_review + 1).unwrap();
    assert_eq!(h.state().session.project().unwrap().differential_pairs()[0].state, nets::PairState::Active {});
    h.state_mut().run("edit.undo", &json!({}));
    assert!(matches!(h.state().session.project().unwrap().differential_pairs()[0].state, nets::PairState::NeedsReview { .. }));
    h.state_mut().run("edit.redo", &json!({}));
    assert_eq!(h.state().session.project().unwrap().differential_pairs()[0].state, nets::PairState::Active {});

    // Retire `negative`: NeedsReview returns, Mark reviewed is disabled, retained endpoint stays selectable.
    let rev = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::DELETE_WIRE, &json!({"wire":wire1,"expected_revision":rev}));
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    let retired_reason = match &h.state().session.project().unwrap().differential_pairs()[0].state {
        nets::PairState::NeedsReview { reason } => reason.clone(),
        other => panic!("expected NeedsReview after retiring negative member, got {other:?}"),
    };
    h.run();

    open(&mut h);
    select(&mut h, "Existing pair", &pair_label);
    h.get_by_label(&format!("Needs review: {retired_reason}"));
    assert!(h.get_by_role_and_label(egui::Role::Button, "Mark reviewed").accesskit_node().is_disabled());
    let retained_label = choice_label("Retained net", negative);
    h.get_by_role_and_label(egui::Role::ComboBox, "Negative net").click();
    h.run();
    h.get_by_role_and_label(egui::Role::Button, &retained_label).click();
    h.run();

    // Switching to New pair filters out both the already-paired positive net and the retired negative net.
    click(&mut h, "New pair");
    h.get_by_role_and_label(egui::Role::ComboBox, "Positive net").click();
    h.run();
    assert!(h.query_by_label(&choices[2].1).is_some(), "expected active unpaired net {spare}");
    assert!(h.query_by_label(&choices[0].1).is_none(), "already-paired positive net must be filtered out");
    assert!(h.query_by_label(&retained_label).is_none(), "retired negative net must be filtered out");
    click(&mut h, &choices[2].1);
    click(&mut h, "Discard pair changes");

    // End on the NeedsReview state so the harness leaves the conflict banner and disabled button visible.
    select(&mut h, "Existing pair", &pair_label);
    h.get_by_label(&format!("Needs review: {retired_reason}"));
    assert!(h.get_by_role_and_label(egui::Role::Button, "Mark reviewed").accesskit_node().is_disabled());
}

/// New pair counts active unpaired nets rather than all retained members, explains cleanly when
/// fewer than two active unpaired nets are available, and keeps existing pairs editable and removable.
#[test]
fn pair_editor_new_pair_explains_fewer_than_two_active_unpaired_nets_while_existing_pair_stays_editable() {
    const NOTICE: &str = "At least two active unpaired nets are needed to create a new pair.";
    let (mut h, choices) = fixture().unwrap();
    let pair = nets::new_pair_id().unwrap();
    let rev = h.state().session.revision();
    h.state_mut().run(nets::CREATE_PAIR, &json!({"pair":pair,"name":"Data","positive":choices[0].0,"negative":choices[1].0,"expected_revision":rev}));
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    h.run();

    // Three active nets exist, but two are already paired, leaving only one active unpaired net.
    open(&mut h);
    h.get_by_label(NOTICE);
    assert!(h.get_by_role_and_label(egui::Role::Button, "Save pair").accesskit_node().is_disabled());

    // Selecting the existing pair hides the new-pair notice and still allows editing and saving.
    let pair_label = choice_label("Data", pair);
    select(&mut h, "Existing pair", &pair_label);
    assert!(h.query_by_label(NOTICE).is_none());
    let before_rename = h.state().session.revision();
    name(&mut h, "DataRenamed");
    click(&mut h, "Save pair");
    wait_for_pair(&mut h, before_rename + 1).unwrap();
    assert_eq!(h.state().session.project().unwrap().differential_pairs()[0].name, "DataRenamed");

    // Reopening and selecting the renamed pair still allows removing it.
    let renamed_label = choice_label("DataRenamed", pair);
    open(&mut h);
    h.get_by_label(NOTICE);
    select(&mut h, "Existing pair", &renamed_label);
    assert!(h.query_by_label(NOTICE).is_none());
    let before_remove = h.state().session.revision();
    click(&mut h, "Remove pair");
    wait_for_pair(&mut h, before_remove + 1).unwrap();
    assert!(h.state().session.project().unwrap().differential_pairs().is_empty());

    // Once all three active nets are unpaired again, New pair shows no shortage notice.
    open(&mut h);
    assert!(h.query_by_label(NOTICE).is_none());
}

/// A deferred pair-edit error completes after the current frame has rendered the busy banner
/// and disabled form controls, so the dialog must request an immediate repaint instead of
/// leaving an idle editor stuck in the busy state.
#[test]
fn deferred_pair_error_requests_repaint_after_rendering_busy_state() {
    let (mut h, choices) = fixture().unwrap();
    open(&mut h);
    name(&mut h, " leading");
    select(&mut h, "Positive net", &choices[0].1);
    select(&mut h, "Negative net", &choices[1].1);
    let revision = h.state().session.revision();
    h.get_by_role_and_label(egui::Role::Button, "Save pair").click();

    let mut saw_error_after_busy = false;
    for _ in 0..32 {
        h.step();
        let finished_with_error = matches!(&h.state().dialog, Dialog::Pairs(edit) if !edit.preparation.busy() && edit.error.is_some());
        if finished_with_error {
            assert!(h.query_by_label("Checking native document capacity…").is_some());
            assert!(h.get_by_role_and_label(egui::Role::Button, "Discard pair changes").accesskit_node().is_disabled());
            assert_eq!(h.output().viewport_output[&egui::ViewportId::ROOT].repaint_delay, Duration::ZERO);
            saw_error_after_busy = true;
            break;
        }
    }
    assert!(saw_error_after_busy, "deferred pair edit did not store an error: {:?}", h.state().dialog);

    h.step();
    assert!(h.query_by_label("Checking native document capacity…").is_none());
    h.get_by_label("Pair names must be trimmed, nonempty, control-free and at most 200 characters.");
    assert!(!h.get_by_role_and_label(egui::Role::Button, "Discard pair changes").accesskit_node().is_disabled());
    assert_eq!(h.state().session.revision(), revision);
    assert!(h.state().session.project().unwrap().differential_pairs().is_empty());
}
