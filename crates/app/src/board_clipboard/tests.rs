use super::*;
use crate::tests::{assert_same_content, click, click_canvas_at, harness};
use egui_kittest::{Harness, kittest::Queryable};
use pcbcraft_engine::{ADD_DRAWING, ADD_TRACKS, INITIALIZE_BOARD_SETUP, INITIALIZE_UPDATE_BASELINE, PLACE_FOOTPRINT, PLACE_SYMBOL};
use std::sync::Arc;

/// A command-created native package with accepted logical ownership, opened as a physical editor.
fn board() -> Harness<'static, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    let app = h.state_mut();
    app.run("project.new", &json!({"name":"PCB clipboard UI", "create_board":true}));
    for (command, params) in [
        (PLACE_SYMBOL, json!({"definition":"resistor","x_nm":0,"y_nm":0})),
        (PLACE_FOOTPRINT, json!({"symbol":"symbol-2","definition":"resistor-0402","mapping":{"1":"2","2":"1"},"x_nm":0,"y_nm":0})),
        (INITIALIZE_BOARD_SETUP, json!({"copper_layers":2,"resolve_existing_footprints":true})),
        (INITIALIZE_UPDATE_BASELINE, json!({})),
    ] {
        let mut params = params;
        params["expected_revision"] = json!(app.session.revision());
        app.run(command, &params);
        assert_eq!(app.message, None);
    }
    app.browser.open = false;
    h.run();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    h.state_mut().views.active.board.layers_collapsed = true;
    h.state_mut().views.active.board.browser_collapsed = true;
    h.state_mut().views.active.board.properties_collapsed = true;
    h.state_mut().select_all_board();
    h.run();
    h
}
/// Read the platform output, proving that the portable payload crosses an OS-text boundary.
fn copy(h: &mut Harness<'_, PcbApp>, cut: bool) -> String {
    h.event(if cut { Event::Cut } else { Event::Copy });
    h.step();
    h.output().platform_output.commands.iter().find_map(|c| if let egui::OutputCommand::CopyText(s) = c { Some(s.clone()) } else { None }).unwrap()
}
/// Count only the current addressed board's physical objects.
fn counts(h: &Harness<'_, PcbApp>) -> (usize, usize, usize, usize) {
    let p = h.state().session.project().unwrap();
    let b = p.board_view(h.state().views.active.board.target.unwrap()).unwrap();
    (b.footprints().count(), b.drawings().len(), b.tracks().len(), b.vias().len())
}
/// Place away from the originals through actual canvas input.
fn place(h: &mut Harness<'_, PcbApp>, x: i64, y: i64) {
    let id = h.state().views.active.id();
    let label = if h.state().views.is_split() { format!("PCB canvas {}", id.0 + 1) } else { "PCB canvas".into() };
    let rect = h.get_by_label(&label).rect();
    let at = h.state().views.active.camera.screen(Point { x_nm: x, y_nm: y }, rect);
    click_canvas_at(h, at);
    h.run();
}

/// Independent instances receive self-contained pinned packages and explicit logical counterparts.
#[test]
fn independent_instance_paste_creates_linked_parts_in_one_undo_step() {
    let mut from = board();
    let text = copy(&mut from, false);
    let mut to = board();
    let before = to.state().session.snapshot().unwrap();
    to.event(Event::Paste(text));
    to.run();
    assert!(matches!(to.state().dialog, Dialog::BoardPasteOptions(_)));
    click(&mut to, "Paste");
    assert_eq!(to.state().message, None);
    assert!(to.state().views.active.board.clipboard.active());
    assert!(Arc::ptr_eq(&before, &to.state().session.snapshot().unwrap()));
    place(&mut to, 10_160_000, 10_160_000);
    assert_eq!(to.state().message, None);
    assert_eq!(counts(&to).0, 2);
    assert_eq!(to.state().session.project().unwrap().symbols().len(), 2);
    to.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    to.run();
    assert_same_content(&before, &to.state().session.snapshot().unwrap());
}

