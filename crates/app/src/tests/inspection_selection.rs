//! #397 exact inspection stays local and cannot acquire authored edit authority.
use super::*;
use pcbcraft_engine::{
    board::correspondence::{PadTarget, Source, TerminalTarget},
    persistence::encode_documents,
};

/// Document state is captured independently of view selection and display settings.
#[derive(Debug, PartialEq)]
struct DocumentState {
    bytes: Vec<(String, Vec<u8>)>,
    revision: u64,
    dirty: bool,
    undo: bool,
    redo: bool,
}

/// Snapshot bytes, history and dirty state must survive every view-only inspection gesture.
fn document(h: &Harness<'_, PcbApp>) -> DocumentState {
    let app = h.state();
    DocumentState {
        bytes: encode_documents(app.session.snapshot().unwrap()).unwrap().into_iter().collect(),
        revision: app.session.revision(),
        dirty: app.files.dirty(&app.session),
        undo: app.session.available("edit.undo"),
        redo: app.session.available("edit.redo"),
    }
}

/// Pick a source without relying on generated symbol numbers or physical display labels.
fn pin(h: &Harness<'_, PcbApp>, index: usize) -> (Source, egui::Pos2) {
    let project = h.state().views.active.snapshot(&h.state().session).unwrap();
    let terminal = project.symbols().iter().find(|symbol| symbol.definition.id == "resistor").unwrap().terminals()[index];
    let point = project.terminal_position(terminal).unwrap();
    (
        Source::Terminal(TerminalTarget { sheet: project.sheet_target(), terminal }),
        h.state().views.active.camera.screen(point, h.get_by_label("Schematic canvas").rect()),
    )
}

/// Full real pointer presses carry modifiers through release, then let double-click time expire.
fn click_at(h: &mut Harness<'_, PcbApp>, at: egui::Pos2, modifiers: Modifiers) {
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.hover_at(at);
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers });
        h.run();
    }
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(20);
}

/// Inspection picking wins over a pin's owning symbol and click-to-wire, with normal modifiers.
#[test]
fn pins_select_add_toggle_and_clear_without_editing() {
    let mut h = harness();
    resistor_at_origin(&mut h);
    h.set_size(egui::vec2(1400., 900.));
    h.run();
    let id = h.state().views.active.id();
    h.state_mut().set_selection_filters(id, &json!({"pins":true})).unwrap();
    h.run();
    let before = document(&h);
    let (a, at) = pin(&h, 0);
    let (b, bt) = pin(&h, 1);
    click_at(&mut h, at, Modifiers::NONE);
    assert_eq!(h.state().views.active.inspection, vec![a]);
    assert!(h.state().views.active.schematic.selection.is_empty());
    assert!(h.state().views.active.schematic.tool.is_none());
    click_at(&mut h, bt, Modifiers::SHIFT);
    assert_eq!(h.state().views.active.inspection, vec![a, b]);
    for modifier in [Modifiers::CTRL, Modifiers::COMMAND] {
        click_at(&mut h, at, modifier);
        assert_eq!(h.state().views.active.inspection, vec![b]);
        click_at(&mut h, at, modifier);
        assert_eq!(h.state().views.active.inspection, vec![b, a]);
    }
    for key in [Key::M, Key::G, Key::R, Key::E, Key::Delete, Key::Backspace, Key::X, Key::Y] {
        h.key_press(key);
        h.run();
    }
    assert!(h.state().views.active.schematic.tool.is_none());
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(document(&h), before);
    h.key_press(Key::Escape);
    h.run();
    assert!(h.state().views.active.inspection.is_empty());
    click_at(&mut h, at, Modifiers::NONE);
    h.state_mut().set_selection_filters(id, &json!({"pins":false})).unwrap();
    h.run();
    assert!(h.state().views.active.inspection.is_empty());
    assert_eq!(document(&h), before);
}

