use super::*;
use egui_kittest::kittest::NodeT;
use pcbcraft_engine::authoring::remap::PinRemap;

/// Open an original two-pin document through the same native command as the editor menu.
fn fixture() -> (PcbApp, u64) {
    let mut app = PcbApp::default();
    app.new_library_item(Kind::Symbol);
    let id = app.library_editors.active.unwrap();
    (app, id)
}
/// Explicit primitive correspondences retain the draft's pin names and positions.
fn swap() -> Vec<PinRemap> {
    vec![PinRemap { unit: "main".into(), object: 1, number: "2".into() }, PinRemap { unit: "main".into(), object: 3, number: "1".into() }]
}

/// Both frontends reach the same preview/apply/cancel handlers; wrong or consumed tokens and
/// replacement pins are refused, and a direct remap action cannot bypass review.
#[test]
fn pin_remap_live_review_has_one_library_undo_and_no_project_effect() {
    let (mut app, id) = fixture();
    let ctx = Context::default();
    let before = app.library_editors.documents[&id].session.snapshot();
    let project_revision = app.session.revision();
    let preview = json!({"document":id.to_string(),"expected_revision":"0","phase":"preview","pins":swap()});
    app.run_window_command(&ctx, "app.library_pin_remap", &preview).unwrap();
    let inspected = app.library_editors.inspect();
    let token = inspected["pin_remap"]["token"].as_str().unwrap().parse::<u64>().unwrap();
    assert_eq!(inspected["pin_remap"]["document"], id.to_string());
    assert_eq!(inspected["pin_remap"]["expected_revision"], "0");
    assert_eq!(inspected["pin_remap"]["project_revision"], project_revision.to_string());
    assert_eq!(inspected["pin_remap"]["report"]["changes"][0]["object"], "1");
    assert_eq!(inspected["pin_remap"]["report"]["changes"][0]["before"], "1");
    assert_eq!(app.library_editors.documents[&id].session.revision(), 0);
    let apply = json!({"document":id.to_string(),"expected_revision":"0","phase":"apply","token":token.to_string()});
    for (field, value) in [("token", token + 1), ("document", id + 1), ("expected_revision", 1)] {
        let mut invalid = apply.clone();
        invalid[field] = json!(value.to_string());
        assert!(app.run_window_command(&ctx, "app.library_pin_remap", &invalid).unwrap_err().contains("does not match"));
    }
    let mut replacement = apply.clone();
    replacement["pins"] = json!(swap());
    assert!(app.run_window_command(&ctx, "app.library_pin_remap", &replacement).unwrap_err().contains("replacement pins"));
    assert!(
        app.run_window_command(
            &ctx,
            "app.library_edit",
            &json!({"document":id,"expected_revision":0,"action":{"operation":"remap_pins","pins":swap()}})
        )
        .unwrap_err()
        .contains("review current nets")
    );
    app.run_window_command(&ctx, "app.library_pin_remap", &apply).unwrap();
    assert_eq!(app.session.revision(), project_revision);
    assert!(app.session.project().is_none());
    let changed = app.library_editors.documents[&id].session.snapshot();
    assert_eq!(changed.symbol().unwrap().pins()[0].number, "2");
    assert_eq!(Item::decode(&changed.encode().unwrap()).unwrap(), *changed);
    assert!(app.run_window_command(&ctx, "app.library_pin_remap", &apply).unwrap_err().contains("Preview"));
    app.library_editors.edit(id, 1, Action::Undo).unwrap();
    assert_eq!(app.library_editors.documents[&id].session.snapshot().native().unwrap(), before.native().unwrap());
    assert!(!app.library_editors.documents[&id].session.can_undo());
    let token = app.preview_library_pin_remap(id, 2, swap()).unwrap();
    app.cancel_library_pin_remap(id, 2, token).unwrap();
    assert_eq!(app.library_editors.documents[&id].session.revision(), 2);
    assert!(app.library_editors.pin_remap.is_none());
}

