//! Mixed workspace interactions use the real picker, canvas and shared session.
use super::*;
use crate::{editor_view::Workspace, preferences::SelectionFollow};
use pcbcraft_engine::{persistence::encode_documents, schematic::Point};

/// Create a resolved starter placement through the same engine commands as MCP. Panels are
/// closed so point clicks exercise the canvas instead of an overlaid card.
fn board_harness<'a>() -> Harness<'a, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400.0, 900.0));
    resistor_at_origin(&mut h);
    let app = h.state_mut();
    app.browser.open = false;
    app.run(
        pcbcraft_engine::PLACE_FOOTPRINT,
        &json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":2,"x_nm":0,"y_nm":0,"expected_revision":2}),
    );
    h.run();
    h
}

/// Workspace switching restores independent cameras, keeps input invalid after switching back,
/// and uses the parked source selection for highlighting without touching authored data.
#[test]
fn switching_workspaces_restores_navigation_and_rejects_stale_schematic_input() {
    let mut h = board_harness();
    click_canvas(&mut h);
    let old = h.state().views.active.input(&h.state().session).unwrap();
    let before = h.state().session.inspect();
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let origin = Point { x_nm: 0, y_nm: 0 };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400., 400.));
    let initial = h.state().views.active.camera.screen(origin, rect);
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    assert_eq!(h.state().views.active.workspace, Workspace::Board);
    assert_eq!(h.state().views.board_highlight_for(ViewId(0), &h.state().session), std::collections::BTreeSet::from(["footprint-3".into()]));
    assert!(h.state().views.active.board.selection.is_empty());
    h.state_mut().views.active.camera.center_on(Point { x_nm: 50_000_000, y_nm: 10_000_000 });
    let board_camera = h.state().views.active.camera.screen(origin, rect);
    for key in [Key::Delete, Key::A, Key::W, Key::R, Key::E] {
        h.key_press(key);
        h.run();
    }
    assert_eq!(h.state().session.inspect(), before);
    click(&mut h, "Choose workspace");
    click(&mut h, "Schematic");
    assert_eq!(h.state().views.active.camera.screen(origin, rect), initial);
    assert!(!h.state().views.active.accepts(old, &h.state().session));
    h.state_mut().view_click(old, Click::Edit(SymbolAction::Delete, "symbol-2".into()));
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    assert_eq!(h.state().views.active.camera.screen(origin, rect), board_camera);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    assert_eq!(h.state().session.inspect(), before);
}

/// Opening the board beside a schematic retains both targets. Selection paints actual counterpart
/// ink in each direction; receiver edit selections and camera transforms remain independent.
#[test]
fn mixed_panes_highlight_both_ways_without_granting_schematic_edit_authority() {
    let mut h = board_harness();
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB layout beside current view");
    h.get_by_label("Schematic canvas 1");
    h.get_by_label("PCB canvas 3");
    assert_eq!(h.state().views.view(ViewId(0)).unwrap().workspace, Workspace::Schematic);
    let before = h.state().session.inspect();
    let at = h.get_by_label("Schematic canvas 1").rect().center();
    click_canvas_at(&mut h, at);
    assert!(h.state().views.board_highlight_for(ViewId(2), &h.state().session).contains("footprint-3"));
    let board_rect = h.get_by_label("PCB canvas 3").rect();
    assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text=="R1" && board_rect.contains(text.pos) && text.fallback_color==Tokens::get().board_highlight)));
    h.key_press(Key::Escape);
    h.run();
    click_canvas_at(&mut h, board_rect.center());
    assert_eq!(h.state().views.active.id(), ViewId(2));
    assert!(h.state().views.active.board.selection.contains("footprint-3"));
    assert_eq!(h.state().views.highlight_for(ViewId(0), &h.state().session).single_symbol(), Some("symbol-2"));
    assert!(h.state().views.view(ViewId(0)).unwrap().schematic.selection.is_empty());
    let schematic_rect = h.get_by_label("Schematic canvas 1").rect();
    assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text=="R1" && schematic_rect.contains(text.pos) && text.fallback_color==Tokens::get().schematic_highlight)));
    h.key_press(Key::Delete);
    h.run();
    assert_eq!(h.state().session.inspect(), before);
    let snapshot = h.state().session.snapshot().unwrap();
    h.state_mut().session.open_snapshot(snapshot);
    h.run();
    assert!(h.state().views.active.board.selection.is_empty());
    assert!(h.state().views.highlight_for(ViewId(0), &h.state().session).is_empty());
}