/// The command rejects foreign, filtered and obsolete targets atomically, and edits never
/// resurrect an inspection selection through Undo/Redo or an offline reopen.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn inspection_command_validates_authority_and_expires_on_revision() {
    let mut h = harness();
    resistor_at_origin(&mut h);
    h.run();
    let id = h.state().views.active.id();
    h.state_mut().set_selection_filters(id, &json!({"pins":true})).unwrap();
    h.run();
    let (source, _) = pin(&h, 0);
    let ctx = h.ctx.clone();
    let before = document(&h);
    let args = json!({"view":id.0,"targets":[source.inspect()],"expected_revision":h.state().session.revision(),"history_root_revision":h.state().session.history_root_revision()});
    h.state_mut().run_window_command(&ctx, "view.select_inspection", &args).unwrap();
    h.run();
    assert_eq!(h.state().views.active.inspection, vec![source]);
    assert_eq!(document(&h), before);
    for field in ["project", "sheet", "terminal"] {
        let mut bad = args.clone();
        bad["targets"][0][field] = json!(if field == "terminal" { "BAD".into() } else { uuid::Uuid::from_u128(397).to_string() });
        assert!(h.state_mut().run_window_command(&ctx, "view.select_inspection", &bad).is_err());
        assert_eq!(h.state().views.active.inspection, vec![source]);
    }
    let mut bad = args.clone();
    bad["history_root_revision"] = json!(99999);
    assert!(h.state_mut().run_window_command(&ctx, "view.select_inspection", &bad).is_err());
    h.state_mut().run("project.rename", &json!({"name":"Peer edit"}));
    h.run();
    assert!(h.state().views.active.inspection.is_empty());
    assert!(h.state_mut().run_window_command(&ctx, "view.select_inspection", &args).is_err());
    h.state_mut().run("edit.undo", &json!({}));
    h.run();
    assert!(h.state().views.active.inspection.is_empty());
    h.state_mut().run("edit.redo", &json!({}));
    h.run();
    assert!(h.state().views.active.inspection.is_empty());
    h.state_mut().select_inspection(id, vec![source], "replace").unwrap();
    assert_eq!(h.state().views.active.inspection, vec![source]);
    let saved = h.state().session.snapshot().unwrap();
    h.state_mut().session.open_snapshot(saved);
    h.run();
    assert!(h.state().views.active.inspection.is_empty());
}

/// Return a real land's qualified identity and current screen point without selecting its owner.
fn first_pad(h: &Harness<'_, PcbApp>) -> (Source, egui::Pos2) {
    let board = h.state().session.project().unwrap().boards().next().unwrap();
    let footprint = board.footprints().next().unwrap();
    let pad = &footprint.pads[0];
    (
        Source::Pad(PadTarget {
            board: board.target,
            pad: pcbcraft_engine::board::Pad::parse(&footprint.id, &pad.number).unwrap().on_board(board.target.board),
        }),
        h.state().views.active.camera.screen(pad.center, h.get_by_label("PCB canvas").rect()),
    )
}

/// A quick drag is classified by where it was pressed, even when its first motion leaves copper.
/// Body drags still move the footprint and undo exactly with Pads enabled.
#[test]
fn pad_inspection_drag_origin_cannot_move_the_owner() {
    for pad_origin in [true, false] {
        let mut h = super::board_tracks::routable();
        let id = h.state().views.active.id();
        h.state_mut().set_selection_filters(id, &json!({"pads":true})).unwrap();
        h.run();
        let at = if pad_origin {
            first_pad(&h).1
        } else {
            let board = h.state().session.project().unwrap().boards().next().unwrap();
            h.state().views.active.camera.screen(board.footprints().next().unwrap().position, h.get_by_label("PCB canvas").rect())
        };
        let before = document(&h);
        h.hover_at(at);
        h.run();
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.step();
        let to = at + egui::vec2(60., 35.);
        h.event(egui::Event::PointerMoved(to));
        h.run();
        assert_eq!(h.state().views.active.board.draft.is_some(), !pad_origin);
        assert!(h.state().views.active.board.reference_drag.is_none());
        h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run();
        if !pad_origin {
            assert_eq!(h.state().session.revision(), before.revision + 1);
            h.state_mut().run("edit.undo", &json!({}));
            h.run();
            assert_eq!(document(&h).bytes, before.bytes);
        } else {
            assert_eq!(document(&h), before);
        }
    }
}

