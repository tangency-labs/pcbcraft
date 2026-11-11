//! Accepted connections remain a presentation overlay over the actual physical board.
use super::*;
use pcbcraft_engine::{
    CONNECT_PINS, PLACE_FOOTPRINT, PLACE_SYMBOL, SET_FOOTPRINT_TRANSFORM, board::update::INITIALIZE_BASELINE, persistence::encode_documents,
};

/// Author and accept one connection through production commands before opening its PCB view.
fn board<'a>(accept: bool) -> Harness<'a, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    h.state_mut().run("project.new", &json!({"create_board":true}));
    let mut symbols = Vec::new();
    for index in 0..2 {
        let revision = h.state().session.revision();
        h.state_mut().run(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":index*10_000_000,"y_nm":0,"expected_revision":revision}));
        let symbol = h.state().session.project().unwrap().symbols().last().unwrap().id();
        symbols.push(symbol.clone());
        let revision = h.state().session.revision();
        h.state_mut().run(PLACE_FOOTPRINT, &json!({"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"2","2":"1"},"x_nm":index*10_000_000,"y_nm":0,"expected_revision":revision}));
    }
    let revision = h.state().session.revision();
    h.state_mut().run(
        CONNECT_PINS,
        &json!({"from":{"symbol":symbols[0],"terminal":"1"},"to":{"symbol":symbols[1],"terminal":"1"},"expected_revision":revision}),
    );
    if accept {
        let revision = h.state().session.revision();
        h.state_mut().run(INITIALIZE_BASELINE, &json!({"expected_revision":revision}));
    }
    h.run();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    h.query_all_by_label("Fit board").next().unwrap().click();
    h.run();
    h
}

/// Extract only the chrome-owned thin accepted-connection strokes from the PCB canvas.
fn airwires(h: &Harness<'_, PcbApp>) -> Vec<[egui::Pos2; 2]> {
    let canvas = h.get_by_label("PCB canvas").rect();
    h.output()
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::LineSegment { points, stroke }
                if shape.clip_rect == canvas && stroke.width == 1. && stroke.color == Tokens::get().board_ink.gamma_multiply(0.55) =>
            {
                Some(*points)
            }
            _ => None,
        })
        .collect()
}

/// Replace a draft text field through its actual input control.
fn type_field(h: &mut Harness<'_, PcbApp>, label: &str, value: &str) {
    use egui::accesskit::Role;
    h.get_by_role_and_label(Role::TextInput, label).click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_role_and_label(Role::TextInput, label).type_text(value);
    h.run();
}
/// Text selection exposes local properties and owner airwires while physical edit selection stays empty.
#[test]
fn reference_selection_highlights_its_owner_and_protects_footprint_geometry() {
    let mut h = board(true);
    h.state_mut().browser.open = false;
    h.state_mut().views.active.board.set_ratsnest_visibility(false);
    h.run();
    let project = h.state().session.snapshot().unwrap();
    let target = project.board_target().unwrap();
    let footprint = project.board_view(target).unwrap().footprints().next().unwrap();
    let id = footprint.id.clone();
    let pads = footprint.pads.clone();
    let pose = (footprint.position, footprint.rotation, footprint.bottom);
    let model_point = footprint.graphics.iter().find(|graphic| graphic.reference).unwrap().points[0];
    let canvas = h.get_by_label("PCB canvas").rect();
    let point = h.state().views.active.camera.screen(model_point, canvas);
    let revision = h.state().session.revision();
    click_canvas_at(&mut h, point);
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(h.state().views.active.board.reference.selected.as_ref(), Some(&id));
    assert!(h.state().views.active.board.selection.is_empty());
    h.get_by_label("Silkscreen text");
    assert!(h.query_all_by_label("Printed text").next().is_none());
    assert!(
        h.output()
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape,egui::Shape::LineSegment {stroke,..} if stroke.width == 1.5 && stroke.color == Tokens::get().accent))
    );
    assert!(h.state().views.active.board.hidden_layers.contains(crate::editor_view::RATSNEST_KEY));
    let schematic = h.state().session.inspect()["project"]["schematic"].clone();
    click(&mut h, "Use part reference");
    type_field(&mut h, "Printed text", "Power input");
    type_field(&mut h, "Height", "1.6 mm");
    type_field(&mut h, "Stroke", "0.16 mm");
    assert_eq!(h.state().session.revision(), revision, "typed fields remain a draft");
    click(&mut h, "Apply changes");
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    assert_eq!(h.state().session.revision(), revision + 1);
    assert_eq!(h.state().session.inspect()["project"]["schematic"], schematic);
    let snapshot = h.state().session.snapshot().unwrap();
    let changed = snapshot.board_view(target).unwrap().footprints().find(|footprint| footprint.id == id).unwrap();
    assert_eq!(changed.reference_appearance.text, "Power input");
    assert_eq!(changed.reference_appearance.size.height_nm, 1_600_000);
    assert_eq!(changed.pads, pads);
    click(&mut h, "Rename…");
    type_field(&mut h, "Reference", "R99");
    click(&mut h, "Rename part");
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    let snapshot = h.state().session.snapshot().unwrap();
    let changed = snapshot.board_view(target).unwrap().footprints().find(|footprint| footprint.id == id).unwrap();
    assert_eq!(changed.reference, "R99");
    assert_eq!(changed.reference_appearance.text, "Power input");
    h.key_press(Key::R);
    h.run();
    let snapshot = h.state().session.snapshot().unwrap();
    let rotated = snapshot.board_view(target).unwrap().footprints().find(|footprint| footprint.id == id).unwrap();
    assert_eq!(rotated.reference_appearance.rotation, 90);
    assert_eq!((rotated.position, rotated.rotation, rotated.bottom), pose);
    assert_eq!(rotated.pads, pads);
    h.key_press(Key::Delete);
    h.run();
    let snapshot = h.state().session.snapshot().unwrap();
    let hidden = snapshot.board_view(target).unwrap().footprints().find(|footprint| footprint.id == id).unwrap();
    assert!(!hidden.reference_appearance.visible);
    assert_eq!(hidden.pads, pads);
    assert_eq!(snapshot.board_view(target).unwrap().footprints().count(), 2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    click(&mut h, "Reset silkscreen");
    let snapshot = h.state().session.snapshot().unwrap();
    let reset = snapshot.board_view(target).unwrap().footprints().find(|footprint| footprint.id == id).unwrap();
    assert!(reset.reference_text.is_none());
    assert_eq!(reset.reference_appearance.text, "R99");
    assert_eq!(reset.reference_appearance.size.height_nm, 1_000_000);
    assert_eq!((reset.position, reset.rotation, reset.bottom), pose);
    click(&mut h, "Draw");
    click(&mut h, "Draw lines on active layer");
    let canvas = h.get_by_label("PCB canvas").rect();
    click_canvas_at(&mut h, canvas.center_top() + egui::vec2(0., 100.));
    assert!(h.state().views.active.board.reference.selected.is_none(), "An outline tool replaces the text edit target");
}