/// Following a schematic selection pans using board coordinates and preserves zoom. The default
/// leaves the board camera alone, and a consumed reveal does not undo subsequent manual navigation.
#[test]
fn mixed_follow_uses_destination_geometry_and_keeps_zoom() {
    for policy in [SelectionFollow::Off, SelectionFollow::Both, SelectionFollow::SecondaryFollowsPrimary, SelectionFollow::PrimaryFollowsSecondary] {
        let mut h = board_harness();
        click(&mut h, "Choose workspace");
        click(&mut h, "Open PCB layout beside current view");
        h.state_mut().preferences.selection_follow = policy;
        h.state_mut().views.view_mut(ViewId(2)).unwrap().camera.center_on(Point { x_nm: 1_000_000_000, y_nm: 0 });
        h.run();
        let rect = h.get_by_label("PCB canvas 3").rect();
        let origin = Point { x_nm: 0, y_nm: 0 };
        let before = h.state().views.view(ViewId(2)).unwrap().camera.screen(origin, rect);
        let zoom = h.state().views.view(ViewId(2)).unwrap().camera.zoom();
        let at = h.get_by_label("Schematic canvas 1").rect().center();
        click_canvas_at(&mut h, at);
        let after = h.state().views.view(ViewId(2)).unwrap().camera.screen(origin, rect);
        if matches!(policy, SelectionFollow::Both | SelectionFollow::SecondaryFollowsPrimary) {
            assert_ne!(after, before);
        } else {
            assert_eq!(after, before);
        }
        assert_eq!(h.state().views.view(ViewId(2)).unwrap().camera.zoom(), zoom);
        h.state_mut().views.view_mut(ViewId(2)).unwrap().camera.center_on(Point { x_nm: 1_000_000_000, y_nm: 0 });
        h.run();
        assert_eq!(h.state().views.view(ViewId(2)).unwrap().camera.screen(origin, rect), before);
    }
}

/// The FigCraft panel arrangement exposes real placement data. Visibility survives a layer
/// rename by ID, duplicates inherit camera/layers, and subsequent changes stay view-local.
#[test]
fn board_browser_properties_and_layers_share_stable_addresses() {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    resistor_at_origin(&mut h);
    let app = h.state_mut();
    app.run(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":2}));
    app.run(
        pcbcraft_engine::PLACE_FOOTPRINT,
        &json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":20_000_000,"y_nm":0,"expected_revision":3}),
    );
    h.run();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    let before = h.state().session.inspect();
    click(&mut h, "R1  1k");
    h.get_by_label("Reference");
    h.get_by_label("generic-through-hole-2");
    let target = h.state().views.active.board.target.unwrap();
    let top = h
        .state()
        .session
        .project()
        .unwrap()
        .board_view(target)
        .unwrap()
        .setup()
        .unwrap()
        .layers()
        .iter()
        .find(|layer| layer.name == "Top")
        .unwrap()
        .id
        .to_string();
    click(&mut h, "Show layer Top");
    assert!(h.state().views.active.board.hidden_layers.contains(&top));
    assert_eq!(h.state().session.inspect(), before);
    let mut setup = before["project"]["board"]["setup"].clone();
    for layer in setup["layers"].as_array_mut().unwrap() {
        if layer["id"] == top {
            layer["name"] = json!("Signal top");
        }
    }
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::SET_BOARD_SETUP, &json!({"setup":setup,"board":target.board,"expected_revision":revision}));
    h.run();
    assert_eq!(h.get_by_label("Show layer Signal top").accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB layout beside current view");
    let original = h.state().views.view(ViewId(0)).unwrap();
    let duplicate = h.state().views.view(ViewId(2)).unwrap();
    let point = Point { x_nm: 0, y_nm: 0 };
    assert_eq!(original.camera.screen(point, egui::Rect::ZERO), duplicate.camera.screen(point, egui::Rect::ZERO));
    assert_eq!(original.board.target, duplicate.board.target);
    assert_eq!(original.board.hidden_layers, duplicate.board.hidden_layers);
    assert!(duplicate.board.selection.is_empty());
    click(&mut h, "Show layer Signal top");
    assert!(!h.state().views.active.board.hidden_layers.contains(&top));
    assert!(h.state().views.view(ViewId(2)).unwrap().board.hidden_layers.contains(&top));
    click(&mut h, "Collapse Layers");
    assert!(h.query_by_label("Show layer Signal top").is_none());
    click(&mut h, "Expand Layers");
    h.get_by_label("Show layer Signal top");
}

/// PCB uses its own near-black canvas and readable ink with either chrome theme; switching
/// back restores schematic controls. Neither palette nor grid opacity changes document state.
#[test]
fn board_canvas_and_ribbon_remain_distinct_from_schematic() {
    for theme in [crate::preferences::ThemeChoice::Dark, crate::preferences::ThemeChoice::Light] {
        let mut h = board_harness();
        h.state_mut().preferences.theme = theme;
        h.run();
        let before = h.state().session.inspect();
        click(&mut h, "Choose workspace");
        click(&mut h, "PCB layout");
        h.get_by_label("Layout");
        assert!(h.query_by_label("Capture").is_none());
        let rect = h.get_by_label("PCB canvas").rect();
        assert!(
            h.output()
                .shapes
                .iter()
                .any(|shape| matches!(&shape.shape,egui::Shape::Rect(fill) if fill.rect==rect && fill.fill==Tokens::get().board_canvas))
        );
        assert!(h.output().shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text=="R1" && rect.contains(text.pos) && text.fallback_color==Tokens::get().board_ink)));
        h.state_mut().preferences.grid_opacity_percent = 0;
        h.run();
        assert_eq!(h.state().session.inspect(), before);
        click(&mut h, "Choose workspace");
        click(&mut h, "Schematic");
        h.get_by_label("Capture");
        assert!(h.query_by_label("Layout").is_none());
    }
}

