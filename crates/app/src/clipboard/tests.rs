use super::*;
use crate::tests::{click, harness};
use egui_kittest::kittest::Queryable;
use pcbcraft_engine::{PLACE_SYMBOL, schematic::GRID_NM, selection::Selection};
use std::sync::Arc;

/// Copy output is an OS clipboard request, not an application-local memory buffer.
fn copied_text(h: &mut egui_kittest::Harness<'_, PcbApp>) -> String {
    h.event(Event::Copy);
    h.step();
    h.output()
        .platform_output
        .commands
        .iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .unwrap()
}

/// Create one selected native part through the same command as the canvas tool.
fn source() -> egui_kittest::Harness<'static, PcbApp> {
    let mut h = harness();
    h.state_mut().session.apply("project.new", &json!({"name":"Clipboard UI"})).unwrap();
    let revision = h.state().session.revision();
    h.state_mut().run(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":revision}));
    let id = h.state().session.project().unwrap().symbols()[0].id();
    h.state_mut().select(Selection::one_symbol(id));
    h.run();
    h
}

/// Portable OS text from one app instance starts a cancellable preview in another; clicking the
/// actual canvas creates fresh objects and one undo step, with no dependency on source memory.
#[test]
fn copy_between_independent_instances_uses_system_clipboard_events() {
    let mut from = source();
    let text = copied_text(&mut from);
    let mut to = source();
    let before = to.state().session.snapshot().unwrap();
    to.event(Event::Paste(text));
    to.run();
    assert!(to.state().views.active.schematic.clipboard.active());
    assert!(Arc::ptr_eq(&before, &to.state().session.snapshot().unwrap()));
    let rect = to.get_by_label("Schematic canvas").rect();
    let at = to.state().views.active.camera.screen(Point { x_nm: 12 * GRID_NM, y_nm: 8 * GRID_NM }, rect);
    crate::tests::click_canvas_at(&mut to, at);
    to.run();
    assert_eq!(to.state().message, None);
    assert_eq!(to.state().session.project().unwrap().symbols().len(), 2);
    assert!(!to.state().views.active.schematic.clipboard.active());
    click(&mut to, "Undo");
    crate::tests::assert_same_content(&before, &to.state().session.snapshot().unwrap());
}

/// Escape, a competing command, replacement opening and native focus loss discard previews.
#[test]
fn clipboard_previews_cancel_without_history() {
    let mut from = source();
    let text = copied_text(&mut from);
    for mode in 0..4 {
        let mut h = source();
        h.event(Event::Paste(text.clone()));
        h.run();
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
        let before = h.state().session.snapshot().unwrap();
        let rect = h.get_by_label("Schematic canvas").rect();
        crate::tests::click_canvas_at(&mut h, rect.center());
        h.run();
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()), "mode {mode}");
        assert!(!h.state().views.active.schematic.clipboard.active(), "mode {mode}");
    }
}

/// Paste Options retains its captured revision; conflicting references leave the complete design
/// unchanged. Duplicate creates a preview without overwriting the person's OS clipboard.
#[test]
fn paste_options_and_duplicate_share_the_validated_path() {
    let mut h = source();
    let text = copied_text(&mut h);
    h.run();
    h.state_mut().receive_schematic_clipboard(text, true);
    h.run();
    h.get_by_label("Keep original references").click();
    h.get_by_label("Paste at original position").click();
    h.run();
    let before = h.state().session.snapshot().unwrap();
    click(&mut h, "Paste");
    assert!(matches!(h.state().dialog, Dialog::PasteOptions(_)));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    click(&mut h, "Cancel");
    h.state_mut().duplicate_schematic_clipboard();
    h.run();
    assert!(h.state().views.active.schematic.clipboard.active());
}

/// Text entry owns its Copy/Paste events; malformed design text fails without creating a tool.
#[test]
fn invalid_clipboard_text_is_rejected_without_a_partial_edit() {
    let mut h = source();
    let before = h.state().session.snapshot().unwrap();
    h.event(Event::Paste("{invalid native clipboard JSON}".into()));
    h.run();
    assert!(h.state().message.as_ref().is_some_and(|s| s.contains("supported PCBcraft")));
    assert!(!h.state().views.active.schematic.clipboard.active());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// A split-pane paste is owned by its destination view; the other camera and selection survive.
#[test]
fn paste_between_split_views_uses_the_destination_camera() {
    let mut h = source();
    h.set_size(egui::vec2(1400., 900.));
    h.state_mut().browser.open = false;
    h.run();
    let text = copied_text(&mut h);
    h.run();
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    let path = h.state().views.active.sheet_path(&h.state().session).unwrap();
    h.get_all_by_label(&path).nth(1).unwrap().click();
    h.run();
    let source_id = crate::ViewId(0);
    let camera_before = h
        .state()
        .views
        .view(source_id)
        .unwrap()
        .camera
        .screen(Point { x_nm: 0, y_nm: 0 }, egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100., 100.)));
    let selected = h.state().views.view(source_id).unwrap().schematic.selection.clone();
    h.event(Event::Paste(text));
    h.run();
    let id = h.state().views.active.id();
    let rect = h.get_by_label(&format!("Schematic canvas {}", id.0 + 1)).rect();
    let at = h.state().views.active.camera.screen(Point { x_nm: 8 * GRID_NM, y_nm: 8 * GRID_NM }, rect);
    crate::tests::click_canvas_at(&mut h, at);
    assert_eq!(h.state().session.project().unwrap().symbols().len(), 2);
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().views.view(source_id).unwrap().schematic.selection, selected);
    assert_eq!(
        h.state()
            .views
            .view(source_id)
            .unwrap()
            .camera
            .screen(Point { x_nm: 0, y_nm: 0 }, egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100., 100.))),
        camera_before
    );
}

