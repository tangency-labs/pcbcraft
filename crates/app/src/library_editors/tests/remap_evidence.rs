use super::*;
use pcbcraft_engine::{self as engine, authoring::remap::PinRemap, error_codes::ErrorCode, library::sources};
use pcbcraft_mcp::Failure;

/// Synthetic project edits retain the production command's runtime revision check.
fn edit(app: &mut PcbApp, command: &str, mut params: Value) -> Value {
    params["expected_revision"] = json!(app.session.revision());
    app.session.execute(command, &params).unwrap()
}

/// Publish/load a native definition, bind its editor when requested, and place two copies.
/// Optional wires and global labels give the two terminals distinct literal net names.
fn fixture(wired: bool, associated: bool) -> (PcbApp, u64) {
    let original = Item::new(Kind::Symbol).unwrap();
    let manifest = engine::authoring::publishing::new_library("remap-live", "Remap live", &original.license).unwrap();
    let prepared = engine::authoring::publishing::prepare(Arc::new(original), &manifest, 0, None, false).unwrap();
    let mut documents = prepared.documents().clone();
    documents.insert("library.json".into(), manifest);
    let item = prepared.item();
    let mut app = PcbApp::default();
    app.session.execute("project.new", &json!({})).unwrap();
    app.session.execute(sources::LOAD, &json!({"source":"remap-live","documents":documents})).unwrap();
    let symbol = app.session.libraries().symbol(&item.library, &item.id, item.revision).unwrap();
    let digest = sources::symbol_digest(&symbol).unwrap();
    for y in [0, 40_640_000] {
        edit(
            &mut app,
            engine::PLACE_SYMBOL,
            json!({"library":item.library,"definition":item.id,"definition_revision":item.revision,"definition_digest":digest,"x_nm":0,"y_nm":y}),
        );
    }
    if wired {
        let symbols = app.session.project().unwrap().symbols().to_vec();
        for (index, name) in ["INPUT", "OUTPUT"].into_iter().enumerate() {
            edit(
                &mut app,
                engine::CONNECT_PINS,
                json!({"from":symbols[0].terminals()[index].inspect(),"to":symbols[1].terminals()[index].inspect()}),
            );
            let at = symbols[0].pins()[index];
            edit(&mut app, engine::net_labels::ADD, json!({"name":name,"style":"global","x_nm":at.x_nm,"y_nm":at.y_nm}));
        }
    }
    let id = app.library_editors.open(item.as_ref().clone(), true).unwrap();
    if associated {
        app.library_editors.documents.get_mut(&id).unwrap().session.bind_published(Some(symbol));
    }
    (app, id)
}

/// Different native body at the fixture's exact address and manifest attribution.
fn replacement(app: &PcbApp, id: u64) -> sources::Documents {
    let mut item = (*app.library_editors.documents[&id].session.snapshot()).clone();
    item.name = "Replacement body".into();
    let manifest = engine::authoring::publishing::new_library("remap-live", "Remap live", &item.license).unwrap();
    let prepared = engine::authoring::publishing::prepare(Arc::new(item), &manifest, 0, None, false).unwrap();
    let original = app.library_editors.documents[&id].session.published().unwrap();
    assert_eq!((&prepared.item().id, prepared.item().revision), (&original.id, original.revision));
    let mut documents = prepared.documents().clone();
    documents.insert("library.json".into(), manifest);
    documents
}

/// Use the actual live adapter, including its modal refusal, command check and inspection reply.
fn live(app: &mut PcbApp, document: u64, revision: u64, phase: &str, input: Value) -> Result<Value, Failure> {
    let mut params = json!({"document":document.to_string(),"expected_revision":revision.to_string(),"phase":phase});
    params[if phase == "preview" { "pins" } else { "token" }] = input;
    pcbcraft_mcp::live::call_editor(
        &mut crate::live_control::LiveEditor { app, ctx: &Context::default() },
        &json!({"method":"engine.execute","params":{"command":"app.library_pin_remap","params":params}}),
    )
}

