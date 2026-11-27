use super::*;
use std::sync::Arc;
/// Authoring histories never advance or dirty the project's document session.
#[test]
fn library_documents_are_independent_and_revision_checked() {
    let mut app = PcbApp::default();
    let revision = app.session.revision();
    let id = app.library_editors.open(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    let before = app.library_editors.documents[&id].session.snapshot();
    app.library_editors.edit(id, 0, Action::Delete { objects: vec![3] }).unwrap();
    assert!(app.library_editors.edit(id, 0, Action::Undo).is_err());
    app.library_editors.edit(id, 1, Action::Undo).unwrap();
    assert_eq!(app.library_editors.documents[&id].session.snapshot().native().unwrap(), before.native().unwrap());
    assert_eq!(app.session.revision(), revision);
    assert!(app.session.project().is_none());
    assert!(app.library_editors.close(id, false).is_err());
    app.library_editors.close(id, true).unwrap();
    let next = app.library_editors.open(Item::new(Kind::Symbol).unwrap(), false).unwrap();
    assert!(next > id);
    assert!(app.library_editors.edit(id, 0, Action::Undo).is_err());
}

use egui_kittest::{Harness, kittest::Queryable};

#[cfg(not(target_arch = "wasm32"))]
mod remap;
#[cfg(not(target_arch = "wasm32"))]
mod remap_evidence;
mod zoom;

/// Explicit document close releases its captured project/report; reopening cannot reuse its token.
/// This shared editor lifecycle also applies to the WebAssembly test target.
#[test]
fn pin_remap_close_cancels_review_and_reopen_has_new_authority() {
    use pcbcraft_engine::authoring::remap::PinRemap;

    let mut app = PcbApp::default();
    app.new_library_item(Kind::Symbol);
    let id = app.library_editors.active.unwrap();
    let item = app.library_editors.documents[&id].session.snapshot();
    let pins = vec![PinRemap { unit: "main".into(), object: 1, number: "2".into() }, PinRemap { unit: "main".into(), object: 3, number: "1".into() }];
    let token = app.preview_library_pin_remap(id, 0, pins).unwrap();
    app.library_editors.close(id, true).unwrap();
    assert!(app.library_editors.pin_remap.is_none());
    let reopened = app.library_editors.open(item.as_ref().clone(), true).unwrap();
    assert_ne!(id, reopened);
    assert!(app.apply_library_pin_remap(reopened, 0, token).unwrap_err().contains("Preview"));
    assert_eq!(app.library_editors.documents[&reopened].session.revision(), 0);
}

/// Definition panes extend their own canvas palette through the exposed ribbon margins, down to
/// the pane's top edge, leaving no unpainted band between the ribbon and the canvas.
#[test]
fn definition_ribbon_backdrop_matches_the_document_canvas() {
    for kind in [Kind::Footprint, Kind::Symbol] {
        let mut app = PcbApp::default();
        app.new_library_item(kind);
        let mut h = Harness::builder().with_size(egui::vec2(1200., 800.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let pane = h.state().views.hosts[&h.state().views.host].rects[0].1;
        let t = pcbcraft_chrome::Tokens::get();
        let fill = if kind == Kind::Footprint { t.board_canvas } else { t.canvas };
        assert!(
            h.output().shapes.iter().any(|shape| match &shape.shape {
                egui::Shape::Vec(shapes) => shapes.iter().any(|shape| matches!(shape, egui::Shape::Rect(rect)
                if rect.fill == fill && (rect.rect.bottom() - pane.top()).abs() < 0.5
                    && (rect.rect.left() - pane.left()).abs() < 0.5
                    && (rect.rect.right() - pane.right()).abs() < 0.5)),
                _ => false,
            }),
            "{kind:?} must carry its canvas palette through the ribbon"
        );
    }
}

/// Definition properties author the default reference, and both editable and exported files
/// retain it without changing the active project or the library primitives.
#[test]
fn footprint_definition_properties_save_a_default_reference_offset() {
    let mut app = PcbApp::default();
    app.new_library_item(Kind::Footprint);
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    let id = h.state().library_editors.active.unwrap();
    let before = h.state().library_editors.documents[&id].session.snapshot();
    crate::tests::click(&mut h, "Generate silkscreen reference");
    let field = "Reference X offset (mm)";
    h.get_by_label(field).click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label(field).type_text("2.5");
    h.run();
    crate::tests::click(&mut h, "Apply");
    let after = h.state().library_editors.documents[&id].session.snapshot();
    let offset = pcbcraft_engine::schematic::Point { x_nm: 2_500_000, y_nm: 0 };
    assert_eq!(after.reference_offset, Some(offset));
    assert_eq!(after.objects, before.objects);
    assert!(h.state().session.project().is_none());
    let definition = after.footprint().unwrap();
    assert_eq!(definition.reference_offset, Some(offset));
    assert!(!definition.reference_paths("REF", Default::default(), pcbcraft_engine::board::DEFAULT_REFERENCE_OFFSET).is_empty());
    assert_eq!(Item::decode(&after.encode().unwrap()).unwrap().reference_offset, Some(offset));
    crate::tests::click(&mut h, "Undo Library Edit");
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot().reference_offset, None);
}

/// Project and definition Browsers share row geometry and disclosure controls. Folding a
/// definition retains its selected primitive and both document histories at normal/compact widths.
#[test]
fn definition_browsers_use_shared_tree_rows_and_selection_safe_folding() {
    for width in [1200.0, 700.0] {
        let mut h = crate::tests::harness();
        h.state_mut().run("project.new", &json!({"name":"Browser comparison"}));
        h.set_size(egui::vec2(width, 900.0));
        h.run();
        let project = h.state().session.snapshot().unwrap();
        let row_height = h.get_by_label("Main").rect().height();
        let header_height = h.get_by_label("Collapse Browser").rect().height();
        for (workspace, primitive) in [("Symbol Editor", "Pin 1 · 1"), ("Footprint Editor", "Pad 1 · 1")] {
            choose_workspace(&mut h, workspace);
            let id = h.state().library_editors.active.unwrap();
            let before = h.state().library_editors.documents[&id].session.snapshot();
            let root = h.get_by_role_and_label(egui::accesskit::Role::Button, "Definition").rect();
            let row = h.get_by_label(primitive).rect();
            assert_eq!(root.height(), row_height);
            assert_eq!(row.height(), row_height);
            assert!(row.left() > root.left());
            assert_eq!(h.get_by_label("Collapse Browser").rect().height(), header_height);
            crate::tests::click(&mut h, primitive);
            crate::tests::click(&mut h, "Collapse Definition");
            assert!(h.query_by_label(primitive).is_none());
            assert_eq!(h.state().library_editors.documents[&id].selected, Some(1));
            crate::tests::click(&mut h, "Expand Definition");
            h.get_by_label(primitive);
            assert_eq!(h.state().library_editors.documents[&id].selected, Some(1));
            assert!(Arc::ptr_eq(&before, &h.state().library_editors.documents[&id].session.snapshot()));
            assert!(Arc::ptr_eq(&project, &h.state().session.snapshot().unwrap()));
        }
    }
}

/// The actual compact and expanded forms share one draft; a bad length is retained and cannot
/// change geometry, undo history or the project. Close and Quit both protect unapplied input.
#[test]
fn properties_forms_validate_atomically_and_protect_unsaved_fields() {
    let mut app = PcbApp::default();
    app.new_library_item(Kind::Footprint);
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    let id = h.state().library_editors.active.unwrap();
    let before = h.state().library_editors.documents[&id].session.snapshot();
    crate::tests::click(&mut h, "Pad 1 · 1");
    assert!(h.query_by_label("Number").is_some());
    h.get_by_label("Width (mm)").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Width (mm)").type_text("bad length");
    h.run();
    crate::tests::click(&mut h, "Apply");
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot().native().unwrap(), before.native().unwrap());
    assert!(h.state().library_editors.documents[&id].pending());
    crate::tests::click(&mut h, "More properties…");
    assert!(matches!(h.state().dialog, Dialog::LibraryProperties(..)));
    h.state_mut().begin_project_close(&Context::default());
    h.run();
    assert!(matches!(h.state().dialog, Dialog::LibraryClose(None)));
    crate::tests::click(&mut h, "Cancel");
    assert!(h.state().library_editors.unsaved());
    crate::tests::click(&mut h, "Reset");
    h.get_by_label("Width (mm)").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Width (mm)").type_text("0.7 mm");
    h.run();
    crate::tests::click(&mut h, "Apply");
    let changed = h.state().library_editors.documents[&id].session.snapshot();
    assert_ne!(changed, before);
    crate::tests::click(&mut h, "Undo Library Edit");
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot().native().unwrap(), before.native().unwrap());
    crate::tests::click(&mut h, "Redo Library Edit");
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot(), changed);
    crate::tests::click(&mut h, "Pad type");
    crate::tests::click(&mut h, "Through hole");
    h.get_by_label("Drill (mm)").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Drill (mm)").type_text("0.3");
    h.run();
    crate::tests::click(&mut h, "Apply");
    let item = h.state().library_editors.documents[&id].session.snapshot();
    let pcbcraft_engine::authoring::Primitive::Pad { pad } = &item.objects[&1] else { panic!("expected pad") };
    assert_eq!(pad.drill_nm, Some(300_000));
    assert_eq!(h.state().session.revision(), 0);
}
/// Live discovery exposes the same authoring actions, requires library revisions and preserves
/// project authority. Unknown IDs and malformed/extra fields fail before any mutation.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_library_commands_share_the_gui_documents() {
    let mut app = PcbApp::default();
    let ctx = Context::default();
    app.run_window_command(&ctx, "app.library_new", &json!({"kind":"symbol"})).unwrap();
    let id = app.library_editors.active.unwrap();
    app.run_window_command(&ctx, "app.library_edit", &json!({"document":id,"expected_revision":0,"action":{"operation":"delete","objects":[3]}}))
        .unwrap();
    assert!(app.run_window_command(&ctx, "app.library_edit", &json!({"document":id,"expected_revision":0,"action":{"operation":"undo"}})).is_err());
    app.run_window_command(&ctx, "app.library_edit", &json!({"document":id,"expected_revision":1,"action":{"operation":"undo"}})).unwrap();
    assert!(
        app.run_window_command(
            &ctx,
            "app.library_edit",
            &json!({"document":id,"expected_revision":2,"action":{"operation":"undo","unexpected":true}})
        )
        .is_err()
    );
    let inspected = crate::window_commands::inspect_editor(&app);
    assert_eq!(inspected["library_editors"]["documents"][0]["state"]["revision"], 2);
    assert!(app.run_window_command(&ctx, "app.library_close", &json!({"document":id})).is_err());
    app.run_window_command(&ctx, "app.library_focus", &json!({})).unwrap();
    assert_eq!(app.library_editors.active, None);
    assert!(app.library_editors.unsaved());
    app.run_window_command(&ctx, "app.library_close", &json!({"document":id,"discard":true})).unwrap();
    assert!(!app.library_editors.has_documents());
    assert_eq!(app.session.revision(), 0);
}