/// An already captured reference retains its authority over a pad, but focus/revision loss
/// still cancels it. A successful placement edits only the reference and remains undoable.
#[test]
fn pad_inspection_does_not_cancel_an_active_reference_move() {
    enum Finish {
        Place,
        Focus,
        Revision,
    }
    for finish in [Finish::Place, Finish::Focus, Finish::Revision] {
        let mut h = super::board_tracks::routable();
        let id = h.state().views.active.id();
        h.state_mut().set_selection_filters(id, &json!({"pads":true})).unwrap();
        h.run();
        let board = h.state().session.project().unwrap().boards().next().unwrap();
        let footprint = board.footprints().next().unwrap();
        let origin = footprint.position;
        let pads = footprint.pads.clone();
        let reference = footprint.graphics.iter().find(|g| g.reference).unwrap().points[0];
        let from = h.state().views.active.camera.screen(reference, h.get_by_label("PCB canvas").rect());
        let to = first_pad(&h).1;
        let before = document(&h);
        h.hover_at(from);
        h.run();
        h.key_press(Key::M);
        h.run();
        assert!(h.state().views.active.board.reference_drag.is_some());
        h.hover_at(to);
        h.run();
        assert!(h.state().views.active.board.reference_drag.is_some());
        assert_eq!(document(&h), before);
        match finish {
            Finish::Place => {
                click_at(&mut h, to, Modifiers::NONE);
                assert_eq!(h.state().session.revision(), before.revision + 1);
                let board = h.state().session.project().unwrap().boards().next().unwrap();
                let footprint = board.footprints().next().unwrap();
                assert_eq!(footprint.position, origin);
                assert_eq!(footprint.pads, pads);
                h.state_mut().run("edit.undo", &json!({}));
                h.run();
                assert_eq!(document(&h).bytes, before.bytes);
            }
            Finish::Focus => {
                h.event(egui::Event::WindowFocused(false));
                h.run();
                assert_eq!(document(&h), before);
            }
            Finish::Revision => {
                h.state_mut().run("project.rename", &json!({"name":"Reference source changed"}));
                h.run();
            }
        }
        assert!(h.state().views.active.board.reference_drag.is_none());
    }
}

/// Pad picking wins over overlapping idle copper, while disabling Pads restores track picking.
#[test]
fn pad_inspection_takes_priority_over_tracks_without_footprint_filter() {
    let mut h = super::board_tracks::routable();
    let pad = h.state().session.project().unwrap().boards().next().unwrap().footprints().next().unwrap().pads[0].center;
    let revision = h.state().session.revision();
    let layer = h.state().views.active.board.active_layer.clone().unwrap();
    h.state_mut().run(
        pcbcraft_engine::ADD_TRACKS,
        &json!({"layer":layer,"width_nm":250_000,"points":[pad,{"x_nm":pad.x_nm,"y_nm":pad.y_nm-2_540_000}],"expected_revision":revision}),
    );
    assert_eq!(h.state().message, None);
    h.run();
    let id = h.state().views.active.id();
    h.state_mut().set_selection_filters(id, &json!({"pads":true,"footprints":false})).unwrap();
    h.run();
    let (target, at) = first_pad(&h);
    let before = document(&h);
    h.hover_at(at);
    h.run();
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    let to = at + egui::vec2(60., 35.);
    h.event(egui::Event::PointerMoved(to));
    h.run();
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run();
    assert_eq!(document(&h), before, "a pad-origin drag must not move the underlying track");
    click_at(&mut h, at, Modifiers::NONE);
    assert_eq!(h.state().views.active.inspection, vec![target]);
    h.key_press_modifiers(Modifiers::SHIFT, Key::Space);
    h.run();
    assert!(h.state().views.active.inspection.is_empty());
    assert_eq!(h.state().views.active.board.tracks.selected.len(), 1);
    h.key_press_modifiers(Modifiers::SHIFT, Key::Space);
    h.run();
    assert_eq!(h.state().views.active.inspection, vec![target]);
    assert!(h.state().views.active.board.tracks.selected.is_empty());
    h.state_mut().set_selection_filters(id, &json!({"pads":false})).unwrap();
    h.run();
    click_at(&mut h, at, Modifiers::NONE);
    assert_eq!(h.state().views.active.board.tracks.selected.len(), 1);
    assert_eq!(document(&h), before);
}