/// A different body at the same address conflicts whether or not its replacement source is
/// loaded. Matching pinned evidence, in contrast, remains available offline.
#[test]
fn pin_remap_live_checks_the_opened_body_even_when_the_catalog_agrees_with_another_project_pin() {
    let (mut app, id) = fixture(true, true);
    let other = replacement(&app, id);
    let pins = json!([{"unit":"main","object":"1","number":"7"}]);
    app.session.execute(sources::REMOVE, &json!({"source":"remap-live"})).unwrap();
    let preview = live(&mut app, id, 0, "preview", pins.clone()).unwrap();
    let review = &preview["library_editors"]["pin_remap"];
    let row = &review["report"]["instances"][0]["pins"][0];
    assert_eq!(row["status"], "current");
    assert_eq!(row["current_nets"], json!(["INPUT"]));
    live(&mut app, id, 0, "cancel", review["token"].clone()).unwrap();

    app.session.execute("project.new", &json!({})).unwrap();
    app.session.execute(sources::LOAD, &json!({"source":"remap-live","documents":other})).unwrap();
    let item = app.library_editors.documents[&id].session.snapshot();
    let loaded = app.session.libraries().symbol(&item.library, &item.id, item.revision).unwrap();
    assert_eq!(loaded.name, "Replacement body");
    let digest = sources::symbol_digest(&loaded).unwrap();
    edit(
        &mut app,
        engine::PLACE_SYMBOL,
        json!({"library":item.library,"definition":item.id,"definition_revision":item.revision,"definition_digest":digest,"x_nm":0,"y_nm":0}),
    );
    for offline in [false, true] {
        if offline {
            app.session.execute(sources::REMOVE, &json!({"source":"remap-live"})).unwrap();
        }
        let preview = live(&mut app, id, 0, "preview", pins.clone()).unwrap();
        let review = &preview["library_editors"]["pin_remap"];
        let row = &review["report"]["instances"][0]["pins"][0];
        assert_eq!(row["status"], "conflicting_definition");
        assert_eq!(row["current_nets"], json!([]));
        assert!(row["current_name"].is_null() && row["current_pads"].is_null());
        live(&mut app, id, 0, "cancel", review["token"].clone()).unwrap();
    }
}

#[test]
fn pin_remap_live_catalog_quarantine_requires_refresh_without_revoking_cancel() {
    let (mut app, id) = fixture(true, true);
    let other = replacement(&app, id);
    let pins = json!([{"unit":"main","object":"1","number":"7"}]);
    let project = app.session.snapshot().unwrap();
    let project_revision = app.session.revision();
    let draft = app.library_editors.documents[&id].session.snapshot();
    let before = live(&mut app, id, 0, "preview", pins.clone()).unwrap();
    let review = &before["library_editors"]["pin_remap"];
    assert_eq!(review["report"]["instances"][0]["pins"][0]["status"], "current");
    let token = review["token"].clone();
    app.session.execute(sources::LOAD, &json!({"source":"remap-live","documents":other})).unwrap();
    let key = sources::Key { library: draft.library.clone(), kind: sources::Kind::Symbol, id: draft.id.clone(), revision: draft.revision };
    assert!(app.session.libraries().conflicting(&key));
    let error = live(&mut app, id, 0, "apply", token.clone()).unwrap_err();
    assert!(error.to_string().contains("catalog evidence changed"), "{error}");
    live(&mut app, id, 0, "cancel", token).unwrap();
    let refreshed = live(&mut app, id, 0, "preview", pins).unwrap();
    let review = &refreshed["library_editors"]["pin_remap"];
    assert_eq!(review["report"]["instances"][0]["pins"][0]["status"], "conflicting_definition");
    live(&mut app, id, 0, "cancel", review["token"].clone()).unwrap();
    assert_eq!(app.session.revision(), project_revision);
    assert!(Arc::ptr_eq(&app.session.snapshot().unwrap(), &project));
    let editor = &app.library_editors.documents[&id].session;
    assert_eq!(editor.revision(), 0);
    assert!(!editor.can_undo());
    assert!(Arc::ptr_eq(&editor.snapshot(), &draft));
}