/// Exercise real native Close delivery and cancellation while an authoring document owns dirty
/// state. The ordinary UI tests verify decisions; this checks that the OS surface survives them.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "opens a native window; run with an available Wayland display"]
fn native_library_close_preserves_unsaved_authoring() -> eframe::Result {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    };
    use winit::platform::wayland::EventLoopBuilderExtWayland as _;
    struct Native {
        app: PcbApp,
        stage: usize,
        due: Instant,
        started: Instant,
        passed: Arc<AtomicBool>,
    }
    impl eframe::App for Native {
        /// Render the real workspace, deliver Close, then verify cancellation on a later native frame.
        fn ui(&mut self, ui: &mut Ui, _: &mut eframe::Frame) {
            let ctx = ui.ctx().clone();
            let closing = ctx.input(|input| input.viewport().close_requested());
            self.app.frame(ui);
            assert!(self.started.elapsed() < Duration::from_secs(20), "native authoring close did not progress");
            match self.stage {
                0 if Instant::now() >= self.due => {
                    self.stage = 1;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                1 if closing => {
                    assert!(matches!(self.app.dialog, Dialog::LibraryClose(None)));
                    assert!(self.app.library_editors.unsaved());
                    assert!(!self.app.close_allowed);
                    self.stage = 2;
                    self.due = Instant::now() + Duration::from_millis(300);
                }
                2 if Instant::now() >= self.due => {
                    assert!(!closing, "native Close was not cancelled");
                    assert!(self.app.library_editors.active.is_some());
                    self.passed.store(true, Ordering::SeqCst);
                    self.app.close_allowed = true;
                    self.stage = 3;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                _ => {}
            }
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
    let passed = Arc::new(AtomicBool::new(false));
    let evidence = passed.clone();
    eframe::run_native(
        "PCBcraft library close regression",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 800.0]),
            persist_window: false,
            event_loop_builder: Some(Box::new(|builder| {
                builder.with_wayland().with_any_thread(true);
            })),
            ..Default::default()
        },
        Box::new(move |cc| {
            let mut app = PcbApp::new(cc);
            app.new_library_item(Kind::Footprint);
            Ok(Box::new(Native { app, stage: 0, due: Instant::now() + Duration::from_millis(500), started: Instant::now(), passed: evidence }))
        }),
    )?;
    assert!(passed.load(Ordering::SeqCst));
    Ok(())
}

/// The rendered canvas selects copper ahead of an enclosing unfilled graphic, commits a move
/// once, and cancels a two-click drawing without consuming history.
#[test]
fn canvas_selection_move_and_cancel_use_library_history() {
    fn click_at(h: &mut Harness<'_, PcbApp>, point: egui::Pos2) {
        h.hover_at(point);
        h.run();
        h.drag_at(point);
        h.run();
        h.drop_at(point);
        h.run();
    }
    let mut app = PcbApp::default();
    app.new_library_item(Kind::Footprint);
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    let id = h.state().library_editors.active.unwrap();
    // Keep this gesture fixture's 1 mm destination visible independently of automatic Fit.
    h.state_mut().library_editors.documents.get_mut(&id).unwrap().camera.set_zoom(8.);
    h.run();
    let area = h.get_by_label("Library canvas").rect();
    let start = h.state().library_editors.documents[&id].session.snapshot().objects[&1].anchor();
    let at = h.state().library_editors.documents[&id].camera.screen(start, area);
    click_at(&mut h, at);
    assert_eq!(h.state().library_editors.documents[&id].selected, Some(1));
    let before = h.state().library_editors.documents[&id].session.snapshot();
    crate::tests::click(&mut h, "Move");
    let target = pcbcraft_engine::schematic::Point { x_nm: 0, y_nm: 1_000_000 };
    let at = h.state().library_editors.documents[&id].camera.screen(target, area);
    h.hover_at(at);
    h.run();
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot().native().unwrap(), before.native().unwrap());
    click_at(&mut h, at);
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot().objects[&1].anchor(), target);
    assert_eq!(h.state().library_editors.documents[&id].session.revision(), 1);
    crate::tests::click(&mut h, "Undo Library Edit");
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot().native().unwrap(), before.native().unwrap());
    h.state_mut().library_editors.documents.get_mut(&id).unwrap().tool =
        canvas::Tool::Draw { kind: canvas::Place::Rectangle, start: None, revision: 2 };
    click_at(&mut h, at);
    h.key_press(egui::Key::Escape);
    h.run();
    assert_eq!(h.state().library_editors.documents[&id].session.revision(), 2);
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot().native().unwrap(), before.native().unwrap());
    assert_eq!(h.state().session.revision(), 0);
}

/// Both definition editors use the shared path handles, while their transactions preserve
/// the library object's style, identity and independent undo history.
#[test]
fn symbol_and_footprint_polygons_share_the_canvas_path_editor() {
    use pcbcraft_engine::{
        authoring::Primitive,
        board::definition::{Artwork, GraphicPurpose},
        library_graphics::{Fill, Geometry, Graphic},
        schematic::Point,
    };
    for kind in [Kind::Symbol, Kind::Footprint] {
        let point = |x_nm, y_nm| Point { x_nm, y_nm };
        let original = [point(5_080_000, -10_160_000), point(10_160_000, -10_160_000), point(10_160_000, -5_080_000), point(5_080_000, -5_080_000)];
        let graphic = |points: Vec<Point>| Graphic {
            geometry: Geometry::Polyline { points, closed: true },
            stroke_nm: 180_000,
            fill: if kind == Kind::Symbol { Fill::Solid } else { Fill::None },
        };
        let primitive = |points: Vec<Point>| {
            if kind == Kind::Symbol {
                Primitive::Graphic { graphic: graphic(points) }
            } else {
                Primitive::Artwork { artwork: Artwork { purpose: GraphicPurpose::Silkscreen, graphic: graphic(points) } }
            }
        };
        let mut app = PcbApp::default();
        app.new_library_item(kind);
        let doc_id = app.library_editors.active.unwrap();
        let doc = app.library_editors.documents.get_mut(&doc_id).unwrap();
        doc.execute(0, Action::Insert { primitive: primitive(original.to_vec()) }).unwrap();
        let object = doc.selected.unwrap();
        let mut h =
            Harness::builder().with_size(egui::vec2(1500., 950.)).with_max_steps(64).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let doc = h.state_mut().library_editors.documents.get_mut(&doc_id).unwrap();
        assert!(doc.camera.set_snap_nm(10_000));
        doc.camera.set_zoom(3.);
        doc.camera.center_on(point(7_620_000, -7_620_000));
        h.run();
        let at = |h: &Harness<'_, PcbApp>, model| {
            let rect = h.get_by_label("Library canvas").rect();
            h.state().library_editors.documents[&doc_id].camera.screen(model, rect)
        };
        let edge = at(&h, point(7_620_000, -10_160_000));
        let resize = at(&h, point(8_890_000, -11_430_000));
        h.hover_at(edge);
        h.run();
        h.drag_at(edge);
        h.run();
        h.hover_at(resize);
        h.run();
        assert_eq!(h.state().library_editors.documents[&doc_id].session.revision(), 1);
        h.drop_at(resize);
        h.run();
        let mut resized = original.to_vec();
        resized[0].y_nm = -11_430_000;
        resized[1].y_nm = -11_430_000;
        let snapshot = |h: &Harness<'_, PcbApp>| h.state().library_editors.documents[&doc_id].session.snapshot();
        assert_eq!(snapshot(&h).objects[&object], primitive(resized.clone()));

        let vertex = point(7_620_000, -11_430_000);
        let edge = at(&h, vertex);
        for label in ["Add vertex", "Remove vertex"] {
            h.hover_at(edge);
            h.run();
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: edge, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
                h.run();
            }
            crate::tests::click(&mut h, label);
            let current = snapshot(&h);
            let authored = &current.objects[&object];
            let mut expected = resized.clone();
            if label == "Add vertex" {
                let geometry = match authored {
                    Primitive::Graphic { graphic } => &graphic.geometry,
                    Primitive::Artwork { artwork } => &artwork.graphic.geometry,
                    other => panic!("expected the original graphic type, got {other:?}"),
                };
                let Geometry::Polyline { points, .. } = geometry else { panic!("expected a vertex path") };
                assert!(points[1].x_nm.abs_diff(vertex.x_nm) <= 4);
                assert_eq!(points[1].y_nm, vertex.y_nm);
                expected.insert(1, points[1]);
            }
            assert_eq!(authored, &primitive(expected));
        }
        assert_eq!(h.state().library_editors.documents[&doc_id].session.revision(), 4);
        for _ in 0..3 {
            crate::tests::click(&mut h, "Undo Library Edit");
        }
        assert_eq!(snapshot(&h).objects[&object], primitive(original.to_vec()));
        assert_eq!(h.state().session.revision(), 0);
    }
}

/// A completed old save cannot clear newer edits, a busy owner's close is refused, and a lost
/// worker releases the slot so file handling remains usable after failure.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn file_completions_keep_their_owner_and_release_disconnected_workers() {
    let mut editors = Editors::default();
    let id = editors.open(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    let old = editors.documents[&id].session.snapshot();
    let send = editors.files.stand_in();
    assert!(editors.close(id, true).is_err());
    editors.edit(id, 0, Action::Delete { objects: vec![3] }).unwrap();
    let other = editors.open(Item::new(Kind::Symbol).unwrap(), false).unwrap();
    send.send(Ok(io::Outcome::Saved(id, old))).unwrap();
    editors.poll();
    assert!(!editors.busy());
    assert!(editors.documents[&id].session.dirty());
    assert!(editors.documents[&other].session.dirty());
    let send = editors.files.stand_in();
    drop(send);
    editors.poll();
    assert!(!editors.busy());
    assert!(editors.notice.as_ref().unwrap().contains("worker stopped"));
    let before = editors.documents[&id].session.snapshot();
    let send = editors.files.stand_in();
    send.send(Ok(io::Outcome::Opened(Box::new((*before).clone())))).unwrap();
    editors.poll();
    assert_eq!(editors.documents.len(), 2);
    assert_eq!(editors.documents[&id].session.snapshot(), before);
    assert!(editors.notice.as_ref().unwrap().contains("already open"));
}

/// Explicit source view IDs select the correct immutable definition, preserve project bytes,
/// and focusing a project pane returns from authoring without discarding its documents.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn definition_forks_and_project_focus_keep_document_authority_separate() {
    let mut app = PcbApp::default();
    app.run("project.new", &json!({"name":"Library fork test"}));
    app.run(pcbcraft_engine::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":1}));
    app.select(pcbcraft_engine::selection::Selection::one_symbol("symbol-2".into()));
    let project = app.session.snapshot().unwrap();
    let view = app.views.active.id();
    let ctx = Context::default();
    app.run_window_command(&ctx, "app.library_fork_selected", &json!({"kind":"symbol","view":view.0})).unwrap();
    let id = app.library_editors.active.unwrap();
    assert_eq!(app.library_editors.documents[&id].session.snapshot().kind, Kind::Symbol);
    assert!(Arc::ptr_eq(&app.session.snapshot().unwrap(), &project));
    assert!(app.run_window_command(&ctx, "app.library_fork_selected", &json!({"kind":"footprint","view":view.0})).is_err());
    app.run_window_command(&ctx, "view.set_workspace", &json!({"view":view.0,"workspace":"schematic"})).unwrap();
    assert!(app.library_editors.active.is_none());
    assert!(app.library_editors.documents.contains_key(&id));
    app.run_window_command(&ctx, "app.library_focus", &json!({"document":id})).unwrap();
    app.run_window_command(&ctx, "app.set_home", &json!({"shown":true})).unwrap();
    assert!(app.library_editors.active.is_none());
    assert!(app.home);
    let send = app.library_editors.files.stand_in();
    use pcbcraft_mcp::live::Editor as _;
    assert!(crate::live_control::LiveEditor { app: &mut app, ctx: &ctx }.refusal("project.rename").is_none());
    drop(send);
}