/// Text rotation follows the screen on both placement sides and in a mirrored board view.
#[test]
fn reference_rotation_is_counterclockwise_on_screen_for_both_sides() {
    for bottom in [false, true] {
        for flipped in [false, true] {
            let mut h = board(true);
            let snapshot = h.state().session.snapshot().unwrap();
            let footprint = snapshot.board_view(snapshot.board_target().unwrap()).unwrap().footprints().next().unwrap().id;
            let revision = h.state().session.revision();
            h.state_mut().run(
                SET_FOOTPRINT_TRANSFORM,
                &json!({"footprint":footprint,"x_nm":0,"y_nm":0,"rotation":0,"side":if bottom {"back"} else {"front"},"expected_revision":revision}),
            );
            let revision = h.state().session.revision();
            h.state_mut().run(
                pcbcraft_engine::SET_FOOTPRINT_REFERENCE_TEXT,
                &json!({"footprint":footprint,"appearance":null,"offset":{"x_nm":-10_000_000,"y_nm":-10_000_000},"expected_revision":revision}),
            );
            assert!(h.state().message.is_none(), "{:?}", h.state().message);
            let policy = h.state().preferences.selection_follow;
            let app = h.state_mut();
            app.views.active.camera.set_mirrored(flipped);
            app.views.select_board_reference(footprint, &app.session, policy);
            h.run();
            let direction = |h: &Harness<'_, PcbApp>| {
                let snapshot = h.state().session.snapshot().unwrap();
                let placement = snapshot.board_view(snapshot.board_target().unwrap()).unwrap().footprints().next().unwrap();
                let points = &placement.graphics.iter().find(|graphic| graphic.reference).unwrap().points;
                let rect = h.get_by_label("PCB canvas").rect();
                let camera = &h.state().views.active.camera;
                camera.screen(points[1], rect) - camera.screen(points[0], rect)
            };
            let before = direction(&h);
            h.key_press(Key::R);
            h.run();
            let after = direction(&h);
            assert!(before.x * after.y - before.y * after.x < 0., "bottom={bottom}, flipped={flipped}");
            h.key_press_modifiers(Modifiers::SHIFT, Key::R);
            h.run();
            assert!((direction(&h) - before).length() < 0.01);
        }
    }
}