/// An unchanged correspondence can be reviewed, applied or cancelled without dirtying either
/// document, changing bytes or allocating undo. Apply and cancel both consume that review.
#[test]
fn pin_remap_live_unchanged_map_is_a_noop_and_consumes_review() {
    for phase in ["apply", "cancel"] {
        let (mut app, id) = fixture();
        app.session.execute("project.new", &json!({})).unwrap();
        let doc = app.library_editors.documents.get_mut(&id).unwrap();
        doc.tool = canvas::Tool::Draw { kind: canvas::Place::Line, start: None, revision: 0 };
        doc.error = Some("Existing canvas guidance".into());
        let ui_before = (format!("{:?}", doc.tool), doc.error.clone(), doc.selection.clone(), doc.selected);
        let before = doc.session.inspect();
        let bytes = doc.session.snapshot().encode().unwrap();
        let project = app.session.inspect();
        let ctx = Context::default();
        app.run_window_command(
            &ctx,
            "app.library_pin_remap",
            &json!({
                "document":id.to_string(),"expected_revision":"0","phase":"preview",
                "pins":[{"unit":"main","object":"1","number":"1"}]
            }),
        )
        .unwrap();
        assert_eq!(app.library_editors.documents[&id].session.inspect(), before);
        let review = app.library_editors.inspect()["pin_remap"].clone();
        let request = json!({"document":id.to_string(),"expected_revision":"0","phase":phase,"token":review["token"]});
        app.run_window_command(&ctx, "app.library_pin_remap", &request).unwrap();
        assert!(app.library_editors.pin_remap.is_none());
        let doc = &app.library_editors.documents[&id];
        assert_eq!((format!("{:?}", doc.tool), doc.error.clone(), doc.selection.clone(), doc.selected), ui_before);
        assert_eq!(app.library_editors.documents[&id].session.inspect(), before);
        assert_eq!(app.library_editors.documents[&id].session.snapshot().encode().unwrap(), bytes);
        assert_eq!(app.session.inspect(), project);
        let error = app.run_window_command(&ctx, "app.library_pin_remap", &request).unwrap_err();
        assert!(error.contains(if phase == "apply" { "Preview" } else { "No pin remap review" }), "{error}");
        assert_eq!(app.library_editors.documents[&id].session.inspect(), before);
        assert_eq!(app.session.inspect(), project);
    }
}

/// Lossless live tokens reach Apply; malformed routing preserves the captured report/history.
/// At the sequence ceiling failed refresh keeps the old owner, while consumption and a refused
/// new capture leave no report/project retention or modal behind.
#[test]
fn pin_remap_live_maximum_token_and_refusals_preserve_authority() {
    let (mut app, id) = fixture();
    app.session.execute("project.new", &json!({})).unwrap();
    let captured_project = app.session.snapshot().unwrap();
    let owners = std::sync::Arc::strong_count(&captured_project);
    app.library_editors.next_remap = u64::MAX - 1;
    let ctx = Context::default();
    app.run_window_command(
        &ctx,
        "app.library_pin_remap",
        &json!({
            "document":id.to_string(),"expected_revision":"0","phase":"preview","pins":swap()
        }),
    )
    .unwrap();
    assert_eq!(app.library_editors.inspect()["pin_remap"]["token"], u64::MAX.to_string());
    let request = json!({"document":id.to_string(),"expected_revision":"0","phase":"apply","token":u64::MAX.to_string()});
    let before = app.library_editors.inspect();
    let project = app.session.inspect();
    for field in ["document", "expected_revision", "token"] {
        for value in [json!(0), json!(1.0), json!("01"), json!("18446744073709551616"), Value::Null] {
            let mut invalid = request.clone();
            invalid[field] = value;
            let error = app.run_window_command(&ctx, "app.library_pin_remap", &invalid).unwrap_err();
            assert!(error.contains(field) && error.contains("canonical decimal string"), "{invalid}: {error}");
            assert_eq!(app.library_editors.inspect(), before);
            assert_eq!(app.session.inspect(), project);
        }
    }
    assert_eq!(std::sync::Arc::strong_count(&captured_project), owners + 1);
    assert_eq!(app.preview_library_pin_remap(id, 0, swap()).unwrap_err(), "Pin remap review ID limit reached.");
    assert_eq!(app.library_editors.inspect(), before);
    app.run_window_command(&ctx, "app.library_pin_remap", &request).unwrap();
    assert_eq!(app.library_editors.documents[&id].session.revision(), 1);
    assert!(app.library_editors.pin_remap.is_none());
    assert_eq!(std::sync::Arc::strong_count(&captured_project), owners);
    let refusal = pcbcraft_mcp::live::call_editor(
        &mut crate::live_control::LiveEditor { app: &mut app, ctx: &ctx },
        &json!({"method":"engine.execute","params":{"command":"app.library_pin_remap","params":{"document":id.to_string(),"expected_revision":"1","phase":"preview","pins":swap()}}}),
    ).unwrap_err();
    assert_eq!(refusal, "Pin remap review ID limit reached.");
    assert!(matches!(app.dialog, Dialog::None));
    assert!(app.library_editors.pin_remap.is_none());
    assert_eq!(std::sync::Arc::strong_count(&captured_project), owners);
    assert_eq!(app.session.inspect(), project);
}

