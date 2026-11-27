//! Flat-sheet acceptance through actual Browser controls and explicitly addressed view input.
use super::*;
use crate::editor_view::DocumentTarget;
use pcbcraft_engine::{
    PLACE_RESISTOR,
    persistence::encode_documents,
    sheets::{CREATE_SHEET, DELETE_SHEET},
};

/// The confirmation limits footprint removal to parts that lose their last placed symbol.
/// Accepted footprints for those parts stay for review in Update PCB.
const DELETE_SHEET_NOTE: &str = "This removes the sheet’s symbols, wires and connections to parent pins. For parts that lose their last placed symbol, footprints are removed now or, if already accepted, when you apply Update PCB. Child sheets become top-level sheets. You can undo the deletion.";

/// Start with one original resistor and the common Browser visible.
fn sheet_harness(width: f32) -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.run("project.new", &json!({"name":"Multi sheet"}));
    app.run(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":app.session.revision()}));
    let mut h = Harness::builder().with_size(egui::vec2(width, 900.0)).with_max_steps(40).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    h
}

/// Open a real Browser row's contextual actions with separate press/release input frames.
fn menu(h: &mut Harness<'_, PcbApp>, label: &str) {
    let pos = h.get_by_role_and_label(egui::Role::Button, label).rect().center();
    h.hover_at(pos);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
}

/// Replace the form's initial name and submit through its actual OK button.
fn name(h: &mut Harness<'_, PcbApp>, text: &str) {
    clear_name(h);
    h.get_by_label("Name").type_text(text);
    h.run();
    click(h, "OK");
}

/// Creating, naming, switching, deleting and undoing are complete GUI workflows at both sizes.
#[test]
fn browser_sheet_lifecycle_and_local_canvas_selection() {
    for width in [1400.0, 700.0] {
        let mut h = sheet_harness(width);
        let main = h.state().session.project().unwrap().sheet_target();
        menu(&mut h, "Sheets");
        click(&mut h, "New Sheet…");
        name(&mut h, "Power");
        assert!(matches!(h.state().dialog, Dialog::None));
        let power = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
        assert_eq!(h.state().views.active.target(), Some(DocumentTarget::Sheet(power)));
        assert!(h.output().shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.job.text=="2 / 2")));
        let revision = h.state().session.revision();
        h.state_mut().run(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":revision}));
        h.run();
        click(&mut h, "Schematic canvas");
        let selected = h.state().views.active.schematic.selection.single_symbol().unwrap().to_owned();
        assert_eq!(h.state().selected_reference().as_deref(), Some("R2"));
        click(&mut h, "Main");
        assert_eq!(h.state().views.active.target(), Some(DocumentTarget::Sheet(main)));
        assert!(h.output().shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.job.text=="1 / 2")));
        click(&mut h, "Schematic canvas");
        assert_eq!(h.state().selected_reference().as_deref(), Some("R1"));
        click(&mut h, "Power");
        assert_eq!(h.state().views.active.schematic.selection.single_symbol(), Some(selected.as_str()));
        menu(&mut h, "Power");
        click(&mut h, "Rename Sheet…");
        name(&mut h, "Supply");
        h.get_by_label("Supply");
        assert_eq!(h.state().views.active.target(), Some(DocumentTarget::Sheet(power)));
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        menu(&mut h, "Supply");
        click(&mut h, "Delete Sheet…");
        click(&mut h, "Cancel");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        menu(&mut h, "Supply");
        click(&mut h, "Delete Sheet…");
        h.get_by_label(DELETE_SHEET_NOTE);
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Delete Sheet").click();
        h.run();
        assert_eq!(h.state().session.project().unwrap().sheets().len(), 1);
        assert_eq!(h.state().views.active.target(), Some(DocumentTarget::Sheet(main)));
        click(&mut h, "Undo");
        h.get_by_label("Supply");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    }
}