/// Dirty property drafts survive unrelated edits, refuse stale submission and retain rename errors.
#[test]
fn reference_properties_refuse_stale_drafts_and_duplicate_component_names() {
    let mut h = board(true);
    let id = h
        .state()
        .session
        .snapshot()
        .unwrap()
        .board_view(h.state().session.project().unwrap().board_target().unwrap())
        .unwrap()
        .footprints()
        .next()
        .unwrap()
        .id;
    let policy = h.state().preferences.selection_follow;
    let app = h.state_mut();
    app.views.select_board_reference(id.clone(), &app.session, policy);
    h.run();
    click(&mut h, "Use part reference");
    type_field(&mut h, "Printed text", "Pending print");
    let revision = h.state().session.revision();
    h.state_mut().run(SET_FOOTPRINT_TRANSFORM, &json!({"footprint":id,"x_nm":1_000_000,"y_nm":0,"rotation":0,"expected_revision":revision}));
    h.run();
    h.get_by_label("The board changed. Reload properties to continue.");
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    assert!(h.query_all_by_label("Apply changes").next().is_none());
    click(&mut h, "Reload properties");
    h.run();
    click(&mut h, "Rename…");
    type_field(&mut h, "Reference", "R2");
    click(&mut h, "Rename part");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    assert!(h.state().message.is_some());
    h.get_by_label("Cancel rename");
    type_field(&mut h, "Reference", "R42");
    click(&mut h, "Rename part");
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    h.get_by_label("Silkscreen text");
}

