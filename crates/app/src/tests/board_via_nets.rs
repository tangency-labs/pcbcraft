//! Via assignment acceptance uses ordinary placement/Properties controls and canvas input.
use super::board_tracks::{at, routable, vias};
use super::*;
use pcbcraft_engine::{persistence::encode_documents, schematic::Point};

/// Add a named singleton through the shared command so chooser rows use a stable readable name.
fn ground(h: &mut Harness<'_, PcbApp>) {
    let revision = h.state().session.revision();
    h.state_mut()
        .run(pcbcraft_engine::net_labels::ADD, &json!({"name":"GND","style":"global","x_nm":-50_000_000,"y_nm":0,"expected_revision":revision}));
    assert_eq!(h.state().message, None);
    h.run();
}
/// Choose net intent through the shared searchable form, then explicitly confirm.
fn choose(h: &mut Harness<'_, PcbApp>, field: &str, option: &str) {
    click(h, field);
    assert!(
        matches!(h.state().dialog, Dialog::ViaNet(_)),
        "dialog={:?} message={:?} via={}",
        h.state().dialog,
        h.state().message,
        h.state().views.active.board.tracks.via_tool
    );
    if option == "Automatic" {
        click(h, "Choose automatic net");
    } else {
        h.get_by_label("Search nets").type_text(option);
        h.run();
        click(h, &format!("Choose net {option}"));
    }
    click(h, "Use net");
    if field == "Via net" {
        settle_properties(h);
    }
}

/// Wait for an existing-via Properties edit, including no-ops, to leave preparation before
/// inspecting state or sending Undo. A pre-existing via does not imply publication is complete.
fn settle_properties(h: &mut Harness<'_, PcbApp>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while h.state().views.active.board.tracks.committing() {
        assert!(std::time::Instant::now() < deadline, "via Properties edit did not publish: {:?}", h.state().message);
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    h.run();
    assert_eq!(h.state().message, None);
}
/// Wait only for the already submitted bounded native preparation, without sending further input.
fn settle(h: &mut Harness<'_, PcbApp>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while h.state().views.active.board.tracks.busy() && vias(h).is_empty() {
        assert!(std::time::Instant::now() < deadline, "via preparation did not settle");
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    h.run();
}
/// Place a via with real canvas input and end repeat placement so ordinary selection is available.
fn place(h: &mut Harness<'_, PcbApp>) {
    let p = at(h, Point { x_nm: 5_080_000, y_nm: 5_080_000 });
    click_canvas_at(h, p);
    settle(h);
    assert_eq!(vias(h).len(), 1, "{:?}", h.state().message);
    h.key_press(Key::Escape);
    h.run();
}

/// Choosing a placement net authors nothing until the click; Properties clears/sets one undo step.
#[test]
fn via_assignment_controls_place_clear_and_restore_history() {
    for theme in [ThemeChoice::Dark, ThemeChoice::Light] {
        let mut h = routable();
        h.state_mut().preferences.theme = theme;
        ground(&mut h);
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        h.state_mut().start_via();
        h.run();
        choose(&mut h, "Via placement net", "GND");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
        place(&mut h);
        let via = vias(&h)[0].clone();
        assert!(via.net_assignment.is_some());
        let project = h.state().session.project().unwrap();
        let view = project.boards().next().unwrap();
        assert_eq!(view.copper_nets().unwrap()[&via.id], "GND");
        assert_eq!(view.inspect_vias()[0]["connected_to_pad"], false);
        h.state_mut().views.active.board.tracks.selected.insert(via.id);
        h.state_mut().views.active.board.properties_collapsed = false;
        h.run();
        let saved = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        choose(&mut h, "Via net", "Automatic");
        assert!(vias(&h)[0].net_assignment.is_none());
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run();
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), saved);
        choose(&mut h, "Via net", "GND");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), saved, "same identity is a no-op");
    }
}

/// Dismissing the search form or placement leaves native bytes/history untouched; new projects reset intent.
#[test]
fn via_assignment_cancel_and_project_replacement_preserve_authority() {
    let mut h = routable();
    ground(&mut h);
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    h.state_mut().start_via();
    h.run();
    click(&mut h, "Via placement net");
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    if !h.state().views.active.board.tracks.via_tool {
        h.state_mut().start_via();
        h.run();
    }
    choose(&mut h, "Via placement net", "GND");
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    h.state_mut().start_via();
    h.run();
    choose(&mut h, "Via placement net", "GND");
    assert!(h.state().views.active.board.tracks.via_tool);
    assert!(h.state().views.active.board.tracks.via_target.is_some());
    h.state_mut().run("project.new", &json!({"create_board":true}));
    h.run();
    assert!(!h.state().views.active.board.tracks.via_tool);
    assert!(h.state().views.active.board.tracks.via_target.is_none());
}