/// Two panes read separate topology/geometry and GUI commands take the focused pane's address.
/// Editing another sheet never retargets, pans or selects the first pane.
#[test]
fn different_sheets_in_split_panes_keep_edits_and_net_inspection_local() {
    let mut h = sheet_harness(1400.0);
    let revision = h.state().session.revision();
    h.state_mut().run(CREATE_SHEET, &json!({"name":"Power","expected_revision":revision}));
    h.run();
    let main = h.state().session.project().unwrap().sheet_target();
    let power = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
    let ctx = h.ctx.clone();
    h.state_mut().split_editors(&ctx, true);
    h.run();
    let first = h.state().views.active.id();
    click(&mut h, "Schematic canvas 2");
    click(&mut h, "Power");
    let second = h.state().views.active.id();
    assert_ne!(first, second);
    let before = h.state().views.view(first).unwrap().camera.saved_state();
    let revision = h.state().session.revision();
    h.state_mut().run(PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":revision}));
    h.run();
    click(&mut h, &format!("Schematic canvas {}", second.0 + 1));
    assert_eq!(h.state().selected_reference().as_deref(), Some("R2"));
    assert_eq!(h.state().views.view(first).unwrap().target(), Some(DocumentTarget::Sheet(main)));
    assert_eq!(h.state().views.active.target(), Some(DocumentTarget::Sheet(power)));
    assert_eq!(h.state().views.view(first).unwrap().camera.saved_state(), before);
    assert!(h.state().views.view(first).unwrap().schematic.selection.is_empty());
    click(&mut h, "Toggle Net Inspector");
    h.get_by_label("R2.1");
    h.get_by_label("R2.2");
    assert!(h.query_by_label("R1.1").is_none());
    click(&mut h, &format!("Schematic canvas {}", first.0 + 1));
    h.get_by_label("R1.1");
    h.get_by_label("R1.2");
    assert!(h.query_by_label("R2.1").is_none());
}

/// A stale lifecycle form cannot edit a new revision, and a view whose sheet disappeared
/// cannot edit a surviving sheet. Undo revives the original address and camera.
#[test]
fn stale_sheet_forms_and_deleted_sheet_views_never_fall_back_to_editing_main() {
    let mut h = sheet_harness(1400.0);
    let revision = h.state().session.revision();
    h.state_mut().run(CREATE_SHEET, &json!({"name":"Power","expected_revision":revision}));
    h.run();
    let power = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
    click(&mut h, "Power");
    let id = h.state().views.active.id();
    let camera = h.state().views.active.camera.saved_state();
    menu(&mut h, "Power");
    click(&mut h, "Rename Sheet…");
    h.state_mut().session.apply("project.rename", &json!({"name":"Changed"})).unwrap();
    h.run();
    let before = h.state().session.snapshot().unwrap();
    name(&mut h, "Supply");
    assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    let old = h.state().views.active.input(&h.state().session).unwrap();
    let revision = h.state().session.revision();
    h.state_mut().session.apply(DELETE_SHEET, &json!({"sheet":power.id(),"expected_revision":revision})).unwrap();
    h.run();
    assert!(h.state().views.view(id).unwrap().unavailable());
    assert!(!h.state().views.view(id).unwrap().accepts(old, &h.state().session));
    assert!(h.state().views.view(id).unwrap().snapshot(&h.state().session).is_none());
    h.state_mut().session.apply("edit.undo", &Value::Null).unwrap();
    h.run();
    assert_eq!(h.state().views.view(id).unwrap().target(), Some(DocumentTarget::Sheet(power)));
    assert_eq!(h.state().views.view(id).unwrap().camera.saved_state(), camera);
}

/// Deleting the active sheet externally cannot disable project-wide Save/Save As. A real
/// folder save still preserves the unavailable view, and its pending job blocks a second save.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn unavailable_sheet_view_can_save_the_project_and_respects_pending_file_jobs() {
    let mut h = sheet_harness(1400.0);
    h.state_mut().files.use_stand_in();
    let revision = h.state().session.revision();
    h.state_mut().run(CREATE_SHEET, &json!({"name":"Power","expected_revision":revision}));
    h.run();
    let power = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
    click(&mut h, "Power");
    let revision = h.state().session.revision();
    h.state_mut().session.apply(DELETE_SHEET, &json!({"sheet":power.id(),"expected_revision":revision})).unwrap();
    h.run();
    assert!(h.state().views.active.unavailable());
    assert!(h.state().views.active.snapshot(&h.state().session).is_none());
    assert!(h.state().can_save_project());
    assert!(!h.get_by_label("Save Project").accesskit_node().is_disabled());
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click(&mut h, "File");
    assert!(!h.get_by_label("Save").accesskit_node().is_disabled());
    assert!(!h.get_by_label("Save As…").accesskit_node().is_disabled());
    click(&mut h, "Save As…");
    assert!(matches!(h.state().dialog, Dialog::SaveEncoding));
    click(&mut h, "Cancel");
    click(&mut h, "File");
    h.get_by_label("Save").click();
    // The stand-in holds the real file job open until the simulated destination is chosen.
    h.run_steps(3);
    assert!(h.state().files.stand_in_waiting());
    assert!(!h.state().can_save_project());
    assert!(h.get_by_label("Save Project").accesskit_node().is_disabled());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved-project");
    h.state_mut().files.finish_stand_in(path.clone());
    h.run();
    assert!(h.state().can_save_project());
    assert!(h.state().views.active.unavailable());
    assert!(!h.state().files.dirty(&h.state().session));
    assert_eq!(encode_documents(pcbcraft_storage::open(&path).unwrap()).unwrap(), before);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Live navigation validates the explicit sheet before selecting, independently of focused pane.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_sheet_navigation_and_selection_validate_the_document_address() {
    let mut h = sheet_harness(1400.0);
    let ctx = h.ctx.clone();
    let revision = h.state().session.revision();
    h.state_mut().run(CREATE_SHEET, &json!({"name":"Power","expected_revision":revision}));
    h.run();
    let power = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
    let before = h.state().session.snapshot().unwrap();
    let source = h.state().views.active.id();
    h.state_mut().run_window_command(&ctx, "view.open_sheet", &json!({"view":source.0,"sheet":power.id()})).unwrap();
    h.run();
    let target = h.state().views.active.id();
    assert_eq!(h.state().views.active.target(), Some(DocumentTarget::Sheet(power)));
    assert!(h.state_mut().run_window_command(&ctx, "view.select", &json!({"view":target.0,"items":{"symbols":["symbol-2"],"wires":[]}})).is_err());
    assert!(
        h.state_mut().run_window_command(&ctx, "view.open_sheet", &json!({"view":target.0,"sheet":"00000000-0000-0000-0000-000000000000"})).is_err()
    );
    assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Saved view targets survive an offline project reopen, including the sheet associated with
/// a PCB view's parked schematic camera. Restoring presentation never changes project bytes.
#[test]
fn reopened_arrangement_keeps_sheet_targets_and_board_return_navigation() {
    let mut h = sheet_harness(1400.0);
    let revision = h.state().session.revision();
    h.state_mut().run("pcb.create_board", &json!({"expected_revision":revision}));
    let revision = h.state().session.revision();
    h.state_mut().run(CREATE_SHEET, &json!({"name":"Power","expected_revision":revision}));
    h.run();
    let power = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
    click(&mut h, "Power");
    h.state_mut().views.active.camera.center_on(pcbcraft_engine::schematic::Point { x_nm: 10_000_000, y_nm: 20_000_000 });
    crate::tests::open_browser_row(&mut h, "Board Board", "Open in PCB layout");
    let state = h.state().views.saved_state(&h.state().session).unwrap();
    let documents = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let mut app = PcbApp::default();
    app.session.open_snapshot(pcbcraft_engine::persistence::decode_documents(&documents).unwrap());
    app.views.restore_state(&state, &app.session).unwrap();
    assert_eq!(app.views.active.workspace, crate::editor_view::Workspace::Board);
    let source = app.views.active.id();
    app.apply_workspace(source, crate::editor_view::Workspace::Schematic, pcbcraft_chrome::workspace::Placement::Here);
    assert_eq!(app.views.active.target(), Some(DocumentTarget::Sheet(power)));
    assert_eq!(encode_documents(app.session.snapshot().unwrap()).unwrap(), documents);
}

/// A sheet's Paper submenu shows its current size and orientation, applies a choice as one undo
/// step through `sheet.set_page`, and keeps everything on the sheet in place.
#[test]
fn browser_paper_menu_sets_the_sheet_paper() {
    use pcbcraft_engine::sheets::page::{Orientation, Paper};
    let mut h = sheet_harness(1400.0);
    let before = h.state().session.snapshot().unwrap();
    let page = |h: &Harness<'_, PcbApp>| {
        let page = h.state().session.project().unwrap().page().unwrap();
        (page.paper(), page.orientation())
    };
    assert_eq!(page(&h), (Paper::A4, Orientation::Landscape));
    menu(&mut h, "Main");
    click(&mut h, "Paper");
    assert_eq!(h.get_by_role_and_label(egui::Role::RadioButton, "A4").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    click(&mut h, "A3");
    assert_eq!(page(&h), (Paper::A3, Orientation::Landscape));
    menu(&mut h, "Main");
    click(&mut h, "Paper");
    click(&mut h, "Portrait");
    assert_eq!(page(&h), (Paper::A3, Orientation::Portrait));
    let after = h.state().session.project().unwrap();
    assert_eq!(after.page().unwrap().at(), before.page().unwrap().at(), "the paper's corner stays put");
    assert_eq!(after.symbols(), before.symbols());
    h.state_mut().run("edit.undo", &Value::Null);
    h.state_mut().run("edit.undo", &Value::Null);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Reducing paper around existing drawing leaves that drawing editable outside the paper: the
/// canvas keeps the theme's background and heading, the grid continues there unless the person
/// turns it off, and Fit includes it.
#[test]
fn off_paper_drawing_keeps_the_theme_background_and_grid() {
    for dark in [true, false] {
        let mut h = sheet_harness(1400.);
        h.state_mut().preferences.theme = if dark { crate::preferences::ThemeChoice::Dark } else { crate::preferences::ThemeChoice::Light };
        let apply = |h: &mut Harness<'_, PcbApp>, command: &str, mut params: Value| {
            params["expected_revision"] = json!(h.state().session.revision());
            h.state_mut().session.apply(command, &params).unwrap();
        };
        apply(&mut h, PLACE_RESISTOR, json!({"x_nm":250_000_000,"y_nm":0}));
        let sheet = h.state().session.project().unwrap().sheet_target().id();
        apply(&mut h, "sheet.set_page", json!({"sheet":sheet,"paper":"a3","orientation":"landscape","at":{"x_nm":-100_000_000,"y_nm":-100_000_000}}));
        h.run();
        click(&mut h, "Fit schematic");
        // The painted surround, and whether any grid dot lies outside the paper.
        let painted = |h: &Harness<'_, PcbApp>| {
            let t = pcbcraft_chrome::Tokens::get();
            let canvas = h.get_by_label("Schematic canvas").rect();
            let [min, max] = h.state().session.project().unwrap().page().unwrap().corners();
            let camera = &h.state().views.active.camera;
            let paper = egui::Rect::from_two_pos(camera.screen(min, canvas), camera.screen(max, canvas));
            let shapes = &h.output().shapes;
            let surround = shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.rect == canvas && r.fill == t.canvas));
            let outside = shapes
                .iter()
                .any(|s| matches!(&s.shape, egui::Shape::Circle(c) if c.radius == pcbcraft_chrome::theme::SCHEMATIC_GRID_DOT_RADIUS && canvas.contains(c.center) && !paper.expand(1.0).contains(c.center)));
            (surround, outside)
        };
        let assert_heading = |h: &Harness<'_, PcbApp>| {
            let t = pcbcraft_chrome::Tokens::get();
            let title = h.state().views.active.sheet_path(&h.state().session).unwrap();
            for (label, ink) in [(title.as_str(), t.text), ("1 / 1", t.text_dim)] {
                assert!(
                    h.output()
                        .shapes
                        .iter()
                        .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label && text.fallback_color == ink)),
                    "heading {label} keeps the theme's ink"
                );
            }
        };
        assert_eq!(painted(&h), (true, true));
        assert_heading(&h);
        let symbols = h.state().session.project().unwrap().symbols().to_vec();
        apply(&mut h, "sheet.set_page", json!({"sheet":sheet,"paper":"a4","orientation":"landscape"}));
        h.run();
        assert_eq!(painted(&h), (true, true), "drawing outside the paper doesn't change the background");
        assert_heading(&h);
        assert_eq!(h.state().session.project().unwrap().symbols(), symbols);
        let revision = h.state().session.revision();
        click(&mut h, "Fit schematic");
        assert_eq!(painted(&h), (true, true), "the grid continues around off-paper drawing");
        h.state_mut().preferences.schematic_grid_outside_paper = false;
        h.run();
        assert_eq!(painted(&h), (true, false), "the grid stays on the paper when the preference is off");
        let project = h.state().session.project().unwrap();
        let canvas = h.get_by_label("Schematic canvas").rect();
        let camera = &h.state().views.active.camera;
        for [low, high] in [project.page().unwrap().corners(), project.drawing_bounds().unwrap()] {
            assert!(canvas.contains_rect(egui::Rect::from_two_pos(camera.screen(low, canvas), camera.screen(high, canvas))));
        }
        assert_eq!(h.state().session.revision(), revision, "the grid preference is not a project edit");
    }
}