/// Live view actions share selection/navigation handlers while preserving definition bytes,
/// revisions and history; incompatible camera requests fail before changing selection.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_library_view_actions_leave_authored_data_unchanged() {
    let mut app = PcbApp::default();
    app.new_library_item(Kind::Footprint);
    let id = app.library_editors.active.unwrap();
    let before = app.library_editors.documents[&id].session.snapshot();
    let ctx = Context::default();
    app.run_window_command(&ctx, "app.library_view", &json!({"document":id,"selected":1,"zoom_percent":200,"grid_spacing_nm":50_000})).unwrap();
    assert_eq!(app.library_editors.documents[&id].selected, Some(1));
    assert_eq!(app.library_editors.documents[&id].camera.zoom(), 2.0);
    assert!(app.run_window_command(&ctx, "app.library_view", &json!({"document":id,"selected":999,"zoom_percent":100})).is_err());
    assert_eq!(app.library_editors.documents[&id].camera.zoom(), 2.0);
    assert!(app.run_window_command(&ctx, "app.library_view", &json!({"document":id,"definition":true,"selected":2})).is_err());
    assert!(app.run_window_command(&ctx, "app.library_view", &json!({"document":id,"fit":true,"zoom_percent":100})).is_err());
    assert!(Arc::ptr_eq(&before, &app.library_editors.documents[&id].session.snapshot()));
    assert_eq!(app.library_editors.documents[&id].session.revision(), 0);
}
/// A completed exact save can release the existing close request, while cancel leaves no
/// remembered approval that could close a later dirty document.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn completed_save_releases_close_without_reusing_a_cancelled_decision() {
    let mut app = PcbApp::default();
    let id = app.library_editors.open(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    let snapshot = app.library_editors.documents[&id].session.snapshot();
    let ctx = Context::default();
    let send = app.library_editors.files.stand_in();
    app.begin_project_close(&ctx);
    assert!(matches!(app.dialog, Dialog::LibraryClose(None)));
    app.dialog = Dialog::None;
    send.send(Ok(io::Outcome::Saved(id, snapshot))).unwrap();
    app.library_editors.poll();
    assert!(!app.close_allowed);
    app.library_close_dialog(&ctx, None);
    assert!(app.close_allowed);
    let mut app = PcbApp::default();
    let id = app.library_editors.open(Item::new(Kind::Footprint).unwrap(), true).unwrap();
    app.library_close_dialog(&ctx, Some(id));
    assert!(!app.library_editors.has_documents());
    assert!(!app.close_allowed);
}

/// The picker enters four real workspaces at both widths. Re-entering a definition workspace
/// resumes its last document instead of creating another; File contains no workspace launchers.
#[test]
fn library_workspaces_share_the_picker_with_schematic_and_pcb() {
    for width in [1200.0, 420.0] {
        let mut h = crate::tests::harness();
        h.state_mut().run("project.new", &json!({"name":"Workspace editors"}));
        h.set_size(egui::vec2(width, 900.0));
        h.run();
        let before = h.state().session.snapshot().unwrap();
        let revision = h.state().session.revision();
        for project in ["Schematic", "PCB Layout"] {
            choose_workspace(&mut h, project);
            assert!(h.state().library_editors.active.is_none());
            crate::tests::click(&mut h, "File");
            for label in ["New Symbol", "New Footprint", "Library Workspaces", "Open Library Item…"] {
                assert!(h.query_by_label(label).is_none(), "{label}");
            }
            h.key_press(egui::Key::Escape);
            h.run();
            for (workspace, kind, tool) in [("Symbol Editor", Kind::Symbol, "Pin"), ("Footprint Editor", Kind::Footprint, "Pad")] {
                choose_workspace(&mut h, workspace);
                assert_eq!(h.state().library_editors.active_kind(), Some(kind));
                h.get_by_label(tool);
                h.get_by_label("Library canvas");
                h.get_by_label("Collapse Browser");
                h.get_by_label("Collapse Properties");
                h.get_by_label("More properties…").scroll_to_me();
                h.run();
                crate::tests::click(&mut h, "More properties…");
                assert!(matches!(h.state().dialog, Dialog::LibraryProperties(..)));
                crate::tests::click(&mut h, "Close");
                crate::tests::click(&mut h, "View");
                crate::tests::click(&mut h, "Fit");
                crate::tests::click(&mut h, "Draw");
                choose_workspace(&mut h, project);
            }
        }
        assert_eq!(h.state().library_editors.documents.len(), 2);
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        assert_eq!(h.state().session.revision(), revision);
    }
}
/// Drive the same workspace tile in project and definition shells, including compact headers.
fn choose_workspace(h: &mut Harness<'_, PcbApp>, label: &str) {
    crate::tests::click(h, "Choose workspace");
    h.get_by_label(label).scroll_to_me();
    h.run();
    crate::tests::click(h, label);
}
/// A pending invalid field, selection, camera and ribbon tab remain owned by their definition;
/// workspace switching also preserves the project view and has no electrical undo effect.
#[test]
fn workspace_roundtrip_retains_definition_drafts_and_project_views() {
    let mut h = crate::tests::harness();
    crate::tests::resistor_at_origin(&mut h);
    let view = h.state().views.active.id();
    let revision = h.state().session.revision();
    choose_workspace(&mut h, "Footprint Editor");
    let id = h.state().library_editors.active.unwrap();
    crate::tests::click(&mut h, "Pad 1 · 1");
    h.get_by_label("Width (mm)").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Width (mm)").type_text("bad length");
    h.run();
    crate::tests::click(&mut h, "View");
    h.state_mut().library_editors.documents.get_mut(&id).unwrap().camera.set_zoom(2.0);
    choose_workspace(&mut h, "Symbol Editor");
    choose_workspace(&mut h, "Schematic");
    assert_eq!(h.state().views.active.id(), view);
    assert_eq!(h.state().session.revision(), revision);
    choose_workspace(&mut h, "Footprint Editor");
    assert_eq!(h.state().library_editors.active, Some(id));
    let doc = &h.state().library_editors.documents[&id];
    assert!(doc.pending());
    assert_eq!(doc.selected, Some(1));
    assert_eq!(doc.ribbon_tab, 2);
    assert_eq!(doc.camera.zoom(), 2.0);
    crate::tests::click(&mut h, "Apply");
    assert!(h.state().library_editors.documents[&id].error.is_some());
    assert_eq!(h.state().session.revision(), revision);
}
/// Definition navigation belongs to the workspace picker, not the project Home menu;
/// selected-copy opens the matching workspace without modifying placed instances.
#[test]
fn home_has_no_editor_shortcuts_and_selected_copy_enters_the_workspace() {
    let mut home = crate::tests::startup_harness();
    assert!(home.query_by_label("Symbol Editor").is_none());
    assert!(home.query_by_label("Footprint Editor").is_none());
    crate::tests::click(&mut home, "New Project");
    choose_workspace(&mut home, "Footprint Editor");
    crate::tests::click(&mut home, "Home");
    assert!(home.state().home);
    assert!(home.query_by_label("Symbol Editor").is_none());
    assert!(home.query_by_label("Footprint Editor").is_none());
    let mut h = crate::tests::harness();
    crate::tests::resistor_at_origin(&mut h);
    h.state_mut().select(pcbcraft_engine::selection::Selection::one_symbol("symbol-2".into()));
    h.run();
    let before = h.state().session.snapshot().unwrap();
    crate::tests::click(&mut h, "Libraries");
    crate::tests::click(&mut h, "Edit the selected definition as an independent copy");
    assert_eq!(h.state().library_editors.active_kind(), Some(Kind::Symbol));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}
/// Shared navigation rejects unavailable project workspaces, invalid placement/keys and a
/// first-entry document-limit failure before changing the active document or its draft.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_workspace_navigation_resumes_and_rejects_without_losing_focus() {
    let mut app = PcbApp::default();
    let ctx = Context::default();
    for _ in 0..4 {
        app.new_library_item(Kind::Symbol);
    }
    let active = app.library_editors.active;
    for workspace in ["board", "schematic", "footprint_editor", "unknown"] {
        assert!(app.run_window_command(&ctx, "app.set_workspace", &json!({"workspace":workspace})).is_err());
        assert_eq!(app.library_editors.active, active);
    }
    assert!(app.enter_workspace(&ctx, "unknown", pcbcraft_chrome::workspace::Placement::Window).is_err());
    assert_eq!(app.library_editors.active, active);
    app.run_window_command(&ctx, "app.set_home", &json!({"shown":true})).unwrap();
    app.run_window_command(&ctx, "app.set_workspace", &json!({"workspace":"symbol_editor"})).unwrap();
    assert_eq!(app.library_editors.active, active);
    assert!(!app.home);
    assert_eq!(app.library_editors.documents.len(), 4);
    assert_eq!(crate::window_commands::inspect_editor(&app)["workspace"], "symbol_editor");
    app.run_window_command(&ctx, "app.library_view", &json!({"document":active.unwrap(),"ribbon_tab":"view","browser":false,"properties":false}))
        .unwrap();
    let doc = &app.library_editors.documents[&active.unwrap()];
    assert_eq!(doc.ribbon_tab, 2);
    assert_eq!(doc.collapsed, [true, true]);
}