/// Both Assistant settings requests replace the remap form and must retire its live token.
#[test]
fn pin_remap_assistant_settings_retires_the_captured_review() {
    for add in [false, true] {
        let (mut app, id) = fixture(false, true);
        let project = app.session.snapshot().unwrap();
        let project_revision = app.session.revision();
        let draft = app.library_editors.documents[&id].session.snapshot();
        let reply = live(&mut app, id, 0, "preview", json!([{"unit":"main","object":"1","number":"7"}])).unwrap();
        let token = reply["library_editors"]["pin_remap"]["token"].clone();
        assert!(token.is_string());
        assert_eq!(crate::window_commands::inspect_editor(&app)["library_editors"]["pin_remap"]["token"], token);
        app.open_assistant_settings(if add {
            crate::assistant::SettingsRequest::Add(Box::default())
        } else {
            crate::assistant::SettingsRequest::Settings
        });
        assert!(if add { matches!(app.dialog, Dialog::Agent { .. }) } else { matches!(app.dialog, Dialog::Preferences(_)) });
        assert!(crate::window_commands::inspect_editor(&app)["library_editors"]["pin_remap"].is_null());
        assert!(live(&mut app, id, 0, "apply", token.clone()).is_err());
        assert!(live(&mut app, id, 0, "cancel", token).is_err());
        assert_eq!(app.session.revision(), project_revision);
        assert!(Arc::ptr_eq(&project, &app.session.snapshot().unwrap()));
        let editor = &app.library_editors.documents[&id].session;
        assert_eq!(editor.revision(), 0);
        assert!(Arc::ptr_eq(&draft, &editor.snapshot()));
    }
}

/// After one draft swap, a second review distinguishes its IN pin from pinned number 2's OUT
/// evidence. Check the exact table-row projection, not rendered cells or accessibility nodes.
#[test]
fn pin_remap_live_published_wired_project_displays_pinned_number_evidence() {
    let (mut app, id) = fixture(true, true);
    let project = app.session.snapshot().unwrap();
    let state = app.session.inspect();
    let bytes = engine::persistence::encode_documents(project.clone()).unwrap();
    let item = app.library_editors.documents[&id].session.snapshot();
    let swap = json!([{"unit":"main","object":"1","number":"2"},{"unit":"main","object":"3","number":"1"}]);
    let first = live(&mut app, id, 0, "preview", swap).unwrap();
    let review = &first["library_editors"]["pin_remap"];
    assert_eq!(review["report"]["instances"][0]["pins"][0]["current_nets"], json!(["INPUT"]));
    assert_eq!(review["report"]["instances"][0]["pins"][1]["current_nets"], json!(["OUTPUT"]));
    live(&mut app, id, 0, "apply", review["token"].clone()).unwrap();
    assert_eq!(app.library_editors.documents[&id].session.revision(), 1);
    let second =
        live(&mut app, id, 1, "preview", json!([{"unit":"main","object":"1","number":"1"},{"unit":"main","object":"3","number":"2"}])).unwrap();
    let review = &second["library_editors"]["pin_remap"];
    let pin = &review["report"]["instances"][0]["pins"][0];
    assert_eq!(pin["before"], "2");
    assert_eq!(pin["name"], "IN");
    assert_eq!(pin["current_name"], "OUT");
    let rows = super::super::remap::report_rows(&review["report"], 0);
    assert_eq!(&rows[0].cells[1..5], &["2 → 1", "IN", "OUT", "Net: OUTPUT"]);
    live(&mut app, id, 1, "cancel", review["token"].clone()).unwrap();
    assert_eq!(app.library_editors.documents[&id].session.revision(), 1);
    app.library_editors.edit(id, 1, Action::Undo).unwrap();
    assert_eq!(app.library_editors.documents[&id].session.snapshot().native().unwrap(), item.native().unwrap());
    assert!(!app.library_editors.documents[&id].session.can_undo());
    assert_eq!(app.session.inspect(), state);
    assert!(Arc::ptr_eq(&project, &app.session.snapshot().unwrap()));
    assert_eq!(engine::persistence::encode_documents(app.session.snapshot().unwrap()).unwrap(), bytes);
}