/// A catalog drag paints background-filled artwork against its current surface before drop,
/// both inside and outside paper, without editing the project.
#[test]
fn catalog_drag_background_follows_paper_before_drop() {
    use pcbcraft_engine::{
        authoring::{Item, Kind, Primitive},
        library_graphics::{Fill, Geometry, Graphic},
        schematic::{GRID_NM, Point},
    };
    let mut item = Item::new(Kind::Symbol).unwrap();
    item.objects.insert(
        2,
        Primitive::Graphic {
            graphic: Graphic {
                geometry: Geometry::Rectangle {
                    min: Point { x_nm: -2 * GRID_NM, y_nm: -2 * GRID_NM },
                    max: Point { x_nm: 2 * GRID_NM, y_nm: 2 * GRID_NM },
                },
                stroke_nm: 150_000,
                fill: Fill::Background,
            },
        },
    );
    let definition = item.symbol().unwrap();
    for dark in [true, false] {
        let mut h = sheet_harness(1400.);
        h.state_mut().preferences.theme = if dark { crate::preferences::ThemeChoice::Dark } else { crate::preferences::ThemeChoice::Light };
        h.run();
        click(&mut h, "Fit schematic");
        let before = h.state().session.snapshot().unwrap();
        h.state_mut().preferences.schematic_paper_background = crate::preferences::PaperBackground::Shaded;
        h.run();
        let source = h.state().views.active.input(&h.state().session).unwrap();
        egui::DragAndDrop::set_payload(&h.ctx, crate::PartDrag { placement: crate::parts_browser::Placement::new(definition.clone()), source });
        let t = pcbcraft_chrome::Tokens::get();
        let [_, max] = before.page().unwrap().corners();
        for (point, expected) in [
            (Point { x_nm: 40 * GRID_NM, y_nm: 10 * GRID_NM }, t.schematic_paper),
            (Point { x_nm: max.x_nm + 8 * GRID_NM, y_nm: 10 * GRID_NM }, t.canvas),
        ] {
            let canvas = h.get_by_label("Schematic canvas").rect();
            let screen = h.state().views.active.camera.screen(point, canvas);
            assert!(canvas.contains(screen));
            h.hover_at(screen);
            h.run();
            assert!(
                h.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Path(path)
                    if path.closed && path.fill == expected && egui::Rect::from_points(&path.points).contains(screen))),
                "catalog preview should match its surface before drop: dark={dark}, point={point:?}"
            );
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        }
    }
}

