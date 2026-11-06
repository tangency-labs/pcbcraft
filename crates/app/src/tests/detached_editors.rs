//! Exercise the real immediate-viewport callback with independent native-style input streams.
//! OS window creation/focus is verified separately; these tests cover shared application state.

use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use egui::{Event, PointerButton, Pos2, Rect, ViewportCommand, ViewportId, pos2, vec2};
use pcbcraft_engine::{persistence::encode, schematic::Point};

use super::*;

#[derive(Default)]
struct Peer {
    focused: bool,
    close: bool,
    events: Vec<Event>,
    root: Option<egui::accesskit::NodeId>,
    nodes: BTreeMap<egui::accesskit::NodeId, egui::accesskit::Node>,
    output: egui::FullOutput,
    frames: usize,
    decorations: Option<bool>,
}

impl Peer {
    /// Apply incremental changes, retaining unchanged descendants but dropping removed controls.
    fn update_tree(&mut self, update: &egui::accesskit::TreeUpdate) {
        if let Some(tree) = &update.tree {
            self.root = Some(tree.root);
        }
        self.nodes.extend(update.nodes.iter().cloned());
        let mut pending: Vec<_> = self.root.into_iter().collect();
        let mut live = std::collections::BTreeSet::new();
        while let Some(id) = pending.pop() {
            if live.insert(id)
                && let Some(node) = self.nodes.get(&id)
            {
                pending.extend(node.children());
            }
        }
        self.nodes.retain(|id, _| live.contains(id));
    }
}

/// Use egui's native viewport callback with separate input and output, not an embedded window.
/// Only the platform rendering backend is replaced; both hosts borrow the actual PcbApp.
fn multi_window() -> (Harness<'static, PcbApp>, Rc<RefCell<Peer>>) {
    let mut h = harness();
    h.set_size(vec2(1400.0, 900.0));
    resistor_at_origin(&mut h);
    h.state_mut().browser.open = false;
    let peer = Rc::new(RefCell::new(Peer::default()));
    let state = peer.clone();
    egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
        let mut input = egui::RawInput {
            viewport_id: viewport.ids.this,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 760.0))),
            time: Some(ctx.input(|i| i.time)),
            ..Default::default()
        };
        {
            let mut peer = state.borrow_mut();
            peer.decorations = viewport.builder.decorations;
            input.focused = peer.focused;
            input.events = std::mem::take(&mut peer.events);
            let mut info = egui::ViewportInfo { parent: Some(ViewportId::ROOT), focused: Some(peer.focused), ..Default::default() };
            if std::mem::take(&mut peer.close) {
                info.events.push(egui::ViewportEvent::Close);
            }
            input.viewports.insert(viewport.ids.this, info);
        }
        let mut output = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
        // This backend inspects shapes and accessibility only; native rendering is checked separately.
        output.textures_delta.clear();
        let mut peer = state.borrow_mut();
        if let Some(update) = &output.platform_output.accesskit_update {
            peer.update_tree(update);
        }
        peer.output = output;
        peer.frames += 1;
    });
    h.ctx.set_embed_viewports(false);
    h.run();
    (h, peer)
}

/// Find a child control from its actual accessible output, not an assumed pixel location.
fn child_rect(peer: &Rc<RefCell<Peer>>, label: &str) -> Rect {
    peer.borrow()
        .nodes
        .values()
        .find_map(|node| {
            if node.label() != Some(label) {
                return None;
            }
            let bounds = node.bounds()?;
            Some(Rect::from_min_max(pos2(bounds.x0 as f32, bounds.y0 as f32), pos2(bounds.x1 as f32, bounds.y1 as f32)))
        })
        .unwrap_or_else(|| panic!("missing child control: {label}"))
}

/// Deliver focus to one host only, keeping its input stream independent of repaint order.
fn focus(h: &mut Harness<'_, PcbApp>, peer: &Rc<RefCell<Peer>>, child: bool) {
    h.input_mut().focused = !child;
    peer.borrow_mut().focused = child;
    h.run();
}