/// Bare pins and unnamed one-pin stubs have no net evidence, even though a stub has a graph
/// group. A real net named Unconnected and a missing publication association are distinct cases.
#[test]
fn pin_remap_display_separates_unconnected_and_unknown_association() {
    for (associated, stub) in [(true, false), (true, true), (false, false)] {
        let (mut app, id) = fixture(false, associated);
        if stub {
            let symbol = &app.session.project().unwrap().symbols()[0];
            let pin = symbol.terminals()[0];
            let at = symbol.pins()[0];
            edit(&mut app, engine::DRAW_WIRE, json!({"from":pin.inspect(),"to":{"x_nm":at.x_nm,"y_nm":at.y_nm+12_700_000},"bends":[]}));
            let graph = app.session.project().unwrap().electrical_connectivity();
            let index = graph.net_index_for(pin).expect("stub must have a group");
            assert_eq!(graph.nets[index], [pin]);
            assert_eq!(graph.net_name(index), "Unconnected");
        }
        let reply = live(&mut app, id, 0, "preview", json!([{"unit":"main","object":"1","number":"1"}])).unwrap();
        let report = &reply["library_editors"]["pin_remap"]["report"];
        let instances = report["instances"].as_array().unwrap();
        assert_eq!(instances.len(), 2);
        for instance in instances {
            assert_eq!(instance["pins"][0]["current_nets"], json!([]));
        }
        let rows = super::super::remap::report_rows(report, 0);
        assert_eq!(rows.len(), 2);
        for row in rows {
            assert_eq!(row.cells[3], if associated { "IN" } else { "Unknown" });
            assert_eq!(row.cells[4], if associated { "Unconnected" } else { "Unknown: no matching pinned revision" });
        }
    }
    let (mut app, id) = fixture(false, true);
    let at = app.session.project().unwrap().symbols()[0].pins()[0];
    edit(&mut app, engine::net_labels::ADD, json!({"name":"Unconnected","style":"global","x_nm":at.x_nm,"y_nm":at.y_nm}));
    let reply = live(&mut app, id, 0, "preview", json!([{"unit":"main","object":"1","number":"1"}])).unwrap();
    let report = &reply["library_editors"]["pin_remap"]["report"];
    assert_eq!(report["instances"][0]["pins"][0]["current_nets"], json!(["Unconnected"]));
    assert_eq!(super::super::remap::report_rows(report, 0)[0].cells[4], "Net: Unconnected");
}

/// Production hierarchy, local labels and clipboard batches create many repeated qualified
/// names below the row cap. The common report limit refuses on the headless query and a visible
/// form refresh before replacing the prior authority. This is in-process, not socket evidence.
#[test]
fn pin_remap_qualified_report_refuses_before_live_capture() {
    let (mut app, id) = fixture(false, true);
    let pins = json!([{"unit":"main","object":"1","number":"2"},{"unit":"main","object":"3","number":"1"}]);
    let first = live(&mut app, id, 0, "preview", pins.clone()).unwrap();
    let old_review = first["library_editors"]["pin_remap"].clone();
    let item = app.library_editors.documents[&id].session.snapshot();
    let digest = sources::symbol_digest(&app.session.libraries().symbol(&item.library, &item.id, item.revision).unwrap()).unwrap();
    let root = app.session.project().unwrap().sheet_target();
    let at = app.session.project().unwrap().symbols()[0].pins()[0];
    edit(&mut app, engine::net_labels::ADD, json!({"sheet":root.id(),"name":"DUPLICATE","style":"local","x_nm":at.x_nm,"y_nm":at.y_nm}));
    let mut sheet = root;
    for level in 0..10 {
        edit(&mut app, engine::hierarchy::CREATE_CHILD, json!({"sheet":sheet.id(),"name":format!("{}{level}", "🦆".repeat(199)),"x_nm":0,"y_nm":0}));
        sheet = app.session.project().unwrap().sheets().last().unwrap().target;
    }
    edit(
        &mut app,
        engine::PLACE_SYMBOL,
        json!({"sheet":sheet.id(),"library":item.library,"definition":item.id,"definition_revision":item.revision,"definition_digest":digest,"x_nm":0,"y_nm":0}),
    );
    let symbol = app.session.project().unwrap().for_sheet(sheet).unwrap().symbols()[0].clone();
    for at in symbol.pins() {
        edit(&mut app, engine::net_labels::ADD, json!({"sheet":sheet.id(),"name":"DUPLICATE","style":"local","x_nm":at.x_nm,"y_nm":at.y_nm}));
    }
    while app.session.project().unwrap().for_sheet(sheet).unwrap().symbols().len() < 400 {
        let projection = app.session.project().unwrap().for_sheet(sheet).unwrap();
        let count = projection.symbols().len();
        let ids: Vec<_> = projection.symbols().iter().take((400 - count).min(count)).map(|symbol| symbol.id()).collect();
        let copied = edit(&mut app, engine::clipboard::COPY, json!({"sheet":sheet.id(),"items":{"symbols":ids,"wires":[]}}));
        edit(&mut app, engine::clipboard::PASTE, json!({"sheet":sheet.id(),"text":copied["clipboard"],"x_nm":0,"y_nm":0}));
        assert_eq!(app.session.project().unwrap().for_sheet(sheet).unwrap().symbols().len(), count + ids.len(), "fixture paste at {count} copies");
    }
    let project = app.session.snapshot().unwrap();
    assert_eq!(project.components().count() * 2, 804);
    let terminal = project.for_sheet(sheet).unwrap().symbols()[0].terminals()[0];
    let graph = project.electrical_connectivity();
    assert!(graph.net_name(graph.net_index_for(terminal).unwrap()).len() > 20_000);
    let project_state = app.session.inspect();
    let project_bytes = engine::persistence::encode_documents(project.clone()).unwrap();
    let counter = app.library_editors.next_remap;
    let query = json!({"item":item.as_ref(),"pins":pins,"base_revision":item.revision.to_string()});
    let error = app.session.execute(sources::PREVIEW_PIN_REMAP, &query).unwrap_err();
    let engine::CommandError::InvalidParams(reason) = error else { panic!("wrong refusal: {error}") };
    assert!(reason.contains("report exceeds") && reason.contains("encoded bytes"), "{reason}");
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    crate::tests::click(&mut h, "Preview current nets");
    let reason =
        format!("Pin remap report exceeds {} encoded bytes; review fewer logical pins at once.", engine::authoring::remap::MAX_PREVIEW_BYTES);
    h.get_by_label(reason.as_str());
    assert!(matches!(h.state().dialog, Dialog::LibraryPinRemap(_)));
    assert_eq!(h.state().library_editors.next_remap, counter);
    assert_eq!(h.state().library_editors.inspect()["pin_remap"], old_review);
    assert_eq!(h.state().session.inspect(), project_state);
    assert!(Arc::ptr_eq(&project, &h.state().session.snapshot().unwrap()));
    assert_eq!(engine::persistence::encode_documents(h.state().session.snapshot().unwrap()).unwrap(), project_bytes);
    crate::tests::click(&mut h, "Cancel");
    assert!(h.state().library_editors.pin_remap.is_none());
}

