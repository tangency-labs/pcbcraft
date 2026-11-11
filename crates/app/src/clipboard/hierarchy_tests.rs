use super::*;
use crate::editor_view::DocumentTarget;
use crate::tests::{click, click_canvas_at, harness};
use egui_kittest::{Harness, kittest::Queryable};
use pcbcraft_engine::{hierarchy as model, schematic::GRID_NM};
use std::sync::Arc;

/// A command-created root/child/grandchild with one live child part and an interface.
fn tree() -> Harness<'static, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    let app = h.state_mut();
    app.run("project.new", &json!({"name":"Tree clipboard"}));
    let root = app.session.project().unwrap().sheet_target();
    app.run(model::CREATE_CHILD, &json!({"sheet":root.id(),"name":"Child","x_nm":0,"y_nm":0,"expected_revision":app.session.revision()}));
    let child = app.session.project().unwrap().sheets().find(|s| s.name == "Child").unwrap().target;
    for (id, mut params) in [
        (model::CREATE_CHILD, json!({"sheet":child.id(),"name":"Grandchild","x_nm":20*GRID_NM,"y_nm":0})),
        (model::ADD_LABEL, json!({"sheet":child.id(),"name":"SIGNAL","direction":"input","x_nm":0,"y_nm":0})),
        (pcbcraft_engine::PLACE_SYMBOL, json!({"sheet":child.id(),"definition":"resistor","x_nm":10*GRID_NM,"y_nm":0})),
    ] {
        params["expected_revision"] = json!(app.session.revision());
        app.run(id, &params);
        assert_eq!(app.message, None);
    }
    app.browser.open = false;
    h.run();
    let rect = h.get_by_label("Schematic canvas").rect();
    let at = h.state().views.active.camera.screen(Point { x_nm: 8 * GRID_NM, y_nm: 5 * GRID_NM }, rect);
    click_canvas_at(&mut h, at);
    h.run();
    assert!(h.state().views.active.schematic.selected_sheet_symbol.is_some());
    h
}
/// Inspect only the platform text output, retaining no source-session memory in the receiver.
fn copy(h: &mut Harness<'_, PcbApp>, cut: bool) -> String {
    h.event(if cut { Event::Cut } else { Event::Copy });
    h.step();
    h.output().platform_output.commands.iter().find_map(|c| if let egui::OutputCommand::CopyText(s) = c { Some(s.clone()) } else { None }).unwrap()
}
/// Place the root body with real canvas input while child documents keep their own coordinates.
fn place(h: &mut Harness<'_, PcbApp>) {
    let rect = h.get_by_label("Schematic canvas").rect();
    let at = h.state().views.active.camera.screen(Point { x_nm: 30 * GRID_NM, y_nm: 20 * GRID_NM }, rect);
    click_canvas_at(h, at);
    h.run();
}

/// A selected sheet body copies the full child tree; receiver placement creates independent
/// documents and component identities and remains one shared undo step.
#[test]
fn sheet_tree_copies_between_instances_and_previews_keep_stable_child_ids() {
    let mut from = tree();
    let text = copy(&mut from, false);
    assert_eq!(Fragment::from_text(&text).unwrap().hierarchy_counts(), (1, 0, 2));
    let mut to = harness();
    to.set_size(egui::vec2(1400., 900.));
    to.state_mut().run("project.new", &json!({"name":"Receiver"}));
    to.state_mut().browser.open = false;
    to.run();
    let before = to.state().session.snapshot().unwrap();
    to.event(Event::Paste(text));
    to.run();
    assert!(to.state().views.active.schematic.clipboard.active());
    let preview = to.state().views.active.schematic.clipboard.paste.as_ref().unwrap().preview.as_ref().unwrap();
    let a = preview.at(Point { x_nm: 0, y_nm: 0 }, 0).unwrap().sheets().map(|s| s.target).collect::<Vec<_>>();
    let b = preview.at(Point { x_nm: 10 * GRID_NM, y_nm: 10 * GRID_NM }, 0).unwrap().sheets().map(|s| s.target).collect::<Vec<_>>();
    assert_eq!(a, b);
    to.key_press(Key::R);
    to.run();
    assert!(to.state().views.active.schematic.clipboard.active());
    assert!(to.state().message.as_ref().is_some_and(|s| s.contains("orientation")));
    place(&mut to);
    assert_eq!(to.state().message, None);
    assert_eq!(to.state().session.project().unwrap().sheets().len(), 3);
    assert_eq!(to.state().session.project().unwrap().components().count(), 1);
    assert!(to.state().views.active.schematic.selected_sheet_symbol.is_some());
    to.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    to.run();
    crate::tests::assert_same_content(&before, &to.state().session.snapshot().unwrap());
}