/// Picker placement mixes project and definition panes; the shared layout never changes
/// the project, and a definition pane cannot authorize commands against its parked sheet.
#[test]
fn picker_opens_definition_beside_project_without_changing_project_authority() {
    let mut h = crate::tests::harness();
    crate::tests::resistor_at_origin(&mut h);
    let project = h.state().session.snapshot().unwrap();
    let source = h.state().views.active.id();
    crate::tests::click(&mut h, "Choose workspace");
    crate::tests::click(&mut h, "Open Footprint Editor beside current view");
    assert!(h.state().views.is_split());
    let definition = h.state().views.active.id();
    assert_ne!(definition, source);
    assert!(h.state().views.view(source).unwrap().input(&h.state().session).is_some());
    assert!(h.state().views.view(definition).unwrap().input(&h.state().session).is_none());
    assert!(h.state().views.view(definition).unwrap().control_input(&h.state().session).is_some());
    h.get_by_label("Library canvas");
    assert!(Arc::ptr_eq(&project, &h.state().session.snapshot().unwrap()));
}
/// Two views share one definition/history while selection, camera, panels and fields stay
/// independently addressed. Stale definition revisions remain invalid after a sibling edits.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn split_definition_views_share_history_with_independent_presentation() {
    let mut h = crate::tests::harness();
    h.state_mut().run("project.new", &json!({"name":"Library split"}));
    h.run();
    choose_workspace(&mut h, "Footprint Editor");
    let document = h.state().library_editors.active.unwrap();
    crate::tests::click(&mut h, "View");
    h.get_by_label("Split library workspace").scroll_to_me();
    h.run();
    crate::tests::click(&mut h, "Split library workspace");
    let panes = h.state().views.hosts[&h.state().views.main].layout.visible_views();
    assert_eq!(panes.len(), 2);
    assert!(panes.iter().all(|id| h.state().views.view(*id).unwrap().definition == Some(document)));
    assert_eq!(h.get_all_by_label("Library canvas").count(), 2);
    let ctx = Context::default();
    for (view, selected, zoom) in [(panes[0], 1, 200), (panes[1], 2, 100)] {
        h.state_mut()
            .run_window_command(&ctx, "app.library_view", &json!({"document":document,"view":view.0,"selected":selected,"zoom_percent":zoom}))
            .unwrap();
    }
    h.run();
    assert_eq!(h.state().inspect_library_view(panes[0]).unwrap()["zoom_percent"].as_f64(), Some(200.0));
    assert_eq!(h.state().inspect_library_view(panes[1]).unwrap()["zoom_percent"].as_f64(), Some(100.0));
    assert_eq!(h.state().inspect_library_view(panes[0]).unwrap()["selected"], 1);
    assert_eq!(h.state().inspect_library_view(panes[1]).unwrap()["selected"], 2);
    h.state_mut()
        .run_window_command(
            &ctx,
            "app.library_edit",
            &json!({"document":document,"expected_revision":0,"action":{"operation":"delete","objects":[3]}}),
        )
        .unwrap();
    assert!(
        h.state_mut()
            .run_window_command(&ctx, "app.library_edit", &json!({"document":document,"expected_revision":0,"action":{"operation":"undo"}}))
            .is_err()
    );
    h.state_mut()
        .run_window_command(&ctx, "app.library_edit", &json!({"document":document,"expected_revision":1,"action":{"operation":"undo"}}))
        .unwrap();
    assert_eq!(h.state().library_editors.documents[&document].session.snapshot().objects.len(), 3);
    assert_eq!(h.state().session.revision(), 1);
}
/// A closed pane's pending numeric input remains owned by the definition. Save and final
/// Close still protect it; Review recovers that input into a surviving pane.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn closing_a_library_pane_retains_pending_fields_for_review() {
    let mut h = crate::tests::harness();
    h.state_mut().new_library_item(Kind::Footprint);
    h.run();
    let document = h.state().library_editors.active.unwrap();
    crate::tests::click(&mut h, "Pad 1 · 1");
    h.get_by_label("Width (mm)").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Width (mm)").type_text("bad length");
    h.run();
    let source = h.state().views.active.id();
    let ctx = Context::default();
    h.state_mut().run_window_command(&ctx, "view.open_beside", &json!({"view":source.0})).unwrap();
    h.run();
    h.state_mut().run_window_command(&ctx, "view.close", &json!({"view":source.0})).unwrap();
    h.run();
    assert!(h.state().views.view(source).is_none());
    assert!(h.state().library_editors.documents[&document].pending_any());
    assert!(h.state_mut().library_editors.save(&ctx, document, false).is_err());
    h.state_mut().begin_project_close(&ctx);
    h.run();
    crate::tests::click(&mut h, "Review edits");
    let doc = &h.state().library_editors.documents[&document];
    assert!(doc.pending());
    assert_eq!(doc.selected, Some(1));
    crate::tests::click(&mut h, "Apply");
    assert!(h.state().library_editors.documents[&document].error.is_some());
    assert!(!h.state().close_allowed);
}
/// Detach/dock and original-host retirement preserve the same definition identity and
/// history; closing a non-last native host does not discard a hidden definition document.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn library_detach_dock_and_host_retirement_keep_the_document_owned() {
    let mut h = crate::tests::harness();
    h.state_mut().run("project.new", &json!({"name":"Library hosts"}));
    h.run();
    choose_workspace(&mut h, "Symbol Editor");
    let document = h.state().library_editors.active.unwrap();
    let original = h.state().views.active.id();
    let ctx = Context::default();
    let snapshot = h.state().library_editors.documents[&document].session.snapshot();
    h.state_mut().run_window_command(&ctx, "view.detach", &json!({"view":original.0})).unwrap();
    h.run();
    let child = h.state().views.owner(original).unwrap();
    assert_ne!(child, h.state().views.main);
    assert_eq!(h.state().views.view(original).unwrap().definition, Some(document));
    h.state_mut().run_window_command(&ctx, "view.dock", &json!({"view":original.0})).unwrap();
    h.run();
    assert_eq!(h.state().views.owner(original), Some(h.state().views.main));
    h.state_mut().run_window_command(&ctx, "view.detach", &json!({"view":original.0})).unwrap();
    h.run();
    let child = h.state().views.owner(original).unwrap();
    let main = h.state().views.main;
    h.state_mut().close_editor_host(main, &ctx);
    h.run();
    assert_eq!(h.state().views.main, child);
    assert!(Arc::ptr_eq(&snapshot, &h.state().library_editors.documents[&document].session.snapshot()));
    h.state_mut().close_editor_host(child, &ctx);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::LibraryClose(None)));
    assert!(h.state().views.hosts.contains_key(&child));
    assert!(!h.state().close_allowed);
}

/// Another view's committed edit invalidates a field draft's revision but cannot erase its
/// pending input. Applying the old draft refuses atomically until the user resets it.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn sibling_edits_preserve_stale_property_input_until_reset() {
    let mut h = crate::tests::harness();
    h.state_mut().new_library_item(Kind::Footprint);
    h.run();
    let document = h.state().library_editors.active.unwrap();
    crate::tests::click(&mut h, "Pad 1 · 1");
    h.get_by_label("Width (mm)").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Width (mm)").type_text("0.8");
    h.run();
    let source = h.state().views.active.id();
    h.state_mut().views.open_beside(source).unwrap();
    h.run();
    let mut primitive = h.state().library_editors.documents[&document].session.snapshot().objects[&2].clone();
    if let pcbcraft_engine::authoring::Primitive::Pad { pad } = &mut primitive {
        pad.width_nm = 800_000;
    }
    h.state_mut().library_editors.documents.get_mut(&document).unwrap().session.execute(0, Action::Replace { object: 2, primitive }).unwrap();
    h.run();
    assert!(h.state().library_editors.documents[&document].pending());
    assert!(h.state().library_editors.documents[&document].error.as_deref().unwrap().contains("reset"));
    assert!(h.state_mut().library_editors.edit(document, 0, Action::Undo).is_err());
    assert_eq!(h.state().library_editors.documents[&document].session.revision(), 1);
}

/// Exercise actual native surfaces, including retiring the original window, keeping the
/// definition host alive, and cancelling the final Close while that definition is unsaved.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "opens native windows; run with an available Wayland display"]
fn native_definition_window_survives_original_close() -> eframe::Result {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    };
    use winit::platform::wayland::EventLoopBuilderExtWayland as _;
    struct Native {
        app: PcbApp,
        stage: u8,
        due: Instant,
        started: Instant,
        child: Option<crate::editor_layout::HostId>,
        passed: Arc<AtomicBool>,
    }
    impl eframe::App for Native {
        /// Native close events travel through the same host/unsaved controller used by the app.
        fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
            let ctx = ui.ctx().clone();
            eframe::App::ui(&mut self.app, ui, frame);
            assert!(self.started.elapsed() < Duration::from_secs(20), "native definition host did not progress");
            if Instant::now() >= self.due {
                match self.stage {
                    0 => {
                        self.app.enter_workspace(&ctx, "footprint_editor", pcbcraft_chrome::workspace::Placement::Window).unwrap();
                        self.child = self.app.views.detached_hosts().first().copied();
                        self.stage = 1;
                    }
                    1 => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        self.stage = 2;
                    }
                    2 if !self.app.views.hosts.contains_key(&crate::editor_layout::HostId::ROOT) => {
                        let child = self.child.unwrap();
                        assert_eq!(self.app.views.main, child);
                        let view = self.app.views.hosts[&child].focused;
                        assert!(self.app.views.view(view).unwrap().definition.is_some());
                        ctx.send_viewport_cmd_to(crate::editor_windows::viewport(child), egui::ViewportCommand::Close);
                        self.stage = 3;
                    }
                    3 if matches!(self.app.dialog, Dialog::LibraryClose(None)) => {
                        assert!(!self.app.close_allowed);
                        assert!(self.app.views.hosts.contains_key(&self.child.unwrap()));
                        self.app.dialog = Dialog::None;
                        self.stage = 4;
                    }
                    4 => {
                        assert!(self.app.library_editors.unsaved());
                        assert!(self.app.views.hosts.contains_key(&self.child.unwrap()));
                        self.passed.store(true, Ordering::SeqCst);
                        self.app.close_allowed = true;
                        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
                        self.stage = 5;
                    }
                    _ => {}
                }
                self.due = Instant::now() + Duration::from_millis(250);
            }
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
    let passed = Arc::new(AtomicBool::new(false));
    let evidence = passed.clone();
    eframe::run_native(
        "PCBcraft definition window regression",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 800.0]),
            persist_window: false,
            event_loop_builder: Some(Box::new(|builder| {
                builder.with_wayland().with_any_thread(true);
            })),
            ..Default::default()
        },
        Box::new(move |cc| {
            let mut app = PcbApp::new(cc);
            app.home = false;
            Ok(Box::new(Native {
                app,
                stage: 0,
                due: Instant::now() + Duration::from_millis(400),
                started: Instant::now(),
                child: None,
                passed: evidence,
            }))
        }),
    )?;
    assert!(passed.load(Ordering::SeqCst));
    Ok(())
}

/// Repainting an unfocused original window cancels its own preview without cancelling an
/// unfinished two-click tool in the focused definition window.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn unfocused_host_does_not_cancel_another_windows_definition_tool() {
    let mut app = PcbApp::default();
    app.new_library_item(Kind::Footprint);
    let document = app.library_editors.active.unwrap();
    let root = app.views.active.id();
    let host = app.views.open_window(root).unwrap();
    let child = app.views.hosts[&host].focused;
    app.bind_library_view(child);
    app.library_editors.documents.get_mut(&document).unwrap().tool =
        canvas::Tool::Draw { kind: canvas::Place::Pad, start: Some(pcbcraft_engine::schematic::Point { x_nm: 0, y_nm: 0 }), revision: 0 };
    app.bind_editor_host(app.views.main);
    app.cancel_library_host_gestures();
    app.bind_library_view(child);
    assert!(matches!(app.library_editors.documents[&document].tool, canvas::Tool::Draw { start: Some(_), .. }));
}

/// A live runtime view can have switched to another definition. Review must restore the
/// requested document's pending fields, rather than trusting that old view ID alone.
#[test]
fn review_pending_fields_after_retargeting_their_original_pane() {
    let mut h = crate::tests::harness();
    h.state_mut().new_library_item(Kind::Footprint);
    h.run();
    let document = h.state().library_editors.active.unwrap();
    crate::tests::click(&mut h, "Pad 1 · 1");
    h.get_by_label("Width (mm)").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Width (mm)").type_text("bad length");
    h.run();
    choose_workspace(&mut h, "Symbol Editor");
    let ctx = Context::default();
    h.state_mut().begin_project_close(&ctx);
    h.run();
    crate::tests::click(&mut h, "Review edits");
    assert_eq!(h.state().views.active.definition, Some(document));
    assert_eq!(h.state().library_editors.active_kind(), Some(Kind::Footprint));
    assert!(h.state().library_editors.documents[&document].pending());
    crate::tests::click(&mut h, "Apply");
    assert!(h.state().library_editors.documents[&document].error.is_some());
}

/// Ribbon arrays use the captured document revision and one undo entry; duplicate cancellation
/// and compact/expanded pin forms leave the project session independent.
#[test]
fn authoring_basics_ribbon_arrays_and_pin_properties() {
    let mut app = PcbApp::default();
    app.new_library_item(Kind::Symbol);
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    let id = h.state().library_editors.active.unwrap();
    crate::tests::click(&mut h, "Pin 1 · 1");
    assert!(h.query_by_label("Length (mm)").is_some());
    crate::tests::click(&mut h, "More properties…");
    assert!(matches!(h.state().dialog, Dialog::LibraryProperties(..)));
    crate::tests::click(&mut h, "Close");
    let before = h.state().library_editors.documents[&id].session.snapshot();
    crate::tests::click(&mut h, "Array");
    assert!(matches!(h.state().dialog, Dialog::LibraryArray(..)));
    for (label, value) in [("Columns", "1"), ("Rows", "4"), ("Row pitch (mm)", "2.54")] {
        h.get_by_label(label).click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label(label).type_text(value);
        h.run();
    }
    crate::tests::click(&mut h, "Create");
    let changed = h.state().library_editors.documents[&id].session.snapshot();
    assert_eq!(changed.objects.len(), before.objects.len() + 3);
    assert_eq!(h.state().library_editors.documents[&id].session.revision(), 1);
    crate::tests::click(&mut h, "Undo Library Edit");
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot().native().unwrap(), before.native().unwrap());
    crate::tests::click(&mut h, "Pin 1 · 1");
    crate::tests::click(&mut h, "Duplicate");
    assert!(matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Paste { .. }));
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Select));
    assert_eq!(h.state().library_editors.documents[&id].session.revision(), 2);
    assert_eq!(h.state().session.revision(), 0);
}