/// A project edit/undo or reopen and a library edit/undo invalidate captured authority even
/// when the visible content returns to the old value. Failed apply changes neither history.
#[test]
fn pin_remap_refuses_stale_project_or_definition_reviews() {
    for case in 0..4 {
        let (mut app, id) = fixture();
        app.session.execute("project.new", &json!({})).unwrap();
        let response = pcbcraft_mcp::live::call_editor(
            &mut crate::live_control::LiveEditor { app: &mut app, ctx: &Context::default() },
            &json!({"method":"engine.execute","params":{"command":"app.library_pin_remap","params":{"document":id.to_string(),"expected_revision":"0","phase":"preview","pins":swap()}}}),
        ).unwrap();
        let token = response["library_editors"]["pin_remap"]["token"].clone();
        match case {
            0 => {
                app.session.execute("project.new", &json!({})).unwrap();
            }
            1 => {
                app.session.execute(pcbcraft_engine::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":app.session.revision()})).unwrap();
                app.session.execute("edit.undo", &Value::Null).unwrap();
            }
            2 => {
                app.library_editors.edit(id, 0, Action::Delete { objects: vec![3] }).unwrap();
            }
            _ => {
                app.library_editors.edit(id, 0, Action::Delete { objects: vec![3] }).unwrap();
                app.library_editors.edit(id, 1, Action::Undo).unwrap();
            }
        }
        let before = app.library_editors.documents[&id].session.inspect();
        let library_bytes = app.library_editors.documents[&id].session.snapshot().encode().unwrap();
        let project = app.session.snapshot().unwrap();
        let project_state = app.session.inspect();
        let project_bytes = pcbcraft_engine::persistence::encode_documents(project.clone()).unwrap();
        let review = app.library_editors.inspect()["pin_remap"].clone();
        let error = pcbcraft_mcp::live::call_editor(
            &mut crate::live_control::LiveEditor { app: &mut app, ctx: &Context::default() },
            &json!({"method":"engine.execute","params":{"command":"app.library_pin_remap","params":{"document":id.to_string(),"expected_revision":"0","phase":"apply","token":token}}}),
        ).unwrap_err();
        assert_eq!(
            error,
            if case < 2 {
                "Project changed; preview current nets again before applying."
            } else {
                "Library document changed; inspect it before editing."
            }
        );
        assert_eq!(app.library_editors.documents[&id].session.inspect(), before);
        assert_eq!(app.library_editors.documents[&id].session.snapshot().encode().unwrap(), library_bytes);
        assert_eq!(app.session.inspect(), project_state);
        assert!(std::sync::Arc::ptr_eq(&project, &app.session.snapshot().unwrap()));
        assert_eq!(pcbcraft_engine::persistence::encode_documents(app.session.snapshot().unwrap()).unwrap(), project_bytes);
        assert_eq!(app.library_editors.inspect()["pin_remap"], review);
    }
}

/// A failed preparation cannot allocate a token, replace the captured report or alter history.
#[test]
fn pin_remap_failed_preparation_retains_review_authority() {
    let (mut app, id) = fixture();
    let token = app.preview_library_pin_remap(id, 0, swap()).unwrap();
    let before = app.library_editors.inspect();
    let counter = app.library_editors.next_remap;
    assert!(app.preview_library_pin_remap(id, 0, vec![swap()[0].clone()]).unwrap_err().contains("collides with an unchanged"));
    assert_eq!(app.library_editors.inspect(), before);
    assert_eq!(app.library_editors.next_remap, counter);
    app.cancel_library_pin_remap(id, 0, token).unwrap();
}

/// A real other document with the same revision cannot apply or cancel this document's review.
/// Both histories, encoded items and the captured authority survive either refusal.
#[test]
fn pin_remap_wrong_open_document_cannot_apply_or_cancel_review() {
    let (mut app, id) = fixture();
    app.new_library_item(Kind::Symbol);
    let other = app.library_editors.active.unwrap();
    assert_ne!(id, other);
    assert_eq!(app.library_editors.documents[&other].session.revision(), 0);
    let token = app.preview_library_pin_remap(id, 0, swap()).unwrap();
    let before = app.library_editors.inspect();
    let bytes: Vec<_> = app.library_editors.documents.values().map(|doc| doc.session.snapshot().encode().unwrap()).collect();
    let project = app.session.inspect();
    for phase in ["apply", "cancel"] {
        let error = app
            .run_window_command(
                &Context::default(),
                "app.library_pin_remap",
                &json!({"document":other.to_string(),"expected_revision":"0","phase":phase,"token":token.to_string()}),
            )
            .unwrap_err();
        assert_eq!(error, "Pin remap review does not match this document, revision or token.");
        assert_eq!(app.library_editors.inspect(), before);
        assert_eq!(app.library_editors.documents.values().map(|doc| doc.session.snapshot().encode().unwrap()).collect::<Vec<_>>(), bytes);
        assert_eq!(app.session.inspect(), project);
    }
    app.cancel_library_pin_remap(id, 0, token).unwrap();
}

/// A matching review token cannot apply or cancel through a form that has not captured it.
/// Stage the unmatched form directly so earlier document/token checks cannot mask this branch.
#[test]
fn pin_remap_matching_token_without_form_owner_is_refused() {
    let (mut app, id) = fixture();
    let token = app.preview_library_pin_remap(id, 0, swap()).unwrap();
    app.begin_library_pin_remap(id).unwrap();
    let before = app.library_editors.inspect();
    let item = app.library_editors.documents[&id].session.snapshot();
    let project = app.session.inspect();
    for phase in ["apply", "cancel"] {
        let error = app
            .run_window_command(
                &Context::default(),
                "app.library_pin_remap",
                &json!({"document":id.to_string(),"expected_revision":"0","phase":phase,"token":token.to_string()}),
            )
            .unwrap_err();
        assert_eq!(error, "Pin remap review has no matching form owner; inspect and preview again.");
        assert_eq!(app.library_editors.inspect(), before);
        assert!(std::sync::Arc::ptr_eq(&item, &app.library_editors.documents[&id].session.snapshot()));
        assert_eq!(app.session.inspect(), project);
        assert!(matches!(&app.dialog, Dialog::LibraryPinRemap(draft) if !draft.has_captured_review()));
    }
}

/// Visible refresh preserves all explicitly supplied mappings, including untouched numbers.
#[test]
fn pin_remap_form_refresh_preserves_explicit_unchanged_rows() {
    for first_number in ["1", "7"] {
        let (mut app, id) = fixture();
        app.run_window_command(
            &Context::default(),
            "app.library_pin_remap",
            &json!({"document":id.to_string(),"expected_revision":"0","phase":"preview","pins":[
                {"unit":"main","object":"1","number":first_number}, {"unit":"main","object":"3","number":"2"}
            ]}),
        )
        .unwrap();
        let before = app.library_editors.inspect()["pin_remap"].clone();
        assert_eq!(before["report"]["changes"].as_array().unwrap().len(), 2);
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        crate::tests::click(&mut h, "Preview current nets");
        let refreshed = &h.state().library_editors.inspect()["pin_remap"];
        assert_ne!(refreshed["token"], before["token"]);
        assert_eq!(refreshed["report"], before["report"]);
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), 0);
    }
}

