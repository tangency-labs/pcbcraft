//! Filter changes must govern every selection path without becoming document edits.
use super::*;
use crate::{
    editor_view::{ViewId, Workspace},
    selection_filters::Filters,
};
use pcbcraft_engine::{
    DRAW_WIRE,
    persistence::encode_documents,
    schematic::{GRID_NM, Point},
    selection::Selection,
};

/// The production startup path shows filters by default; coordinate-test fixtures opt out explicitly.
#[test]
fn a_new_project_shows_the_selection_filter_card_by_default() {
    let mut h = startup_harness();
    new_project_tile(&h).click();
    h.run();
    h.get_by_label("Collapse Selection filters");
    assert!(h.state().views.active.filters_open);
    assert_eq!(h.state().views.active.filters, Filters::default());
}

/// An original symbol and a distant wire keep selection tests independent of contact shortcuts.
fn fixture() -> Harness<'static, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    h.state_mut().views.active.filters_open = true;
    resistor_at_origin(&mut h);
    h.state_mut().browser.open = false;
    let revision = h.state().session.revision();
    h.state_mut().run(
        DRAW_WIRE,
        &json!({"from":{"x_nm":-10*GRID_NM,"y_nm":10*GRID_NM},"to":{"x_nm":10*GRID_NM,"y_nm":10*GRID_NM},"bends":[],"expected_revision":revision}),
    );
    h.run();
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    h
}

/// Use the current pane transform, since Fit accounts for the new filter card.
fn at(h: &Harness<'_, PcbApp>, x: i64, y: i64) -> egui::Pos2 {
    let label = if h.state().views.active.workspace == Workspace::Board { "PCB canvas" } else { "Schematic canvas" };
    h.state().views.active.camera.screen(Point { x_nm: x * GRID_NM, y_nm: y * GRID_NM }, h.get_by_label(label).rect())
}

/// The snapshot includes saved bytes, revision, dirty state and both directions of history.
#[derive(Debug, PartialEq)]
struct DocumentState {
    bytes: Vec<(String, Vec<u8>)>,
    revision: u64,
    dirty: bool,
    undo: bool,
    redo: bool,
}

/// Capture document state independently of any current view or selection.
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

/// A full canvas box selects only enabled categories; disabled symbols cannot be moved by hover M.
#[test]
fn checkbox_filters_click_box_and_hover_shortcuts_without_changing_the_document() {
    let mut h = fixture();
    let before = document(&h);
    click(&mut h, "Select symbols");
    assert!(!h.state().views.active.filters.symbols);
    let origin = at(&h, 0, 0);
    click_canvas_at(&mut h, origin);
    assert!(h.state().views.active.schematic.selection.is_empty());
    h.hover_at(origin);
    h.key_press(Key::M);
    h.run();
    assert!(h.state().views.active.schematic.tool.is_none());
    let from = at(&h, -15, -5);
    let to = at(&h, 15, 15);
    h.drag_at(from);
    h.run();
    h.hover_at(to);
    h.run();
    h.drop_at(to);
    h.run();
    assert_eq!(h.state().views.active.schematic.selection.wires.len(), 1);
    assert!(h.state().views.active.schematic.selection.symbols.is_empty());
    click(&mut h, "Select wires");
    assert!(h.state().views.active.schematic.selection.is_empty());
    let wire = at(&h, 0, 10);
    click_canvas_at(&mut h, wire);
    assert!(h.state().views.active.schematic.selected_leg.is_none());
    assert!(h.state().views.active.schematic.selection.is_empty());
    click(&mut h, "Reset");
    assert_eq!(h.state().views.active.filters, Filters::default());
    click(&mut h, "All items");
    assert!(
        !h.state().views.active.filters.symbols
            && !h.state().views.active.filters.wires
            && !h.state().views.active.filters.labels
            && !h.state().views.active.filters.sheets
    );
    click(&mut h, "All items");
    assert_eq!(h.state().views.active.filters, Filters::default());
    assert_eq!(document(&h), before);
    assert!(h.get_by_label("Select pins").accesskit_node().is_disabled());
}

/// Disabling a category invalidates an existing move and clears its destructive edit selection.
#[test]
fn disabling_a_category_cancels_captured_input_before_its_release() {
    let mut h = fixture();
    let before = document(&h);
    let origin = at(&h, 0, 0);
    h.hover_at(origin);
    h.key_press(Key::M);
    h.run();
    assert!(h.state().views.active.schematic.tool.is_some());
    let id = h.state().views.active.id();
    h.state_mut().set_selection_filters(id, &json!({"symbols":false})).unwrap();
    let destination = at(&h, 20, 0);
    click_canvas_at(&mut h, destination);
    h.key_press(Key::Delete);
    h.run();
    assert!(h.state().views.active.schematic.tool.is_none());
    assert!(h.state().views.active.schematic.selection.is_empty());
    assert_eq!(document(&h), before);
}