/// Addressed reference selection uses stable correspondence, and renaming finds its owning sheet.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_reference_selection_and_rename_reach_a_component_on_another_sheet() {
    let mut h = board(false);
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::sheets::CREATE_SHEET, &json!({"name":"Power","expected_revision":revision}));
    let snapshot = h.state().session.snapshot().unwrap();
    let main = snapshot.sheet_target();
    let power = snapshot.sheets().nth(1).unwrap().target;
    let revision = h.state().session.revision();
    h.state_mut().run(PLACE_SYMBOL, &json!({"sheet":power.id(),"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":revision}));
    let snapshot = h.state().session.snapshot().unwrap();
    let scoped = snapshot.for_sheet(power).unwrap();
    let symbol = scoped.symbols().last().unwrap().id();
    let revision = h.state().session.revision();
    h.state_mut().run(PLACE_FOOTPRINT, &json!({"sheet":power.id(),"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"1","2":"2"},"x_nm":20_000_000,"y_nm":0,"expected_revision":revision}));
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    let snapshot = h.state().session.snapshot().unwrap();
    let footprint = snapshot.board_view(snapshot.board_target().unwrap()).unwrap().footprints().last().unwrap().id;
    let main_symbols = snapshot.for_sheet(main).unwrap().symbols().to_vec();
    let board = h.state().views.active.id();
    let ctx = h.ctx.clone();
    h.state_mut().run_window_command(&ctx, "view.set_workspace", &json!({"view":board.0,"workspace":"schematic","placement":"beside"})).unwrap();
    h.run();
    let schematic =
        h.state_mut().views.all_mut().find(|view| view.workspace == crate::editor_view::Workspace::Schematic && view.id() != board).unwrap().id();
    h.state_mut().run_window_command(&ctx, "view.open_sheet", &json!({"view":schematic.0,"sheet":power.id()})).unwrap();
    let schematic = h.state_mut().views.all_mut().find(|view| view.target() == Some(crate::editor_view::DocumentTarget::Sheet(power))).unwrap().id();
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    assert!(
        h.state_mut().run_window_command(&ctx, "view.select_footprints", &json!({"view":board.0,"footprints":[],"element":"reference"})).is_err()
    );
    h.state_mut()
        .run_window_command(&ctx, "view.select_footprints", &json!({"view":board.0,"footprints":[footprint],"element":"reference"}))
        .unwrap();
    h.run();
    assert!(h.state().views.view(board).unwrap().board.selection.is_empty());
    assert_eq!(h.state().views.view(board).unwrap().board.reference.selected.as_ref(), Some(&footprint));
    assert_eq!(h.state().views.highlight_for(schematic, &h.state().session).single_symbol(), Some(symbol.as_str()));
    assert!(h.state().views.view(schematic).unwrap().schematic.selection.is_empty());
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    h.state_mut().views.bind(board);
    h.run();
    click(&mut h, "Rename…");
    type_field(&mut h, "Reference", "R88");
    click(&mut h, "Rename part");
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    let snapshot = h.state().session.snapshot().unwrap();
    let scoped = snapshot.for_sheet(power).unwrap();
    assert_eq!(scoped.reference(scoped.symbols().last().unwrap()), "R88");
    assert_eq!(snapshot.for_sheet(main).unwrap().symbols(), main_symbols);
}

/// Real canvas strokes use engine-mapped centers, remain behind pads and track authored transforms/Undo.
#[test]
fn ratsnest_projects_accepted_pad_centers_and_recomputes_after_move_and_undo() {
    let mut h = board(true);
    assert_eq!(airwires(&h).len(), 1);
    h.get_by_label("Board Outline · 1 airwire");
    let wire_index = h.output().shapes.iter().position(|shape| matches!(&shape.shape, egui::Shape::LineSegment { stroke, .. } if stroke.width == 1. && stroke.color == Tokens::get().board_ink.gamma_multiply(0.55))).unwrap();
    let pad_index =
        h.output().shapes.iter().position(|shape| matches!(&shape.shape, egui::Shape::Path(path) if path.fill == Tokens::get().board_front)).unwrap();
    assert!(wire_index < pad_index);

    let canvas = h.get_by_label("PCB canvas").rect();
    let cached = h.state().views.active.board.ratsnest_cache.as_ref().unwrap().1.clone();
    let line = &cached.lines[0];
    assert_eq!(
        airwires(&h)[0],
        [h.state().views.active.camera.screen(line.from.position, canvas), h.state().views.active.camera.screen(line.to.position, canvas)]
    );
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    let footprint = line.from.footprint.clone();
    h.state_mut()
        .run(SET_FOOTPRINT_TRANSFORM, &json!({"footprint":footprint,"x_nm":5_000_000,"y_nm":4_000_000,"rotation":90,"expected_revision":revision}));
    h.run();
    let moved = h.state().views.active.board.ratsnest_cache.as_ref().unwrap().1.clone();
    assert_ne!(cached.lines, moved.lines);
    assert_eq!(cached.source, moved.source);
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert_eq!(h.state().views.active.board.ratsnest_cache.as_ref().unwrap().1.as_ref(), cached.as_ref());
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::DELETE_FOOTPRINTS, &json!({"footprints":[line.from.footprint],"expected_revision":revision}));
    h.run();
    assert!(airwires(&h).is_empty());
    assert_eq!(h.state().views.active.board.ratsnest_cache.as_ref().unwrap().1.missing.len(), 1);
    h.get_by_label("Board Outline · 0 airwires · 1 missing endpoint");
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    let after_undo = h.state().views.active.board.ratsnest_cache.as_ref().unwrap().1.clone();
    h.state_mut().views.active.camera.center_on(pcbcraft_engine::schematic::Point { x_nm: 2_000_000, y_nm: 1_000_000 });
    h.run();
    assert!(Arc::ptr_eq(&h.state().views.active.board.ratsnest_cache.as_ref().unwrap().1, &after_undo));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// The local toggle is restored per view, uses the live command handler too, and never edits the design.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn ratsnest_visibility_is_saved_per_view_and_exposed_to_live_control() {
    let mut h = board(true);
    let before = h.state().session.inspect();
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click(&mut h, "Ratsnest");
    assert!(airwires(&h).is_empty());
    for action in ["Isolate active", "Show all"] {
        click(&mut h, action);
        assert!(h.state().views.active.board.hidden_layers.contains("ratsnest"));
        assert!(airwires(&h).is_empty());
    }
    let saved = h.state().views.active.saved_state(&h.state().session).unwrap();
    let mut restored = crate::editor_view::EditorView::from_state(ViewId(99), &saved, &h.state().session);
    restored.sync(&h.state().session);
    assert!(restored.board.hidden_layers.contains("ratsnest"));
    let ctx = h.ctx.clone();
    let view = h.state().views.active.id().0;
    h.state_mut().run_window_command(&ctx, "view.set_ratsnest_visibility", &json!({"view":view,"visible":true})).unwrap();
    h.run();
    assert_eq!(airwires(&h).len(), 1);
    assert!(restored.board.hidden_layers.contains("ratsnest"));
    assert!(h.state_mut().run_window_command(&ctx, "view.set_ratsnest_visibility", &json!({"view":999,"visible":false})).is_err());
    h.set_size(egui::vec2(760., 560.));
    h.run();
    let checkbox = h.get_by_label("Ratsnest").rect();
    let toolbar = h.get_by_label("Show all").rect();
    assert!((checkbox.center().y - toolbar.center().y).abs() < 2.);
    assert!(checkbox.right() <= h.get_by_label("Collapse Layers").rect().right());
    assert!(checkbox.bottom() < 540.);
    assert_eq!(h.state().session.inspect(), before);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
}

/// Proposed connections are neither drawn nor counted until the board has accepted inputs.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn ratsnest_controls_and_counts_are_unavailable_before_acceptance() {
    let mut h = board(false);
    assert!(airwires(&h).is_empty());
    assert!(h.query_all_by_label("Ratsnest").next().is_none());
    assert!(h.state().views.active.board.ratsnest_cache.is_none());
    let ctx = h.ctx.clone();
    let view = h.state().views.active.id().0;
    let before = h.state().session.inspect();
    assert!(h.state_mut().run_window_command(&ctx, "view.set_ratsnest_visibility", &json!({"view":view,"visible":true})).is_err());
    assert_eq!(h.state().session.inspect(), before);
}