/// Same-project Cut moves only the physical placement and reuses its current logical identity.
#[test]
fn cut_relocation_reuses_components_and_ghost_cannot_cut_the_source() {
    let mut h = board();
    let symbols = h.state().session.project().unwrap().symbols().to_vec();
    let text = copy(&mut h, true);
    assert_eq!(counts(&h).0, 0);
    h.event(Event::Paste(text));
    h.run();
    assert!(h.state().views.active.board.clipboard.active());
    let before = h.state().session.snapshot().unwrap();
    h.event(Event::Cut);
    h.key_press_modifiers(Modifiers::COMMAND, Key::D);
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    place(&mut h, 10_160_000, 10_160_000);
    assert_eq!(h.state().message, None);
    assert_eq!(counts(&h).0, 1);
    assert_eq!(h.state().session.project().unwrap().symbols()[0].id(), symbols[0].id());
    assert_eq!(h.state().session.project().unwrap().symbols().len(), 1);
}

/// The native via key event leaves selected footprints untouched; Shift+Delete's Cut event
/// still removes physical placement through the clipboard without deleting logical components.
#[test]
fn via_chord_preserves_selection_while_shift_delete_cut_remains_available() {
    for modifiers in [
        Modifiers { ctrl: true, command: true, shift: true, ..Modifiers::NONE },
        Modifiers { mac_cmd: true, command: true, shift: true, ..Modifiers::NONE },
    ] {
        let mut h = board();
        let before = h.state().session.snapshot().unwrap();
        h.key_press_modifiers(modifiers, Key::X);
        h.run();
        assert!(h.state().views.active.board.tracks.via_tool);
        assert_eq!(counts(&h).0, 1);
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
    let mut h = board();
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    let text = copy(&mut h, true);
    assert_eq!(BoardFragment::from_text(&text).unwrap().counts().0, 1);
    assert_eq!(counts(&h).0, 0);
    assert!(!h.state().views.active.board.tracks.via_tool);
    assert_eq!(h.state().session.project().unwrap().symbols().len(), 1);
}

/// Rotation belongs to the physical group; Escape, stale input and replacement openings discard it.
#[test]
fn board_previews_cancel_without_partial_mutation() {
    for mode in 0..4 {
        let mut h = board();
        // Geometry-only clipboard avoids logical duplication options for this cancellation test.
        let p = h.state().session.project().unwrap();
        let layer = p
            .board_view(p.board_target().unwrap())
            .unwrap()
            .layers()
            .into_iter()
            .find(|l| l.purpose == pcbcraft_engine::board::setup::LayerPurpose::Outline)
            .unwrap()
            .id;
        let revision = h.state().session.revision();
        h.state_mut().run(
            ADD_DRAWING,
            &json!({"layer":layer,"closed":false,"points":[{"x_nm":0,"y_nm":0},{"x_nm":10_160_000,"y_nm":0}],"expected_revision":revision}),
        );
        h.run();
        let project = h.state().session.project().unwrap();
        let drawing = project.board_view(project.board_target().unwrap()).unwrap().drawings()[0].id;
        let fragment = BoardFragment::capture(project, &BoardSelection { drawings: BTreeSet::from([drawing]), ..Default::default() }, false)
            .unwrap()
            .to_text()
            .unwrap();
        h.event(Event::Paste(fragment));
        h.run();
        assert!(h.state().views.active.board.clipboard.active());
        h.key_press(Key::R);
        h.run();
        assert_eq!(h.state().views.active.board.clipboard.paste.as_ref().unwrap().options.items.quarter_turns, 1);
        match mode {
            0 => h.key_press(Key::Escape),
            1 => {
                h.state_mut().session.apply("project.rename", &json!({"name":"Changed"})).unwrap();
            }
            2 => {
                let p = h.state().session.snapshot().unwrap();
                h.state_mut().session.open_snapshot(p);
            }
            _ => h.event(Event::WindowFocused(false)),
        }
        h.run();
        assert!(!h.state().views.active.board.clipboard.active(), "mode {mode}");
        let before = h.state().session.snapshot().unwrap();
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
}

/// Mixed visible geometry can be selected, copied and deleted atomically; hidden layers and
/// filters constrain selection without removing any authored data.
#[test]
fn select_all_mixed_items_respects_visibility_and_delete_is_atomic() {
    let mut h = board();
    let p = h.state().session.project().unwrap();
    let layers = p.board_view(p.board_target().unwrap()).unwrap().layers();
    let outline = layers.iter().find(|l| l.purpose == pcbcraft_engine::board::setup::LayerPurpose::Outline).unwrap().id;
    let top = layers.iter().find(|l| l.purpose == pcbcraft_engine::board::setup::LayerPurpose::Copper).unwrap().id;
    for (command, mut params) in [
        (ADD_DRAWING, json!({"layer":outline,"closed":false,"points":[{"x_nm":0,"y_nm":10_000_000},{"x_nm":10_000_000,"y_nm":10_000_000}]})),
        (ADD_DRAWING, json!({"layer":outline,"closed":false,"points":[{"x_nm":0,"y_nm":12_000_000},{"x_nm":10_000_000,"y_nm":12_000_000}]})),
        (ADD_TRACKS, json!({"layer":top,"width_nm":250_000,"points":[{"x_nm":0,"y_nm":5_000_000},{"x_nm":10_000_000,"y_nm":5_000_000}]})),
    ] {
        params["expected_revision"] = json!(h.state().session.revision());
        h.state_mut().run(command, &params);
        assert_eq!(h.state().message, None);
    }
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    assert_eq!(counts(&h), (1, 2, 1, 0));
    let selected = selection(&h.state().views.active.board, h.state().session.project().unwrap());
    assert_eq!((selected.footprints.len(), selected.drawings.len(), selected.tracks.len()), (1, 2, 1));
    let text = copy(&mut h, false);
    assert_eq!(BoardFragment::from_text(&text).unwrap().counts(), (1, 2, 1, 0));
    let before = h.state().session.snapshot().unwrap();
    h.key_press(Key::Delete);
    h.run();
    assert_eq!(counts(&h), (0, 0, 0, 0));
    assert_eq!(h.state().session.project().unwrap().symbols().len(), 1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_same_content(&before, &h.state().session.snapshot().unwrap());
    h.state_mut().views.active.board.hidden_layers.insert(top.to_string());
    h.state_mut().select_all_board();
    assert!(h.state().views.active.board.tracks.selected.is_empty());
    let id = h.state().views.active.id();
    h.state_mut().set_selection_filters(id, &json!({"graphics":false})).unwrap();
    h.state_mut().select_all_board();
    assert!(h.state().views.active.board.drawing.selection().is_empty());
}

/// PCB context actions remain usable while their popup owns pointer input.
#[test]
fn context_menu_copy_and_duplicate_use_the_system_clipboard_path() {
    let mut h = board();
    let rect = h.get_by_label("PCB canvas").rect();
    let at = rect.center() + egui::vec2(80., 100.);
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run();
    h.get_by_label("Copy").click();
    h.step();
    assert!(h.output().platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(_))));
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::D);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::BoardPasteOptions(_)));
}