/// Forking, detach/dock and serialized local arrangements preserve policy with fresh input authority.
#[test]
fn filters_follow_the_view_and_restore_from_local_arrangements() {
    let mut h = fixture();
    let id = h.state().views.active.id();
    h.state_mut().set_selection_filters(id, &json!({"symbols":false,"collapsed":true})).unwrap();
    let state = h.state().views.active.saved_state(&h.state().session).unwrap();
    let json = serde_json::to_value(&state).unwrap();
    let restored: crate::editor_view::ViewState = serde_json::from_value(json.clone()).unwrap();
    let view = EditorView::from_state(ViewId(900), &restored, &h.state().session);
    assert!(!view.filters.symbols && view.filters_collapsed);
    let fork = h.state_mut().views.active.fork(ViewId(901));
    assert_eq!(fork.filters, view.filters);
    let app = h.state_mut();
    app.views.switch_workspace(id, Workspace::Board, &app.session);
    app.set_selection_filters(id, &json!({"footprints":false})).unwrap();
    app.views.switch_workspace(id, Workspace::Schematic, &app.session);
    assert!(!app.views.active.filters.symbols && !app.views.active.filters.footprints && app.views.active.filters_collapsed);
    app.views.active.bind_opened_project(&app.session);
    assert!(!app.views.active.filters.symbols && !app.views.active.filters.footprints);
    let child = h.state_mut().views.open_beside(id).unwrap();
    h.state_mut().set_selection_filters(child, &json!({"symbols":true})).unwrap();
    assert!(!h.state().views.view(id).unwrap().filters.symbols);
    assert!(h.state().views.view(child).unwrap().filters.symbols);
    let host = h.state_mut().views.detach_view(child).unwrap();
    assert_eq!(h.state().views.hosts[&host].focused, child);
    assert!(h.state().views.view(child).unwrap().filters.symbols);
    h.state_mut().views.dock_view(child);
    assert!(h.state().views.view(child).unwrap().filters.symbols);
    let mut legacy = json;
    legacy.as_object_mut().unwrap().remove("filters");
    legacy.as_object_mut().unwrap().remove("filters_collapsed");
    legacy.as_object_mut().unwrap().remove("filters_open");
    let legacy = serde_json::from_value(legacy).unwrap();
    assert_eq!(EditorView::from_state(ViewId(902), &legacy, &h.state().session).filters, Filters::default());
    assert!(EditorView::from_state(ViewId(902), &legacy, &h.state().session).filters_open);
}

/// View and live window toggles hide only the card, preserving filters and electrical history.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn filter_card_visibility_uses_view_and_live_panel_controls() {
    let mut h = fixture();
    let before = document(&h);
    click(&mut h, "Select symbols");
    click(&mut h, "View");
    click(&mut h, "Toggle Selection filters");
    assert!(h.query_by_label("Collapse Selection filters").is_none());
    assert!(!h.state().views.active.filters.symbols);
    click(&mut h, "Toggle Selection filters");
    h.get_by_label("Collapse Selection filters");
    let ctx = h.ctx.clone();
    let host = h.state().views.host.0;
    h.state_mut().run_window_command(&ctx, "window.set_panel", &json!({"window":host,"panel":"selection_filters","open":false})).unwrap();
    h.run();
    assert!(h.query_by_label("Collapse Selection filters").is_none());
    let view = h.state().views.active.id().0;
    h.state_mut().run_window_command(&ctx, "view.set_selection_filters", &json!({"view":view,"open":true})).unwrap();
    h.run();
    h.get_by_label("Collapse Selection filters");
    assert_eq!(document(&h), before);
}