/// The owner leader is independent of electrical airwires and follows a disposable text move.
#[test]
fn reference_owner_link_follows_moving_text_without_a_ratsnest() {
    let mut h = board(false);
    h.state_mut().browser.open = false;
    let snapshot = h.state().session.snapshot().unwrap();
    let target = snapshot.board_target().unwrap();
    let footprint = snapshot.board_view(target).unwrap().footprints().next().unwrap();
    let id = footprint.id;
    let position = footprint.position;
    let policy = h.state().preferences.selection_follow;
    let app = h.state_mut();
    app.views.select_board_reference(id, &app.session, policy);
    h.run();
    assert!(h.state().views.active.board.ratsnest_cache.is_none());
    let canvas = h.get_by_label("PCB canvas").rect();
    let owner = h.state().views.active.camera.screen(position, canvas);
    let marker = |h: &Harness<'_, PcbApp>| {
        h.output().shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::Circle(circle) if circle.center.distance(owner) < 0.01 && circle.radius==3. && circle.fill==egui::Color32::TRANSPARENT)
    })
    };
    assert!(marker(&h), "Selected ink links to the physical footprint without electrical airwires");
    let text = footprint.graphics.iter().find(|graphic| graphic.reference).unwrap().points[0];
    let at = h.state().views.active.camera.screen(text, canvas);
    h.hover_at(at);
    h.run();
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.key_press(Key::M);
    h.run();
    h.hover_at(at + egui::vec2(80., -30.));
    h.run();
    assert!(h.state().views.active.board.reference_drag.is_some());
    assert!(marker(&h), "The owner endpoint stays on the footprint during a text move");
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
}

/// Compact forms align labels and controls and retain their actions below the scrolling fields.
#[test]
fn reference_properties_align_controls_and_keep_actions_visible_in_short_hosts() {
    let mut h = board(true);
    h.state_mut().browser.open = false;
    let snapshot = h.state().session.snapshot().unwrap();
    let id = snapshot.board_view(snapshot.board_target().unwrap()).unwrap().footprints().next().unwrap().id;
    let policy = h.state().preferences.selection_follow;
    let app = h.state_mut();
    app.views.select_board_reference(id, &app.session, policy);
    h.set_size(egui::vec2(1000., 520.));
    h.run();
    let height = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Height").rect();
    let font = h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Font").rect();
    assert!((height.left() - font.left()).abs() < 1.);
    assert!((height.right() - font.right()).abs() < 1.);
    for label in ["Apply changes", "Reset silkscreen"] {
        let rect = h.get_by_label(label).rect();
        assert!(rect.top() > height.bottom() && rect.bottom() < 500., "{label}: {rect:?}");
    }
    assert!(h.query_all_by_label("Override text size").next().is_none());
}