/// A stale array cannot overwrite edits from another pane; selection is available through the
/// same live contract and an invalid duplicate selection is refused before changing presentation.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn library_group_selection_and_stale_array_do_not_overwrite_edits() {
    let mut app = PcbApp::default();
    app.new_library_item(Kind::Footprint);
    let id = app.library_editors.active.unwrap();
    let view = app.views.active.id();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    let ctx = h.ctx.clone();
    h.state_mut().run_window_command(&ctx, "app.library_view", &json!({"document":id,"view":view.0,"selected_objects":[1,2]})).unwrap();
    assert_eq!(h.state().library_editors.documents[&id].selection.len(), 2);
    assert!(h.state_mut().run_window_command(&ctx, "app.library_view", &json!({"document":id,"view":view.0,"selected_objects":[1,1]})).is_err());
    let doc = &h.state().library_editors.documents[&id];
    let draft = editing::ArrayDraft::new(id, view, doc);
    h.state_mut().library_editors.edit(id, 0, Action::Delete { objects: vec![3] }).unwrap();
    h.state_mut().dialog = Dialog::LibraryArray(Box::new(draft));
    h.run();
    crate::tests::click(&mut h, "Create");
    assert!(matches!(h.state().dialog, Dialog::LibraryArray(..)));
    assert_eq!(h.state().library_editors.documents[&id].session.revision(), 1);
    assert_eq!(h.state().library_editors.documents[&id].session.snapshot().objects.len(), 2);
    crate::tests::click(&mut h, "Cancel");
}

/// Unit selection keeps the inactive drawings, while adding a draft never duplicates electrical pins.
#[test]
fn multiunit_symbol_editor_selects_and_adds_units_through_shared_actions() {
    for width in [1200.0, 420.0] {
        let definition =
            pcbcraft_engine::library::parse_symbol(include_str!("../../../../examples/native-parts/symbols/dual-amplifier.json")).unwrap();
        let mut item = Item::from_symbol(&definition).unwrap();
        item.license = "MIT OR Apache-2.0".into();
        let mut app = PcbApp::default();
        let id = app.library_editors.open(item, false).unwrap();
        app.run_window_command(&Context::default(), "app.library_focus", &json!({"document":id})).unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(width, 900.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let before = h.state().library_editors.documents[&id].session.snapshot();
        h.get_by_role_and_label(egui::Role::ComboBox, "Unit").click();
        h.run();
        crate::tests::click(&mut h, "b");
        let selected = h.state().library_editors.documents[&id].session.snapshot();
        assert_eq!(selected.unit, "b");
        assert_eq!(selected.other_units["a"], before.objects);
        assert_eq!(selected.symbol().unwrap().units().len(), 2);
        crate::tests::click(&mut h, "Add unit");
        let added = h.state().library_editors.documents[&id].session.snapshot();
        assert_eq!(added.unit, "unit-1");
        assert!(added.objects.is_empty());
        assert_eq!(added.other_units.len(), 2);
        assert!(added.native().is_err());
        assert!(h.state().session.project().is_none());
    }
}

/// Normalized Copy/Paste events reach the same internal clipboard operations as the raw-key
/// path: canvas targeting still gates them, one event performs one operation and pending
/// property fields still refuse paste.
#[test]
fn normalized_clipboard_events_drive_library_copy_and_paste() {
    for kind in [Kind::Symbol, Kind::Footprint] {
        let mut app = PcbApp::default();
        app.new_library_item(kind);
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
        h.run();
        let id = h.state().library_editors.active.unwrap();
        let first = *h.state().library_editors.documents[&id].session.snapshot().objects.keys().next().unwrap();
        h.state_mut().library_editors.documents.get_mut(&id).unwrap().selection = [first].into_iter().collect();
        let revision = h.state().library_editors.documents[&id].session.revision();
        // Away from the canvas the events keep their targeting gate.
        h.hover_at(egui::pos2(10.0, 10.0));
        h.event(egui::Event::Copy);
        h.event(egui::Event::Paste(String::new()));
        h.run();
        assert!(
            matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Select),
            "{kind:?} events away from the canvas must not arm paste"
        );
        let area = h.get_by_label("Library canvas").rect();
        h.hover_at(area.center());
        h.run();
        h.event(egui::Event::Copy);
        h.run();
        assert!(matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Select));
        h.event(egui::Event::Paste(String::new()));
        h.run();
        assert!(
            matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Paste { .. }),
            "{kind:?} normalized Paste must arm the preview"
        );
        assert_eq!(h.state().library_editors.documents[&id].session.revision(), revision, "{kind:?} copy/paste must not change authored state");
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Select));
        // Raw-key presses still work when an extra modifier bypasses native translation.
        h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::C);
        h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::V);
        h.run();
        assert!(
            matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Paste { .. }),
            "{kind:?} raw-key path must still arm the preview"
        );
        // Same-frame input resolves in event order: an Escape after a paste
        // cancels it, while a paste after the frame's Escape still arms.
        h.event(egui::Event::Paste(String::new()));
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(
            matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Select),
            "{kind:?} Escape later in the frame must cancel the paste"
        );
        h.key_press(egui::Key::Escape);
        h.event(egui::Event::Paste(String::new()));
        h.run();
        assert!(
            matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Paste { .. }),
            "{kind:?} a paste after the frame's Escape must still arm"
        );
        // The raw shifted chord follows the same ordering.
        h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::V);
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(
            matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Select),
            "{kind:?} raw-key paste before a same-frame Escape must cancel"
        );
        // Ctrl+D arms the same preview and resolves in the same event order.
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(
            matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Select),
            "{kind:?} Escape after a same-frame duplicate must cancel it"
        );
        h.key_press(egui::Key::Escape);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
        h.run();
        assert!(
            matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Paste { .. }),
            "{kind:?} a duplicate after the frame's Escape must still arm"
        );
        h.key_press(egui::Key::Escape);
        h.run();
        // Cut is copy + delete as one undoable edit; the cut selection still pastes.
        let count = h.state().library_editors.documents[&id].session.snapshot().objects.len();
        // Seed different content so Paste cannot pass using the preceding Copy's clipboard.
        let ctx = h.ctx.clone();
        let doc = h.state_mut().library_editors.documents.get_mut(&id).unwrap();
        let item = doc.session.snapshot();
        let expected = item.objects[&first].clone();
        let other = *item.objects.keys().find(|object| **object != first).unwrap();
        doc.selection = [other].into_iter().collect();
        doc.copy(&ctx);
        doc.selection = [first].into_iter().collect();
        h.event(egui::Event::Cut);
        h.run();
        let doc = &h.state().library_editors.documents[&id];
        assert_eq!(doc.session.snapshot().objects.len(), count - 1, "{kind:?} cut removes the selection");
        assert_eq!(doc.session.revision(), revision + 1, "{kind:?} cut is a single undoable edit");
        h.event(egui::Event::Paste(String::new()));
        h.run();
        assert!(
            matches!(&h.state().library_editors.documents[&id].tool, canvas::Tool::Paste { primitives, .. } if primitives == &[expected]),
            "{kind:?} the cut selection must reach the internal clipboard"
        );
        h.key_press(egui::Key::Escape);
        h.run();
        crate::tests::click(&mut h, "Undo Library Edit");
        h.run();
        assert_eq!(h.state().library_editors.documents[&id].session.snapshot().objects.len(), count, "{kind:?} one undo restores the cut objects");
        // The raw shifted chord reaches the same cut op (re-hover after the
        // Undo click moved the pointer off the canvas).
        h.state_mut().library_editors.documents.get_mut(&id).unwrap().selection = [first].into_iter().collect();
        h.hover_at(area.center());
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::X);
        h.run();
        assert_eq!(h.state().library_editors.documents[&id].session.snapshot().objects.len(), count - 1, "{kind:?} raw shifted X must cut as well");
    }
    // Pending property fields still refuse a normalized Paste.
    let mut app = PcbApp::default();
    app.new_library_item(Kind::Symbol);
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    let id = h.state().library_editors.active.unwrap();
    crate::tests::click(&mut h, "Pin 1 · 1");
    h.get_by_label("Length (mm)").click();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_label("Length (mm)").type_text("9.9");
    h.run();
    assert!(h.state().library_editors.documents[&id].pending(), "field input must leave the draft pending");
    // Reselecting the same pin releases the field without touching the draft or selection.
    crate::tests::click(&mut h, "Pin 1 · 1");
    let area = h.get_by_label("Library canvas").rect();
    h.hover_at(area.center());
    h.run();
    h.event(egui::Event::Paste(String::new()));
    h.run();
    let doc = &h.state().library_editors.documents[&id];
    assert!(doc.pending(), "unapplied fields must survive focus changes");
    assert!(matches!(doc.tool, canvas::Tool::Select));
    assert!(doc.error.is_some(), "pending property fields must refuse a normalized Paste");
    // A pending draft also refuses Cut rather than discarding unapplied fields.
    let revision = doc.session.revision();
    let count = doc.session.snapshot().objects.len();
    h.state_mut().library_editors.documents.get_mut(&id).unwrap().error = None;
    h.event(egui::Event::Cut);
    h.run();
    let doc = &h.state().library_editors.documents[&id];
    assert_eq!(doc.session.revision(), revision, "pending property fields must refuse a normalized Cut");
    assert_eq!(doc.session.snapshot().objects.len(), count);
    assert!(doc.error.is_some(), "cut must surface the pending-fields error");
}

/// A native input batch may end with different modifiers from its X press. Cut follows
/// the pressed event, retains focus gating, and runs once if both event forms arrive.
#[test]
fn library_cut_uses_the_pressed_event_modifiers() {
    for kind in [Kind::Symbol, Kind::Footprint] {
        for (command_at_press, command_at_end, focused, normalized, cuts) in
            [(true, false, true, false, true), (false, true, true, false, false), (true, false, false, false, false), (true, false, true, true, true)]
        {
            let mut app = PcbApp::default();
            app.new_library_item(kind);
            let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
            h.run();
            let id = h.state().library_editors.active.unwrap();
            let doc = h.state_mut().library_editors.documents.get_mut(&id).unwrap();
            let before = doc.session.snapshot();
            let first = *before.objects.keys().next().unwrap();
            doc.selection = [first].into_iter().collect();
            let revision = doc.session.revision();
            let area = h.get_by_label("Library canvas").rect();
            h.hover_at(area.center());
            h.run();
            let modifiers = |command| egui::Modifiers { ctrl: command, command, shift: true, ..Default::default() };
            // Harness::event gives each queued event its own frame. RawInput is needed
            // to reproduce a press and a later modifier change in one actual batch.
            h.input_mut().focused = focused;
            h.input_mut().events.extend([
                egui::Event::ModifiersChanged(modifiers(command_at_press)),
                egui::Event::Key {
                    key: egui::Key::X,
                    physical_key: Some(egui::Key::X),
                    pressed: true,
                    repeat: false,
                    modifiers: modifiers(command_at_press),
                },
                egui::Event::Key {
                    key: egui::Key::X,
                    physical_key: Some(egui::Key::X),
                    pressed: false,
                    repeat: false,
                    modifiers: modifiers(command_at_press),
                },
                egui::Event::ModifiersChanged(modifiers(command_at_end)),
            ]);
            if normalized {
                h.input_mut().events.push(egui::Event::Cut);
            }
            h.run();
            let doc = &h.state().library_editors.documents[&id];
            assert_eq!(
                doc.session.revision(),
                revision + u64::from(cuts),
                "{kind:?}: press={command_at_press}, end={command_at_end}, focus={focused}, normalized={normalized}"
            );
            assert_eq!(doc.session.snapshot().objects.len(), before.objects.len() - usize::from(cuts));
            if !cuts {
                assert_eq!(doc.session.snapshot().as_ref(), before.as_ref());
            }
        }
    }
}