/// Options retain captured authority, reject incomplete ownership/layer choices and stay in the
/// initiating native host instead of interpreting the main window as a new destination.
#[test]
fn options_validate_choices_staleness_and_detached_host() {
    let mut h = board();
    let text = copy(&mut h, false);
    let source = h.state().views.active.id();
    let host = h.state_mut().views.open_window(source).unwrap();
    h.state_mut().bind_editor_host(host);
    h.state_mut().receive_board_clipboard(text, true);
    assert_eq!(h.state().board_paste_options_host(), Some(host));
    if let Dialog::BoardPasteOptions(draft) = &mut h.state_mut().dialog {
        draft.targets[0].sheet.clear();
    }
    let before = h.state().session.snapshot().unwrap();
    h.run();
    click(&mut h, "Paste");
    assert!(matches!(&h.state().dialog,Dialog::BoardPasteOptions(d) if d.error.as_ref().is_some_and(|e| e.contains("Choose a sheet"))));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    let sheet = h.state().session.project().unwrap().sheet_target().id().to_string();
    if let Dialog::BoardPasteOptions(draft) = &mut h.state_mut().dialog {
        draft.targets[0].sheet = sheet;
    }
    h.state_mut().session.apply("project.rename", &json!({"name":"Changed while choosing"})).unwrap();
    h.run();
    click(&mut h, "Paste");
    assert!(matches!(&h.state().dialog,Dialog::BoardPasteOptions(d) if d.error.as_ref().is_some_and(|e| e.contains("destination changed"))));
    assert_eq!(counts(&h).0, 1);
}