/// Explicit same-project relocation retains child identity/content. Cancelling a ghost leaves
/// only the earlier Cut in history, and a later paste requires a fresh relocation choice.
#[test]
fn cut_paste_relocates_original_child_sheets_and_escape_is_read_only() {
    let mut h = tree();
    let child = h.state().session.project().unwrap().sheets().find(|s| s.name == "Child").unwrap().target;
    let text = copy(&mut h, true);
    h.run();
    let cut = h.state().session.snapshot().unwrap();
    assert_eq!(cut.parent_sheet(child), None);
    h.event(Event::Paste(text.clone()));
    h.run();
    assert!(matches!(&h.state().dialog, Dialog::PasteOptions(draft) if !draft.placement.options.reuse_sheets));
    click(&mut h, "Reuse original unparented sheets");
    click(&mut h, "Paste");
    assert!(h.state().views.active.schematic.clipboard.paste.as_ref().unwrap().options.reuse_sheets);
    h.key_press(Key::Escape);
    h.run();
    assert!(Arc::ptr_eq(&cut, &h.state().session.snapshot().unwrap()));
    h.event(Event::Paste(text));
    h.run();
    assert!(matches!(&h.state().dialog, Dialog::PasteOptions(draft) if !draft.placement.options.reuse_sheets));
    click(&mut h, "Reuse original unparented sheets");
    click(&mut h, "Paste");
    place(&mut h);
    assert_eq!(h.state().message, None);
    let p = h.state().session.project().unwrap();
    assert_eq!(p.sheets().len(), 3);
    assert_eq!(p.components().count(), 1);
    assert_eq!(p.parent_sheet(child), Some(p.sheet_target()));
}

/// An interface's context Copy and keyboard Duplicate use explicit target identity. Name
/// conflicts remain in the form until a suffix gives the new port a distinct name.
#[test]
fn interface_context_copy_and_hover_duplicate_use_native_options() {
    let mut h = tree();
    let child = h.state().session.project().unwrap().sheets().find(|s| s.name == "Child").unwrap().target;
    let app = h.state_mut();
    let view = app.views.active.id();
    app.views.open_document(view, DocumentTarget::Sheet(child), &app.session);
    h.run();
    let rect = h.get_by_label("Schematic canvas").rect();
    let at = h.state().views.active.camera.screen(Point { x_nm: 0, y_nm: 0 }, rect);
    h.hover_at(at);
    h.run();
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run();
    h.get_by_label("Copy").click();
    h.step();
    let text = h
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|c| if let egui::OutputCommand::CopyText(s) = c { Some(s.clone()) } else { None })
        .unwrap();
    assert_eq!(Fragment::from_text(&text).unwrap().hierarchy_counts(), (0, 1, 0));
    h.run();
    h.hover_at(at);
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::D);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::PasteOptions(_)));
    click(&mut h, "Paste");
    assert!(matches!(&h.state().dialog,Dialog::PasteOptions(d) if d.error.is_some()));
    h.get_by_label("Net-name suffix").click();
    h.run();
    h.get_by_label("Net-name suffix").type_text("_COPY");
    h.run();
    click(&mut h, "Paste");
    assert!(
        h.state().views.active.schematic.clipboard.active(),
        "error={:?}, message={:?}",
        if let Dialog::PasteOptions(d) = &h.state().dialog { d.error.as_deref() } else { None },
        h.state().message
    );
    place(&mut h);
    assert_eq!(h.state().message, None);
    let p = h.state().views.active.snapshot(&h.state().session).unwrap();
    assert_eq!(p.hierarchical_labels().len(), 2);
    assert!(p.hierarchical_labels().iter().any(|l| l.name == "SIGNAL_COPY"));
}