/// Failed visible refresh keeps ownership of a live unchanged-map review. Both Cancel and
/// Escape retire that exact token; no authored bytes or history change at any stage.
#[test]
fn pin_remap_form_failed_refresh_cancel_and_dismiss_retire_live_review() {
    for dismiss in [false, true] {
        let (mut app, id) = fixture();
        app.session.execute("project.new", &json!({})).unwrap();
        let ctx = Context::default();
        app.run_window_command(
            &ctx,
            "app.library_pin_remap",
            &json!({"document":id.to_string(),"expected_revision":"0","phase":"preview","pins":[{"unit":"main","object":"1","number":"1"}]}),
        )
        .unwrap();
        let before = app.library_editors.documents[&id].session.inspect();
        let bytes = app.library_editors.documents[&id].session.snapshot().encode().unwrap();
        let project = app.session.inspect();
        let review = app.library_editors.inspect()["pin_remap"].clone();
        app.library_editors.next_remap = u64::MAX;
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        crate::tests::click(&mut h, "Preview current nets");
        h.get_by_label("Pin remap review ID limit reached.");
        assert_eq!(h.state().library_editors.inspect()["pin_remap"], review);
        assert!(matches!(h.state().dialog, Dialog::LibraryPinRemap(_)));
        if dismiss {
            h.key_press(egui::Key::Escape);
            h.run();
        } else {
            crate::tests::click(&mut h, "Cancel");
        }
        assert!(matches!(h.state().dialog, Dialog::None));
        assert!(h.state().library_editors.pin_remap.is_none());
        for phase in ["apply", "cancel"] {
            let error = h
                .state_mut()
                .run_window_command(
                    &ctx,
                    "app.library_pin_remap",
                    &json!({"document":id.to_string(),"expected_revision":"0","phase":phase,"token":review["token"]}),
                )
                .unwrap_err();
            assert!(error.contains(if phase == "apply" { "Preview" } else { "No pin remap review" }), "{error}");
        }
        assert_eq!(h.state().library_editors.documents[&id].session.inspect(), before);
        assert_eq!(h.state().library_editors.documents[&id].session.snapshot().encode().unwrap(), bytes);
        assert_eq!(h.state().session.inspect(), project);
    }
}