/// A delayed platform response may never move a captured menu request into a newer revision.
#[test]
fn late_clipboard_reads_do_not_retarget_after_a_competing_edit() {
    let mut h = source();
    let text = copied_text(&mut h);
    h.run();
    let ctx = h.ctx.clone();
    h.state_mut().request_clipboard(&ctx, false);
    h.state_mut().session.apply("project.rename", &json!({"name":"New revision"})).unwrap();
    h.run();
    let before = h.state().session.snapshot().unwrap();
    h.event(Event::Paste(text));
    h.run();
    assert!(!h.state().views.active.schematic.clipboard.active());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// A visible paste ghost does not grant Cut authority over the retained source selection.
#[test]
fn cut_and_duplicate_ignore_source_selection_during_preview() {
    let mut h = source();
    let text = copied_text(&mut h);
    h.event(Event::Paste(text));
    h.run();
    let before = h.state().session.snapshot().unwrap();
    h.event(Event::Cut);
    h.run();
    h.state_mut().duplicate_schematic_clipboard();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    assert!(h.state().views.active.schematic.clipboard.active());
}

/// Native menu reads belong to the initiating canvas, even if a text dialog opens before delivery.
#[test]
fn pending_canvas_read_cannot_paste_native_json_into_a_dialog() {
    let mut h = source();
    let text = copied_text(&mut h);
    h.run();
    let ctx = h.ctx.clone();
    h.state_mut().request_clipboard(&ctx, false);
    h.state_mut().dialog = Dialog::Name { command: "project.rename", text: "Keep this draft".into(), error: None, focused: false };
    h.run();
    let before = h.state().session.snapshot().unwrap();
    h.event(Event::Paste(text));
    h.run();
    assert!(matches!(&h.state().dialog, Dialog::Name { text, .. } if text == "Keep this draft"));
    assert!(h.state().clipboard_request.is_none());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    assert!(!h.state().views.active.schematic.clipboard.active());
}

/// An empty platform result retires its options request, without marking a later direct paste.
#[test]
fn empty_clipboard_completion_clears_pending_options() {
    let mut h = source();
    let text = copied_text(&mut h);
    h.run();
    let ctx = h.ctx.clone();
    h.state_mut().request_clipboard(&ctx, true);
    h.event(Event::Paste(String::new()));
    h.run();
    assert!(h.state().clipboard_request.is_none());
    assert!(matches!(h.state().dialog, Dialog::None));
    h.event(Event::Paste(text));
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    assert!(h.state().views.active.schematic.clipboard.active());
}

/// An egui 0.37 image-only result completes a captured text read without editing the project
/// or applying its Paste Options authority to the next ordinary clipboard event.
#[test]
fn image_only_clipboard_completion_clears_pending_options() {
    let mut h = source();
    let text = copied_text(&mut h);
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let ctx = h.ctx.clone();
    h.state_mut().request_clipboard(&ctx, true);
    h.event(Event::PasteImage(Arc::new(egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]))));
    h.run();
    assert!(h.state().clipboard_request.is_none());
    assert!(!h.state().views.active.schematic.clipboard.active());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    h.event(Event::Paste(text));
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    assert!(h.state().views.active.schematic.clipboard.active());
}

/// A detached initiating pane keeps its own Paste Options form and placement authority.
#[test]
fn paste_options_are_hosted_in_the_initiating_window() {
    let mut h = source();
    let text = copied_text(&mut h);
    let source = h.state().views.active.id();
    let host = h.state_mut().views.open_window(source).unwrap();
    h.state_mut().bind_editor_host(host);
    h.state_mut().receive_schematic_clipboard(text, true);
    assert_eq!(h.state().paste_options_host(), Some(host));
    let origin = h.state().views.active.id();
    let main = h.state().views.main;
    h.state_mut().bind_editor_host(main);
    assert_ne!(h.state().views.active.id(), origin);
    h.state_mut().bind_editor_host(host);
    h.run();
    click(&mut h, "Paste");
    assert!(matches!(h.state().dialog, Dialog::None));
    assert!(h.state().views.view(origin).unwrap().schematic.clipboard.active());
}