/// Clipboard navigation changes only the receiving camera; tool shortcuts cannot edit the
/// retained source selection while a read-only preview owns input.
#[test]
fn schematic_preview_navigation_has_no_authoring_authority() {
    let mut h = tree();
    let text = copy(&mut h, false);
    h.event(Event::Paste(text));
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let rect = h.get_by_label("Schematic canvas").rect();
    h.hover_at(rect.center());
    h.run();
    let zoom = h.state().views.active.camera.zoom();
    h.event(Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0., 2.),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    h.run();
    assert_ne!(h.state().views.active.camera.zoom(), zoom);
    h.key_press(Key::Delete);
    h.run();
    assert!(h.state().views.active.schematic.clipboard.active());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// A current floating card owns its pointer even when the preceding canvas frame left a model
/// point cached. Only an explicit hierarchy selection may be copied from beneath that card.
#[test]
fn floating_panel_blocks_stale_hierarchy_hover_clipboard_targets() {
    use crate::workspace_panels::FILTERS;
    use pcbcraft_chrome::workspace::panels::Position;
    for action in 0..4 {
        let mut h = tree();
        let rect = h.get_by_label("Schematic canvas").rect();
        let body = h.state().views.active.camera.screen(Point { x_nm: 8 * GRID_NM, y_nm: 5 * GRID_NM }, rect);
        h.state_mut().views.active.filters_open = true;
        h.state_mut().views.active.panels[FILTERS].set_position(Position::Floating, Some([body.x - rect.left() - 130., body.y - rect.top() - 12.]));
        h.run();
        let over_panel = h.get_by_label("Move Selection filters").rect().center();
        let project = h.state().views.active.snapshot(&h.state().session).unwrap();
        assert!(crate::hierarchy::hovered(&project, &h.state().views.active.camera, over_panel, h.state().views.active.filters));
        assert!(h.ctx.layer_id_at(over_panel).is_some_and(|layer| layer.order != egui::Order::Background));
        if action < 3 {
            h.state_mut().views.active.schematic.selected_sheet_symbol = None;
        }
        h.hover_at(over_panel);
        h.run();
        // Exercise the shortcut handler with the preceding canvas frame's cached point.
        h.state_mut().pointer = Some(Point { x_nm: 8 * GRID_NM, y_nm: 5 * GRID_NM });
        let before = h.state().session.snapshot().unwrap();
        let ctx = h.ctx.clone();
        match action {
            0 | 3 => h.state_mut().copy_schematic_clipboard(&ctx, false),
            1 => h.state_mut().copy_schematic_clipboard(&ctx, true),
            _ => h.state_mut().duplicate_schematic_clipboard_from(Some(&ctx)),
        }
        let copied = ctx.output(|output| output.commands.iter().any(|command| matches!(command, egui::OutputCommand::CopyText(_))));
        assert_eq!(copied, action == 3, "action={action}");
        assert!(!h.state().views.active.schematic.clipboard.active(), "action={action}");
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()), "action={action}");
    }
}

/// Externally editable Cut metadata may offer relocation, but cannot opt the user into changing
/// ownership of live sheets. The default paste creates independent children only after placement.
#[test]
fn cut_metadata_requires_an_explicit_relocation_choice() {
    let mut h = tree();
    let child = h.state().session.project().unwrap().sheets().find(|sheet| sheet.name == "Child").unwrap().target;
    let text = copy(&mut h, true);
    h.run();
    let before = h.state().session.snapshot().unwrap();
    h.event(Event::Paste(text));
    h.run();
    assert!(matches!(&h.state().dialog, Dialog::PasteOptions(draft) if !draft.placement.options.reuse_sheets));
    assert!(!h.state().views.active.schematic.clipboard.active());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    click(&mut h, "Paste");
    assert!(!h.state().views.active.schematic.clipboard.paste.as_ref().unwrap().options.reuse_sheets);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    place(&mut h);
    assert_eq!(h.state().message, None);
    let project = h.state().session.project().unwrap();
    assert_eq!(project.sheets().len(), 5);
    assert_eq!(project.components().count(), 2);
    assert_eq!(project.parent_sheet(child), None);
}
