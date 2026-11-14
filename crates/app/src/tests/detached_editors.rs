//! Exercise the real immediate-viewport callback with independent native-style input streams.
//! OS window creation/focus is verified separately; these tests cover shared application state.

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    rc::Rc,
};

use egui::{Event, PointerButton, Pos2, Rect, ViewportCommand, ViewportId, pos2, vec2};
use pcbcraft_engine::{persistence::encode_documents, schematic::Point};

use super::*;
use crate::editor_layout::HostId;

mod board_properties;
mod pane_drag;

/// A nested Assistant viewport does not begin editor collection and must not publish its
/// window-local controls into the original editor's background layer.
#[test]
fn ui_automation_ignores_detached_assistant_targets() {
    let (mut h, peers) = many_windows();
    h.state_mut().automation.observing = true;
    click(&mut h, "Assistant");
    click(&mut h, "Pop out Assistant");
    h.run_steps(3);
    let child = peers.borrow().get(&ViewportId::from_hash_of("pcbcraft_assistant")).unwrap().clone();
    child_rect(&child, "Dock Assistant");
    let ctx = h.ctx.clone();
    let root = h.state_mut().automation_command(&ctx, "ui.handles", &json!({"window":0})).unwrap();
    assert!(!root["handles"].as_array().unwrap().iter().any(|target| target["name"] == "button:Dock Assistant"), "{root}");
    assert!(h.state_mut().automation_command(&ctx, "ui.at", &json!({"window":0,"handle":"button:Dock Assistant"})).is_err());
}

/// Named geometry remains window-local even though immediate hosts share the toolkit registry.
/// The pinned framework lacks the input hook for these child windows, which must refuse input.
#[test]
fn ui_automation_lookup_keeps_detached_hosts_separate() {
    let (mut h, peer) = multi_window();
    let ctx = h.ctx.clone();
    let root = h.state().views.active.id();
    h.state_mut().run_window_command(&ctx, "view.open_window", &json!({"view":root.0})).unwrap();
    h.state_mut().automation.observing = true;
    peer.borrow_mut().focused = true;
    h.run_steps(4);
    let root_targets = h.state_mut().automation_command(&ctx, "ui.handles", &json!({"window":0})).unwrap();
    let child_targets = h.state_mut().automation_command(&ctx, "ui.handles", &json!({"window":1})).unwrap();
    let child = h.state().views.hosts[&HostId(1)].focused;
    assert_eq!(root_targets["window"], 0);
    assert_eq!(child_targets["window"], 1);
    for (targets, view) in [(&root_targets, root), (&child_targets, child)] {
        assert!(targets["handles"].as_array().unwrap().iter().any(|target| target["name"] == format!("canvas:{}", view.0)), "{targets}");
    }
    assert!(!child_targets["handles"].as_array().unwrap().iter().any(|target| target["name"] == format!("canvas:{}", root.0)));
    assert!(
        h.state_mut()
            .automation_command(&ctx, "ui.click", &json!({"window":1,"target":{"handle":"tab:View"}}))
            .unwrap_err()
            .contains("detached windows support named lookup only")
    );
}

#[derive(Default)]
struct Peer {
    origin: Option<Pos2>,
    focused: bool,
    close: bool,
    events: Vec<Event>,
    root: Option<egui::accesskit::NodeId>,
    nodes: BTreeMap<egui::accesskit::NodeId, egui::accesskit::Node>,
    output: egui::FullOutput,
    frames: usize,
    decorations: Option<bool>,
    title: Option<String>,
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
    let (h, peers) = many_windows();
    let peer = peers.borrow_mut().entry(editor_windows::viewport(HostId(1))).or_default().clone();
    (h, peer)
}

type Peers = Rc<RefCell<HashMap<ViewportId, Rc<RefCell<Peer>>>>>;

/// Give every host its own input stream, accessibility tree and paint output.
fn many_windows() -> (Harness<'static, PcbApp>, Peers) {
    let mut h = harness();
    h.set_size(vec2(1400.0, 900.0));
    resistor_with_board_at_origin(&mut h);
    h.state_mut().browser.open = false;
    let peers = Peers::default();
    let state = peers.clone();
    egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
        let state = state.borrow_mut().entry(viewport.ids.this).or_default().clone();
        let mut input = egui::RawInput {
            viewport_id: viewport.ids.this,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 760.0))),
            time: Some(ctx.input(|i| i.time)),
            ..Default::default()
        };
        {
            let mut peer = state.borrow_mut();
            peer.decorations = viewport.builder.decorations;
            peer.title = viewport.builder.title.clone();
            input.focused = peer.focused;
            input.events = std::mem::take(&mut peer.events);
            let mut info = egui::ViewportInfo { parent: Some(ViewportId::ROOT), focused: Some(peer.focused), ..Default::default() };
            info.inner_rect = peer.origin.map(|origin| Rect::from_min_size(origin, vec2(1100.0, 760.0)));
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
    (h, peers)
}

/// Find a child control from its actual accessible output, not an assumed pixel location.
fn child_rect(peer: &Rc<RefCell<Peer>>, label: &str) -> Rect {
    peer.borrow()
        .nodes
        .values()
        .find_map(|node| {
            if node.label().or_else(|| node.value()) != Some(label) {
                return None;
            }
            let bounds = node.bounds()?;
            Some(Rect::from_min_max(pos2(bounds.x0 as f32, bounds.y0 as f32), pos2(bounds.x1 as f32, bounds.y1 as f32)))
        })
        .unwrap_or_else(|| {
            let named: Vec<_> = peer.borrow().nodes.values().filter_map(|node| node.label().or_else(|| node.value())).map(str::to_owned).collect();
            panic!("missing child control: {label}; available names: {named:?}")
        })
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
    assert!(!h.state().views.detached_hosts().is_empty());
}

/// The common Browser navigates and selects within its native host, without changing the
/// primary host's document, camera or edit selection. Browsing leaves the project untouched.
#[test]
fn detached_browser_navigation_and_selection_stay_in_the_child_host() {
    let (mut h, peer) = multi_window();
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::PLACE_FOOTPRINT,
        &json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":1,
            "x_nm":127_000_000,"y_nm":25_400_000,"expected_revision":revision}),
    );
    h.run();
    detach(&mut h);
    let root = h.state().views.active.id();
    let target = h.state().views.active.target();
    let camera = h.state().views.active.camera.saved_state();
    let before = h.state().session.snapshot().unwrap();
    for label in ["Sheets", "Main", "Boards"] {
        child_rect(&peer, label);
    }
    let at = child_rect(&peer, "Board Board").center();
    child_click_at(&mut h, &peer, at);
    let child = h.state().views.hosts[&HostId(1)].focused;
    assert_eq!(h.state().views.view(child).unwrap().workspace, crate::editor_view::Workspace::Schematic);
    for pressed in [true, false] {
        peer.borrow_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
    h.run_steps(3);
    let at = child_rect(&peer, "Open in PCB layout").center();
    child_click_at(&mut h, &peer, at);
    // Shared ribbon metrics leave less vertical room in this 760 pt child. Reveal the
    // footprint row through the child's Browser, retaining its independent input stream.
    let at = child_rect(&peer, "Board Board").center();
    peer.borrow_mut().events.extend([
        Event::PointerMoved(at),
        Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -52.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        },
    ]);
    h.run_steps(5);
    let at = child_rect(&peer, "Footprint R1  R").center();
    child_click_at(&mut h, &peer, at);
    let child = h.state().views.hosts[&HostId(1)].focused;
    let app = h.state();
    assert_eq!(app.views.view(child).unwrap().workspace, crate::editor_view::Workspace::Board);
    assert_eq!(app.views.view(child).unwrap().board.selection.len(), 1);
    assert_eq!(app.views.active.id(), root);
    assert_eq!(app.views.active.target(), target);
    assert_eq!(app.views.active.camera.saved_state(), camera);
    assert!(app.views.active.board.selection.is_empty());
    assert!(app.views.active.schematic.selection.is_empty());
    assert!(std::sync::Arc::ptr_eq(&before, &app.session.snapshot().unwrap()));
}

/// A detached pane renders and edits its explicitly selected sheet while the primary host
/// retains another sheet. The real viewport callback supplies only the child's key events.
#[test]
fn detached_sheet_navigation_and_symbol_rotation_keep_other_sheets_untouched() {
    let (mut h, peer) = multi_window();
    let revision = h.state().session.revision();
    h.state_mut().session.apply(pcbcraft_engine::sheets::CREATE_SHEET, &json!({"name":"Power","expected_revision":revision})).unwrap();
    let power = h.state().session.project().unwrap().sheets().nth(1).unwrap().target;
    let revision = h.state().session.revision();
    h.state_mut()
        .session
        .apply(pcbcraft_engine::PLACE_RESISTOR, &json!({"sheet":power.id(),"x_nm":0,"y_nm":0,"expected_revision":revision}))
        .unwrap();
    h.run();
    detach(&mut h);
    let root = h.state().views.active.id();
    let target = h.state().views.active.target();
    let camera = h.state().views.active.camera.saved_state();
    let at = child_rect(&peer, "Power").center();
    child_click_at(&mut h, &peer, at);
    // Deferred navigation and folding settle in the child's next paint. This callback-only
    // backend does not automatically follow the child viewport's repaint request.
    h.run();
    let at = child_rect(&peer, "Symbols").center();
    child_click_at(&mut h, &peer, at);
    h.run();
    let at = child_rect(&peer, "R2  R").center();
    child_click_at(&mut h, &peer, at);
    h.run();
    child_key(&mut h, &peer, Key::R, Modifiers::NONE);
    let app = h.state();
    let child = app.views.hosts[&HostId(1)].focused;
    assert_eq!(app.views.view(child).unwrap().target(), Some(crate::editor_view::DocumentTarget::Sheet(power)));
    assert_eq!(app.views.active.id(), root);
    assert_eq!(app.views.active.target(), target);
    assert_eq!(app.views.active.camera.saved_state(), camera);
    assert_eq!(app.session.project().unwrap().symbols()[0].turns(), 0);
    assert_eq!(app.session.project().unwrap().for_sheet(power).unwrap().symbols()[0].turns(), 1);
    // The shared form lives in the project-controls host but retains the child address across
    // the native-style focus transition; it must neither rename nor navigate the root's sheet.
    h.state_mut().begin_sheet_edit(child, pcbcraft_engine::sheets::RENAME_SHEET, Some(power));
    focus(&mut h, &peer, false);
    clear_name(&mut h);
    h.get_by_label("Name").type_text("Supply");
    h.run();
    click(&mut h, "OK");
    assert_eq!(h.state().session.project().unwrap().for_sheet(power).unwrap().sheet_name(), "Supply");
    assert_eq!(h.state().views.active.id(), root);
    assert_eq!(h.state().views.active.target(), target);
}