/// The actual host-close handler displaces its remap before either unsaved question. Cancel
/// preserves authored bytes/history but cannot restore an invisible actionable token.
#[test]
fn pin_remap_last_host_close_cancel_retires_authority() {
    for library_unsaved in [false, true] {
        let (mut app, mut id) = fixture(false, true);
        if library_unsaved {
            let session = &app.library_editors.documents[&id].session;
            let item = session.snapshot();
            let symbol = session.published_symbol().cloned();
            app.library_editors.close(id, true).unwrap();
            id = app.library_editors.open(item.as_ref().clone(), false).unwrap();
            app.library_editors.documents.get_mut(&id).unwrap().session.bind_published(symbol);
        }
        let other = app.library_editors.open(Item::new(Kind::Symbol).unwrap(), true).unwrap();
        let project = app.session.snapshot().unwrap();
        let owners = Arc::strong_count(&project);
        let state = app.session.inspect();
        let bytes = engine::persistence::encode_documents(project.clone()).unwrap();
        let documents: Vec<_> =
            app.library_editors.documents.values().map(|doc| (doc.session.inspect(), doc.session.snapshot().encode().unwrap())).collect();
        let reply =
            live(&mut app, id, 0, "preview", json!([{"unit":"main","object":"1","number":"2"},{"unit":"main","object":"3","number":"1"}])).unwrap();
        let token = reply["library_editors"]["pin_remap"]["token"].clone();
        assert_eq!(Arc::strong_count(&project), owners + 1);
        let ctx = Context::default();
        app.close_editor_host(crate::editor_layout::HostId::ROOT, &ctx);
        if library_unsaved {
            assert!(matches!(app.dialog, Dialog::LibraryClose(None)));
        } else {
            assert!(matches!(app.dialog, Dialog::SaveChanges(crate::unsaved::Then::Close)));
        }
        assert!(app.library_editors.pin_remap.is_none());
        assert_eq!(Arc::strong_count(&project), owners);
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        crate::tests::click(&mut h, "Cancel");
        assert!(matches!(h.state().dialog, Dialog::None));
        assert!(!h.state().close_allowed);
        for phase in ["apply", "cancel"] {
            let reason = live(h.state_mut(), id, 0, phase, token.clone()).unwrap_err();
            assert_eq!(reason.code(), ErrorCode::Unavailable.as_str());
            assert!(reason.message().contains(if phase == "apply" { "Preview the pin remap" } else { "No pin remap review" }), "{reason}");
        }
        h.state_mut().begin_library_pin_remap(other).unwrap();
        h.run();
        h.get_by_label("Pin 1").click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Pin 1").type_text("7");
        h.run();
        let form = format!("{:?}", h.state().dialog);
        let refusal = live(h.state_mut(), id, 0, "apply", token).unwrap_err();
        assert_eq!(refusal.code(), ErrorCode::Unavailable.as_str());
        assert!(refusal.message().contains("Preview the pin remap"));
        assert_eq!(format!("{:?}", h.state().dialog), form);
        assert_eq!(
            h.state()
                .library_editors
                .documents
                .values()
                .map(|doc| (doc.session.inspect(), doc.session.snapshot().encode().unwrap()))
                .collect::<Vec<_>>(),
            documents
        );
        assert_eq!(h.state().session.inspect(), state);
        assert_eq!(engine::persistence::encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    }
}

/// A deferred save prompt drops the displaced capture; cancelling still keeps unsaved work open.
#[test]
fn pin_remap_deferred_save_prompt_retires_review() {
    use crate::unsaved::{Then, Waiting};

    let (mut app, id) = fixture(false, true);
    assert!(app.files.dirty(&app.session));
    let project = app.session.snapshot().unwrap();
    let state = app.session.inspect();
    let editors = app.library_editors.inspect();
    let reviewed =
        live(&mut app, id, 0, "preview", json!([{"unit":"main","object":"1","number":"2"},{"unit":"main","object":"3","number":"1"}])).unwrap();
    assert!(matches!(app.dialog, Dialog::LibraryPinRemap(_)));
    assert!(reviewed["library_editors"]["pin_remap"]["token"].is_string());
    assert!(!app.files.busy());
    // Model the first frame after a deferred close's file job has finished.
    app.waiting = Some(Waiting::ToDecide(Then::Close));
    app.finish_waiting(&Context::default());
    assert!(app.waiting.is_none());
    assert!(matches!(app.dialog, Dialog::SaveChanges(Then::Close)));
    let inspected = crate::window_commands::inspect_editor(&app);
    assert_eq!(inspected["dialog_open"], true);
    assert_eq!(inspected["unsaved"], true);
    assert!(inspected["library_editors"]["pin_remap"].is_null());
    assert_eq!(inspected["library_editors"], editors);
    assert!(app.library_editors.pin_remap.is_none());

    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    crate::tests::click(&mut h, "Cancel");
    assert!(matches!(h.state().dialog, Dialog::None));
    assert!(!h.state().close_allowed);
    assert!(h.state().files.dirty(&h.state().session));
    assert_eq!(h.state().session.inspect(), state);
    assert!(Arc::ptr_eq(&project, &h.state().session.snapshot().unwrap()));
    let editors_after_cancel = h.state().library_editors.inspect();
    assert!(editors_after_cancel["pin_remap"].is_null());
    // The first app frame may adopt the library pane; compare the authored session, not the pane.
    assert_eq!(editors_after_cancel["documents"][0]["document"], id);
    assert_eq!(editors_after_cancel["documents"][0]["state"], editors["documents"][0]["state"]);
}

/// The supplied-review preflight and real live reply agree even with unsynchronized pane state.
/// Check the actual frame reader at the last accepted byte and first refused byte; no socket run.
#[test]
fn pin_remap_live_reply_preflight_matches_actual_inspection_bytes() {
    let (mut app, id) = fixture(false, true);
    app.place_library_document(&Context::default(), id, pcbcraft_chrome::workspace::Placement::Beside).unwrap();
    app.library_editors.active = None;
    app.message = Some(String::new());
    let pins = json!([{"unit":"main","object":"1","number":"2"},{"unit":"main","object":"3","number":"1"}]);
    let first = live(&mut app, id, 0, "preview", pins.clone()).unwrap();
    let overhead = serde_json::to_vec(&json!({"result":first})).unwrap().len();
    live(&mut app, id, 0, "cancel", first["library_editors"]["pin_remap"]["token"].clone()).unwrap();
    app.message = Some("x".repeat(pcbcraft_mcp::live::MAX_REPLY_BYTES - overhead));
    let exact = live(&mut app, id, 0, "preview", pins.clone()).unwrap();
    assert_eq!(exact, crate::window_commands::inspect_editor(&app));
    assert!(app.library_editors.active.is_none());
    let mut encoded = serde_json::to_vec(&json!({"result":exact})).unwrap();
    assert_eq!(encoded.len(), pcbcraft_mcp::live::MAX_REPLY_BYTES);
    encoded.push(b'\n');
    assert!(matches!(
        craft_control::read_frame(&mut std::io::Cursor::new(encoded), pcbcraft_mcp::live::MAX_REPLY_BYTES).unwrap(),
        Some(craft_control::Frame::Message(_))
    ));
    let review = app.library_editors.inspect()["pin_remap"].clone();
    let counter = app.library_editors.next_remap;
    app.message.as_mut().unwrap().push('x');
    let typed =
        vec![PinRemap { unit: "main".into(), object: 1, number: "2".into() }, PinRemap { unit: "main".into(), object: 3, number: "1".into() }];
    let reason = app.preview_library_pin_remap(id, 0, typed).unwrap_err();
    assert!(reason.contains("review cannot be captured") && reason.contains("reply byte limit"), "{reason}");
    assert_eq!(app.library_editors.inspect()["pin_remap"], review);
    assert_eq!(app.library_editors.next_remap, counter);
    let mut actual = serde_json::to_vec(&json!({"result":crate::window_commands::inspect_editor(&app)})).unwrap();
    assert_eq!(actual.len(), pcbcraft_mcp::live::MAX_REPLY_BYTES + 1);
    actual.push(b'\n');
    assert!(matches!(
        craft_control::read_frame(&mut std::io::Cursor::new(actual), pcbcraft_mcp::live::MAX_REPLY_BYTES).unwrap(),
        Some(craft_control::Frame::Oversized)
    ));
    live(&mut app, id, 0, "cancel", review["token"].clone()).unwrap();
    let reason = live(&mut app, id, 0, "preview", pins).unwrap_err();
    assert_eq!(reason.code(), ErrorCode::Unavailable.as_str());
    assert!(reason.message().contains("review cannot be captured") && reason.message().contains("reply byte limit"), "{reason}");
    assert!(matches!(app.dialog, Dialog::None));
    assert!(app.library_editors.pin_remap.is_none());
    assert_eq!(app.library_editors.next_remap, counter);
}

/// Actual dispatch distinguishes malformed row shape from scalar validation; inspect:false
/// acknowledges a capture that remains inspectable/cancellable without consuming either history.
#[test]
fn pin_remap_live_dispatch_shapes_acknowledgement_and_modal_refusal() {
    let (mut app, id) = fixture(false, true);
    let ctx = Context::default();
    let params = json!({"document":id.to_string(),"expected_revision":"0","phase":"preview","pins":[{"unit":"main","object":"1","number":"2"},{"unit":"main","object":"3","number":"1"}]});
    let before = app.session.inspect();
    let editors = app.library_editors.inspect();
    for (value, reason) in
        [(json!(["main", "1", "2"]), "row must be an object"), (json!({"unit":"main","object":1,"number":"2"}), "canonical decimal string")]
    {
        let mut request = params.clone();
        request["pins"][0] = value;
        let error = pcbcraft_mcp::live::call_editor(
            &mut crate::live_control::LiveEditor { app: &mut app, ctx: &ctx },
            &json!({"method":"engine.execute","params":{"command":"app.library_pin_remap","params":request}}),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidParams.as_str());
        assert!(error.message().contains("pins") && error.message().contains(reason), "{error}");
        assert_eq!(app.library_editors.inspect(), editors);
        assert_eq!(app.session.inspect(), before);
    }
    for item in [Value::Null, json!([]), json!({})] {
        let error = pcbcraft_mcp::live::call_editor(
            &mut crate::live_control::LiveEditor { app: &mut app, ctx: &ctx },
            &json!({"method":"engine.execute","params":{"command":sources::PREVIEW_PIN_REMAP,"params":{"item":item,"pins":params["pins"]}}}),
        )
        .unwrap_err();
        let reason = if item.is_object() { "missing field `format`" } else { "`item` Pin remap item must be an object." };
        assert_eq!(error.code(), ErrorCode::InvalidParams.as_str());
        assert!(error.message().contains(reason), "{error}");
        assert_eq!(app.session.inspect(), before);
    }
    let acknowledgement = pcbcraft_mcp::live::call_editor(
        &mut crate::live_control::LiveEditor { app: &mut app, ctx: &ctx },
        &json!({"method":"engine.execute","params":{"command":"app.library_pin_remap","params":params,"inspect":false}}),
    )
    .unwrap();
    assert_eq!(acknowledgement, json!({"revision":app.session.revision()}));
    let inspected =
        pcbcraft_mcp::live::call_editor(&mut crate::live_control::LiveEditor { app: &mut app, ctx: &ctx }, &json!({"method":"editor.inspect"}))
            .unwrap();
    assert_eq!(inspected["library_editors"]["pin_remap"]["report"]["changes"].as_array().unwrap().len(), 2);
    assert_eq!(app.library_editors.documents[&id].session.revision(), 0);
    assert!(!app.library_editors.documents[&id].session.can_undo());
    let project_revision = app.session.revision();
    let error = pcbcraft_mcp::live::call_editor(&mut crate::live_control::LiveEditor { app:&mut app, ctx:&ctx },
        &json!({"method":"engine.execute","params":{"command":"project.rename","params":{"name":"Not committed","expected_revision":project_revision}}})).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Unavailable.as_str());
    assert!(error.message().contains("Finish the current dialog") && error.message().contains("project.rename"), "{error}");
    live(&mut app, id, 0, "cancel", inspected["library_editors"]["pin_remap"]["token"].clone()).unwrap();
    assert_eq!(app.library_editors.inspect(), editors);
    assert_eq!(app.session.inspect(), before);
}