/// Live MCP and GUI share state; a mixed invalid request fails atomically and disabled selection is refused.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_filters_match_the_card_and_refuse_unsupported_or_filtered_targets() {
    let mut h = fixture();
    let ctx = h.ctx.clone();
    let id = h.state().views.active.id().0;
    let before = document(&h);
    h.state_mut().run_window_command(&ctx, "view.set_selection_filters", &json!({"view":id,"symbols":false})).unwrap();
    h.run();
    assert_eq!(h.get_by_label("Select symbols").accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));
    assert_eq!(crate::window_commands::inspect_editor(h.state())["views"][0]["selection_filters"]["symbols"], false);
    assert!(h.state_mut().run_window_command(&ctx, "view.select", &json!({"view":id,"items":{"symbols":["symbol-2"],"wires":[]}})).is_err());
    let filters = h.state().views.active.filters;
    for bad in [json!({"view":id,"symbols":true,"footprints":false}), json!({"view":id,"symbols":"false"}), json!({"view":999,"symbols":false})] {
        assert!(h.state_mut().run_window_command(&ctx, "view.set_selection_filters", &bad).is_err());
        assert_eq!(h.state().views.active.filters, filters);
    }
    h.state_mut().select(Selection::one_symbol("symbol-2".into()));
    assert!(h.state().views.active.schematic.selection.is_empty());
    click(&mut h, "Collapse Selection filters");
    h.get_by_label("Expand Selection filters");
    h.state_mut().run_window_command(&ctx, "view.set_selection_filters", &json!({"view":id,"collapsed":false,"symbols":true})).unwrap();
    h.run();
    h.get_by_label("Collapse Selection filters");
    assert!(h.state().views.active.filters.symbols);
    assert_eq!(document(&h), before);
}

/// Filtering footprints leaves their rendered pads visible and prevents clicks, hover moves and MCP picks.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn board_filters_prevent_footprint_selection_and_keep_graphics_independent() {
    let mut h = fixture();
    let revision = h.state().session.revision();
    h.state_mut().run("pcb.create_board", &json!({"expected_revision":revision}));
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::PLACE_FOOTPRINT,
        &json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":2,"x_nm":0,"y_nm":0,"expected_revision":revision}),
    );
    h.run();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    let before = document(&h);
    let origin = at(&h, 0, 0);
    click_canvas_at(&mut h, origin);
    assert_eq!(h.state().views.active.board.selection.len(), 1);
    click(&mut h, "Select footprints");
    assert!(h.state().views.active.board.selection.is_empty());
    let origin = at(&h, 0, 0);
    click_canvas_at(&mut h, origin);
    h.hover_at(origin);
    h.key_press(Key::M);
    h.run();
    assert!(h.state().views.active.board.selection.is_empty() && h.state().views.active.board.draft.is_none());
    assert!(h.state().views.active.filters.graphics);
    assert!(h.get_by_label("Select pads").accesskit_node().is_disabled());
    let ctx = h.ctx.clone();
    let id = h.state().views.active.id().0;
    let footprint = h.state().session.project().unwrap().boards().next().unwrap().footprints().next().unwrap().id;
    assert!(h.state_mut().run_window_command(&ctx, "view.select_footprints", &json!({"view":id,"footprints":[footprint]})).is_err());
    let board = h.state().views.active.id();
    let schematic = h.state_mut().views.open_beside(board).unwrap();
    let app = h.state_mut();
    app.views.switch_workspace(schematic, Workspace::Schematic, &app.session);
    app.views.bind(schematic);
    app.select(Selection::one_symbol("symbol-2".into()));
    assert!(!app.views.board_highlight_for(board, &app.session).is_empty());
    assert!(app.views.view(board).unwrap().board.selection.is_empty());
    app.views.bind(board);
    assert_eq!(document(&h), before);
}

/// Compact cards scroll to every supported category and retain working collapse/reset controls.
#[test]
fn compact_filter_rows_remain_reachable_and_do_not_overlap_properties() {
    let mut h = fixture();
    h.set_size(egui::vec2(420., 900.));
    h.state_mut().properties_open = true;
    h.run();
    let before = document(&h);
    let property = h.get_by_label("Collapse Properties").rect();
    let filters = h.get_by_label("Collapse Selection filters").rect();
    assert!(property.bottom() < filters.top());
    assert!(h.get_by_label("Reset").rect().right() <= filters.right() + 1.0, "Reset must stay inside the narrow card");
    h.get_by_label("Select sheets").scroll_to_me();
    h.run();
    click(&mut h, "Select sheets");
    assert!(!h.state().views.active.filters.sheets);
    click(&mut h, "Collapse Selection filters");
    assert!(h.state().views.active.filters_collapsed);
    click(&mut h, "Expand Selection filters");
    h.get_by_label("Reset").scroll_to_me();
    h.run();
    click(&mut h, "Reset");
    assert_eq!(h.state().views.active.filters, Filters::default());
    assert_eq!(document(&h), before);
}

/// Full-row filter controls retain keyboard activation and checkbox state for assistive tools.
#[test]
fn filter_checkboxes_support_keyboard_activation() {
    let mut h = fixture();
    let before = document(&h);
    h.get_by_label("Select symbols").focus();
    h.run();
    h.key_press(Key::Space);
    h.run();
    assert!(!h.state().views.active.filters.symbols);
    assert_eq!(h.get_by_label("Select symbols").accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));
    h.key_press(Key::Space);
    h.run();
    assert!(h.state().views.active.filters.symbols);
    assert_eq!(document(&h), before);
}