/// Press and release inside the child, with an intervening frame for each actual event.
fn child_click_at(h: &mut Harness<'_, PcbApp>, peer: &Rc<RefCell<Peer>>, pos: Pos2) {
    focus(h, peer, true);
    peer.borrow_mut().events.push(Event::PointerMoved(pos));
    h.run();
    for pressed in [true, false] {
        peer.borrow_mut().events.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
}

/// Send a keyboard press to the child. Root input contains no copy of these events.
fn child_key(h: &mut Harness<'_, PcbApp>, peer: &Rc<RefCell<Peer>>, key: Key, modifiers: Modifiers) {
    focus(h, peer, true);
    for pressed in [true, false] {
        peer.borrow_mut().events.push(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
        h.run();
    }
}

/// Open the current editor through its real ribbon action.
fn detach(h: &mut Harness<'_, PcbApp>) {
    click(h, "View");
    click(h, "Detach active editor view");
    assert!(h.state().views.detached.is_some());
}

/// A direct detach inherits the primary window's chrome policy. Integrated controls address
/// only the child viewport, whose close event dismisses the view without changing the root layout.
#[test]
fn detached_window_uses_the_primary_chrome_policy_and_child_window_controls() {
    for integrated in [false, true] {
        let (mut h, peer) = multi_window();
        h.state_mut().integrated_titlebar = integrated;
        assert!(!h.state().views.split, "detaching does not require splitting first");
        let before = h.state().session.snapshot().unwrap();
        detach(&mut h);
        assert_eq!(peer.borrow().decorations, Some(!integrated));
        assert_eq!(peer.borrow().nodes.values().any(|node| node.label() == Some("Close window")), integrated);
        if integrated {
            for (label, command) in [
                ("Minimize window", ViewportCommand::Minimized(true)),
                ("Maximize window", ViewportCommand::Maximized(true)),
                ("Close window", ViewportCommand::Close),
            ] {
                let button = child_rect(&peer, label).center();
                focus(&mut h, &peer, true);
                peer.borrow_mut().events.push(Event::PointerMoved(button));
                h.run();
                for pressed in [true, false] {
                    peer.borrow_mut().events.push(Event::PointerButton {
                        pos: button,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    });
                    h.step();
                }
                assert!(h.output().viewport_output[&editor_windows::viewport()].commands.contains(&command), "{label} targets the child window");
                assert!(!h.output().viewport_output[&ViewportId::ROOT].commands.contains(&command));
                assert!(!h.state().project_action_pending());
            }
            // The OS acknowledges the child Close command as a close request on the next frame.
            peer.borrow_mut().close = true;
            h.run();
            assert!(h.state().views.detached.is_none());
            assert!(!h.state().views.split);
            assert!(matches!(h.state().dialog, Dialog::None));
        }
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        assert_eq!(h.state().integrated_titlebar, integrated);
    }
}

/// Either split view can detach, then close or explicitly dock from either host. Close keeps
/// the root alone; Dock restores the split. All transitions preserve navigation and project
/// bytes while invalidating callbacks captured before the transition. Child Close never prompts to save.
#[test]
fn detach_close_and_dock_preserve_views_without_copying_the_project() {
    #[derive(Debug, PartialEq)]
    enum Finish {
        Close,
        DockRoot,
        DockChild,
    }
    for id in [ViewId(0), ViewId(1)] {
        for action in [Finish::Close, Finish::DockRoot, Finish::DockChild] {
            let (mut h, peer) = multi_window();
            let ctx = h.ctx.clone();
            h.state_mut().split_editors(&ctx, true);
            h.state_mut().views.bind(id);
            h.state_mut().views.active.camera.center_on(Point { x_nm: 5_080_000, y_nm: 2_540_000 });
            h.state_mut().set_view_grid(id, 500_000);
            h.state_mut().select(Selection::one_symbol("symbol-2".to_owned()));
            h.run();
            let app = h.state();
            let before = app.session.snapshot().unwrap();
            let revision = app.session.revision();
            let bytes = encode(before.clone()).unwrap();
            let origin = Point { x_nm: 0, y_nm: 0 };
            let positions = [ViewId(0), ViewId(1)].map(|id| app.views.view(id).unwrap().camera.screen(origin, Rect::ZERO));
            let old = app.views.active.input(&app.session).unwrap();
            detach(&mut h);
            assert_eq!(h.state().views.detached, Some(id));
            assert_ne!(h.state().views.active.id(), id, "the root owns the remaining view");
            assert!(peer.borrow().frames > 0);
            assert!(!h.state().views.view(id).unwrap().accepts(old, &h.state().session));
            assert_eq!(h.state().views.view(id).unwrap().selection.single_symbol(), Some("symbol-2"));
            assert_eq!(h.state().views.view(id).unwrap().camera.snap_nm(), 500_000);
            let root_id = h.state().views.active.id();
            let child_input = h.state().views.view(id).unwrap().input(&h.state().session).unwrap();
            match action {
                Finish::Close => {
                    peer.borrow_mut().close = true;
                    h.run();
                }
                Finish::DockRoot => click(&mut h, "Dock editor view"),
                Finish::DockChild => {
                    let view_tab = child_rect(&peer, "View").center();
                    child_click_at(&mut h, &peer, view_tab);
                    let dock = child_rect(&peer, "Dock editor view").center();
                    child_click_at(&mut h, &peer, dock);
                }
            }
            assert!(h.state().views.detached.is_none());
            assert_eq!(h.state().views.split, action != Finish::Close, "{action:?}");
            assert_eq!(h.state().views.active.id(), if action == Finish::Close { root_id } else { id });
            assert!(!h.state().views.view(id).unwrap().accepts(child_input, &h.state().session));
            assert_eq!(h.state().views.view(id).unwrap().selection.single_symbol(), Some("symbol-2"));
            assert_eq!(h.state().views.view(id).unwrap().camera.snap_nm(), 500_000);
            assert!(matches!(h.state().dialog, Dialog::None));
            for (index, id) in [ViewId(0), ViewId(1)].into_iter().enumerate() {
                assert_eq!(h.state().views.view(id).unwrap().camera.screen(origin, Rect::ZERO), positions[index]);
            }
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
            assert_eq!(h.state().session.revision(), revision);
            assert_eq!(encode(h.state().session.snapshot().unwrap()).unwrap(), bytes);
        }
    }
}

/// Child input edits the shared session exactly once; root Delete cannot act on a highlight,
/// while undo/redo from either window restore the same immutable project snapshots.
#[test]
fn detached_editing_highlights_and_history_share_one_session() {
    let (mut h, peer) = multi_window();
    detach(&mut h);
    let id = h.state().views.detached.unwrap();
    let root = h.state().views.active.id();
    let origin = h.state().views.view(id).unwrap().camera.screen(Point { x_nm: 0, y_nm: 0 }, child_rect(&peer, "Schematic canvas"));
    child_click_at(&mut h, &peer, origin);
    let before = h.state().session.snapshot().unwrap();
    let revision = h.state().session.revision();
    child_key(&mut h, &peer, Key::R, Modifiers::NONE);
    let after = h.state().session.snapshot().unwrap();
    assert_eq!(h.state().session.revision(), revision + 1);
    assert_eq!(after.symbols()[0].turns(), (before.symbols()[0].turns() + 1) % 4);
    assert_eq!(h.state().views.highlight_for(root, &h.state().session).single_symbol(), Some("symbol-2"));
    focus(&mut h, &peer, false);
    h.key_press(Key::Delete);
    h.run();
    assert!(Arc::ptr_eq(&after, &h.state().session.snapshot().unwrap()));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    child_key(&mut h, &peer, Key::Y, Modifiers::COMMAND);
    assert!(Arc::ptr_eq(&after, &h.state().session.snapshot().unwrap()));
    let bound = h.state().views.view(id).unwrap().input(&h.state().session).unwrap();
    focus(&mut h, &peer, false);
    assert!(!h.state().views.view(id).unwrap().accepts(bound, &h.state().session));
}

/// A fields dialog requested by the child is shown once in the root and keeps its originating
/// view address. While it is open, canvas keys from the other host cannot alter the project.
#[test]
fn detached_fields_dialog_keeps_its_target_and_blocks_other_input() {
    let (mut h, peer) = multi_window();
    detach(&mut h);
    let id = h.state().views.detached.unwrap();
    let origin = h.state().views.view(id).unwrap().camera.screen(Point { x_nm: 0, y_nm: 0 }, child_rect(&peer, "Schematic canvas"));
    child_click_at(&mut h, &peer, origin);
    child_key(&mut h, &peer, Key::A, Modifiers::NONE);
    child_key(&mut h, &peer, Key::Escape, Modifiers::NONE);
    child_key(&mut h, &peer, Key::E, Modifiers::NONE);
    assert!(matches!(h.state().dialog, Dialog::EditFields { .. }));
    let before = h.state().session.inspect();
    child_key(&mut h, &peer, Key::Delete, Modifiers::NONE);
    let part = child_rect(&peer, "Resistor").center();
    child_click_at(&mut h, &peer, part);
    assert!(h.state().views.view(id).unwrap().tool.is_none(), "a floating card in the child must respect the root modal");
    assert_eq!(h.state().session.inspect(), before);
    focus(&mut h, &peer, false);
    h.get_by_label("Reference").click();
    h.run();
    clear_name(&mut h);
    h.get_by_label("Reference").type_text("R42");
    h.run();
    click(&mut h, "OK");
    assert_eq!(h.state().message, None);
    let project = h.state().session.project().unwrap();
    assert_eq!(project.reference(&project.symbols()[0]), "R42");
}

/// A dialog cannot edit through a dismissed child's captured address. Closing that child
/// during the modal leaves the root alone and rejects a later submission without an edit.
#[test]
fn closing_a_detached_view_invalidates_its_pending_fields_dialog() {
    let (mut h, peer) = multi_window();
    detach(&mut h);
    let child = h.state().views.detached.unwrap();
    let root = h.state().views.active.id();
    let origin = h.state().views.view(child).unwrap().camera.screen(Point { x_nm: 0, y_nm: 0 }, child_rect(&peer, "Schematic canvas"));
    child_click_at(&mut h, &peer, origin);
    child_key(&mut h, &peer, Key::E, Modifiers::NONE);
    assert!(matches!(h.state().dialog, Dialog::EditFields { .. }));
    let before = h.state().session.inspect();
    focus(&mut h, &peer, false);
    h.get_by_label("Reference").click();
    h.run();
    clear_name(&mut h);
    h.get_by_label("Reference").type_text("R42");
    h.run();
    peer.borrow_mut().close = true;
    h.run();
    assert!(h.state().views.detached.is_none());
    assert!(!h.state().views.split);
    assert_eq!(h.state().views.active.id(), root);
    click(&mut h, "OK");
    assert_eq!(h.state().session.inspect(), before);
    assert!(h.state().message.is_some(), "the stale dialog reports why its edit was rejected");
}

/// Simulate the OS acknowledging a root Close command as a close request on its input stream.
fn close_root(h: &mut Harness<'_, PcbApp>) {
    h.input_mut().viewports.entry(ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    h.step();
    h.run();
}

/// Root close uses one decision; cancel preserves both windows and discard permits root exit.
/// Closing only the child never asks to save or changes the electrical history.
#[test]
fn closing_the_project_prompts_once_and_cancel_keeps_both_editors() {
    let (mut h, peer) = multi_window();
    detach(&mut h);
    let before = h.state().session.inspect();
    close_root(&mut h);
    h.get_by_label("Save");
    child_key(&mut h, &peer, Key::R, Modifiers::NONE);
    close_root(&mut h);
    assert_eq!(h.state().session.inspect(), before);
    focus(&mut h, &peer, false);
    click(&mut h, "Cancel");
    assert!(!h.state().project_action_pending());
    assert!(h.state().views.detached.is_some());
    assert!(!h.output().viewport_output[&ViewportId::ROOT].commands.contains(&ViewportCommand::Close));
    close_root(&mut h);
    h.get_by_label("Don't Save").click();
    h.step();
    assert!(h.state().close_allowed);
    assert!(h.output().viewport_output[&ViewportId::ROOT].commands.contains(&ViewportCommand::Close));
}

/// Quit and project replacement from the child use the merged project controller in the root.
/// Cancel preserves both hosts; New replaces their shared document and rejects old view input.
#[test]
fn detached_project_actions_use_the_primary_unsaved_changes_flow() {
    let (mut h, peer) = multi_window();
    detach(&mut h);
    let child = h.state().views.detached.unwrap();
    let before = h.state().session.snapshot().unwrap();
    let old = h.state().views.view(child).unwrap().input(&h.state().session).unwrap();
    focus(&mut h, &peer, true);
    peer.borrow_mut().events.push(Event::Key { key: Key::Q, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND });
    h.step();
    assert!(h.output().viewport_output[&ViewportId::ROOT].commands.contains(&ViewportCommand::Close));
    assert!(!h.output().viewport_output[&editor_windows::viewport()].commands.contains(&ViewportCommand::Close));
    peer.borrow_mut().events.push(Event::Key { key: Key::Q, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::COMMAND });
    h.run();
    close_root(&mut h);
    assert!(matches!(h.state().dialog, Dialog::SaveChanges(unsaved::Then::Close)));
    focus(&mut h, &peer, false);
    click(&mut h, "Cancel");
    child_key(&mut h, &peer, Key::O, Modifiers::COMMAND);
    assert!(matches!(h.state().dialog, Dialog::SaveChanges(unsaved::Then::Open)));
    focus(&mut h, &peer, false);
    click(&mut h, "Cancel");
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    child_key(&mut h, &peer, Key::N, Modifiers::COMMAND);
    assert!(matches!(h.state().dialog, Dialog::SaveChanges(unsaved::Then::New)));
    focus(&mut h, &peer, false);
    click(&mut h, "Don't Save");
    assert_eq!(h.state().views.detached, Some(child));
    assert!(!h.state().session.snapshot().unwrap().same_document(&before));
    assert!(!h.state().views.view(child).unwrap().accepts(old, &h.state().session));
    assert!(h.state().views.active.shares_sheet(h.state().views.view(child).unwrap(), &h.state().session));
}

/// Text input and panels belong to their window. Leaving a native host cancels its unfinished
/// wire, and returning cannot revive it or send a later click to the old gesture.
#[test]
fn detached_text_input_and_focus_do_not_leak_canvas_commands() {
    let (mut h, peer) = multi_window();
    detach(&mut h);
    let id = h.state().views.detached.unwrap();
    let before = h.state().session.snapshot().unwrap();
    child_key(&mut h, &peer, Key::A, Modifiers::NONE);
    peer.borrow_mut().events.push(Event::Text("wr".to_owned()));
    h.run();
    child_key(&mut h, &peer, Key::W, Modifiers::NONE);
    child_key(&mut h, &peer, Key::R, Modifiers::NONE);
    assert!(h.state().views.view(id).unwrap().tool.is_none());
    assert!(!h.state().parts.open, "child Parts does not open the root panel");
    assert!(peer.borrow().nodes.values().any(|node| node.value() == Some("wr")));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    child_key(&mut h, &peer, Key::Escape, Modifiers::NONE);
    child_key(&mut h, &peer, Key::W, Modifiers::NONE);
    assert!(matches!(h.state().views.view(id).unwrap().tool, Some(Tool::Wire { .. })));
    focus(&mut h, &peer, false);
    assert!(h.state().views.view(id).unwrap().tool.is_none());
    focus(&mut h, &peer, true);
    let rect = child_rect(&peer, "Schematic canvas");
    child_click_at(&mut h, &peer, rect.right_bottom() - vec2(40.0, 40.0));
    assert!(h.state().views.view(id).unwrap().tool.is_none());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Either native host consumes a follow request once without taking focus, changing zoom or
/// acquiring the source selection's edit authority. Subsequent manual navigation stays put.
#[test]
fn detached_selection_follow_reaches_either_host_once() {
    for child_source in [false, true] {
        let (mut h, peer) = multi_window();
        detach(&mut h);
        focus(&mut h, &peer, child_source);
        h.state_mut().preferences.selection_follow = preferences::SelectionFollow::Both;
        let root = h.state().views.active.id();
        let child = h.state().views.detached.unwrap();
        let (source, target) = if child_source { (child, root) } else { (root, child) };
        let far = Point { x_nm: 1_270_000_000, y_nm: 0 };
        h.state_mut().views.bind(target);
        h.state_mut().views.active.camera.center_on(far);
        h.state_mut().views.bind(root);
        h.run();
        let target_rect = if child_source { h.get_by_label("Schematic canvas").rect() } else { child_rect(&peer, "Schematic canvas") };
        let origin = Point { x_nm: 0, y_nm: 0 };
        let zoom = h.state().views.view(target).unwrap().camera.zoom();
        let before = h.state().session.snapshot().unwrap();
        h.state_mut().views.bind(source);
        h.state_mut().select(Selection::one_symbol("symbol-2".to_owned()));
        h.state_mut().views.bind(root);
        h.run();
        let view = h.state().views.view(target).unwrap();
        assert!(target_rect.contains(view.camera.screen(origin, target_rect)));
        assert_eq!(view.camera.zoom(), zoom);
        assert!(view.selection.is_empty());
        assert_eq!(peer.borrow().focused, child_source);
        h.state_mut().views.bind(target);
        h.state_mut().views.active.camera.center_on(far);
        h.state_mut().views.bind(root);
        h.run();
        assert!(!target_rect.contains(h.state().views.view(target).unwrap().camera.screen(origin, target_rect)));
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    }
}

/// Read the child's current painted coordinates, excluding stale accessibility-cache nodes.
fn child_readout(peer: &Rc<RefCell<Peer>>) -> Option<String> {
    peer.borrow().output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.job.text.starts_with("X ") => Some(text.galley.job.text.clone()),
        _ => None,
    })
}

/// The merged mil/mm controls remain shared while each host keeps its own pointer readout.
/// Moving the pointer between windows cannot show coordinates from the other host's camera.
#[test]
fn detached_units_and_pointer_readouts_preserve_the_display_units_workflow() {
    let (mut h, peer) = multi_window();
    detach(&mut h);
    let child = h.state().views.detached.unwrap();
    let before = h.state().session.inspect();
    let root_rect = h.get_by_label("Schematic canvas").rect();
    let point = h.state().views.active.camera.screen(Point { x_nm: 2_540_000, y_nm: -3_810_000 }, root_rect);
    h.hover_at(point);
    h.run();
    assert_eq!(h.state().pointer, Some(Point { x_nm: 2_540_000, y_nm: -3_810_000 }));
    assert_eq!(child_readout(&peer), None);
    h.event(Event::PointerGone);
    focus(&mut h, &peer, true);
    let rect = child_rect(&peer, "Schematic canvas");
    let at = h.state().views.view(child).unwrap().camera.screen(Point { x_nm: 6_350_000, y_nm: 2_540_000 }, rect);
    peer.borrow_mut().events.push(Event::PointerMoved(at));
    h.run();
    assert_eq!(h.state().pointer, None);
    assert_eq!(child_readout(&peer).as_deref(), Some("X 250.00  Y 100.00"));
    child_key(&mut h, &peer, Key::U, Modifiers::COMMAND);
    h.get_by_label("Units: mm");
    assert_eq!(child_readout(&peer).as_deref(), Some("X 6.3500  Y 2.5400"));
    peer.borrow_mut().events.push(Event::PointerGone);
    h.run();
    assert_eq!(child_readout(&peer), None);
    assert_eq!(h.state().session.inspect(), before);
}