/// No project and no matching placed identity are unavailable evidence, not a known unconnected
/// terminal or proof that the pin has no package. Check the projected row strings.
#[test]
fn pin_remap_fallback_display_does_not_invent_placed_evidence() {
    for open_project in [false, true] {
        let mut app = PcbApp::default();
        if open_project {
            app.session.execute("project.new", &json!({})).unwrap();
        }
        app.new_library_item(Kind::Symbol);
        let id = app.library_editors.active.unwrap();
        let reply = live(&mut app, id, 0, "preview", json!([{"unit":"main","object":"1","number":"7"}])).unwrap();
        let report = &reply["library_editors"]["pin_remap"]["report"];
        assert!(report["instances"].as_array().unwrap().is_empty());
        assert_eq!(report["project"].is_null(), !open_project);
        let rows = super::super::remap::report_rows(report, 0);
        assert_eq!(rows[0].cells[3], "Unknown");
        assert_eq!(rows[0].cells[4], "Unknown: no associated placed copy");
        assert_eq!(rows[0].cells[5], "Unknown: no associated placed copy");
    }
}

/// Bulk cleanup retires a retained capture independently of entering-close cleanup. The
/// deliberately retained internal review models defensive cleanup, not a native OS observation.
#[test]
fn pin_remap_bulk_discard_releases_retained_review() {
    let (mut app, id) = fixture(false, true);
    app.new_library_item(Kind::Symbol);
    let project = app.session.snapshot().unwrap();
    let owners = Arc::strong_count(&project);
    let before = app.session.inspect();
    let bytes = engine::persistence::encode_documents(project.clone()).unwrap();
    let token = app.preview_library_pin_remap(id, 0, vec![PinRemap { unit: "main".into(), object: 1, number: "1".into() }]).unwrap();
    assert_eq!(Arc::strong_count(&project), owners + 1);
    app.dialog = Dialog::LibraryClose(None);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .with_context_setup(pcbcraft_chrome::install)
        .build_ui_state(|ui, app: &mut PcbApp| app.dialogs(ui.ctx()), app);
    h.run();
    crate::tests::click(&mut h, "Discard library edits");
    assert!(h.state().library_editors.documents.is_empty());
    assert!(h.state().library_editors.pin_remap.is_none());
    assert_eq!(Arc::strong_count(&project), owners);
    assert!(matches!(h.state().dialog, Dialog::SaveChanges(crate::unsaved::Then::Close)));
    crate::tests::click(&mut h, "Cancel");
    assert!(matches!(h.state().dialog, Dialog::None));
    let refusal = live(h.state_mut(), id, 0, "apply", json!(token.to_string())).unwrap_err();
    assert_eq!(refusal.code(), ErrorCode::Unavailable.as_str());
    assert_eq!(refusal.message(), "Preview the pin remap before applying it.");
    assert_eq!(h.state().session.inspect(), before);
    assert_eq!(engine::persistence::encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
}