/// Named anchors follow their visible category, clear disabled selection and previews, and
/// ignore hover edit shortcuts without altering the electrical document.
#[test]
fn named_labels_and_power_symbols_follow_their_selection_categories() {
    use pcbcraft_engine::net_labels::{self as engine, Style};
    for style in [Style::Local, Style::Global, Style::Power, Style::Ground] {
        let mut h = fixture();
        let id = h.state().views.active.id();
        let revision = h.state().session.revision();
        h.state_mut().run(engine::ADD, &json!({"name":"FILTER_TEST","style":style,"x_nm":0,"y_nm":-8*GRID_NM,"expected_revision":revision}));
        h.run();
        assert!(h.state().message.is_none(), "{:?}", h.state().message);
        let label = h.state().session.project().unwrap().net_labels()[0].clone();
        let before = document(&h);
        let symbols = matches!(style, Style::Power | Style::Ground);
        let category = if symbols { "symbols" } else { "labels" };
        let other = if symbols { "labels" } else { "symbols" };
        h.state_mut().select(pcbcraft_engine::selection::Selection::one_net_label(label.id));
        h.state_mut().set_selection_filters(id, &json!({other:false})).unwrap();
        assert_eq!(h.state().views.active.schematic.selection.single_net_label(), Some(label.id));
        let revision = h.state().session.revision();
        h.state_mut().views.active.schematic.tool = Some(Tool::NetLabel { label: label.clone(), revision, drag: None });
        h.state_mut().set_selection_filters(id, &json!({category:false})).unwrap();
        assert!(h.state().views.active.schematic.selection.single_net_label().is_none());
        assert!(h.state().views.active.schematic.tool.is_none());
        let point = at(&h, 0, -8);
        h.hover_at(point);
        h.run();
        for key in [Key::E, Key::R, Key::M, Key::Delete] {
            h.key_press(key);
            h.run();
            assert!(matches!(h.state().dialog, Dialog::None), "{style:?} {key:?}");
            assert!(h.state().views.active.schematic.tool.is_none(), "{style:?} {key:?}");
            assert_eq!(document(&h), before);
        }
        h.state_mut().set_selection_filters(id, &json!({category:true})).unwrap();
        h.key_press(Key::E);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::NetLabel(_)), "{style:?}");
        assert_eq!(document(&h), before);
    }
}

/// A mixed host exposes filter visibility only for its focused project pane. Definition-pane
/// show/hide requests fail atomically and preserve the parked project filter preference.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn definition_views_refuse_filter_panel_actions_and_report_the_card_hidden() {
    use crate::window_commands::{Panel, inspect_editor};
    for editor in ["Symbol Editor", "Footprint Editor"] {
        let mut h = fixture();
        let ctx = h.ctx.clone();
        let project_view = h.state().views.active.id().0;
        let host = h.state().views.host.0;
        click(&mut h, "Choose workspace");
        click(&mut h, &format!("Open {editor} beside current view"));
        let definition_view = h.state().views.active.id().0;
        assert_ne!(project_view, definition_view);
        assert!(h.state().views.is_split());
        assert!(h.state().views.active.definition.is_some());
        for stored in [true, false] {
            h.state_mut().views.active.filters_open = stored;
            h.run();
            assert!(h.query_by_label("Collapse Selection filters").is_none());
            assert!(!h.state().panel_open(Panel::Filters));
            let before = inspect_editor(h.state());
            let window = before["windows"].as_array().unwrap().iter().find(|window| window["id"] == host).unwrap();
            assert_eq!(window["panels"]["selection_filters"], false);
            let view = before["views"].as_array().unwrap().iter().find(|view| view["id"] == definition_view).unwrap();
            assert_eq!(view["selection_filters_open"], false);
            for open in [false, true] {
                let error =
                    h.state_mut().run_window_command(&ctx, "window.set_panel", &json!({"window":host,"panel":"selection_filters","open":open}));
                assert_eq!(error.unwrap_err(), "Selection filters require a project view");
                assert_eq!(h.state().views.active.filters_open, stored);
                assert_eq!(inspect_editor(h.state()), before);
            }
        }
        h.state_mut().run_window_command(&ctx, "view.focus", &json!({"view":project_view})).unwrap();
        for open in [false, true] {
            h.state_mut().run_window_command(&ctx, "window.set_panel", &json!({"window":host,"panel":"selection_filters","open":open})).unwrap();
            h.run();
            assert_eq!(h.state().panel_open(Panel::Filters), open);
            assert_eq!(h.query_by_label("Collapse Selection filters").is_some(), open);
        }
    }
}