/// Unapplied property input is not part of a captured report, so neither preview nor apply may
/// silently discard it. Resolve it before starting or completing another library transaction.
#[test]
fn pin_remap_pending_property_fields_refuse_preview_and_apply() {
    let (mut app, id) = fixture();
    let token = app.preview_library_pin_remap(id, 0, swap()).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    crate::tests::click(&mut h, "Pin 1 · 1");
    h.get_by_label("Number").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Number").type_text("3");
    h.run();
    let before = h.state().library_editors.documents[&id].session.inspect();
    assert!(h.state_mut().preview_library_pin_remap(id, 0, swap()).unwrap_err().contains("pending fields"));
    assert!(h.state_mut().apply_library_pin_remap(id, 0, token).unwrap_err().contains("pending property fields"));
    assert_eq!(h.state().library_editors.documents[&id].session.inspect(), before);
    // Reattach actual typed input after capturing through the live path, so Apply reaches the
    // pending-fields guard rather than failing earlier on mismatched transport/form authority.
    let pending = h.state_mut().library_editors.documents.get_mut(&id).unwrap().draft.take();
    let response = pcbcraft_mcp::live::call_editor(
        &mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &Context::default() },
        &json!({"method":"engine.execute","params":{"command":"app.library_pin_remap","params":{"document":id.to_string(),"expected_revision":"0","phase":"preview","pins":swap()}}}),
    ).unwrap();
    h.state_mut().library_editors.documents.get_mut(&id).unwrap().draft = pending;
    let state = h.state().library_editors.inspect();
    let error = pcbcraft_mcp::live::call_editor(
        &mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &Context::default() },
        &json!({"method":"engine.execute","params":{"command":"app.library_pin_remap","params":{"document":id.to_string(),"expected_revision":"0","phase":"apply","token":response["library_editors"]["pin_remap"]["token"]}}}),
    ).unwrap_err();
    assert_eq!(error, "Apply or reset all pending property fields before remapping.");
    assert_eq!(h.state().library_editors.inspect(), state);
    assert_eq!(h.state().library_editors.documents[&id].session.inspect(), before);
}