/// The fourth ribbon field participates in the existing compact wrapping layout in both themes.
#[test]
fn via_net_picker_fits_compact_and_wide_ribbons() {
    for (size, theme) in [(egui::vec2(1600., 1000.), ThemeChoice::Dark), (egui::vec2(700., 700.), ThemeChoice::Light)] {
        let mut h = routable();
        ground(&mut h);
        h.set_size(size);
        h.state_mut().preferences.theme = theme;
        h.run();
        h.state_mut().start_via();
        h.run();
        assert!(fully_visible(&h, "Via placement net"));
        choose(&mut h, "Via placement net", "GND");
        assert!(h.state().views.active.board.tracks.via_target.is_some());
        assert!(fully_visible(&h, "Via placement net"));
    }
}

/// A split peer keeps its own placement policy while both panes inspect the committed via identity.
#[test]
fn split_via_placement_choices_stay_view_local() {
    let mut h = routable();
    ground(&mut h);
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB Layout beside current view");
    let views = h.state().views.hosts[&crate::editor_layout::HostId::ROOT].layout.visible_views();
    assert_eq!(views.len(), 2);
    let ctx = h.ctx.clone();
    h.state_mut().activate_editor(&ctx, views[0]);
    h.run();
    h.state_mut().start_via();
    h.run();
    choose(&mut h, "Via placement net", "GND");
    assert!(h.state().views.view(views[0]).unwrap().board.tracks.via_target.is_some());
    assert!(h.state().views.view(views[1]).unwrap().board.tracks.via_target.is_none());
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.state_mut().activate_editor(&ctx, views[1]);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    h.state_mut().start_via();
    h.run();
    assert!(h.state().views.active.board.tracks.via_target.is_none());
}

/// A Properties chooser cannot apply to a selection replaced while the form is open.
#[test]
fn via_net_properties_reject_replaced_selection() {
    let mut h = routable();
    ground(&mut h);
    h.state_mut().start_via();
    h.run();
    place(&mut h);
    let id = vias(&h)[0].id;
    h.state_mut().views.active.board.tracks.selected.insert(id);
    h.state_mut().views.active.board.properties_collapsed = false;
    h.run();
    click(&mut h, "Via net");
    click(&mut h, "Choose net GND");
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.state_mut().views.active.board.tracks.selected.clear();
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// A focus-loss width commit wins a same-frame net-button click; reopening after publication
/// captures the resulting revision, so neither the numeric edit nor a stale chooser can win later.
#[test]
fn width_commit_precedes_same_frame_via_net_picker() {
    let mut h = routable();
    h.state_mut().preferences.units = pcbcraft_chrome::units::Unit::Mm;
    ground(&mut h);
    h.state_mut().start_via();
    h.run();
    place(&mut h);
    let layer = h.state().views.active.board.active_layer.clone().unwrap();
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::ADD_TRACKS,
        &json!({"layer":layer,"width_nm":250_000,
        "points":[{"x_nm":15_000_000,"y_nm":10_000_000},{"x_nm":20_000_000,"y_nm":10_000_000}],"expected_revision":revision}),
    );
    let via = vias(&h)[0].id;
    let track = super::board_tracks::tracks(&h)[0].id;
    h.state_mut().views.active.board.tracks.selected = std::collections::BTreeSet::from([track, via]);
    h.state_mut().views.active.board.properties_collapsed = false;
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    click(&mut h, "Selected track width");
    h.get_by_label("Selected track width").type_text("0.4");
    h.run();
    click(&mut h, "Via net");
    assert!(matches!(h.state().dialog, Dialog::None), "finish the numeric commit before opening a modal");
    settle_properties(&mut h);
    assert_eq!(h.state().session.revision(), revision + 1);
    assert_eq!(super::board_tracks::tracks(&h)[0].width_nm, 400_000);
    let resized = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    choose(&mut h, "Via net", "GND");
    assert!(vias(&h)[0].net_assignment.is_some());
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), resized);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}