/// Keyboard navigation reaches available workspaces and placement actions without leaking keys
/// into the canvas. Planned workspaces are skipped when moving between rows.
#[test]
fn workspace_keyboard_navigation_reaches_board_and_placement_actions() {
    let mut h = board_harness();
    let before = h.state().session.inspect();
    click(&mut h, "Choose workspace");
    h.key_press(Key::ArrowDown);
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().views.active.workspace, Workspace::Board);
    click(&mut h, "Choose workspace");
    h.key_press(Key::ArrowDown);
    h.run();
    h.key_press(Key::Space);
    h.run();
    assert_eq!(h.state().views.active.workspace, Workspace::Schematic);
    click(&mut h, "Choose workspace");
    let card = h.get_by_label("PCB layout").rect();
    for label in ["Open PCB layout beside current view", "Open PCB layout in new window"] {
        let icon = h.get_by_label(label).rect();
        assert!(icon.top() >= card.top() && icon.bottom() <= card.bottom());
        assert!(icon.left() >= card.right());
    }
    assert!(h.query_by_label("Open beside").is_none());
    assert!(h.query_by_label("New window").is_none());
    h.get_by_label("Open PCB layout beside current view").focus();
    h.run();
    h.key_press(Key::Enter);
    h.run();
    h.get_by_label("Schematic canvas 1");
    h.get_by_label("PCB canvas 3");
    assert_eq!(h.state().session.inspect(), before);
}

/// Either mixed pane closes through its own tab button with either workspace's panels visible,
/// including stacked panes in narrow windows. The survivor retains its camera and session,
/// and returning to one view removes the extra pane header.
#[test]
fn mixed_pane_close_buttons_preserve_the_surviving_view() {
    for size in [egui::vec2(1400.0, 900.0), egui::vec2(760.0, 600.0)] {
        for focused in [ViewId(0), ViewId(2)] {
            for closed in [ViewId(0), ViewId(2)] {
                let mut h = board_harness();
                h.state_mut().browser.open = true;
                h.set_size(size);
                click(&mut h, "Choose workspace");
                click(&mut h, "Open PCB layout beside current view");
                let path = h.state().views.view(focused).unwrap().sheet_path(&h.state().session).unwrap();
                click(&mut h, &path);
                assert_eq!(h.state().views.active.id(), focused);
                let remaining = if closed == ViewId(0) { ViewId(2) } else { ViewId(0) };
                let origin = Point { x_nm: 0, y_nm: 0 };
                h.state_mut().views.view_mut(remaining).unwrap().camera.center_on(Point { x_nm: 10_000_000, y_nm: 5_000_000 });
                h.run();
                let camera = h.state().views.view(remaining).unwrap().camera.screen(origin, egui::Rect::ZERO);
                let before = h.state().session.inspect();
                let path = h.state().views.view(closed).unwrap().sheet_path(&h.state().session).unwrap();
                click(&mut h, &format!("Close view {path}"));
                assert!(h.state().views.view(closed).is_none());
                assert_eq!(h.state().views.active.id(), remaining);
                assert!(!h.state().views.is_split());
                assert_eq!(h.state().views.active.camera.screen(origin, egui::Rect::ZERO), camera);
                assert_eq!(h.state().session.inspect(), before);
                let path = h.state().views.active.sheet_path(&h.state().session).unwrap();
                assert!(h.query_by_label(&format!("Close view {path}")).is_none());
            }
        }
    }
}