/// The actual shared form requires preview, displays the before/after table, and uses the same
/// single transaction. Unchanged input explains how to start a preview. This is
/// accessibility/interaction coverage, not rendered visual proof.
#[test]
fn pin_remap_form_previews_before_enabling_apply_and_cancels_cleanly() {
    for width in [700.0, 1200.0] {
        let (mut app, id) = fixture();
        app.begin_library_pin_remap(id).unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(width, 900.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        assert!(h.state().library_editors.pin_remap.is_none());
        assert!(h.get_by_label("Apply remap").accesskit_node().is_disabled());
        let before = h.state().library_editors.documents[&id].session.inspect();
        crate::tests::click(&mut h, "Preview current nets");
        h.get_by_label("Change at least one pin number before previewing.");
        assert!(h.state().library_editors.pin_remap.is_none());
        assert_eq!(h.state().library_editors.documents[&id].session.inspect(), before);
        for (label, value) in [("Pin 1", "2"), ("Pin 2", "1")] {
            h.get_by_label(label).click();
            h.run();
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            h.get_by_label(label).type_text(value);
            h.run();
        }
        crate::tests::click(&mut h, "Preview current nets");
        assert!(h.state().library_editors.pin_remap.is_some());
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), 0);
        h.get_by_label("Before → after");
        let token = h.state().library_editors.inspect()["pin_remap"]["token"].clone();
        h.get_by_label("Pin 1").click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Pin 1").type_text("3");
        h.run();
        assert!(h.state().library_editors.pin_remap.is_none());
        assert!(h.get_by_label("Apply remap").accesskit_node().is_disabled());
        let reason = pcbcraft_mcp::live::call_editor(
            &mut crate::live_control::LiveEditor { app: h.state_mut(), ctx: &Context::default() },
            &json!({"method":"engine.execute","params":{"command":"app.library_pin_remap","params":{"document":id.to_string(),"expected_revision":"0","phase":"apply","token":token}}}),
        ).unwrap_err();
        assert_eq!(reason, "Preview the pin remap before applying it.");
        h.get_by_label("Pin 1").click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Pin 1").type_text("2");
        h.run();
        crate::tests::click(&mut h, "Preview current nets");
        assert_ne!(h.state().library_editors.inspect()["pin_remap"]["token"], token);
        crate::tests::click(&mut h, "Cancel");
        assert!(matches!(h.state().dialog, Dialog::None));
        assert!(h.state().library_editors.pin_remap.is_none());
        assert_eq!(h.state().library_editors.documents[&id].session.inspect(), before);
        h.state_mut().begin_library_pin_remap(id).unwrap();
        h.run();
        assert!(h.get_by_label("Apply remap").accesskit_node().is_disabled());
        for (label, value) in [("Pin 1", "2"), ("Pin 2", "1")] {
            h.get_by_label(label).click();
            h.run();
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            h.get_by_label(label).type_text(value);
            h.run();
        }
        crate::tests::click(&mut h, "Preview current nets");
        assert_ne!(h.state().library_editors.inspect()["pin_remap"]["token"], token);
        crate::tests::click(&mut h, "Apply remap");
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), 1);
        h.state_mut().begin_library_pin_remap(id).unwrap();
        h.run();
        crate::tests::click(&mut h, "Cancel");
        assert!(matches!(h.state().dialog, Dialog::None));
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), 1);
    }
}