/// A direct detach inherits the primary window's chrome policy. Integrated controls address
/// only the child viewport, whose close event dismisses the view without changing the root layout.
#[test]
fn detached_window_uses_the_primary_chrome_policy_and_child_window_controls() {
    for integrated in [false, true] {
        let (mut h, peer) = multi_window();
        h.state_mut().integrated_titlebar = integrated;
        assert!(!h.state().views.is_split(), "detaching does not require splitting first");
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
                assert!(
                    h.output().viewport_output[&editor_windows::viewport(HostId(1))].commands.contains(&command),
                    "{label} targets the child window"
                );
                assert!(!h.output().viewport_output[&ViewportId::ROOT].commands.contains(&command));
                assert!(!h.state().project_action_pending());
            }
            // The OS acknowledges the child Close command as a close request on the next frame.
            peer.borrow_mut().close = true;
            h.run();
            assert!(h.state().views.detached_hosts().is_empty());
            assert!(!h.state().views.is_split());
            assert!(matches!(h.state().dialog, Dialog::None));
        }
        assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        assert_eq!(h.state().integrated_titlebar, integrated);
    }
}

/// Deliver native pointer input without waiting for a pending file worker to stop repainting.
/// Inspect the release frame so one-shot viewport commands have not already been consumed.
fn window_click(h: &mut Harness<'_, PcbApp>, peer: Option<&Rc<RefCell<Peer>>>, pos: Pos2) {
    h.input_mut().focused = peer.is_none();
    if let Some(peer) = peer {
        peer.borrow_mut().focused = true;
    }
    h.run_steps(3);
    let mut send = |event| {
        if let Some(peer) = peer {
            peer.borrow_mut().events.push(event);
        } else {
            h.input_mut().events.push(event);
        }
        h.step();
    };
    send(Event::PointerMoved(pos));
    for pressed in [true, false] {
        send(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
}

/// Native move/resize must be requested on the initial press, while the OS still owns its
/// pointer serial. Both hosts use the same chrome, including when project controls are blocked.
#[test]
fn native_window_gestures_start_on_press_in_each_host() {
    for workspace in [None, Some("library"), Some("symbol_editor")] {
        native_window_gestures_for_workspace(workspace);
    }
}

/// Service workspaces must not bypass native gestures, including disabled modal frames.
fn native_window_gestures_for_workspace(workspace: Option<&str>) {
    use egui::ResizeDirection::*;
    let auxiliary = workspace.is_some();
    for child in [false, true] {
        for modal in [false, true] {
            let (mut h, peer) = multi_window();
            h.state_mut().integrated_titlebar = true;
            detach(&mut h);
            if let Some(workspace) = workspace {
                let ctx = h.ctx.clone();
                for id in h.state_mut().views.all_mut().map(|v| v.id()).collect::<Vec<_>>() {
                    h.state_mut().run_window_command(&ctx, "app.set_workspace", &json!({"view":id.0,"workspace":workspace})).unwrap();
                }
            }
            h.run_steps(3);
            let host = if child { HostId(1) } else { HostId::ROOT };
            let target = child.then_some(&peer);
            let width = if child { 1100.0 } else { 1400.0 };
            let height = if child { 760.0 } else { 900.0 };
            let preferences = if child { child_rect(&peer, "Preferences") } else { h.get_by_label("Preferences").rect() };
            let create = if auxiliary {
                preferences
            } else if child {
                child_rect(&peer, "New arrangement")
            } else {
                h.get_by_label("New arrangement").rect()
            };
            let t = pcbcraft_chrome::Tokens::get();
            let brand = pos2(t.space_8 + (t.space_16 + t.space_4) / 2.0, preferences.center().y);
            let title = if child || auxiliary {
                None
            } else {
                Some(h.query_all_by_label("Untitled").find(|node| node.rect().bottom() <= preferences.bottom()).unwrap().rect())
            };
            let drag = pos2(if auxiliary { width / 2.0 } else { (create.right() + preferences.left()) / 2.0 }, preferences.center().y);
            if modal {
                h.state_mut().dialog = Dialog::About;
            }
            h.input_mut().focused = !child;
            peer.borrow_mut().focused = child;
            h.run_steps(3);
            let before = h.state().session.inspect();
            let mut cases = vec![
                (pos2(width / 2.0, 2.0), ViewportCommand::BeginResize(North)),
                (pos2(width / 2.0, height - 2.0), ViewportCommand::BeginResize(South)),
                (pos2(2.0, height / 2.0), ViewportCommand::BeginResize(West)),
                (pos2(width - 2.0, height / 2.0), ViewportCommand::BeginResize(East)),
                (pos2(2.0, 2.0), ViewportCommand::BeginResize(NorthWest)),
                (pos2(width - 2.0, 2.0), ViewportCommand::BeginResize(NorthEast)),
                (pos2(2.0, height - 2.0), ViewportCommand::BeginResize(SouthWest)),
                (pos2(width - 2.0, height - 2.0), ViewportCommand::BeginResize(SouthEast)),
                (drag, ViewportCommand::StartDrag),
                (brand, ViewportCommand::StartDrag),
            ];
            if let Some(title) = title {
                cases.push((title.center(), ViewportCommand::StartDrag));
            }
            for (pos, command) in cases {
                for event in [
                    Event::PointerMoved(pos),
                    Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
                ] {
                    if let Some(peer) = target {
                        peer.borrow_mut().events.push(event);
                    } else {
                        h.event(event);
                    }
                    h.step();
                }
                assert!(
                    h.output().viewport_output[&editor_windows::viewport(host)].commands.contains(&command),
                    "{host:?}, modal={modal}: {command:?} at {pos:?}"
                );
                let other = if child { HostId::ROOT } else { HostId(1) };
                assert!(!h.output().viewport_output[&editor_windows::viewport(other)].commands.contains(&command));
                h.step();
                assert!(
                    !h.output().viewport_output[&editor_windows::viewport(host)].commands.contains(&command),
                    "holding must not restart {command:?}"
                );
                let release = Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE };
                if let Some(peer) = target {
                    peer.borrow_mut().events.push(release);
                } else {
                    h.event(release);
                }
                h.step();
                assert_eq!(h.state().session.inspect(), before);
                assert_eq!(matches!(h.state().dialog, Dialog::About), modal);
            }
        }
    }
}

/// Wayland can return the pointer after a native grab without a button release. The previous
/// window gesture must not suppress resize cursors or keep ownership of the next edge press.
#[test]
fn native_resize_recovers_after_the_compositor_consumes_release() {
    for child in [false, true] {
        for start_move in [false, true] {
            let (mut h, peer) = multi_window();
            h.state_mut().integrated_titlebar = true;
            detach(&mut h);
            h.input_mut().focused = !child;
            peer.borrow_mut().focused = child;
            h.run_steps(3);
            let before = h.state().session.inspect();
            let host = if child { HostId(1) } else { HostId::ROOT };
            let (width, height) = if child { (1100.0, 760.0) } else { (1400.0, 900.0) };
            let send = |h: &mut Harness<'_, PcbApp>, event| {
                if child {
                    peer.borrow_mut().events.push(event);
                } else {
                    h.event(event);
                }
                h.step();
            };
            let start = if start_move { pos2(18.0, 24.0) } else { pos2(width - 2.0, height - 2.0) };
            send(&mut h, Event::PointerMoved(start));
            send(&mut h, Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            let command = if start_move { ViewportCommand::StartDrag } else { ViewportCommand::BeginResize(egui::ResizeDirection::SouthEast) };
            assert!(h.output().viewport_output[&editor_windows::viewport(host)].commands.contains(&command));
            // The compositor owns this gesture and consumes its release outside the client.
            send(&mut h, Event::PointerGone);
            let edge = pos2(width - 2.0, height / 2.0);
            send(&mut h, Event::PointerMoved(edge));
            let cursor = if child { peer.borrow().output.platform_output.cursor_icon } else { h.output().platform_output.cursor_icon };
            assert_eq!(cursor, egui::CursorIcon::ResizeHorizontal, "{host:?}, move={start_move}: returning to an edge");
            send(&mut h, Event::PointerButton { pos: edge, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            assert!(
                h.output().viewport_output[&editor_windows::viewport(host)]
                    .commands
                    .contains(&ViewportCommand::BeginResize(egui::ResizeDirection::East))
            );
            send(&mut h, Event::PointerButton { pos: edge, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            let preferences = if child { child_rect(&peer, "Preferences") } else { h.get_by_label("Preferences").rect() };
            window_click(&mut h, child.then_some(&peer), preferences.center());
            assert!(matches!(h.state().dialog, Dialog::Preferences(_)), "ordinary controls work after native grabs");
            assert_eq!(h.state().session.inspect(), before);
        }
    }
}

/// Home, a dialog in the main host and a queued project replacement block child editing and
/// project actions, but its integrated titlebar must still control that native window.
#[test]
fn disabled_editors_keep_native_window_controls_available() {
    for blocked_by in ["home", "dialog", "pending"] {
        let (mut h, peer) = multi_window();
        h.state_mut().integrated_titlebar = true;
        detach(&mut h);
        match blocked_by {
            "home" => h.state_mut().show_home(),
            "dialog" => h.state_mut().dialog = Dialog::About,
            _ => {
                h.state_mut().files.use_stand_in();
                h.key_press_modifiers(Modifiers::COMMAND, Key::N);
                h.run();
                h.get_by_label("Save").click();
                h.run_steps(3);
                assert_eq!(h.state().waiting, Some(unsaved::Waiting::ForSave(unsaved::Then::New)));
            }
        }
        h.run_steps(3);
        let before = h.state().session.inspect();
        let home = h.state().home;
        let dialog = std::mem::discriminant(&h.state().dialog);
        let waiting = h.state().waiting.clone();
        for label in ["Home", "Preferences", "Undo", "Save Project", "Schematic canvas"] {
            let pos = child_rect(&peer, label).center();
            window_click(&mut h, Some(&peer), pos);
            assert_eq!(h.state().home, home, "{blocked_by}: {label}");
            assert_eq!(std::mem::discriminant(&h.state().dialog), dialog, "{blocked_by}: {label}");
            assert_eq!(h.state().waiting, waiting, "{blocked_by}: {label}");
            assert_eq!(h.state().session.inspect(), before, "{blocked_by}: {label}");
        }
        for pressed in [true, false] {
            peer.borrow_mut().events.push(Event::Key { key: Key::R, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
            h.run_steps(3);
        }
        assert_eq!(h.state().session.inspect(), before, "{blocked_by}: rotate shortcut");
        for (label, command) in [
            ("Minimize window", ViewportCommand::Minimized(true)),
            ("Maximize window", ViewportCommand::Maximized(true)),
            ("Close window", ViewportCommand::Close),
        ] {
            let pos = child_rect(&peer, label).center();
            window_click(&mut h, Some(&peer), pos);
            assert!(h.output().viewport_output[&editor_windows::viewport(HostId(1))].commands.contains(&command), "{blocked_by}: {label}");
            assert!(!h.output().viewport_output[&ViewportId::ROOT].commands.contains(&command));
            assert_eq!(h.state().waiting, waiting);
        }
        peer.borrow_mut().close = true;
        h.run_steps(3);
        assert_eq!(h.state().views.hosts.len(), 1);
        assert!(!h.state().close_allowed);
        assert_eq!(h.state().session.inspect(), before);
        assert_eq!(h.state().waiting, waiting);
    }
}

/// A pending Quit leaves native controls available in the main host, including after promotion.
/// Titlebar clicks must not dismiss the wait modal or bypass its save/close decision.
#[test]
fn native_controls_above_the_wait_dialog_preserve_the_pending_close() {
    let (mut h, peer) = multi_window();
    h.state_mut().integrated_titlebar = true;
    h.state_mut().files.use_stand_in();
    detach(&mut h);
    let labels = ["Minimize window", "Maximize window", "Close window"];
    let positions = labels.map(|label| h.get_by_label(label).rect().center());
    h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    h.run_steps(3);
    let ctx = h.ctx.clone();
    h.state_mut().begin_project_close(&ctx);
    h.run_steps(3);
    let before = h.state().session.inspect();
    for host in [HostId::ROOT, HostId(1)] {
        for (index, command) in [ViewportCommand::Minimized(true), ViewportCommand::Maximized(true), ViewportCommand::Close].into_iter().enumerate() {
            let child = (host != HostId::ROOT).then_some(&peer);
            let pos = child.map_or(positions[index], |peer| child_rect(peer, labels[index]).center());
            window_click(&mut h, child, pos);
            assert!(h.output().viewport_output[&editor_windows::viewport(host)].commands.contains(&command), "{host:?}: {}", labels[index]);
            assert_eq!(h.state().waiting, Some(unsaved::Waiting::ToDecide(unsaved::Then::Close)));
            assert!(h.state().files.stand_in_waiting());
            assert!(!h.state().close_allowed);
        }
        if host == HostId::ROOT {
            h.input_mut().viewports.entry(ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
        } else {
            peer.borrow_mut().close = true;
        }
        h.run_steps(3);
        assert_eq!(h.state().views.main, HostId(1));
        assert_eq!(h.state().views.hosts.len(), 1);
        assert_eq!(h.state().waiting, Some(unsaved::Waiting::ToDecide(unsaved::Then::Close)));
        assert!(!h.state().close_allowed);
        assert_eq!(h.state().session.inspect(), before);
    }
    let cancel = child_rect(&peer, "Cancel close").center();
    window_click(&mut h, Some(&peer), cancel);
    assert_eq!(h.state().waiting, None, "the modal's own controls remain usable");
    assert!(h.state().files.stand_in_waiting());
    h.state_mut().files.end_stand_in(Some("Disk full"));
    h.run();
    assert!(!h.state().close_allowed);
    assert_eq!(h.state().session.inspect(), before);
}

/// Either split view can detach, then close or explicitly dock from its native host. Close keeps
/// the root alone; Dock restores the split. All transitions preserve navigation and project
/// bytes while invalidating callbacks captured before the transition. Child Close never prompts to save.
#[test]
fn detach_close_and_dock_preserve_views_without_copying_the_project() {
    #[derive(Debug, PartialEq)]
    enum Finish {
        Close,
        DockChild,
    }
    for id in [ViewId(0), ViewId(1)] {
        for action in [Finish::Close, Finish::DockChild] {
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
            let bytes = encode_documents(before.clone()).unwrap();
            let origin = Point { x_nm: 0, y_nm: 0 };
            let positions = [ViewId(0), ViewId(1)].map(|id| app.views.view(id).unwrap().camera.screen(origin, Rect::ZERO));
            let old = app.views.active.input(&app.session).unwrap();
            detach(&mut h);
            assert_eq!(h.state().views.owner(id), Some(HostId(1)));
            assert_ne!(h.state().views.active.id(), id, "the root owns the remaining view");
            assert!(peer.borrow().frames > 0);
            assert!(!h.state().views.view(id).unwrap().accepts(old, &h.state().session));
            assert_eq!(h.state().views.view(id).unwrap().schematic.selection.single_symbol(), Some("symbol-2"));
            assert_eq!(h.state().views.view(id).unwrap().camera.snap_nm(), 500_000);
            let root_id = h.state().views.active.id();
            let child_input = h.state().views.view(id).unwrap().input(&h.state().session).unwrap();
            match action {
                Finish::Close => {
                    peer.borrow_mut().close = true;
                    h.run();
                }
                Finish::DockChild => {
                    let view_tab = child_rect(&peer, "View").center();
                    child_click_at(&mut h, &peer, view_tab);
                    let dock = child_rect(&peer, "Dock editor view").center();
                    child_click_at(&mut h, &peer, dock);
                }
            }
            assert!(h.state().views.detached_hosts().is_empty());
            assert_eq!(h.state().views.is_split(), action != Finish::Close, "{action:?}");
            assert_eq!(h.state().views.active.id(), if action == Finish::Close { root_id } else { id });
            if action == Finish::Close {
                assert!(h.state().views.view(id).is_none());
            } else {
                let child = h.state().views.view(id).unwrap();
                assert!(!child.accepts(child_input, &h.state().session));
                assert_eq!(child.schematic.selection.single_symbol(), Some("symbol-2"));
                assert_eq!(child.camera.snap_nm(), 500_000);
            }
            assert!(matches!(h.state().dialog, Dialog::None));
            for (index, id) in [ViewId(0), ViewId(1)].into_iter().enumerate() {
                if let Some(view) = h.state().views.view(id) {
                    assert_eq!(view.camera.screen(origin, Rect::ZERO), positions[index]);
                }
            }
            assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
            assert_eq!(h.state().session.revision(), revision);
            assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
        }
    }
}

/// Child input edits the shared session exactly once; root Delete cannot act on a highlight,
/// while undo/redo from either window restore the same immutable project snapshots.
#[test]
fn detached_editing_highlights_and_history_share_one_session() {
    let (mut h, peer) = multi_window();
    detach(&mut h);
    let id = h.state().views.hosts[&HostId(1)].focused;
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
    let id = h.state().views.hosts[&HostId(1)].focused;
    let origin = h.state().views.view(id).unwrap().camera.screen(Point { x_nm: 0, y_nm: 0 }, child_rect(&peer, "Schematic canvas"));
    child_click_at(&mut h, &peer, origin);
    child_key(&mut h, &peer, Key::A, Modifiers::NONE);
    child_key(&mut h, &peer, Key::Escape, Modifiers::NONE);
    child_key(&mut h, &peer, Key::E, Modifiers::NONE);
    assert!(matches!(h.state().dialog, Dialog::EditFields(_)));
    let before = h.state().session.inspect();
    child_key(&mut h, &peer, Key::Delete, Modifiers::NONE);
    child_key(&mut h, &peer, Key::A, Modifiers::NONE);
    assert!(!peer.borrow().nodes.values().any(|node| node.label() == Some("Close Parts Catalog")));
    child_click_at(&mut h, &peer, origin);
    assert!(h.state().views.view(id).unwrap().schematic.tool.is_none(), "the child canvas must respect the root modal");
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
    let child = h.state().views.hosts[&HostId(1)].focused;
    let root = h.state().views.active.id();
    let origin = h.state().views.view(child).unwrap().camera.screen(Point { x_nm: 0, y_nm: 0 }, child_rect(&peer, "Schematic canvas"));
    child_click_at(&mut h, &peer, origin);
    child_key(&mut h, &peer, Key::E, Modifiers::NONE);
    assert!(matches!(h.state().dialog, Dialog::EditFields(_)));
    let before = h.state().session.inspect();
    focus(&mut h, &peer, false);
    h.get_by_label("Reference").click();
    h.run();
    clear_name(&mut h);
    h.get_by_label("Reference").type_text("R42");
    h.run();
    peer.borrow_mut().close = true;
    h.run();
    assert!(h.state().views.detached_hosts().is_empty());
    assert!(!h.state().views.is_split());
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

/// Matching document targets never stand in for native host identity. Closing either window
/// keeps the other's view IDs and camera; closing the original never rehosts a surviving tab group.
#[test]
fn closing_matching_or_mixed_windows_keeps_the_addressed_survivor() {
    for board in [false, true] {
        for close_original in [false, true] {
            let (mut h, peers) = many_windows();
            let original = h.state().views.active.id();
            let child = h.state_mut().views.open_window(original).unwrap();
            h.run();
            let peer = peers.borrow()[&editor_windows::viewport(child)].clone();
            focus_host(&mut h, &peers, child);
            if board {
                let at = child_rect(&peer, "Choose workspace").center();
                child_click_at(&mut h, &peer, at);
                // egui 0.37 sizes a new popup in a hidden child pass. The harness only
                // settles root repaints, so deliver the child's requested paint too.
                h.run();
                let at = child_rect(&peer, "PCB Layout").center();
                child_click_at(&mut h, &peer, at);
            }
            let survivor = if close_original { child } else { HostId::ROOT };
            let closed = if close_original { HostId::ROOT } else { child };
            let focused = h.state().views.hosts[&survivor].focused;
            let views = h.state().views.hosts[&survivor].layout.views();
            let camera = h.state().views.view(focused).unwrap().camera.saved_state();
            let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
            if close_original {
                focus_host(&mut h, &peers, HostId::ROOT);
                close_root(&mut h);
            } else {
                peer.borrow_mut().close = true;
                h.run();
            }
            assert_eq!(h.state().views.main, survivor);
            assert_eq!(h.state().views.hosts.len(), 1);
            assert!(!h.state().views.hosts.contains_key(&closed));
            assert_eq!(h.state().views.hosts[&survivor].layout.views(), views);
            assert_eq!(h.state().views.hosts[&survivor].focused, focused);
            assert_eq!(h.state().views.view(focused).unwrap().camera.saved_state(), camera);
            assert!(matches!(h.state().dialog, Dialog::None));
            assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
            if close_original {
                child_rect(&peer, if board { "PCB canvas" } else { "Schematic canvas" });
                peer.borrow_mut().close = true;
                h.run();
                assert!(matches!(h.state().dialog, Dialog::SaveChanges(unsaved::Then::Close)));
                focus_host(&mut h, &peers, survivor);
                let at = child_rect(&peer, "Cancel").center();
                child_click_at(&mut h, &peer, at);
                assert_eq!(h.state().views.main, survivor);
            } else {
                h.get_by_label("Schematic canvas");
            }
        }
    }
}

/// Explicit Quit uses one decision even with several windows; Cancel preserves them all.
#[test]
fn closing_the_project_prompts_once_and_cancel_keeps_both_editors() {
    let (mut h, peer) = multi_window();
    detach(&mut h);
    let before = h.state().session.inspect();
    let ctx = h.ctx.clone();
    h.state_mut().request_project_close(&ctx);
    close_root(&mut h);
    h.get_by_label("Save");
    child_key(&mut h, &peer, Key::R, Modifiers::NONE);
    let ctx = h.ctx.clone();
    h.state_mut().request_project_close(&ctx);
    close_root(&mut h);
    assert_eq!(h.state().session.inspect(), before);
    focus(&mut h, &peer, false);
    click(&mut h, "Cancel");
    assert!(!h.state().project_action_pending());
    assert!(!h.state().views.detached_hosts().is_empty());
    assert!(!h.output().viewport_output[&ViewportId::ROOT].commands.contains(&ViewportCommand::Close));
    let ctx = h.ctx.clone();
    h.state_mut().request_project_close(&ctx);
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
    let child = h.state().views.hosts[&HostId(1)].focused;
    let before = h.state().session.snapshot().unwrap();
    let old = h.state().views.view(child).unwrap().input(&h.state().session).unwrap();
    focus(&mut h, &peer, true);
    peer.borrow_mut().events.push(Event::Key { key: Key::Q, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND });
    h.step();
    assert!(h.output().viewport_output[&ViewportId::ROOT].commands.contains(&ViewportCommand::Close));
    assert!(!h.output().viewport_output[&editor_windows::viewport(HostId(1))].commands.contains(&ViewportCommand::Close));
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
    assert_eq!(h.state().views.owner(child), Some(HostId(1)));
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
    let id = h.state().views.hosts[&HostId(1)].focused;
    let before = h.state().session.snapshot().unwrap();
    child_key(&mut h, &peer, Key::A, Modifiers::NONE);
    peer.borrow_mut().events.push(Event::Text("wr".to_owned()));
    h.run();
    child_key(&mut h, &peer, Key::W, Modifiers::NONE);
    child_key(&mut h, &peer, Key::R, Modifiers::NONE);
    assert!(h.state().views.view(id).unwrap().schematic.tool.is_none());
    assert!(!h.state().parts.open, "child Parts does not open the root panel");
    assert!(peer.borrow().nodes.values().any(|node| node.value() == Some("wr")));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    child_key(&mut h, &peer, Key::Escape, Modifiers::NONE);
    child_key(&mut h, &peer, Key::W, Modifiers::NONE);
    assert!(matches!(h.state().views.view(id).unwrap().schematic.tool, Some(Tool::Wire { .. })));
    focus(&mut h, &peer, false);
    assert!(h.state().views.view(id).unwrap().schematic.tool.is_none());
    focus(&mut h, &peer, true);
    let rect = child_rect(&peer, "Schematic canvas");
    child_click_at(&mut h, &peer, rect.right_bottom() - vec2(40.0, 40.0));
    assert!(h.state().views.view(id).unwrap().schematic.tool.is_none());
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
        let child = h.state().views.hosts[&HostId(1)].focused;
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
        assert!(view.schematic.selection.is_empty());
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
    let child = h.state().views.hosts[&HostId(1)].focused;
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

/// Transfer OS-style focus between several native hosts, never duplicating keyboard input.
fn focus_host(h: &mut Harness<'_, PcbApp>, peers: &Peers, host: HostId) {
    h.input_mut().focused = host == HostId::ROOT;
    for (id, peer) in peers.borrow().iter() {
        peer.borrow_mut().focused = *id == editor_windows::viewport(host);
    }
    h.run();
}

/// Keep a root split while creating two additional windows and splitting one of those too.
/// Edits, highlights and undo share one session; closing a window cannot collapse another host.
#[test]
fn split_and_multiple_native_windows_coexist_and_share_edits() {
    let (mut h, peers) = many_windows();
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    let root_panes = h.state().views.hosts[&HostId::ROOT].layout.views();
    let before = h.state().session.snapshot().unwrap();
    let root_camera = h.state().views.active.camera.screen(Point { x_nm: 0, y_nm: 0 }, Rect::ZERO);
    click(&mut h, "Open another editor window");
    click(&mut h, "Open another editor window");
    assert_eq!(h.state().views.detached_hosts(), vec![HostId(1), HostId(2)]);
    assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.views(), root_panes);
    assert_eq!(h.state().views.active.camera.screen(Point { x_nm: 0, y_nm: 0 }, Rect::ZERO), root_camera);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    let first = peers.borrow()[&editor_windows::viewport(HostId(1))].clone();
    let second = peers.borrow()[&editor_windows::viewport(HostId(2))].clone();
    focus_host(&mut h, &peers, HostId(1));
    let view_tab = child_rect(&first, "View").center();
    child_click_at(&mut h, &first, view_tab);
    let split = child_rect(&first, "Split editor views").center();
    child_click_at(&mut h, &first, split);
    let child_panes = h.state().views.hosts[&HostId(1)].layout.views();
    assert_eq!(child_panes.len(), 2);
    assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.views(), root_panes);
    let editing = child_panes[1];
    let rect = child_rect(&first, &format!("Schematic canvas {}", editing.0 + 1));
    let point = h.state().views.view(editing).unwrap().camera.screen(Point { x_nm: 0, y_nm: 0 }, rect);
    child_click_at(&mut h, &first, point);
    assert_eq!(h.state().views.hosts[&HostId(1)].focused, editing);
    for id in root_panes.iter().copied().chain([child_panes[0], h.state().views.hosts[&HostId(2)].focused]) {
        assert_eq!(h.state().views.highlight_for(id, &h.state().session).single_symbol(), Some("symbol-2"));
        assert!(h.state().views.view(id).unwrap().schematic.selection.is_empty());
    }
    child_key(&mut h, &first, Key::R, Modifiers::NONE);
    let edited = h.state().session.snapshot().unwrap();
    assert!(!Arc::ptr_eq(&before, &edited));
    focus_host(&mut h, &peers, HostId::ROOT);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    focus_host(&mut h, &peers, HostId(2));
    child_key(&mut h, &second, Key::Y, Modifiers::COMMAND);
    assert!(Arc::ptr_eq(&edited, &h.state().session.snapshot().unwrap()));
    let second_view = h.state().views.hosts[&HostId(2)].focused;
    let second_camera = h.state().views.view(second_view).unwrap().camera.screen(Point { x_nm: 0, y_nm: 0 }, Rect::ZERO);
    first.borrow_mut().close = true;
    h.run();
    assert_eq!(h.state().views.detached_hosts(), vec![HostId(2)]);
    assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.views(), root_panes);
    assert!(child_panes.iter().all(|id| h.state().views.view(*id).is_none()));
    assert_eq!(h.state().views.view(second_view).unwrap().camera.screen(Point { x_nm: 0, y_nm: 0 }, Rect::ZERO), second_camera);
    assert!(Arc::ptr_eq(&edited, &h.state().session.snapshot().unwrap()));
    focus_host(&mut h, &peers, HostId::ROOT);
    click(&mut h, "Open another editor window");
    assert_eq!(h.state().views.detached_hosts(), vec![HostId(2), HostId(3)], "closed host IDs are never reused");
    assert!(h.state().views.hosts[&HostId(3)].focused > editing);
}

/// Dock a pane into an already split root without displacing its panes or another window.
/// Closing an individual pane similarly changes only the branch that contained it.
#[test]
fn docking_into_an_existing_split_preserves_other_views() {
    let (mut h, peers) = many_windows();
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    click(&mut h, "Open another editor window");
    click(&mut h, "Open another editor window");
    let root = h.state().views.hosts[&HostId::ROOT].layout.views();
    let docked = h.state().views.hosts[&HostId(1)].focused;
    let other = h.state().views.hosts[&HostId(2)].focused;
    let before = h.state().session.inspect();
    let first = peers.borrow()[&editor_windows::viewport(HostId(1))].clone();
    focus_host(&mut h, &peers, HostId(1));
    let tab = child_rect(&first, "View").center();
    child_click_at(&mut h, &first, tab);
    let dock = child_rect(&first, "Dock editor view").center();
    child_click_at(&mut h, &first, dock);
    let layout = h.state().views.hosts[&HostId::ROOT].layout.views();
    assert_eq!(layout.len(), 3);
    assert!(root.iter().all(|id| layout.contains(id)));
    assert!(layout.contains(&docked));
    focus_host(&mut h, &peers, HostId::ROOT);
    click(&mut h, "Split editor views");
    assert!(!h.state().views.is_split());
    click(&mut h, "Split editor views");
    assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.views(), layout, "restore the entire split tree in its original order");

    assert_eq!(h.state().views.detached_hosts(), vec![HostId(2)]);
    assert_eq!(h.state().views.hosts[&HostId(2)].focused, other);
    let ctx = h.ctx.clone();
    h.state_mut().queue_view_action(&ctx, docked, editor_windows::ViewAction::Close);
    h.run();
    assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.views(), root);
    assert!(h.state().views.view(docked).is_none());
    assert_eq!(h.state().session.inspect(), before);
}

/// A queued action cannot resurrect a closed view or act on a replacement project.
#[test]
fn queued_host_actions_reject_closed_views_and_replacement_projects() {
    for replace_project in [false, true] {
        let (mut h, _) = many_windows();
        click(&mut h, "View");
        click(&mut h, "Open another editor window");
        let child = h.state().views.hosts[&HostId(1)].focused;
        let ctx = h.ctx.clone();
        h.state_mut().queue_view_action(&ctx, child, editor_windows::ViewAction::Dock);
        if replace_project {
            let snapshot = h.state().session.snapshot().unwrap();
            h.state_mut().session.open_snapshot(snapshot);
        } else {
            h.state_mut().views.close_host(HostId(1));
        }
        h.run();
        assert!(!h.state().views.is_split());
        assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.views(), vec![ViewId(0)]);
    }
}

/// Navigation fans out once to each eligible view, with stable primary-directed policies.
/// Manual navigation is not repeatedly undone, and each recipient keeps its edit selection.
#[test]
fn selection_follow_fans_out_across_hosts_without_focus_or_selection_transfer() {
    use preferences::SelectionFollow::{Both, Off, PrimaryFollowsSecondary, SecondaryFollowsPrimary};
    for (policy, source_primary) in [
        (Both, true),
        (Both, false),
        (Off, true),
        (SecondaryFollowsPrimary, true),
        (SecondaryFollowsPrimary, false),
        (PrimaryFollowsSecondary, false),
    ] {
        let (mut h, peers) = many_windows();
        click(&mut h, "View");
        click(&mut h, "Open another editor window");
        click(&mut h, "Open another editor window");
        focus_host(&mut h, &peers, if source_primary { HostId::ROOT } else { HostId(1) });
        let primary = ViewId(0);
        let first = h.state().views.hosts[&HostId(1)].focused;
        let second = h.state().views.hosts[&HostId(2)].focused;
        let source = if source_primary { primary } else { first };
        let far = Point { x_nm: 1_270_000_000, y_nm: 0 };
        for id in [primary, first, second].into_iter().filter(|id| *id != source) {
            h.state_mut().views.view_mut(id).unwrap().camera.center_on(far);
        }
        h.state_mut().preferences.selection_follow = policy;
        h.run();
        let before = h.state().session.inspect();
        h.state_mut().views.bind(source);
        h.state_mut().select(Selection::one_symbol("symbol-2".to_owned()));
        h.state_mut().views.bind_main();
        h.run();
        for (host, id) in [(HostId::ROOT, primary), (HostId(1), first), (HostId(2), second)].into_iter().filter(|(_, id)| *id != source) {
            let rect = if host == HostId::ROOT {
                h.get_by_label("Schematic canvas").rect()
            } else {
                child_rect(&peers.borrow()[&editor_windows::viewport(host)], "Schematic canvas")
            };
            let follows =
                policy == Both || (policy == SecondaryFollowsPrimary && source_primary) || (policy == PrimaryFollowsSecondary && id == primary);
            let view = h.state().views.view(id).unwrap();
            assert_eq!(rect.contains(view.camera.screen(Point { x_nm: 0, y_nm: 0 }, rect)), follows, "{policy:?} for {id:?}");
            assert!(view.schematic.selection.is_empty());
            assert_eq!(view.camera.zoom(), 1.0);
            assert_eq!(h.state().views.highlight_for(id, &h.state().session).single_symbol(), Some("symbol-2"));
            h.state_mut().views.view_mut(id).unwrap().camera.center_on(far);
            h.run();
            assert!(!rect.contains(h.state().views.view(id).unwrap().camera.screen(Point { x_nm: 0, y_nm: 0 }, rect)));
        }
        assert_eq!(h.state().session.inspect(), before);
        assert_eq!(h.state().views.hosts[&HostId::ROOT].focused, primary);
        assert_eq!(peers.borrow()[&editor_windows::viewport(HostId(1))].borrow().focused, !source_primary);
    }
}

/// The tab menu addresses the clicked pane, including an inactive one. It can create another
/// pane and close just that pane without stealing the existing pane's camera or selection.
#[test]
fn tab_actions_open_and_close_the_addressed_view() {
    let (mut h, _) = many_windows();
    click(&mut h, "View");
    click(&mut h, "Split editor views");
    let path = h.state().views.active.sheet_path(&h.state().session).unwrap();
    let before = h.state().session.inspect();
    let original = h.state().views.hosts[&HostId::ROOT].layout.views();
    let tab = h.get_all_by_label(&path).nth(1).unwrap().rect().center();
    h.hover_at(tab);
    h.run();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: tab, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
    click(&mut h, "Open beside");
    let panes = h.state().views.hosts[&HostId::ROOT].layout.views();
    assert_eq!(panes.len(), 3);
    assert_eq!(&panes[..2], &original);
    assert_eq!(h.state().views.active.id(), original[1], "the context click focuses its addressed pane");
    let tab = h.get_all_by_label(&path).nth(2).unwrap().rect().center();
    h.hover_at(tab);
    h.run();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: tab, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.run();
    }
    click(&mut h, "Close view");
    assert_eq!(h.state().views.hosts[&HostId::ROOT].layout.views(), original);
    assert_eq!(h.state().session.inspect(), before);
}

/// Closing an unrelated host during a modal must not expire the dialog's surviving view.
/// Closing the originating host is covered separately and still rejects the captured edit.
#[test]
fn closing_another_window_keeps_a_surviving_fields_dialog_valid() {
    let (mut h, peers) = many_windows();
    click(&mut h, "View");
    click(&mut h, "Open another editor window");
    click(&mut h, "Open another editor window");
    let first = peers.borrow()[&editor_windows::viewport(HostId(1))].clone();
    let second = peers.borrow()[&editor_windows::viewport(HostId(2))].clone();
    let id = h.state().views.hosts[&HostId(2)].focused;
    focus_host(&mut h, &peers, HostId(2));
    let rect = child_rect(&second, "Schematic canvas");
    let origin = h.state().views.view(id).unwrap().camera.screen(Point { x_nm: 0, y_nm: 0 }, rect);
    child_click_at(&mut h, &second, origin);
    child_key(&mut h, &second, Key::E, Modifiers::NONE);
    assert!(matches!(h.state().dialog, Dialog::EditFields(_)));
    focus_host(&mut h, &peers, HostId::ROOT);
    h.get_by_label("Reference").click();
    h.run();
    clear_name(&mut h);
    h.get_by_label("Reference").type_text("R42");
    h.run();
    first.borrow_mut().close = true;
    h.run();
    click(&mut h, "OK");
    assert_eq!(h.state().message, None);
    let project = h.state().session.project().unwrap();
    assert_eq!(project.reference(&project.symbols()[0]), "R42");
    assert_eq!(h.state().views.detached_hosts(), vec![HostId(2)]);
}

/// Docking the only visible pane discards that host's collapsed panes. If one of those held
/// the primary follow role, both directional policies must immediately use the surviving view.
#[test]
fn docking_prunes_a_parked_primary_before_directional_following() {
    use preferences::SelectionFollow::{PrimaryFollowsSecondary, SecondaryFollowsPrimary};
    let (mut h, _) = many_windows();
    let ctx = h.ctx.clone();
    let child = h.state_mut().views.detach_view(ViewId(0)).unwrap();
    let docked = h.state_mut().views.open_beside(ViewId(0)).unwrap();
    h.state_mut().views.host = child;
    h.state_mut().views.bind(docked);
    h.state_mut().split_editors(&ctx, false);
    h.state_mut().views.bind_main();
    h.state_mut().views.dock_view(docked);
    h.run();
    assert!(h.state().views.view(ViewId(0)).is_none());
    assert!(h.state().views.detached_hosts().is_empty());
    let before = h.state().session.inspect();
    let origin = Point { x_nm: 0, y_nm: 0 };
    let far = Point { x_nm: 1_270_000_000, y_nm: 0 };
    for (policy, source, target) in [(SecondaryFollowsPrimary, docked, ViewId(1)), (PrimaryFollowsSecondary, ViewId(1), docked)] {
        h.state_mut().preferences.selection_follow = policy;
        h.state_mut().views.view_mut(target).unwrap().camera.center_on(far);
        h.state_mut().views.bind(source);
        h.state_mut().select(Selection::one_symbol("symbol-2".to_owned()));
        h.state_mut().views.bind_main();
        h.run();
        let rect = h.get_by_label(&format!("Schematic canvas {}", target.0 + 1)).rect();
        assert!(rect.contains(h.state().views.view(target).unwrap().camera.screen(origin, rect)), "{policy:?}");
        assert_eq!(h.state().session.inspect(), before);
    }
}

/// The original native window can disappear while two editors keep their native IDs, layouts,
/// cameras and selections. Project dialogs and shared edit/history continue in the survivor.
#[test]
fn closing_the_original_window_promotes_a_survivor_without_rehosting_its_views() {
    let (mut h, peers) = many_windows();
    click(&mut h, "View");
    click(&mut h, "Open another editor window");
    click(&mut h, "Open another editor window");
    let first = peers.borrow()[&editor_windows::viewport(HostId(1))].clone();
    let second = peers.borrow()[&editor_windows::viewport(HostId(2))].clone();
    focus_host(&mut h, &peers, HostId(1));
    let tab = child_rect(&first, "View").center();
    child_click_at(&mut h, &first, tab);
    let split = child_rect(&first, "Split editor views").center();
    child_click_at(&mut h, &first, split);
    let panes = h.state().views.hosts[&HostId(1)].layout.views();
    let origin = Point { x_nm: 0, y_nm: 0 };
    let cameras: Vec<_> = panes.iter().map(|id| h.state().views.view(*id).unwrap().camera.screen(origin, Rect::ZERO)).collect();
    let before = h.state().session.inspect();
    close_root(&mut h);
    assert_eq!(h.state().views.main, HostId(1));
    assert!(!h.state().views.hosts.contains_key(&HostId::ROOT));
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(h.state().session.inspect(), before);
    assert_eq!(h.state().views.hosts[&HostId(1)].layout.views(), panes);
    for (id, position) in panes.iter().zip(cameras) {
        assert_eq!(h.state().views.view(*id).unwrap().camera.screen(origin, Rect::ZERO), position);
    }
    child_rect(&first, "Detach active editor view");
    let view = h.state().views.hosts[&HostId(2)].focused;
    let at = h.state().views.view(view).unwrap().camera.screen(origin, child_rect(&second, "Schematic canvas"));
    focus_host(&mut h, &peers, HostId(2));
    child_click_at(&mut h, &second, at);
    child_key(&mut h, &second, Key::R, Modifiers::NONE);
    assert_eq!(h.state().session.revision(), before["revision"].as_u64().unwrap() + 1);
    focus_host(&mut h, &peers, HostId(1));
    child_key(&mut h, &first, Key::Z, Modifiers::COMMAND);
    assert_eq!(h.state().session.project().unwrap().symbols()[0].turns(), 0);
    child_key(&mut h, &first, Key::Comma, Modifiers::COMMAND);
    child_rect(&first, "Preferences");
    assert!(matches!(h.state().dialog, Dialog::Preferences(_)));
}

/// Closing the new main window can promote again; only the last editor asks to save. Cancel,
/// failed saves and repeated Close keep that same native window available for recovery.
#[test]
fn the_last_surviving_window_owns_save_cancel_and_failure_recovery() {
    let (mut h, peers) = many_windows();
    h.state_mut().files.use_stand_in();
    click(&mut h, "View");
    click(&mut h, "Open another editor window");
    click(&mut h, "Open another editor window");
    let first = peers.borrow()[&editor_windows::viewport(HostId(1))].clone();
    let last = peers.borrow()[&editor_windows::viewport(HostId(2))].clone();
    close_root(&mut h);
    first.borrow_mut().close = true;
    h.run();
    assert_eq!(h.state().views.main, HostId(2));
    assert_eq!(h.state().views.hosts.len(), 1);
    assert!(matches!(h.state().dialog, Dialog::None));
    last.borrow_mut().close = true;
    h.run();
    assert!(matches!(h.state().dialog, Dialog::SaveChanges(unsaved::Then::Close)));
    assert!(!h.state().close_allowed);
    focus_host(&mut h, &peers, HostId(2));
    let cancel = child_rect(&last, "Cancel").center();
    child_click_at(&mut h, &last, cancel);
    assert!(matches!(h.state().dialog, Dialog::None));
    last.borrow_mut().close = true;
    h.run();
    let save = child_rect(&last, "Save").center();
    // A pending stand-in save repaints until the test supplies its result.
    for pressed in [true, false] {
        last.borrow_mut().events.push(Event::PointerMoved(save));
        last.borrow_mut().events.push(Event::PointerButton { pos: save, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(3);
    }
    assert!(h.state().files.stand_in_waiting());
    last.borrow_mut().close = true;
    h.run_steps(3);
    assert_eq!(h.state().waiting, Some(unsaved::Waiting::ForSave(unsaved::Then::Close)));
    h.state_mut().files.end_stand_in(Some("Disk full"));
    h.run();
    assert!(!h.state().close_allowed);
    assert_eq!(h.state().views.main, HostId(2));
    assert_eq!(h.state().message.as_deref(), Some("Disk full"));
    last.borrow_mut().close = true;
    h.run();
    let discard = child_rect(&last, "Don't Save").center();
    child_click_at(&mut h, &last, discard);
    assert!(h.state().close_allowed);
}

/// Closing the original window during a save leaves the worker alive. The last window can
/// cancel a pending close, receive the save result, and subsequently close without another prompt.
#[test]
fn a_pending_save_survives_original_window_close_and_cancel_from_the_last_window() {
    let (mut h, peer) = multi_window();
    let dir = tempfile::tempdir().unwrap();
    h.state_mut().files.use_stand_in();
    detach(&mut h);
    h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    h.run_steps(3);
    assert!(h.state().files.busy());
    h.input_mut().viewports.entry(ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    h.run_steps(3);
    assert_eq!(h.state().views.main, HostId(1));
    assert!(h.state().files.busy());
    assert_eq!(h.state().waiting, None, "closing one of several windows does not request shutdown");
    peer.borrow_mut().close = true;
    h.run_steps(3);
    assert_eq!(h.state().waiting, Some(unsaved::Waiting::ToDecide(unsaved::Then::Close)));
    let cancel = child_rect(&peer, "Cancel close").center();
    peer.borrow_mut().focused = true;
    h.input_mut().focused = false;
    for pressed in [true, false] {
        peer.borrow_mut().events.push(Event::PointerMoved(cancel));
        peer.borrow_mut().events.push(Event::PointerButton { pos: cancel, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(3);
    }
    assert_eq!(h.state().waiting, None);
    assert!(h.state().files.stand_in_waiting());
    h.state_mut().files.finish_stand_in(dir.path().join("surviving-window"));
    h.run();
    assert!(!h.state().close_allowed);
    assert!(!h.state().files.dirty(&h.state().session));
    assert_eq!(h.state().session.project().unwrap().symbols().len(), 1);
    peer.borrow_mut().close = true;
    h.run();
    assert!(h.state().close_allowed);
    assert!(matches!(h.state().dialog, Dialog::None));
}

/// Window commands address views and windows by ID in every native host, through the same
/// handlers as the ribbon and sheet tabs: panels stay per window, splits and docking move only
/// the addressed view, and the project snapshot never changes.
#[test]
fn window_commands_reach_views_and_panels_in_every_window() {
    let (mut h, _peers) = many_windows();
    let ctx = h.ctx.clone();
    let before = h.state().session.snapshot().unwrap();
    let revision = h.state().session.revision();
    let run = |h: &mut Harness<'_, PcbApp>, id: &str, params: Value| {
        let result = h.state_mut().run_window_command(&ctx, id, &params);
        h.run();
        result
    };
    let root_view = h.state().views.hosts[&HostId::ROOT].focused;
    h.state_mut().run_window_command(&ctx, "view.open_window", &json!({"view": root_view.0})).unwrap();
    assert_eq!(h.state().views.detached_hosts(), vec![HostId(1)]);
    assert_eq!(h.state().views.hosts[&HostId::ROOT].focused, root_view);

    // Inspect the fresh host before it has been drawn, then after creating its panel state.
    assert!(h.state().host_panels(HostId(1)).properties);
    assert!(h.state().saved_editor_panels(HostId(1)).properties);
    h.run();
    assert!(h.state().host_panels(HostId(1)).properties);
    run(&mut h, "window.set_panel", json!({"window": 1, "panel": "properties", "open": false})).unwrap();
    assert!(!h.state().host_panels(HostId(1)).properties);
    assert!(!h.state().saved_editor_panels(HostId(1)).properties);

    run(&mut h, "window.set_panel", json!({"window": 1, "panel": "nets", "open": true})).unwrap();
    run(&mut h, "window.set_ribbon_tab", json!({"window": 1, "tab": "view"})).unwrap();
    let inspected = window_commands::inspect_editor(h.state());
    assert_eq!(inspected["windows"][1]["panels"]["nets"], true);
    assert_eq!(inspected["windows"][1]["ribbon_tab"], "view");
    assert_eq!(inspected["windows"][0]["panels"]["nets"], false);
    assert_eq!(inspected["windows"][0]["ribbon_tab"], "capture");

    let child_view = h.state().views.hosts[&HostId(1)].focused;
    run(&mut h, "view.split", json!({"view": child_view.0, "split": true})).unwrap();
    assert!(h.state().views.hosts[&HostId(1)].split());
    assert!(!h.state().views.hosts[&HostId::ROOT].split());
    let moved = h.state().views.hosts[&HostId(1)].layout.views()[1];
    run(&mut h, "view.dock", json!({"view": moved.0})).unwrap();
    assert_eq!(h.state().views.owner(moved), Some(HostId::ROOT));
    assert_eq!(h.state().views.hosts[&HostId(1)].layout.views(), vec![child_view]);
    assert!(run(&mut h, "view.dock", json!({"view": moved.0})).unwrap_err().contains("already in the main window"));

    run(&mut h, "view.detach", json!({"view": moved.0})).unwrap();
    let host = h.state().views.owner(moved).unwrap();
    assert_eq!(h.state().views.detached_hosts(), vec![HostId(1), host]);
    run(&mut h, "window.close", json!({"window": host.0})).unwrap();
    assert!(h.state().views.view(moved).is_none());
    assert!(run(&mut h, "view.fit", json!({"view": moved.0})).unwrap_err().contains(&format!("no open view {}", moved.0)));
    assert_eq!(h.state().views.detached_hosts(), vec![HostId(1)]);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    assert_eq!(h.state().session.revision(), revision);
}

/// Window commands reach PCB views as the workspace picker and board selection do: switching a
/// view, opening the board in a new window, selecting footprints and the Layout ribbon tab, all
/// without changing the project. Each view's selection command matches what it shows.
#[test]
fn window_commands_switch_workspaces_and_select_footprints() {
    let (mut h, _peers) = many_windows();
    let ctx = h.ctx.clone();
    let app = h.state_mut();
    app.run(pcbcraft_engine::PLACE_FOOTPRINT,&json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":0,"y_nm":0,"expected_revision":app.session.revision()}));
    h.run();
    let before = h.state().session.inspect();
    let run = |h: &mut Harness<'_, PcbApp>, id: &str, params: Value| {
        let result = h.state_mut().run_window_command(&ctx, id, &params);
        h.run();
        result
    };
    let root = h.state().views.hosts[&HostId::ROOT].focused;
    let pick = json!({"view": root.0, "footprints": ["footprint-3"]});
    assert!(run(&mut h, "view.select_footprints", pick).unwrap_err().contains("shows the schematic"));

    run(&mut h, "view.set_workspace", json!({"view": root.0, "workspace": "board"})).unwrap();
    let board = h.state().views.hosts[&HostId::ROOT].focused;
    assert_ne!(board, root, "the board opens as a tab in the same pane, as the picker does");
    assert_eq!(h.state().views.view(board).map(|view| view.workspace), Some(Workspace::Board));
    let shown = |h: &Harness<'_, PcbApp>, id: ViewId| {
        let inspected = window_commands::inspect_editor(h.state());
        inspected["views"].as_array().into_iter().flatten().find(|view| view["id"] == id.0).cloned().unwrap_or_default()
    };
    assert_eq!(shown(&h, board)["workspace"], "board");
    assert_eq!(window_commands::inspect_editor(h.state())["windows"][0]["ribbon_tab"], "layout");
    run(&mut h, "window.set_ribbon_tab", json!({"window":0,"tab":"libraries"})).unwrap();
    assert_eq!(window_commands::inspect_editor(h.state())["windows"][0]["ribbon_tab"], "libraries");
    run(&mut h, "window.set_ribbon_tab", json!({"window":0,"tab":"layout"})).unwrap();
    run(&mut h, "view.select_footprints", json!({"view": board.0, "footprints": ["footprint-3"]})).unwrap();
    assert_eq!(shown(&h, board)["board"]["selection"], json!(["footprint-3"]));
    let missing = json!({"view": board.0, "footprints": ["footprint-99"]});
    assert!(run(&mut h, "view.select_footprints", missing).unwrap_err().contains("aren't on this view's board"));
    let symbols = json!({"view": board.0, "items": {"symbols": ["symbol-2"], "wires": []}});
    assert!(run(&mut h, "view.select", symbols).unwrap_err().contains("shows the PCB layout"));
    run(&mut h, "window.set_ribbon_tab", json!({"window": 0, "tab": "view"})).unwrap();
    assert_eq!(window_commands::inspect_editor(h.state())["windows"][0]["ribbon_tab"], "view");

    run(&mut h, "view.set_workspace", json!({"view": board.0, "workspace": "schematic", "placement": "window"})).unwrap();
    let hosts = h.state().views.detached_hosts();
    assert_eq!(hosts.len(), 1);
    let child = h.state().views.hosts[&hosts[0]].focused;
    assert_eq!(h.state().views.view(child).map(|view| view.workspace), Some(Workspace::Schematic));
    assert_eq!(h.state().views.view(board).map(|view| view.workspace), Some(Workspace::Board));
    assert_eq!(h.state().session.inspect(), before);
}

/// New-window workspace choices use the existing viewport lifecycle without an intermediate
/// split. Board selection highlights the schematic; child deletion/undo shares project history,
/// and closing one child leaves its peers intact.
#[test]
fn board_windows_open_directly_and_share_counterpart_highlights() {
    let (mut h, peers) = many_windows();
    let app = h.state_mut();
    app.run(pcbcraft_engine::PLACE_FOOTPRINT,&json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":0,"y_nm":0,"expected_revision":app.session.revision()}));
    h.run();
    let before = h.state().session.inspect();
    for _ in 0..2 {
        click(&mut h, "Choose workspace");
        click(&mut h, "Open PCB Layout in new window");
        h.get_by_label("Schematic canvas");
        assert!(!h.state().views.is_split());
    }
    let hosts = h.state().views.detached_hosts();
    assert_eq!(hosts.len(), 2);
    let child = h.state().views.hosts[&hosts[0]].focused;
    let peer = peers.borrow()[&editor_windows::viewport(hosts[0])].clone();
    child_rect(&peer, "Layout");
    h.get_by_label("Capture");
    let canvas = child_rect(&peer, "PCB canvas");
    child_click_at(&mut h, &peer, canvas.center());
    assert!(h.state().views.view(child).unwrap().board.selection.contains("footprint-3"));
    h.get_by_label("Capture");
    child_rect(&peer, "Layout");
    assert_eq!(h.state().views.highlight_for(ViewId(0), &h.state().session).single_symbol(), Some("symbol-2"));
    child_key(&mut h, &peer, Key::Delete, Modifiers::NONE);
    assert!(h.state().session.project().unwrap().boards().next().unwrap().footprints().next().is_none());
    assert_eq!(h.state().session.inspect()["project"]["schematic"], before["project"]["schematic"]);
    child_key(&mut h, &peer, Key::Z, Modifiers::COMMAND);
    assert_eq!(h.state().session.inspect()["project"], before["project"]);
    peer.borrow_mut().close = true;
    h.run();
    assert_eq!(h.state().views.detached_hosts(), vec![hosts[1]]);
    assert!(!h.state().views.is_split());
    assert!(h.state().views.highlight_for(ViewId(0), &h.state().session).is_empty());
    assert_eq!(h.state().session.inspect()["project"], before["project"]);
}

/// Arrangement naming belongs to the initiating host and must neither rehost nor close it.
#[test]
fn arrangement_forms_in_detached_windows_preserve_native_addresses() {
    let (mut h, peer) = multi_window();
    h.state_mut().integrated_titlebar = true;
    let source = h.state().views.active.id();
    h.state_mut().views.open_window(source).unwrap();
    h.run();
    let hosts: Vec<_> = h.state().views.hosts.keys().copied().collect();
    let snapshot = h.state().arrangement_snapshot().unwrap();
    let revision = h.state().session.revision();
    child_tab_menu(&mut h, &peer, "Arrangement Views");
    let at = child_rect(&peer, "Rename…").center();
    window_click(&mut h, Some(&peer), at);
    h.run();
    assert_eq!(h.state().arrangement_dialog_host(), Some(HostId(1)));
    assert!(peer.borrow().nodes.values().any(|node| node.role() == egui::accesskit::Role::TextInput));
    assert!(h.query_by_label("Arrangement name").is_none());
    assert!(h.get_by_label("File").accesskit_node().is_disabled());
    peer.borrow_mut().events.push(Event::Text("Detached review".into()));
    h.run();
    let rename = child_rect(&peer, "Rename").center();
    window_click(&mut h, Some(&peer), rename);
    assert!(!h.output().viewport_output.values().any(|output| output.commands.contains(&ViewportCommand::Close)));
    h.run();
    assert_eq!(h.state().arrangements.tabs()[0].1, "Detached review");
    assert_eq!(h.state().views.hosts.keys().copied().collect::<Vec<_>>(), hosts);
    assert_eq!(h.state().arrangement_snapshot().unwrap(), snapshot);
    assert_eq!(h.state().session.revision(), revision);

    child_tab_menu(&mut h, &peer, "Arrangement Detached review");
    for label in ["Duplicate…", "Create"] {
        let at = child_rect(&peer, label).center();
        window_click(&mut h, Some(&peer), at);
        h.run();
    }
    assert_eq!(h.state().arrangements.tabs().len(), 2);
    assert_eq!(h.state().views.hosts.keys().copied().collect::<Vec<_>>(), hosts);
    assert_eq!(h.state().arrangement_snapshot().unwrap(), snapshot);

    let at = child_rect(&peer, "New arrangement").center();
    window_click(&mut h, Some(&peer), at);
    h.run();
    assert_eq!(h.state().arrangement_dialog_host(), Some(HostId(1)));
    peer.borrow_mut().close = true;
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(h.state().arrangements.tabs().len(), 2);
    assert_eq!(h.state().views.hosts.len(), 1);
}

/// Open a tab's context menu with native secondary-button events in the addressed child.
fn child_tab_menu(h: &mut Harness<'_, PcbApp>, peer: &Rc<RefCell<Peer>>, label: &str) {
    focus(h, peer, true);
    let pos = child_rect(peer, label).center();
    peer.borrow_mut().events.push(Event::PointerMoved(pos));
    h.run();
    for pressed in [true, false] {
        peer.borrow_mut().events.push(Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run();
}

/// File adoption rebinds a detached PCB view and the main schematic together, while retaining
/// window membership and clearing outgoing document identities before layout capture.
#[test]
fn project_open_rebinds_detached_pcb_and_main_schematic() {
    let (mut h, peer) = multi_window();
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB Layout in new window");
    let child = h.state().views.hosts[&HostId(1)].focused;
    let stale = h.state().views.view(child).unwrap().input(&h.state().session).unwrap();
    let (_copy, path) = dual_led_copy();
    let revision = h.state().session.revision();
    h.state_mut().files.open_path_for_test(&path, revision);
    h.run();
    assert_eq!(h.state().views.detached_hosts(), vec![HostId(1)]);
    assert_eq!(h.state().views.view(child).unwrap().workspace, Workspace::Board);
    assert!(!h.state().views.view(child).unwrap().unavailable());
    assert!(!h.state().views.view(child).unwrap().accepts(stale, &h.state().session));
    assert_eq!(h.state().views.view(child).unwrap().board.target, Some(h.state().session.project().unwrap().board_target().unwrap()));
    h.get_by_label("Schematic canvas");
    child_rect(&peer, "PCB canvas");
    assert!(h.state().arrangement_snapshot().unwrap().valid());
    assert!(!h.state().files.dirty(&h.state().session));
}

/// M in a detached schematic starts only that view's pickup and shares one completed edit
/// with the main window's history. Closing a host or changing focus still expires its input.
#[test]
fn schematic_move_hotkey_in_detached_window_uses_its_own_view() {
    let (mut h, peer) = multi_window();
    detach(&mut h);
    let child = h.state().views.hosts[&HostId(1)].focused;
    let main = h.state().views.hosts[&HostId::ROOT].focused;
    let canvas = child_rect(&peer, "Schematic canvas");
    let origin = h.state().views.view(child).unwrap().camera.screen(Point { x_nm: 0, y_nm: 0 }, canvas);
    let before = h.state().session.snapshot().unwrap();
    child_click_at(&mut h, &peer, origin);
    child_key(&mut h, &peer, Key::M, Modifiers::NONE);
    assert!(matches!(h.state().views.view(child).unwrap().schematic.tool, Some(Tool::Move { disconnect: true, .. })));
    assert!(h.state().views.view(main).unwrap().schematic.tool.is_none());
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    child_click_at(&mut h, &peer, origin + vec2(60., 60.));
    assert_ne!(h.state().session.project().unwrap().symbols()[0].position, Point { x_nm: 0, y_nm: 0 });
    focus(&mut h, &peer, false);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// A report hosted by the root brings the source window forward after navigating to its pin,
/// without editing the document or changing which editor owns the target view.
#[test]
fn no_connect_show_focuses_the_detached_source_window() {
    let (mut h, peer) = multi_window();
    let p = h.state().session.project().unwrap();
    let target = p.no_connect_target(p.symbols()[0].terminals()[0]).unwrap();
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::no_connect::SET, &json!({"target":target,"marked":true,"expected_revision":revision}));
    detach(&mut h);
    focus(&mut h, &peer, true);
    let source = h.state().views.hosts[&HostId(1)].focused;
    h.state_mut().bind_editor_host(HostId(1));
    h.state_mut().inspect_no_connects();
    h.state_mut().bind_editor_host(HostId::ROOT);
    h.run();
    focus(&mut h, &peer, false);
    let before = h.state().session.snapshot().unwrap();
    h.get_by_label("Show R1 pin 1").click();
    h.step();
    assert!(matches!(h.state().dialog, Dialog::None));
    assert!(h.output().viewport_output[&editor_windows::viewport(HostId(1))].commands.contains(&ViewportCommand::Focus));
    assert_eq!(h.state().views.owner(source), Some(HostId(1)));
    assert_eq!(h.state().views.view(source).unwrap().schematic.selection.single_symbol(), Some(before.symbols()[0].id().as_str()));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Detached PCB tools use that host's input, cancel on focus loss, and commit through the shared
/// session. Quarter-turn UI steps also preserve a pre-existing non-quarter-turn orientation.
#[test]
fn footprint_transforms_in_detached_windows_share_history_and_cancel_on_focus_loss() {
    use pcbcraft_engine::{PLACE_FOOTPRINT, SET_FOOTPRINT_TRANSFORM, persistence::encode_documents};
    let (mut h, peer) = multi_window();
    let revision = h.state().session.revision();
    h.state_mut().run(
        PLACE_FOOTPRINT,
        &json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":0,"y_nm":0,"expected_revision":revision}),
    );
    let revision = h.state().session.revision();
    h.state_mut().run(SET_FOOTPRINT_TRANSFORM, &json!({"footprint":"footprint-3","x_nm":0,"y_nm":0,"rotation":45,"expected_revision":revision}));
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB Layout in new window");
    let child = h.state().views.hosts[&HostId(1)].focused;
    let canvas = child_rect(&peer, "PCB canvas");
    let origin = h.state().views.view(child).unwrap().camera.screen(Point { x_nm: 0, y_nm: 0 }, canvas);
    child_click_at(&mut h, &peer, origin);
    child_key(&mut h, &peer, Key::M, Modifiers::NONE);
    assert!(h.state().views.view(child).unwrap().board.draft.is_some());
    focus(&mut h, &peer, false);
    assert!(h.state().views.view(child).unwrap().board.draft.is_none());
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    child_key(&mut h, &peer, Key::M, Modifiers::NONE);
    let destination = origin + vec2(80., 60.);
    peer.borrow_mut().events.push(Event::PointerMoved(destination));
    h.run();
    child_key(&mut h, &peer, Key::R, Modifiers::NONE);
    child_click_at(&mut h, &peer, destination);
    assert_eq!(h.state().session.inspect()["project"]["board"]["footprints"][0]["rotation"], 135);
    assert_eq!(h.state().views.view(child).unwrap().camera.screen(Point { x_nm: 0, y_nm: 0 }, canvas), origin);
    focus(&mut h, &peer, false);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// A package chooser binds its originating detached editor; closing that view expires the form
/// without creating an assignment or granting the remaining window edit authority.
#[test]
fn closing_detached_editor_expires_footprint_assignment() {
    let (mut h, peer) = multi_window();
    detach(&mut h);
    let root = h.state().views.active.id();
    let child = h.state().views.hosts[&HostId(1)].focused;
    h.state_mut().views.bind(child);
    h.state_mut().begin_footprint_assignment(None);
    h.state_mut().views.bind(root);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::Footprint(_)));
    let before = h.state().session.inspect();
    peer.borrow_mut().close = true;
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(h.state().session.inspect(), before);
}

/// Edge previews belong to the drawing view; focus loss or closing that host cannot author them.
#[test]
fn detached_board_edge_preview_cancels_on_focus_loss_and_close() {
    use pcbcraft_engine::persistence::encode_documents;
    let (mut h, peer) = multi_window();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB Layout in new window");
    let child = h.state().views.hosts[&HostId(1)].focused;
    let layer = child_rect(&peer, "Activate layer Board Outline").center();
    child_click_at(&mut h, &peer, layer);
    let draw = child_rect(&peer, "Draw").center();
    child_click_at(&mut h, &peer, draw);
    let tool = child_rect(&peer, "Draw rectangle on active layer").center();
    child_click_at(&mut h, &peer, tool);
    let origin = child_rect(&peer, "PCB canvas").center();
    child_click_at(&mut h, &peer, origin);
    assert!(h.state().views.view(child).unwrap().board.drawing.source().is_some());
    focus(&mut h, &peer, false);
    assert!(h.state().views.view(child).unwrap().board.drawing.source().is_none());
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    child_click_at(&mut h, &peer, tool);
    child_click_at(&mut h, &peer, origin);
    assert!(h.state().views.view(child).unwrap().board.drawing.source().is_some());
    peer.borrow_mut().close = true;
    h.run();
    assert!(h.state().views.view(child).is_none());
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// A main-host definition pane cannot render an arrangement form owned by a detached project host.
#[test]
fn arrangement_dialog_stays_in_its_host_when_main_enters_a_library_workspace() {
    let (mut h, peer) = multi_window();
    let source = h.state().views.active.id();
    h.state_mut().views.open_window(source).unwrap();
    h.run();
    child_tab_menu(&mut h, &peer, "Arrangement Views");
    let at = child_rect(&peer, "Rename…").center();
    window_click(&mut h, Some(&peer), at);
    h.run();
    assert_eq!(h.state().arrangement_dialog_host(), Some(HostId(1)));
    h.state_mut().new_library_item(pcbcraft_engine::authoring::Kind::Symbol);
    h.run();
    assert!(h.state().views.active.definition.is_some());
    assert_eq!(h.state().arrangement_dialog_host(), Some(HostId(1)));
    assert!(h.query_by_label("Arrangement name").is_none());
    assert!(peer.borrow().nodes.values().any(|n| n.role() == egui::accesskit::Role::TextInput));
}

/// Parts Library obeys the same arrangement-form owner as project and definition hosts.
#[test]
fn review_arrangement_dialog_owner_in_parts_library() {
    let (mut h, peer) = multi_window();
    let source = h.state().views.active.id();
    h.state_mut().views.open_window(source).unwrap();
    h.run();
    child_tab_menu(&mut h, &peer, "Arrangement Views");
    let at = child_rect(&peer, "Rename…").center();
    window_click(&mut h, Some(&peer), at);
    h.run();
    let ctx = h.ctx.clone();
    h.state_mut().place_library_workspace(&ctx, pcbcraft_chrome::workspace::Placement::Here).unwrap();
    h.run();
    assert_eq!(h.state().arrangement_dialog_host(), Some(HostId(1)));
    assert!(h.query_by_label("Arrangement name").is_none());
    assert!(peer.borrow().nodes.values().any(|n| n.role() == egui::accesskit::Role::TextInput));
}

/// A detached host switches only its own catalog, preserving its query and the shared
/// document while the main host retains its independent catalog presentation.
#[test]
fn detached_catalog_switches_forms_without_moving_state_to_the_main_host() {
    let (mut h, peer) = multi_window();
    detach(&mut h);
    let before = h.state().session.snapshot().unwrap();
    child_key(&mut h, &peer, Key::A, Modifiers::NONE);
    peer.borrow_mut().events.push(Event::Text("cap".into()));
    h.run();
    let dock = child_rect(&peer, "Dock Parts Catalog").center();
    child_click_at(&mut h, &peer, dock);
    assert_eq!(h.state().host_panels(HostId(1)).catalog_mode, "compact");
    assert!(!h.state().parts.open);
    let expand = child_rect(&peer, "Expand Parts Catalog").center();
    child_click_at(&mut h, &peer, expand);
    assert_eq!(h.state().host_panels(HostId(1)).catalog_mode, "expanded");
    assert!(peer.borrow().nodes.values().any(|node| node.value() == Some("cap")));
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// A root dialog blocks an already-open child catalog in either presentation, including
/// placement, Enter and mode controls; closing the dialog returns the same catalog state.
#[test]
fn root_dialog_blocks_both_child_catalog_presentations() {
    for compact in [false, true] {
        let (mut h, peer) = multi_window();
        detach(&mut h);
        child_key(&mut h, &peer, Key::A, Modifiers::NONE);
        if compact {
            let dock = child_rect(&peer, "Dock Parts Catalog").center();
            child_click_at(&mut h, &peer, dock);
        }
        let child = h.state().views.hosts[&HostId(1)].focused;
        focus(&mut h, &peer, false);
        click(&mut h, "Preferences");
        let before = h.state().session.inspect();
        let mode = h.state().host_panels(HostId(1)).catalog_mode;
        let place = child_rect(&peer, "Place").center();
        child_click_at(&mut h, &peer, place);
        child_key(&mut h, &peer, Key::Enter, Modifiers::NONE);
        let switch = child_rect(&peer, if compact { "Expand Parts Catalog" } else { "Dock Parts Catalog" }).center();
        child_click_at(&mut h, &peer, switch);
        assert!(h.state().views.view(child).unwrap().schematic.tool.is_none());
        assert_eq!(h.state().host_panels(HostId(1)).catalog_mode, mode);
        assert!(h.state().host_panels(HostId(1)).parts);
        assert_eq!(h.state().session.inspect(), before);
        focus(&mut h, &peer, false);
        click(&mut h, "Cancel");
        assert_eq!(h.state().host_panels(HostId(1)).catalog_mode, mode);
    }
}

/// A physical footprint selected by an ordinary native-style left-click can be deleted from
/// that window's keyboard without any ribbon action or transfer to the main editor.
#[test]
fn detached_physical_footprint_click_then_delete_uses_its_own_input() {
    use pcbcraft_engine::persistence::encode_documents;
    for configured in [false, true] {
        let (mut h, peers) = many_windows();
        if configured {
            let revision = h.state().session.revision();
            h.state_mut().run(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":4,"expected_revision":revision}));
        }
        let symbol = h.state().session.project().unwrap().symbols()[0].id();
        let revision = h.state().session.revision();
        h.state_mut().run(
            pcbcraft_engine::PLACE_FOOTPRINT,
            &json!({"symbol":symbol,"definition":"resistor-0402","mapping":{"1":"1","2":"2"},"x_nm":0,"y_nm":0,"expected_revision":revision}),
        );
        h.run();
        click(&mut h, "Choose workspace");
        click(&mut h, "Open PCB Layout in new window");
        let host = h.state().views.detached_hosts()[0];
        let peer = peers.borrow()[&editor_windows::viewport(host)].clone();
        let view = h.state().views.hosts[&host].focused;
        let canvas = child_rect(&peer, "PCB canvas");
        let origin = h.state().views.view(view).unwrap().camera.screen(Point { x_nm: 0, y_nm: 0 }, canvas);
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let revision = h.state().session.revision();
        child_click_at(&mut h, &peer, origin);
        assert_eq!(h.state().views.view(view).unwrap().board.selection.len(), 1);
        child_key(&mut h, &peer, Key::Delete, Modifiers::NONE);
        assert_eq!(h.state().session.revision(), revision + 1);
        assert!(h.state().session.project().unwrap().boards().next().unwrap().footprints().next().is_none());
        child_key(&mut h, &peer, Key::Z, Modifiers::COMMAND);
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    }
}

/// E over a field renders an inline textbox in the native child, never the root host or a
/// new properties window. Enter commits to that child's captured symbol through shared history.
#[test]
fn detached_inline_text_remains_in_the_child_canvas() {
    let (mut h, peer) = multi_window();
    detach(&mut h);
    let id = h.state().views.hosts[&HostId(1)].focused;
    focus(&mut h, &peer, true);
    h.run();
    let point = peer
        .borrow()
        .output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(text) if text.galley.job.text == "R1" => Some(text.visual_bounding_rect().center()),
            _ => None,
        })
        .unwrap();
    peer.borrow_mut().events.push(Event::PointerMoved(point));
    h.run();
    child_key(&mut h, &peer, Key::E, Modifiers::NONE);
    assert!(matches!(h.state().dialog, Dialog::None));
    let rect = child_rect(&peer, "Edit Reference");
    assert!(child_rect(&peer, "Schematic canvas").contains_rect(rect));
    assert!(h.query_by_label("Edit Reference").is_none());
    let before = h.state().session.snapshot().unwrap();
    child_key(&mut h, &peer, Key::A, Modifiers::COMMAND);
    peer.borrow_mut().events.push(Event::Text("R42".into()));
    h.run();
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
    child_key(&mut h, &peer, Key::Enter, Modifiers::NONE);
    assert!(h.state().views.view(id).unwrap().schematic.text_edit.is_none());
    assert_eq!(h.state().session.project().unwrap().components().next().unwrap().reference(), "R42");
    child_key(&mut h, &peer, Key::Z, Modifiers::COMMAND);
    assert!(Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
}

/// Titles sent to the main native window, stepping one frame at a time so a command sent in an
/// intermediate frame isn't hidden behind `run`'s final output. Queue at most one event before
/// calling: kittest's `step` runs one frame per queued event and keeps only the last output.
fn root_titles_over(h: &mut Harness<'_, PcbApp>, frames: usize) -> Vec<String> {
    let mut titles = Vec::new();
    for _ in 0..frames {
        h.step();
        let Some(output) = h.output().viewport_output.get(&ViewportId::ROOT) else {
            continue;
        };
        titles.extend(output.commands.iter().filter_map(|command| match command {
            ViewportCommand::Title(title) => Some(title.clone()),
            _ => None,
        }));
    }
    titles
}

/// The main window's OS title names its focused document the way a detached window's does
/// (#220); it used to stay "PCBCraft", so taskbars couldn't tell two projects apart. Home shows
/// the app name alone, an unchanged title isn't resent every frame, and a main window that an
/// arrangement recreates after it was closed is titled again.
#[test]
fn main_window_title_names_its_document_like_a_detached_window() {
    let (mut h, peers) = many_windows();
    focus_host(&mut h, &peers, HostId::ROOT);
    let document = h.state().view_title(h.state().views.hosts[&HostId::ROOT].focused);
    assert!(document.starts_with(&h.state().session.project().unwrap().name), "{document}");
    let titled = format!("{document} — PCBCraft");
    h.get_by_label("Home").click();
    assert_eq!(root_titles_over(&mut h, 3), ["PCBCraft"]);
    assert!(h.state().home);
    let back = format!("Back to {}", h.state().session.project().unwrap().name);
    h.get_by_label(&back).click();
    assert_eq!(root_titles_over(&mut h, 3), [titled.as_str()]);
    assert!(!h.state().home);
    assert!(root_titles_over(&mut h, 3).is_empty(), "an unchanged title is not resent");

    let original = h.state().views.active.id();
    let child = h.state_mut().views.open_window(original).unwrap();
    h.run();
    let peer = peers.borrow()[&editor_windows::viewport(child)].clone();
    assert_eq!(peer.borrow().title.as_deref(), Some(titled.as_str()));

    close_root(&mut h);
    assert!(!h.state().views.hosts.contains_key(&HostId::ROOT));
    let ctx = h.ctx.clone();
    h.state_mut()
        .queue_arrangement(&ctx, crate::arrangements::Action::Create { name: "One window".into(), start: crate::arrangements::Start::SingleView });
    let titles = root_titles_over(&mut h, 4);
    assert!(h.state().views.hosts.contains_key(&HostId::ROOT), "the arrangement recreates the main window");
    assert_eq!(titles, [titled.as_str()]);
}