/// A native input batch may end with different modifiers from its C press. Copy
/// follows the pressed event and retains focus gating. Both event forms store the
/// same immutable snapshot, so a duplicated chord cannot corrupt the clipboard.
#[test]
fn library_copy_uses_the_pressed_event_modifiers() {
    for kind in [Kind::Symbol, Kind::Footprint] {
        for (command_at_press, command_at_end, focused, copied) in
            [(true, false, true, true), (false, true, true, false), (true, false, false, false)]
        {
            let mut app = PcbApp::default();
            app.new_library_item(kind);
            let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
            h.run();
            let id = h.state().library_editors.active.unwrap();
            let ctx = h.ctx.clone();
            let doc = h.state_mut().library_editors.documents.get_mut(&id).unwrap();
            let item = doc.session.snapshot();
            let first = *item.objects.keys().next().unwrap();
            let expected = item.objects[&first].clone();
            // Seed the internal clipboard with a different object so a suppressed
            // copy still leaves observable, wrong clipboard content.
            let other = *item.objects.keys().find(|object| **object != first).unwrap();
            let seeded = item.objects[&other].clone();
            doc.selection = [other].into_iter().collect();
            doc.copy(&ctx);
            doc.selection = [first].into_iter().collect();
            let area = h.get_by_label("Library canvas").rect();
            h.hover_at(area.center());
            h.run();
            let modifiers = |command| egui::Modifiers { ctrl: command, command, shift: true, ..Default::default() };
            h.input_mut().focused = focused;
            h.input_mut().events.extend([
                egui::Event::ModifiersChanged(modifiers(command_at_press)),
                egui::Event::Key {
                    key: egui::Key::C,
                    physical_key: Some(egui::Key::C),
                    pressed: true,
                    repeat: false,
                    modifiers: modifiers(command_at_press),
                },
                egui::Event::Key {
                    key: egui::Key::C,
                    physical_key: Some(egui::Key::C),
                    pressed: false,
                    repeat: false,
                    modifiers: modifiers(command_at_press),
                },
                egui::Event::ModifiersChanged(modifiers(command_at_end)),
            ]);
            h.run();
            // The preview reveals exactly what the chord stored on the clipboard.
            h.input_mut().focused = true;
            h.event(egui::Event::Paste(String::new()));
            h.run();
            let expected_primitives = if copied { vec![expected.clone()] } else { vec![seeded.clone()] };
            assert!(
                matches!(&h.state().library_editors.documents[&id].tool, canvas::Tool::Paste { primitives, .. } if primitives == &expected_primitives),
                "{kind:?}: press={command_at_press}, end={command_at_end}, focus={focused} must copy={copied}"
            );
        }
    }
}

/// A native input batch may end with different modifiers from its command chord.
/// Select-all and duplicate follow the pressed event and retain focus gating —
/// a modifier released later in the batch cannot suppress them, a modifier only
/// appearing at batch end cannot fire them, and focus loss cannot arm them.
#[test]
fn library_command_chords_use_the_pressed_event_modifiers() {
    for kind in [Kind::Symbol, Kind::Footprint] {
        for key in [egui::Key::A, egui::Key::D] {
            for (command_at_press, command_at_end, focused, fires) in
                [(true, false, true, true), (false, true, true, false), (true, false, false, false)]
            {
                let mut app = PcbApp::default();
                app.new_library_item(kind);
                let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
                h.run();
                let id = h.state().library_editors.active.unwrap();
                let first = *h.state().library_editors.documents[&id].session.snapshot().objects.keys().next().unwrap();
                let count = h.state().library_editors.documents[&id].session.snapshot().objects.len();
                h.state_mut().library_editors.documents.get_mut(&id).unwrap().selection = [first].into_iter().collect();
                let area = h.get_by_label("Library canvas").rect();
                h.hover_at(area.center());
                h.run();
                // Alt is the extra modifier: Ctrl+Shift+A already belongs to the
                // Assistant toggle, which consumes the chord before the canvas.
                let modifiers = |command| egui::Modifiers { ctrl: command, command, alt: true, ..Default::default() };
                h.input_mut().focused = focused;
                h.input_mut().events.extend([
                    egui::Event::ModifiersChanged(modifiers(command_at_press)),
                    egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: modifiers(command_at_press) },
                    egui::Event::Key { key, physical_key: Some(key), pressed: false, repeat: false, modifiers: modifiers(command_at_press) },
                    egui::Event::ModifiersChanged(modifiers(command_at_end)),
                ]);
                h.run();
                let doc = &h.state().library_editors.documents[&id];
                match (key, fires) {
                    (egui::Key::A, true) => {
                        assert_eq!(doc.selection.len(), count, "{kind:?}: Ctrl+A at press must select all (end={command_at_end}, focus={focused})");
                    }
                    (egui::Key::D, true) => {
                        assert!(
                            matches!(doc.tool, canvas::Tool::Paste { .. }),
                            "{kind:?}: Ctrl+D at press must arm the preview (end={command_at_end}, focus={focused})"
                        );
                    }
                    _ => {
                        assert_eq!(
                            doc.selection.len(),
                            1,
                            "{kind:?}: {key:?} without command at press must not fire (end={command_at_end}, focus={focused})"
                        );
                        assert!(
                            matches!(doc.tool, canvas::Tool::Select),
                            "{kind:?}: {key:?} without command at press must not arm (end={command_at_end}, focus={focused})"
                        );
                    }
                }
            }
        }
    }
}

/// Tool letters and R's direction read the pressed event's own modifiers too: a
/// command released before batch end cannot leak a chord into a tool start, a
/// command arriving after the letter cannot suppress it, and a shift released
/// before batch end cannot flip the rotation direction.
#[test]
fn library_tool_keys_and_rotate_use_the_pressed_event_modifiers() {
    for kind in [Kind::Symbol, Kind::Footprint] {
        for key in [egui::Key::P, egui::Key::L, egui::Key::T] {
            for (command_at_press, command_at_end, focused, starts) in
                [(false, false, true, true), (true, false, true, false), (false, true, true, true), (false, false, false, false)]
            {
                let mut app = PcbApp::default();
                app.new_library_item(kind);
                let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
                h.run();
                let id = h.state().library_editors.active.unwrap();
                let area = h.get_by_label("Library canvas").rect();
                h.hover_at(area.center());
                h.run();
                let modifiers = |command| egui::Modifiers { ctrl: command, command, ..Default::default() };
                h.input_mut().focused = focused;
                h.input_mut().events.extend([
                    egui::Event::ModifiersChanged(modifiers(command_at_press)),
                    egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: modifiers(command_at_press) },
                    egui::Event::Key { key, physical_key: Some(key), pressed: false, repeat: false, modifiers: modifiers(command_at_press) },
                    egui::Event::ModifiersChanged(modifiers(command_at_end)),
                ]);
                h.run();
                assert_eq!(
                    matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Draw { .. }),
                    starts,
                    "{kind:?}: {key:?} tool start must follow the press (press={command_at_press}, end={command_at_end}, focus={focused})"
                );
            }
        }
        // The selection rotates around the first object's anchor, so the last
        // object's anchor lands in a different place for +90 and -90 turns.
        let anchor_after = |shift_at_press: bool, shift_at_end: bool, focused: bool| {
            let mut app = PcbApp::default();
            app.new_library_item(kind);
            let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
            h.run();
            let id = h.state().library_editors.active.unwrap();
            let all: Vec<u64> = h.state().library_editors.documents[&id].session.snapshot().objects.keys().copied().collect();
            let last = *all.last().unwrap();
            h.state_mut().library_editors.documents.get_mut(&id).unwrap().selection = all.into_iter().collect();
            let area = h.get_by_label("Library canvas").rect();
            h.hover_at(area.center());
            h.run();
            let modifiers = |shift| egui::Modifiers { shift, ..Default::default() };
            h.input_mut().focused = focused;
            h.input_mut().events.extend([
                egui::Event::ModifiersChanged(modifiers(shift_at_press)),
                egui::Event::Key {
                    key: egui::Key::R,
                    physical_key: Some(egui::Key::R),
                    pressed: true,
                    repeat: false,
                    modifiers: modifiers(shift_at_press),
                },
                egui::Event::Key {
                    key: egui::Key::R,
                    physical_key: Some(egui::Key::R),
                    pressed: false,
                    repeat: false,
                    modifiers: modifiers(shift_at_press),
                },
                egui::Event::ModifiersChanged(modifiers(shift_at_end)),
            ]);
            h.run();
            h.state().library_editors.documents[&id].session.snapshot().objects[&last].anchor()
        };
        assert_eq!(anchor_after(true, false, true), anchor_after(true, true, true), "{kind:?}: Shift+R direction must follow the press event");
        assert_eq!(anchor_after(false, true, true), anchor_after(false, false, true), "{kind:?}: R must ignore a shift arriving after the letter");
        assert_ne!(anchor_after(true, true, true), anchor_after(false, false, true), "{kind:?}: +90 and -90 quarter turns must land differently");
        assert_ne!(anchor_after(true, false, false), anchor_after(true, true, true), "{kind:?}: a focus-loss batch must not rotate");
    }
}

/// Click extend reads the release event's own modifiers too: a shift held at the
/// click still extends the selection even if released before batch end, and a
/// shift arriving after the release cannot fake an extend.
#[test]
fn library_click_extend_uses_the_release_event_modifiers() {
    for kind in [Kind::Symbol, Kind::Footprint] {
        // Click extend: release carries shift, the batch ends without it.
        let click = |release_shift: bool, end_shift: bool| {
            let mut app = PcbApp::default();
            app.new_library_item(kind);
            let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
            h.run();
            let id = h.state().library_editors.active.unwrap();
            let first = *h.state().library_editors.documents[&id].session.snapshot().objects.keys().next().unwrap();
            h.state_mut().library_editors.documents.get_mut(&id).unwrap().selection = [first].into_iter().collect();
            // Center on empty model space so the click hits no primitive or panel.
            h.state_mut()
                .library_editors
                .documents
                .get_mut(&id)
                .unwrap()
                .camera
                .center_on(pcbcraft_engine::schematic::Point { x_nm: 30_000_000, y_nm: 30_000_000 });
            h.run();
            let empty = h.get_by_label("Library canvas").rect().center();
            h.hover_at(empty);
            h.run();
            let modifiers = |shift| egui::Modifiers { shift, ..Default::default() };
            h.input_mut().events.extend([
                egui::Event::ModifiersChanged(modifiers(release_shift)),
                egui::Event::PointerButton { pos: empty, button: egui::PointerButton::Primary, pressed: true, modifiers: modifiers(release_shift) },
                egui::Event::PointerButton { pos: empty, button: egui::PointerButton::Primary, pressed: false, modifiers: modifiers(release_shift) },
                egui::Event::ModifiersChanged(modifiers(end_shift)),
            ]);
            h.run();
            h.state().library_editors.documents[&id].selection.clone()
        };
        assert_eq!(click(true, false), click(true, true), "{kind:?}: shift-click must extend when the release carries shift");
        assert_eq!(click(false, true), click(false, false), "{kind:?}: a shift arriving after the release must not extend");
        assert_eq!(click(true, true).len(), 1, "{kind:?}: extend keeps the selected object on empty space");
        assert!(click(false, false).is_empty(), "{kind:?}: a plain click on empty space clears the selection");
    }
}