/// A paste preview in a second PCB pane uses that pane's camera and leaves the source selection
/// and camera intact. Both panes still share one logical/physical history entry.
#[test]
fn split_pcb_paste_keeps_source_view_presentation() {
    let mut h = board();
    let text = copy(&mut h, false);
    let source = h.state().views.active.id();
    let selected = h.state().views.active.board.selection.clone();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100., 100.));
    let origin = Point { x_nm: 0, y_nm: 0 };
    let camera = h.state().views.active.camera.screen(origin, rect);
    let dest = h.state_mut().views.open_beside(source).unwrap();
    let ctx = h.ctx.clone();
    h.state_mut().activate_editor(&ctx, dest);
    h.run();
    h.event(Event::Paste(text));
    h.run();
    click(&mut h, "Paste");
    assert!(h.state().views.view(dest).unwrap().board.clipboard.active());
    place(&mut h, 10_160_000, 10_160_000);
    assert_eq!(h.state().message, None);
    assert_eq!(counts(&h).0, 2);
    assert_eq!(h.state().views.view(source).unwrap().board.selection, selected);
    assert_eq!(h.state().views.view(source).unwrap().camera.screen(origin, rect), camera);
}

/// A directional marquee can capture real copper and via geometry along with multiple graphics.
#[test]
fn mixed_marquee_copies_and_rotates_a_routed_group() {
    let mut h = board();
    let b = h.state().session.project().unwrap().boards().next().unwrap();
    let copper: Vec<_> = b.layers().into_iter().filter(|l| l.purpose == pcbcraft_engine::board::setup::LayerPurpose::Copper).map(|l| l.id).collect();
    let revision = h.state().session.revision();
    h.state_mut().run(ADD_TRACKS,&json!({"layer":copper[0],"width_nm":250_000,"points":[{"x_nm":0,"y_nm":5_080_000},{"x_nm":5_080_000,"y_nm":5_080_000},{"x_nm":10_160_000,"y_nm":5_080_000}],"layer_changes":[{"at":1,"layer":copper[1]}],"via_diameter_nm":600_000,"via_drill_nm":300_000,"expected_revision":revision}));
    assert_eq!(h.state().message, None);
    h.run();
    h.state_mut().select_board_items(Default::default());
    let rect = h.get_by_label("PCB canvas").rect();
    let from = h.state().views.active.camera.screen(Point { x_nm: -2_540_000, y_nm: -2_540_000 }, rect);
    let to = h.state().views.active.camera.screen(Point { x_nm: 12_700_000, y_nm: 7_620_000 }, rect);
    for event in [
        Event::PointerMoved(from),
        Event::PointerButton { pos: from, button: egui::PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE },
        Event::PointerMoved(to),
        Event::PointerButton { pos: to, button: egui::PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE },
    ] {
        h.event(event);
        h.run();
    }
    let selected = selection(&h.state().views.active.board, h.state().session.project().unwrap());
    assert_eq!((selected.footprints.len(), selected.tracks.len(), selected.vias.len()), (1, 2, 1));
    let text = copy(&mut h, false);
    assert_eq!(BoardFragment::from_text(&text).unwrap().counts(), (1, 0, 2, 1));
    h.event(Event::Paste(text));
    h.run();
    click(&mut h, "Paste");
    h.key_press(Key::R);
    h.run();
    place(&mut h, 20_320_000, 20_320_000);
    assert_eq!(h.state().message, None);
    assert_eq!(counts(&h), (2, 0, 4, 2));
}

/// Choosing a new authored tool cannot leave a clipboard ghost with competing click authority.
#[test]
fn clipboard_preview_yields_to_a_new_authored_tool() {
    let mut h = board();
    h.state_mut().duplicate_board_clipboard();
    h.run();
    click(&mut h, "Paste");
    let before = h.state().session.snapshot().unwrap();
    let project = h.state().session.project().unwrap();
    let layer = project
        .boards()
        .next()
        .unwrap()
        .layers()
        .into_iter()
        .find(|l| l.purpose == pcbcraft_engine::board::setup::LayerPurpose::Outline)
        .unwrap()
        .id
        .to_string();
    h.state_mut().views.active.board.active_layer = Some(layer.clone());
    h.state_mut().views.active.board.drawing.start(crate::board_drawing::Tool::Lines, Some(layer));
    h.run();
    assert!(!h.state().views.active.board.clipboard.active());
    assert_eq!(h.state().views.active.board.drawing.tool, Some(crate::board_drawing::Tool::Lines));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}