/// Explicitly armed wire tools retain Backspace after inspection; idle shortcuts still cannot
/// edit an inspected pin's owner (covered by the modifier/authority test above).
#[test]
fn pin_inspection_preserves_wire_backspace() {
    let mut h = harness();
    resistor_at_origin(&mut h);
    let id = h.state().views.active.id();
    h.state_mut().set_selection_filters(id, &json!({"pins":true})).unwrap();
    h.run();
    let (target, _) = pin(&h, 0);
    h.state_mut().select_inspection(id, vec![target], "replace").unwrap();
    let before = document(&h);
    h.hover_at(canvas_center(&h) + egui::vec2(-101.6, -101.6));
    h.run();
    h.key_press(Key::W);
    h.run();
    // Accept either W policy: this branch starts at the pointer; #415 arms an idle tool.
    if matches!(h.state().views.active.schematic.tool, Some(Tool::Wire { start: None, .. })) {
        click_canvas_offset(&mut h, egui::vec2(-101.6, -101.6));
    }
    click_canvas_offset(&mut h, egui::vec2(0., -50.8));
    assert_eq!(h.state().views.active.inspection, vec![target]);
    h.key_press(Key::Backspace);
    h.run();
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Wire { start: Some(_), .. })));
    h.key_press(Key::Backspace);
    h.run();
    assert!(matches!(h.state().views.active.schematic.tool, Some(Tool::Wire { start: None, .. })));
    assert_eq!(document(&h), before);
}

/// Additive inspection cannot revoke explicit drawing/zone deletion authority or delete pads.
#[test]
fn pad_inspection_preserves_explicit_drawing_and_zone_delete() {
    for zone in [false, true] {
        let mut h = super::board_tracks::routable();
        let revision = h.state().session.revision();
        let layer = if zone {
            h.state().views.active.board.active_layer.clone().unwrap()
        } else {
            h.state()
                .session
                .project()
                .unwrap()
                .boards()
                .next()
                .unwrap()
                .layers()
                .into_iter()
                .find(|layer| layer.purpose == pcbcraft_engine::board::setup::LayerPurpose::Outline)
                .unwrap()
                .id
                .to_string()
        };
        let net = h.state().session.project().unwrap().inspect_net_registry().unwrap()["nets"][0]["id"].clone();
        let mut params = json!({"layer":layer,"points":[{"x_nm":-5_080_000,"y_nm":5_080_000},{"x_nm":5_080_000,"y_nm":5_080_000},{"x_nm":5_080_000,"y_nm":10_160_000}],"expected_revision":revision});
        if zone {
            params["net"] = net;
        } else {
            params["closed"] = json!(true);
        }
        h.state_mut().run(if zone { pcbcraft_engine::board::zones::ADD_ZONE } else { pcbcraft_engine::ADD_DRAWING }, &params);
        assert_eq!(h.state().message, None);
        h.run();
        let board = h.state().session.project().unwrap().boards().next().unwrap();
        let selected = if zone { board.zones()[0].id } else { board.drawings()[0].id };
        let mut items = pcbcraft_engine::board::clipboard::BoardSelection::default();
        if zone {
            items.zones.insert(selected);
        } else {
            items.drawings.insert(selected);
        }
        let id = h.state().views.active.id();
        h.state_mut().set_selection_filters(id, &json!({"pads":true})).unwrap();
        h.run();
        h.state_mut().select_board_items(items);
        let (target, _) = first_pad(&h);
        h.state_mut().select_inspection(id, vec![target], "add").unwrap();
        h.run();
        let before = document(&h);
        h.key_press(Key::Delete);
        h.run();
        assert_eq!(h.state().session.revision(), before.revision + 1);
        let board = h.state().session.project().unwrap().boards().next().unwrap();
        assert!(board.drawings().is_empty());
        assert!(board.zones().is_empty());
        assert_eq!(board.footprints().count(), 2);
        h.state_mut().run("edit.undo", &json!({}));
        h.run();
        assert_eq!(document(&h).bytes, before.bytes);
    }
}