/// Paper appearance is live personal state: the colour controls follow the selected mode,
/// while Cancel restores the initial appearance without editing the schematic.
#[test]
fn paper_preferences_are_optional_and_cancelable() {
    use crate::preferences::PaperBackground;
    for dark in [true, false] {
        let mut h = sheet_harness(1400.);
        h.state_mut().preferences.theme = if dark { crate::preferences::ThemeChoice::Dark } else { crate::preferences::ThemeChoice::Light };
        h.run();
        let before = h.state().session.snapshot().unwrap();
        assert_eq!(h.state().preferences.schematic_paper_background, PaperBackground::Uniform);
        click(&mut h, "Preferences");
        assert!(h.get_by_role_and_label(egui::Role::ColorWell, "Sheet color").accesskit_node().is_disabled());
        assert!(!h.get_by_role_and_label(egui::Role::ColorWell, "Canvas color").accesskit_node().is_disabled());
        click(&mut h, "Shaded");
        assert_eq!(h.state().preferences.schematic_paper_background, PaperBackground::Shaded);
        click(&mut h, "Custom");
        assert!(!h.get_by_role_and_label(egui::Role::ColorWell, "Sheet color").accesskit_node().is_disabled());
        h.get_by_role_and_label(egui::Role::ColorWell, "Canvas color").click();
        h.run();
        assert!(egui::Popup::is_any_open(&h.ctx), "the canvas colour picker opens");
        h.key_press(Key::Escape);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::Preferences(_)), "Escape closes the colour picker before Preferences");
        click(&mut h, "Cancel");
        assert_eq!(h.state().preferences.schematic_paper_background, PaperBackground::Uniform);
        assert!(h.state().preferences.schematic_canvas_color_dark.is_none() && h.state().preferences.schematic_canvas_color_light.is_none());
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
}

/// A sheet's Frame submenu shows its current template, switches between a plain border and the
/// Default frame through `sheet.set_template`, one undo step each, without moving the paper.
#[test]
fn browser_frame_menu_sets_the_sheet_template() {
    let mut h = sheet_harness(1400.0);
    let before = h.state().session.snapshot().unwrap();
    let title = |h: &Harness<'_, PcbApp>| h.state().session.project().unwrap().page().unwrap().template().map(|t| t.title().to_owned());
    assert_eq!(title(&h).as_deref(), Some("Default"));
    menu(&mut h, "Main");
    click(&mut h, "Frame");
    assert_eq!(h.get_by_role_and_label(egui::Role::RadioButton, "Default").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    click(&mut h, "None");
    assert_eq!(title(&h), None);
    assert_eq!(h.state().session.project().unwrap().page().unwrap().at(), before.page().unwrap().at());
    menu(&mut h, "Main");
    click(&mut h, "Frame");
    click(&mut h, "Default");
    assert_eq!(title(&h).as_deref(), Some("Default"));
    h.state_mut().run("edit.undo", &Value::Null);
    h.state_mut().run("edit.undo", &Value::Null);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}