/// Native keyboard→clipboard translation drives the library paste op for every OS
/// clipboard state: `egui-winit` emits `Paste("")` on an empty or unsupported read
/// and `PasteImage` on an image-only read. Both arm the same internal preview —
/// exactly one event per physical chord, cancellable without authored changes, and
/// the commit is a single undoable edit. Events come from the real
/// `egui_winit::State` translation (vendored seams), not from injected egui events.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_paste_translation_reaches_the_internal_library_paste() {
    use winit::event::ElementState;
    use winit::keyboard::{Key as WinitKey, KeyCode, PhysicalKey};

    struct NoDisplay;
    impl winit::raw_window_handle::HasDisplayHandle for NoDisplay {
        fn display_handle(&self) -> Result<winit::raw_window_handle::DisplayHandle<'_>, winit::raw_window_handle::HandleError> {
            Err(winit::raw_window_handle::HandleError::NotSupported)
        }
    }

    /// Drive the real egui-winit translation for one Cmd/Ctrl+key chord with a
    /// seeded OS clipboard payload (None/None = empty read).
    fn native_chord(code: KeyCode, ch: &str, text: Option<String>, image: Option<egui::ColorImage>) -> Vec<egui::Event> {
        let mut state = egui_winit::State::new(egui::Context::default(), egui::ViewportId::ROOT, &NoDisplay, None, None, None);
        state.set_modifiers_for_testing(egui::Modifiers { ctrl: true, command: true, ..Default::default() });
        state.set_test_clipboard(text, image);
        state.translate_keyboard_input_for_testing(PhysicalKey::Code(code), &WinitKey::Character(ch.into()), None, ElementState::Pressed);
        state.egui_input().events.clone()
    }

    let image_only = egui::ColorImage::new([1, 1], vec![egui::Color32::RED]);
    for kind in [Kind::Symbol, Kind::Footprint] {
        for (label, text, image) in [("empty", None, None), ("image-only", None, Some(image_only.clone()))] {
            let mut app = PcbApp::default();
            app.new_library_item(kind);
            let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
            h.run();
            let id = h.state().library_editors.active.unwrap();
            let first = *h.state().library_editors.documents[&id].session.snapshot().objects.keys().next().unwrap();
            h.state_mut().library_editors.documents.get_mut(&id).unwrap().selection = [first].into_iter().collect();
            let revision = h.state().library_editors.documents[&id].session.revision();
            let count = h.state().library_editors.documents[&id].session.snapshot().objects.len();

            let area = h.get_by_label("Library canvas").rect();
            h.hover_at(area.center());
            h.run();
            // Copy through the same native boundary: chord C → Event::Copy.
            for event in native_chord(KeyCode::KeyC, "c", None, None) {
                h.event(event);
            }
            h.run();
            // Ordinary Cmd/Ctrl+V: exactly one translated event per physical chord.
            let events = native_chord(KeyCode::KeyV, "v", text.clone(), image.clone());
            assert_eq!(events.len(), 1, "{kind:?}/{label}: one chord must produce one event");
            for event in events {
                h.event(event);
            }
            h.run();
            assert!(
                matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Paste { .. }),
                "{kind:?}/{label}: native paste chord must arm the internal preview"
            );
            assert_eq!(
                h.state().library_editors.documents[&id].session.revision(),
                revision,
                "{kind:?}/{label}: the preview leaves authored state untouched"
            );
            // Cancellation keeps authored state.
            h.key_press(egui::Key::Escape);
            h.run();
            assert!(matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Select));
            assert_eq!(h.state().library_editors.documents[&id].session.revision(), revision);
            // A second chord re-arms, placement commits exactly one undoable edit.
            for event in native_chord(KeyCode::KeyV, "v", text.clone(), image.clone()) {
                h.event(event);
            }
            h.run();
            h.get_by_label("Library canvas").click();
            h.run();
            let doc = &h.state().library_editors.documents[&id];
            assert!(matches!(doc.tool, canvas::Tool::Select), "{kind:?}/{label}: placement ends the tool");
            assert_eq!(doc.session.snapshot().objects.len(), count + 1, "{kind:?}/{label}: placement adds the copied objects");
            assert_eq!(doc.session.revision(), revision + 1);
            crate::tests::click(&mut h, "Undo Library Edit");
            assert_eq!(
                h.state().library_editors.documents[&id].session.snapshot().objects.len(),
                count,
                "{kind:?}/{label}: one undo restores the pre-paste document"
            );
        }
    }
}

/// A clipboard chord on the focused host arms only its bound document: a library
/// document detached into another window's view is left untouched — the `active`
/// ownership gate holds across split/detached view arrangements.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_paste_chord_only_arms_the_focused_library_view() {
    use winit::event::ElementState;
    use winit::keyboard::{Key as WinitKey, KeyCode, PhysicalKey};

    struct NoDisplay;
    impl winit::raw_window_handle::HasDisplayHandle for NoDisplay {
        fn display_handle(&self) -> Result<winit::raw_window_handle::DisplayHandle<'_>, winit::raw_window_handle::HandleError> {
            Err(winit::raw_window_handle::HandleError::NotSupported)
        }
    }

    let mut app = PcbApp::default();
    app.new_library_item(Kind::Symbol);
    let doc_a = app.library_editors.active.unwrap();
    let main_view = app.views.active.id();
    app.new_library_item(Kind::Footprint);
    let doc_b = app.library_editors.active.unwrap();
    // Detach doc B into another window's view, then refocus the main host on A.
    let host = app.views.open_window(main_view).unwrap();
    let child = app.views.hosts[&host].focused;
    app.bind_library_view(child);
    app.bind_editor_host(app.views.main);
    app.run_window_command(&Context::default(), "app.library_focus", &json!({ "document": doc_a })).unwrap();

    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    assert_eq!(h.state().library_editors.active, Some(doc_a));

    let first = *h.state().library_editors.documents[&doc_a].session.snapshot().objects.keys().next().unwrap();
    h.state_mut().library_editors.documents.get_mut(&doc_a).unwrap().selection = [first].into_iter().collect();
    // Two "Library canvas" nodes exist once a second host is open: hover the one
    // inside the active view's pane — only that canvas receives `active: true`.
    let active_view = h.state().views.active.id();
    let pane = h.state().views.hosts[&h.state().views.host].rects.iter().find(|(view, _)| *view == active_view).map(|(_, rect)| *rect).unwrap();
    // The detached window renders as an overlay layer above the main host: hover a
    // canvas spot it does not cover — the uncovered-point gate is what the test checks.
    let spot = (0..10)
        .map(|i| egui::pos2(pane.min.x + pane.width() * (i as f32 + 0.5) / 10.0, pane.center().y))
        .find(|p| h.ctx.layer_id_at(*p).is_none_or(|layer| layer.order == egui::Order::Background))
        .expect("a canvas spot uncovered by the detached window must exist");
    h.hover_at(spot);
    h.run();

    let mut state = egui_winit::State::new(egui::Context::default(), egui::ViewportId::ROOT, &NoDisplay, None, None, None);
    state.set_modifiers_for_testing(egui::Modifiers { ctrl: true, command: true, ..Default::default() });
    state.set_test_clipboard(None, None);
    state.translate_keyboard_input_for_testing(PhysicalKey::Code(KeyCode::KeyC), &WinitKey::Character("c".into()), None, ElementState::Pressed);
    for event in state.egui_input().events.clone() {
        h.event(event);
    }
    state.translate_keyboard_input_for_testing(PhysicalKey::Code(KeyCode::KeyV), &WinitKey::Character("v".into()), None, ElementState::Pressed);
    for event in state.egui_input().events.clone() {
        h.event(event);
    }
    h.run();

    assert!(
        matches!(h.state().library_editors.documents[&doc_a].tool, canvas::Tool::Paste { .. }),
        "the focused document must arm its paste preview"
    );
    assert!(
        matches!(h.state().library_editors.documents[&doc_b].tool, canvas::Tool::Select),
        "the document bound to the detached view must not consume the chord"
    );
}