/// Pads select their exact qualified address, respect physical visibility and clear with filters.
#[test]
fn pads_select_without_footprint_authority_and_hide_with_their_layer() {
    let mut h = super::board_tracks::routable();
    let id = h.state().views.active.id();
    h.state_mut().set_selection_filters(id, &json!({"pads":true})).unwrap();
    h.run();
    let board = h.state().session.project().unwrap().boards().next().unwrap();
    let footprint = board.footprints().next().unwrap();
    let pad = footprint.pads[0].clone();
    let target = Source::Pad(PadTarget {
        board: board.target,
        pad: pcbcraft_engine::board::Pad::parse(&footprint.id, &pad.number).unwrap().on_board(board.target.board),
    });
    let at = h.state().views.active.camera.screen(pad.center, h.get_by_label("PCB canvas").rect());
    let before = document(&h);
    click_at(&mut h, at, Modifiers::NONE);
    assert_eq!(h.state().views.active.inspection, vec![target]);
    assert!(h.state().views.active.board.selection.is_empty());
    for key in [Key::M, Key::R, Key::F, Key::E, Key::Delete] {
        h.key_press(key);
        h.run();
    }
    assert_eq!(document(&h), before);
    assert!(!h.state().views.active.board.editing_footprint());
    let layers = match pad.copper_layers.unwrap() {
        pcbcraft_engine::board::view::CopperLayers::Resolved { layers, .. } => layers,
        _ => panic!("configured land"),
    };
    for layer in layers {
        h.state_mut().views.active.board.set_layer(&layer.to_string(), false, Some(false));
    }
    h.run();
    assert!(h.state().views.active.inspection.is_empty());
    assert!(h.state_mut().select_inspection(id, vec![target], "replace").is_err());
    assert_eq!(document(&h), before);
}

/// Inspection belongs to the addressed pane, survives host relocation without being copied,
/// and cannot be sent to an incompatible workspace or a closed view.
#[test]
fn inspection_is_local_to_split_and_detached_views() {
    let mut h = harness();
    resistor_at_origin(&mut h);
    h.run();
    let primary = h.state().views.active.id();
    h.state_mut().set_selection_filters(primary, &json!({"pins":true})).unwrap();
    h.run();
    let (source, _) = pin(&h, 0);
    h.state_mut().select_inspection(primary, vec![source], "replace").unwrap();
    let before = document(&h);
    let child = h.state_mut().views.open_beside(primary).unwrap();
    assert!(h.state().views.view(child).unwrap().inspection.is_empty());
    h.state_mut().set_selection_filters(child, &json!({"pins":true})).unwrap();
    h.state_mut().select_inspection(child, vec![source], "replace").unwrap();
    let host = h.state_mut().views.detach_view(child).unwrap();
    assert_eq!(h.state().views.hosts[&host].focused, child);
    assert_eq!(h.state().views.view(child).unwrap().inspection, vec![source]);
    assert_eq!(h.state().views.view(primary).unwrap().inspection, vec![source]);
    h.state_mut().views.dock_view(child);
    let app = h.state_mut();
    app.views.view_mut(child).unwrap().switch(Workspace::Board, &app.session);
    assert!(app.views.view(child).unwrap().inspection.is_empty());
    assert!(app.select_inspection(child, vec![source], "replace").is_err());
    app.views.active.definition = Some(397);
    app.views.active.sync(&app.session);
    assert!(app.views.active.inspection.is_empty());
    app.views.active.definition = None;
    app.views.close_view(child);
    assert!(app.select_inspection(child, vec![source], "replace").is_err());
    assert_eq!(document(&h), before);
}

/// Modal ownership and a pin drag cannot acquire owner edit authority.
#[test]
fn pin_inspection_respects_modal_focus_and_does_not_drag_its_owner() {
    let mut h = harness();
    resistor_at_origin(&mut h);
    h.run();
    let id = h.state().views.active.id();
    h.state_mut().set_selection_filters(id, &json!({"pins":true})).unwrap();
    h.run();
    let (source, at) = pin(&h, 0);
    let before = document(&h);
    h.state_mut().open_preferences();
    h.run();
    click_at(&mut h, at, Modifiers::NONE);
    assert!(h.state().views.active.inspection.is_empty());
    h.key_press(Key::Escape);
    h.run();
    h.hover_at(at);
    h.run();
    h.drag_at(at);
    h.run();
    h.hover_at(at + egui::vec2(50., 25.));
    h.run();
    h.event(egui::Event::PointerButton {
        pos: at + egui::vec2(50., 25.),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(20);
    assert!(h.state().views.active.schematic.tool.is_none());
    assert_eq!(document(&h), before);
    h.state_mut().select_inspection(id, vec![source], "replace").unwrap();
    assert_eq!(h.state().views.active.inspection, vec![source]);
}

/// Browser pin rows and the canvas share the semantic selection handler, while ordinary
/// owner selections retain their original live-command payload and clear inspection.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn browser_pins_share_semantics_and_legacy_selection_clears_inspection() {
    let mut h = harness();
    resistor_at_origin(&mut h);
    h.set_size(egui::vec2(1400., 900.));
    let id = h.state().views.active.id();
    h.state_mut().browser.open = true;
    h.state_mut().set_selection_filters(id, &json!({"pins":true})).unwrap();
    h.run();
    let (source, _) = pin(&h, 0);
    let before = document(&h);
    click(&mut h, "Expand Symbols");
    click(&mut h, "Expand Pins");
    let label = crate::inspection_selection::labels(&h.state().session, &[source])[0].clone();
    click(&mut h, &label);
    assert_eq!(h.state().views.active.inspection, vec![source]);
    assert!(h.state().views.active.schematic.selection.is_empty());
    let ctx = h.ctx.clone();
    h.state_mut().run_window_command(&ctx, "view.select", &json!({"view":id.0,"items":{"symbols":[],"wires":[]}})).unwrap();
    assert!(h.state().views.active.inspection.is_empty());
    assert_eq!(document(&h), before);
}

/// Canvas pad picking uses actual rotated copper contours rather than their bounding corners.
#[test]
fn pad_inspection_uses_shape_contours() {
    let h = super::board_tracks::routable();
    let board = h.state().session.project().unwrap().boards().next().unwrap();
    let mut pad = board.footprints().next().unwrap().pads[0].clone();
    for shape in [
        pcbcraft_engine::board::definition::PadShape::Rectangle,
        pcbcraft_engine::board::definition::PadShape::Circle,
        pcbcraft_engine::board::definition::PadShape::Oval,
        pcbcraft_engine::board::definition::PadShape::RoundedRectangle,
    ] {
        pad.shape = shape;
        pad.corner_radius_nm = 50_000;
        assert!(pad.contains_point(pad.center));
        for coordinate in [i64::MIN, i64::MAX] {
            assert!(!pad.contains_point(pcbcraft_engine::schematic::Point { x_nm: coordinate, y_nm: coordinate }));
        }
        assert_eq!(pad.contains_point(pad.copper.unwrap()[0]), shape == pcbcraft_engine::board::definition::PadShape::Rectangle);
    }
}

/// Repeated logical supply terminals on two symbol units retain distinct sheet/appearance
/// addresses, and saved arrangements contain filters but never an inspection selection.
#[test]
fn shared_unit_terminals_are_inspected_by_placed_appearance() {
    let mut h = harness();
    h.state_mut().run("project.new", &json!({}));
    h.state_mut().run(
        pcbcraft_engine::library::sources::LOAD,
        &json!({"source":"units","documents":{
        "library.json":include_str!("../../../../examples/native-parts/library.json"),
        "symbols/dual.json":include_str!("../../../../examples/native-parts/symbols/dual-amplifier.json")}}),
    );
    let definition = h.state().session.libraries().latest_symbol("parts-demo", "dual-amplifier").unwrap();
    let revision = h.state().session.revision();
    h.state_mut().place_catalog_symbol(&definition, json!({"x_nm":0,"y_nm":0,"expected_revision":revision}));
    let first = h.state().session.project().unwrap().symbols()[0].clone();
    let root = h.state().session.project().unwrap().sheet_target();
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::sheets::CREATE_SHEET, &json!({"name":"Second","expected_revision":revision}));
    let second = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::PLACE_UNIT,
        &json!({"sheet":second.id(),"component":first.component_id(),"unit":"b","x_nm":0,"y_nm":0,"expected_revision":revision}),
    );
    h.run();
    assert_eq!(h.state().message, None);
    let id = h.state().views.active.id();
    h.state_mut().set_selection_filters(id, &json!({"pins":true})).unwrap();
    let a = Source::Terminal(TerminalTarget { sheet: root, terminal: first.terminals().into_iter().find(|pin| pin.number() == "3").unwrap() });
    h.state_mut().select_inspection(id, vec![a], "replace").unwrap();
    let saved = h.state().views.active.saved_state(&h.state().session).unwrap();
    let json = serde_json::to_value(&saved).unwrap();
    assert!(json.get("inspection").is_none());
    let restored = EditorView::from_state(ViewId(997), &saved, &h.state().session);
    assert!(restored.inspection.is_empty());
    assert!(restored.filters.pins);
    let before = document(&h);
    let app = h.state_mut();
    app.views.active.open_document(crate::editor_view::DocumentTarget::Sheet(second), &app.session);
    assert!(app.views.active.inspection.is_empty());
    assert!(app.select_inspection(id, vec![a], "replace").is_err());
    let terminal =
        app.session.project().unwrap().for_sheet(second).unwrap().symbols()[0].terminals().into_iter().find(|pin| pin.number() == "3").unwrap();
    let b = Source::Terminal(TerminalTarget { sheet: second, terminal });
    assert_ne!(a, b);
    app.select_inspection(id, vec![b], "replace").unwrap();
    assert_eq!(app.views.active.inspection, vec![b]);
    assert_eq!(document(&h), before);
}