/// The shared GUI/MCP `app.library_edit` command supports dry-run change records, applies a
/// ball-grid array in one undo step, and orders pads naturally in the definition browser.
#[test]
fn ball_grid_command_supports_dry_run_single_undo_and_natural_browser_sort() {
    let mut app = PcbApp::default();
    let ctx = Context::default();
    app.run_window_command(&ctx, "app.library_new", &json!({"kind":"footprint"})).unwrap();
    let id = app.library_editors.active.unwrap();
    app.run_window_command(&ctx, "app.library_edit", &json!({"document":id,"expected_revision":0,"action":{"operation":"delete","objects":[2,3]}}))
        .unwrap();

    let action = json!({
        "operation": "array",
        "objects": [1],
        "columns": 12,
        "rows": 22,
        "column_step": {"x_nm": 800_000, "y_nm": 0},
        "row_step": {"x_nm": 0, "y_nm": 800_000},
        "numbering": {"mode": "ball_grid", "skip_jedec": true}
    });

    // Dry run validates and records a readable change summary without mutating state or revision.
    app.run_window_command(&ctx, "app.library_edit", &json!({"document":id,"expected_revision":1,"action":action,"dry_run":true})).unwrap();
    let inspected = crate::window_commands::inspect_editor(&app);
    let doc_state = &inspected["library_editors"]["documents"][0];
    assert_eq!(doc_state["state"]["revision"], 1);
    assert_eq!(doc_state["state"]["item"]["objects"].as_object().unwrap().len(), 1);
    assert_eq!(doc_state["last_change"]["dry_run"], true);
    assert_eq!(doc_state["last_change"]["changed"], true);
    assert_eq!(doc_state["last_change"]["renamed_pads"], json!(["1 -> A1"]));
    assert_eq!(doc_state["last_change"]["added_pads"].as_array().unwrap().len(), 263);

    // Applying executes in one undo step and keeps the browser in natural pad order.
    app.run_window_command(&ctx, "app.library_edit", &json!({"document":id,"expected_revision":1,"action":action})).unwrap();
    let inspected = crate::window_commands::inspect_editor(&app);
    let doc_state = &inspected["library_editors"]["documents"][0];
    assert_eq!(doc_state["state"]["revision"], 2);
    assert_eq!(doc_state["last_change"]["dry_run"], false);
    assert_eq!(doc_state["state"]["item"]["objects"].as_object().unwrap().len(), 264);

    let item = app.library_editors.documents[&id].session.snapshot();
    let browser_pads: Vec<&str> = super::browser_objects(&item)
        .into_iter()
        .filter_map(|(_, primitive)| match primitive {
            pcbcraft_engine::authoring::Primitive::Pad { pad } => Some(pad.number.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(browser_pads.len(), 264);
    let pos = |label: &str| browser_pads.iter().position(|pad| *pad == label).unwrap();
    assert!(pos("A2") < pos("A10"), "browser must sort A2 before A10");
    assert!(pos("Y12") < pos("AA1"), "browser must sort Y12 before AA1");
    assert_eq!(browser_pads.first().copied(), Some("A1"));
    assert_eq!(browser_pads.last().copied(), Some("AB12"));

    // One undo step restores the single seed pad.
    app.run_window_command(&ctx, "app.library_edit", &json!({"document":id,"expected_revision":2,"action":{"operation":"undo"}})).unwrap();
    assert_eq!(app.library_editors.documents[&id].session.snapshot().objects.len(), 1);
}

/// A batch from an unfocused viewport must not run any canvas keyboard command: the
/// destructive M/E/Delete/Enter keys stay inert, and the same keys still work focused.
#[test]
fn unfocused_batches_ignore_library_canvas_keyboard_commands() {
    for kind in [Kind::Symbol, Kind::Footprint] {
        for (key, focused, mutates) in
            [(egui::Key::Delete, false, false), (egui::Key::M, false, false), (egui::Key::E, false, false), (egui::Key::Delete, true, true)]
        {
            let mut app = PcbApp::default();
            app.new_library_item(kind);
            let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
            h.run();
            let id = h.state().library_editors.active.unwrap();
            let doc = h.state_mut().library_editors.documents.get_mut(&id).unwrap();
            let before = doc.session.snapshot();
            doc.selection = before.objects.keys().copied().collect();
            let area = h.get_by_label("Library canvas").rect();
            h.hover_at(area.center());
            h.run();
            h.input_mut().focused = focused;
            for pressed in [true, false] {
                h.input_mut().events.push(egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            h.run();
            let doc = &h.state().library_editors.documents[&id];
            assert_eq!(doc.session.snapshot().as_ref() == before.as_ref(), !mutates, "{kind:?} {key:?} focus={focused}");
            assert!(!doc.details_requested, "{kind:?} {key:?} focus={focused}");
            assert!(mutates || matches!(doc.tool, canvas::Tool::Select), "{kind:?} {key:?}: an unfocused batch must not arm a tool");
            if mutates {
                assert!(doc.session.snapshot().objects.is_empty(), "{kind:?}: focused Delete clears the selection");
            }
        }
    }
}

/// Tool-start keys resolve against a same-batch Escape in event order: a press before
/// Escape stays cancelled, a press after it still arms — the same ordered resolution
/// the paste/duplicate preview chords use.
#[test]
fn library_tool_keys_resolve_against_same_batch_escape() {
    for kind in [Kind::Symbol, Kind::Footprint] {
        for (batch, armed) in [
            (vec![egui::Key::P, egui::Key::Escape], false),
            (vec![egui::Key::Escape, egui::Key::P], true),
            (vec![egui::Key::P, egui::Key::Escape, egui::Key::P], true),
            (vec![egui::Key::L, egui::Key::Escape], false),
            (vec![egui::Key::M, egui::Key::Escape], false),
        ] {
            let mut app = PcbApp::default();
            app.new_library_item(kind);
            let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
            h.run();
            let id = h.state().library_editors.active.unwrap();
            h.state_mut().library_editors.documents.get_mut(&id).unwrap().selection =
                h.state().library_editors.documents[&id].session.snapshot().objects.keys().copied().collect();
            let area = h.get_by_label("Library canvas").rect();
            h.hover_at(area.center());
            h.run();
            for key in &batch {
                h.input_mut().events.push(egui::Event::Key {
                    key: *key,
                    physical_key: Some(*key),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            h.run();
            let tool = &h.state().library_editors.documents[&id].tool;
            assert_eq!(matches!(tool, canvas::Tool::Select), !armed, "{kind:?} {batch:?}: tool={tool:?}");
            assert_eq!(armed, matches!(tool, canvas::Tool::Draw { .. } | canvas::Tool::Move { .. } | canvas::Tool::MoveGroup { .. }));
        }
        // The footprint courtyard letter resolves the same way.
        if kind == Kind::Footprint {
            for (batch, armed) in [(vec![egui::Key::C, egui::Key::Escape], false), (vec![egui::Key::Escape, egui::Key::C], true)] {
                let mut app = PcbApp::default();
                app.new_library_item(kind);
                let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
                h.run();
                let id = h.state().library_editors.active.unwrap();
                let area = h.get_by_label("Library canvas").rect();
                h.hover_at(area.center());
                h.run();
                for key in &batch {
                    h.input_mut().events.push(egui::Event::Key {
                        key: *key,
                        physical_key: Some(*key),
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                h.run();
                let tool = &h.state().library_editors.documents[&id].tool;
                assert_eq!(matches!(tool, canvas::Tool::Select), !armed, "{batch:?}: tool={tool:?}");
                assert_eq!(armed, matches!(tool, canvas::Tool::Draw { kind: canvas::Place::Courtyard, .. }));
            }
        }
    }
}

/// Deletion follows the tool at the press, even when Escape or a tool start shares its batch.
#[test]
fn library_delete_keys_act_only_in_select_at_the_press() {
    use egui::Key::{Backspace, Delete, Escape, P};
    for kind in [Kind::Symbol, Kind::Footprint] {
        for key in [Delete, Backspace] {
            for (armed_before, batch, deleted, armed_after) in [
                (false, vec![P, key, Escape], false, false),
                (true, vec![key, Escape], false, false),
                (true, vec![Escape, key], true, false),
                (false, vec![key, P], true, true),
            ] {
                let mut app = PcbApp::default();
                app.new_library_item(kind);
                let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
                h.run();
                let id = h.state().library_editors.active.unwrap();
                let before = h.state().library_editors.documents[&id].session.snapshot();
                assert!(!before.objects.is_empty());
                let selection = before.objects.keys().copied().collect();
                h.state_mut().library_editors.documents.get_mut(&id).unwrap().selection = selection;
                let area = h.get_by_label("Library canvas").rect();
                h.hover_at(area.center());
                h.run();
                if armed_before {
                    h.key_press(P);
                    h.run();
                    assert!(matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Draw { .. }));
                }
                let revision = h.state().library_editors.documents[&id].session.revision();
                for key in &batch {
                    h.input_mut().events.push(egui::Event::Key {
                        key: *key,
                        physical_key: Some(*key),
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                h.run();
                let document = &h.state().library_editors.documents[&id];
                assert_eq!(matches!(document.tool, canvas::Tool::Draw { .. }), armed_after, "{kind:?} {batch:?}");
                let after = document.session.snapshot();
                if deleted {
                    assert!(after.objects.is_empty(), "{kind:?} {batch:?}");
                    assert_eq!(document.session.revision(), revision + 1);
                } else {
                    assert_eq!(after.as_ref(), before.as_ref(), "{kind:?} {batch:?}");
                    assert_eq!(document.session.revision(), revision);
                    assert_eq!(document.selection, before.objects.keys().copied().collect());
                }
            }
        }
    }
}

/// Rotation follows the tool at each press, including a preview armed in an earlier batch.
#[test]
fn library_rotation_uses_the_tool_active_at_the_press() {
    use egui::Key::{Escape, M, P, R};
    for kind in [Kind::Symbol, Kind::Footprint] {
        for (start, batch, edits) in [
            (None, vec![P, R, Escape], 0),
            (Some(P), vec![R, Escape], 0),
            (Some(M), vec![R, Escape], 0),
            (Some(P), vec![Escape, R], 1),
            (Some(M), vec![Escape, R, R], 2),
        ] {
            let mut app = PcbApp::default();
            app.new_library_item(kind);
            let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
            h.run();
            let id = h.state().library_editors.active.unwrap();
            let doc = h.state_mut().library_editors.documents.get_mut(&id).unwrap();
            let object = *doc.session.snapshot().objects.keys().next().unwrap();
            doc.select(Some(object)).unwrap();
            let before = doc.session.snapshot();
            let revision = doc.session.revision();
            let area = h.get_by_label("Library canvas").rect();
            h.hover_at(area.center());
            h.run();
            if let Some(key) = start {
                h.key_press(key);
                h.run();
                assert!(!matches!(h.state().library_editors.documents[&id].tool, canvas::Tool::Select));
            }
            for key in &batch {
                h.input_mut().events.extend([true, false].map(|pressed| egui::Event::Key {
                    key: *key,
                    physical_key: Some(*key),
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }));
            }
            h.run();
            let doc = &h.state().library_editors.documents[&id];
            assert!(matches!(doc.tool, canvas::Tool::Select), "{kind:?} {start:?} {batch:?}");
            assert_eq!(doc.session.revision(), revision + edits, "{kind:?} {start:?} {batch:?}");
            assert_eq!(doc.session.snapshot().as_ref() == before.as_ref(), edits == 0, "{kind:?} {start:?} {batch:?}");
        }
    }
}

/// A rotation that commits the selection exits outline drawing before the next delete key.
#[test]
fn library_delete_keys_see_the_select_tool_after_rotation() {
    use egui::Key::{Backspace, Delete, R};
    use pcbcraft_engine::schematic::Point;
    for kind in [Kind::Symbol, Kind::Footprint] {
        for key in [Delete, Backspace] {
            for (batch, deleted) in [([R, key], true), ([key, R], false)] {
                let mut app = PcbApp::default();
                app.new_library_item(kind);
                let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
                h.run();
                let id = h.state().library_editors.active.unwrap();
                let doc = h.state_mut().library_editors.documents.get_mut(&id).unwrap();
                let object = *doc.session.snapshot().objects.keys().next().unwrap();
                doc.select(Some(object)).unwrap();
                let revision = doc.session.revision();
                doc.tool = canvas::Tool::Polyline { points: vec![Point { x_nm: 0, y_nm: 0 }], revision };
                let area = h.get_by_label("Library canvas").rect();
                h.hover_at(area.center());
                h.run();
                for key in batch {
                    h.input_mut().events.push(egui::Event::Key {
                        key,
                        physical_key: Some(key),
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                h.run();
                let doc = &h.state().library_editors.documents[&id];
                assert!(matches!(doc.tool, canvas::Tool::Select), "{kind:?} {batch:?}");
                assert_eq!(doc.session.snapshot().objects.contains_key(&object), !deleted, "{kind:?} {batch:?}");
                assert_eq!(doc.session.revision(), revision + if deleted { 2 } else { 1 }, "{kind:?} {batch:?}");
            }
        }
    }
}

/// The definition name stays in the app bar and the Library menu exposes applicable actions.
#[test]
fn definition_menus_separate_file_operations_from_library_actions() {
    for kind in [Kind::Symbol, Kind::Footprint] {
        let mut app = PcbApp::default();
        app.preferences.theme = if kind == Kind::Symbol { crate::ThemeChoice::Dark } else { crate::ThemeChoice::Light };
        app.new_library_item(kind);
        let id = app.library_editors.active.unwrap();
        let title = app.library_editors.documents[&id].session.snapshot().name.clone();
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| app.frame(ui), app);
        h.run();
        let file = h.get_by_label("File").rect();
        assert!(
            h.query_all_by_label(title.as_str()).any(|node| file.y_range().contains(node.rect().center().y)),
            "the definition name belongs in the File menu's app bar row"
        );
        crate::tests::click(&mut h, "File");
        h.get_by_label(if IN_BROWSER { "Download Library Item…" } else { "Save Library Copy…" });
        assert!(h.query_by_label("Remap pins…").is_none());
        h.key_press(egui::Key::Escape);
        h.run();

        h.get_by_role_and_label(egui::Role::Button, "Library").click();
        h.run();
        h.get_by_label("Publish…");
        h.get_by_label("Edit copy");
        assert!(h.query_by_label("Save Library Copy…").is_none());
        for label in ["Preview pin table…", "Remap pins…"] {
            assert_eq!(h.query_by_label(label).is_some(), kind == Kind::Symbol, "{label}");
        }
    }
}