/// A Browser land picks the same physical address as the canvas, with no footprint selection.
#[test]
fn browser_pads_share_the_canvas_handler() {
    let mut h = super::board_tracks::routable();
    let id = h.state().views.active.id();
    h.state_mut().browser.open = true;
    h.state_mut().set_selection_filters(id, &json!({"pads":true})).unwrap();
    h.run();
    let board = h.state().session.project().unwrap().boards().next().unwrap();
    let footprint = board.footprints().next().unwrap();
    let source = Source::Pad(PadTarget {
        board: board.target,
        pad: pcbcraft_engine::board::Pad::parse(&footprint.id, &footprint.pads[0].number).unwrap().on_board(board.target.board),
    });
    let label = crate::inspection_selection::labels(&h.state().session, &[source])[0].clone();
    let before = document(&h);
    h.query_all_by_label("Expand Pads").next().unwrap().click();
    h.run();
    click(&mut h, &label);
    assert_eq!(h.state().views.active.inspection, vec![source]);
    assert!(h.state().views.active.board.selection.is_empty());
    assert_eq!(document(&h), before);
}

/// Inspecting a land highlights its filled copper alone, preserving its number and owner's
/// normal selection state. Removing inspection restores the layer ink without a circle badge.
#[test]
fn inspected_pad_highlights_its_copper_instead_of_drawing_a_ring() {
    let mut h = super::board_tracks::routable();
    let id = h.state().views.active.id();
    h.state_mut().set_selection_filters(id, &json!({"pads":true})).unwrap();
    h.run();
    let board = h.state().session.project().unwrap().boards().next().unwrap();
    let footprint = board.footprints().next().unwrap();
    let pad = footprint.pads[0].clone();
    let target = Source::Pad(PadTarget {
        board: board.target,
        pad: pcbcraft_engine::board::Pad::parse(&footprint.id, &pad.number).unwrap().on_board(board.target.board),
    });
    let at = h.state().views.active.camera.screen(pad.center, h.get_by_label("PCB canvas").rect());
    let base = pcbcraft_chrome::Tokens::get().board_front;
    let highlight = pcbcraft_chrome::board::copper_emphasis(base, Some(true));
    let fills = |h: &Harness<'_, PcbApp>, color| {
        h.output().shapes.iter().filter(|shape| matches!(&shape.shape,egui::Shape::Path(path) if path.closed && path.fill==color && egui::Rect::from_points(&path.points).contains(at))).count()
    };
    assert_eq!(fills(&h, highlight), 0);
    assert_eq!(fills(&h, base), 1);
    h.state_mut().select_inspection(id, vec![target], "replace").unwrap();
    h.run();
    assert_eq!(fills(&h, highlight), 1);
    assert_eq!(fills(&h, base), 0);
    assert!(h.state().views.active.board.selection.is_empty());
    assert!(!h.output().shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Circle(circle) if circle.center.distance(at)<0.01 && circle.radius==7. && circle.stroke.color==pcbcraft_chrome::Tokens::get().accent)));
    h.state_mut().select_inspection(id, vec![], "replace").unwrap();
    h.run();
    assert_eq!(fills(&h, base), 1);
}
