//! Contextual cards move independently of selections, document history and editor hosts.
use super::*;
use crate::{
    editor_view::Workspace,
    workspace_panels::{FILTERS, LAYERS, PROPERTIES},
};
use pcbcraft_chrome::workspace::PanelStyle;
use pcbcraft_chrome::workspace::panels::{Panel, Position};
use pcbcraft_engine::{persistence::encode_documents, selection::Selection};

/// Keep all requested cards visible, with a real selected object and a full-sized host.
fn fixture(board: bool) -> Harness<'static, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    if board {
        resistor_with_board_at_origin(&mut h);
    } else {
        resistor_at_origin(&mut h);
    }
    h.state_mut().properties_open = true;
    h.state_mut().views.active.filters_open = true;
    h.state_mut().select(Selection::one_symbol("symbol-2".into()));
    h.run();
    if board {
        click(&mut h, "Choose workspace");
        click(&mut h, "PCB Layout");
    }
    h
}

/// Compare the document independently of presentation and local arrangements.
fn document(h: &Harness<'_, PcbApp>) -> Value {
    json!({"inspection":h.state().session.inspect(),"revision":h.state().session.revision(),"dirty":h.state().files.dirty(&h.state().session),"bytes":encode_documents(h.state().session.snapshot().unwrap()).unwrap()})
}

/// Movable card headers keep the compact catalog's leading grip without a hover fill;
/// collapsing or hovering them must not look like selecting project content.
#[test]
fn card_headers_keep_grips_near_the_edge_without_hover_fill() {
    for (board, title) in [(true, "Layers"), (true, "Properties"), (true, "Selection filters"), (false, "Properties")] {
        let mut h = fixture(board);
        let before = document(&h);
        click(&mut h, &format!("Collapse {title}"));
        let header = h.get_by_label(&format!("Move {title}")).rect();
        h.hover_at(header.center());
        h.run();
        let dots: Vec<_> = h
            .output()
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Circle(dot) if header.contains(dot.center) && (dot.radius - 1.1).abs() < 0.01 => Some(dot.center),
                _ => None,
            })
            .collect();
        assert_eq!(dots.len(), 6, "{title}: grip must remain visible");
        let leading = dots.iter().map(|dot| dot.x).min_by(f32::total_cmp).unwrap();
        assert!(leading - header.left() <= 8.0, "{title}: duplicated header padding places the grip too far inward");
        assert!(
            !h.output().shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Rect(tile) if header.contains_rect(tile.rect) && tile.rect.contains(header.center()) && tile.fill != egui::Color32::TRANSPARENT)),
            "{title}: panel titles must not paint a hover fill"
        );
        assert_eq!(document(&h), before);
    }
}

/// Browser headers used to register three tooltips at once. Both workspace hosts must show
/// just the collapse action and leave the title background unchanged while hovered.
#[test]
fn browser_headers_have_one_action_tooltip_and_no_hover_fill() {
    for board in [false, true] {
        let mut h = fixture(board);
        h.ctx.global_style_mut(|style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.show_tooltips_only_when_still = false;
        });
        let before = document(&h);
        let header = h.get_by_role_and_label(egui::Role::Button, "Collapse Browser").rect();
        h.hover_at(header.center());
        h.run_steps(3);
        let painted = |text: &str| {
            h.output()
                .shapes
                .iter()
                .filter(|shape| {
                    matches!(&shape.shape,
            egui::Shape::Text(glyphs) if glyphs.galley.job.text == text)
                })
                .count()
        };
        assert_eq!(painted("Browser"), 1, "the visible title must not be repeated as a tooltip");
        assert_eq!(painted("Collapse Browser"), 1, "the header has exactly one action tooltip");
        assert_eq!(painted("Collapse panel"), 0);
        assert!(!h.output().shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Rect(tile) if header.contains_rect(tile.rect) && tile.fill != egui::Color32::TRANSPARENT)));
        assert_eq!(document(&h), before);
    }
}

/// Menus and live requests use the same anchor, and co-located cards occupy separate slots.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn contextual_card_menus_and_live_docking_preserve_document_state() {
    let mut h = fixture(true);
    let before = document(&h);
    let ctx = h.ctx.clone();
    let id = h.state().views.active.id().0;
    for (name, key, index) in
        [("Layers", "layers", LAYERS), ("Properties", "properties", PROPERTIES), ("Selection filters", "selection_filters", FILTERS)]
    {
        click(&mut h, &format!("{name} position"));
        click(&mut h, "Float");
        assert_eq!(h.state().views.active.panels[index].position(), Position::Floating);
        click(&mut h, &format!("{name} position"));
        click(&mut h, "Dock left");
        assert_eq!(h.state().views.active.panels[index].position(), Position::Left);
        h.state_mut().run_window_command(&ctx, "view.set_panel_position", &json!({"view":id,"panel":key,"position":"right"})).unwrap();
        h.run();
        assert_eq!(h.state().views.active.panels[index].position(), Position::Right);
        let canvas = h.get_by_label("PCB canvas").rect();
        assert!(canvas.contains_rect(h.get_by_label(&format!("{name} position")).rect()));
        assert!(canvas.contains_rect(h.get_by_label(&format!("Collapse {name}")).rect()));
    }
    let slots = crate::workspace_panels::areas(
        h.get_by_label("PCB canvas").rect(),
        h.state().browser.rect(),
        &h.state().views.active.panels,
        [true, true, true, false],
        false,
    );
    assert!(slots[LAYERS].bottom() <= slots[PROPERTIES].top());
    assert!(slots[PROPERTIES].bottom() <= slots[FILTERS].top());
    let headers = [h.get_by_label("Move Layers").rect(), h.get_by_label("Move Properties").rect(), h.get_by_label("Move Selection filters").rect()];
    for pair in headers.windows(2) {
        assert!(pair[0].bottom() < pair[1].top());
    }
    assert_eq!(document(&h), before);
}

/// In Compact, a card docked into a column sits directly beneath the card above it rather than
/// at the column's bottom share, in both the schematic and PCB columns and on either side.
/// Floating keeps the established split, with Selection filters at the column's bottom.
#[test]
fn compact_columns_stack_selection_filters_under_the_card_above() {
    for (board, above, side) in
        [(false, "Move Properties", Position::Right), (true, "Move Properties", Position::Right), (true, "Move Layers", Position::Left)]
    {
        let mut h = fixture(board);
        h.state_mut().views.active.panels[FILTERS].set_position(side, None);
        h.run();
        let floating = card_rect(&h, "Move Selection filters");
        assert!(floating.top() > card_rect(&h, above).bottom() + 8.0, "{above}: floating keeps its column split");

        h.state_mut().preferences.panel_style = PanelStyle::Compact;
        h.run();
        let (card, filters) = (card_rect(&h, above), card_rect(&h, "Move Selection filters"));
        assert!((card.left() - filters.left()).abs() < 1.0, "{above}: filters joined the same column: {card:?} {filters:?}");
        assert!((filters.top() - card.bottom()).abs() < 1.0, "{above}: filters start right under the card above: {card:?} {filters:?}");
        assert_eq!(h.state().views.active.panels[FILTERS].position(), side);
    }
}

/// With DRC and a right-docked Selection filters card both open in Compact, the DRC card starts
/// below the filters stacked under Properties instead of covering them.
#[test]
fn compact_drc_card_stacks_below_right_docked_selection_filters() {
    let mut h = fixture(true);
    h.state_mut().views.active.board.drc.open = true;
    h.state_mut().preferences.panel_style = PanelStyle::Compact;
    h.run();
    let (properties, filters, drc) = (card_rect(&h, "Move Properties"), card_rect(&h, "Move Selection filters"), card_rect(&h, "Collapse DRC"));
    assert!((filters.top() - properties.bottom()).abs() < 1.0, "filters stay under Properties: {properties:?} {filters:?}");
    assert!(!drc.intersects(filters.shrink(0.5)), "DRC must not cover Selection filters: {filters:?} {drc:?}");
    assert!((drc.top() - filters.bottom()).abs() < 1.0, "DRC starts right under the filters: {filters:?} {drc:?}");
}

/// The whole card around the header control labelled `label`: the smallest visible area holding it.
fn card_rect(h: &Harness<'_, PcbApp>, label: &str) -> egui::Rect {
    let header = h.get_by_label(label).rect();
    h.ctx.memory(|memory| {
        memory
            .areas()
            .visible_layer_ids()
            .into_iter()
            .filter_map(|layer| memory.area_rect(layer.id))
            .filter(|rect| rect.contains_rect(header))
            .min_by(|a, b| a.area().total_cmp(&b.area()))
            .unwrap_or_else(|| panic!("no card area for {label}"))
    })
}

/// The Panel style choice switches the whole shell between SolidCraft's floating and compact
/// layouts: the workspace tile, Browser and docked cards meet their edges in Compact and return
/// to their inset positions in Floating, without changing stored dock sides or the document.
#[test]
fn panel_style_switches_the_shell_between_floating_and_compact() {
    let mut h = fixture(false);
    let before = document(&h);
    assert_eq!(h.state().preferences.panel_style, PanelStyle::Floating);
    let rects = |h: &Harness<'_, PcbApp>| {
        ["Schematic canvas", "Move Properties", "Collapse Browser", "Choose workspace"].map(|label| h.get_by_label(label).rect())
    };
    let [canvas, properties, browser, tile] = rects(&h);
    assert!(canvas.contains_rect(properties));
    assert!(properties.right() <= canvas.right() - 8.0, "a floating docked card keeps its edge gap");
    assert!(browser.left() >= canvas.left() + 8.0, "the floating Browser keeps its edge gap");
    assert!(tile.left() >= 8.0, "the floating workspace tile keeps its window margin");

    click(&mut h, "Preferences");
    click(&mut h, "Compact");
    click(&mut h, "OK");
    assert_eq!(h.state().preferences.panel_style, PanelStyle::Compact);
    assert_eq!(h.state().views.active.panels[PROPERTIES].position(), Position::Right);
    let [compact_canvas, compact_properties, compact_browser, compact_tile] = rects(&h);
    assert!(compact_canvas.top() < canvas.top(), "the joined ribbon leaves the canvas more room: {canvas:?} {compact_canvas:?}");
    assert!(compact_properties.right() > properties.right() + 8.0, "a compact card meets its dock edge: {properties:?} {compact_properties:?}");
    assert!(compact_properties.top() < properties.top() - 8.0, "a compact card meets the host's top edge");
    assert!(compact_browser.left() < browser.left() - 8.0, "the compact Browser meets the canvas edge: {browser:?} {compact_browser:?}");
    assert!(compact_tile.left() < tile.left() - 4.0, "the compact workspace tile meets the window edge: {tile:?} {compact_tile:?}");
    assert_eq!(document(&h), before);
    assert_eq!(crate::window_commands::inspect_editor(h.state())["preferences"]["panel_style"], "compact");

    let ctx = h.ctx.clone();
    h.state_mut().run_window_command(&ctx, "app.set_preferences", &json!({"panel_style": "floating"})).unwrap();
    h.run();
    assert_eq!(h.state().preferences.panel_style, PanelStyle::Floating);
    assert_eq!(rects(&h), [canvas, properties, browser, tile]);
    assert_eq!(h.state().views.active.panels[PROPERTIES].position(), Position::Right);
    assert_eq!(document(&h), before);
}

/// Escape restores the pre-drag dock before canvas shortcuts run; a normal release snaps left.
#[test]
fn header_drag_cancellation_and_edge_drop_keep_selection() {
    let mut h = fixture(false);
    let before = document(&h);
    let selection = h.state().views.active.schematic.selection.clone();
    let from = h.get_by_label("Move Selection filters").rect().center();
    let to = h.get_by_label("Schematic canvas").rect().center();
    h.drag_at(from);
    h.run();
    h.hover_at(to);
    h.run();
    assert_eq!(h.state().views.active.panels[FILTERS].position(), Position::Floating);
    h.key_press(Key::Escape);
    h.run();
    h.drop_at(to);
    h.run();
    assert_eq!(h.state().views.active.panels[FILTERS].position(), Position::Right);
    let from = h.get_by_label("Move Selection filters").rect().center();
    let canvas = h.get_by_label("Schematic canvas").rect();
    let to = egui::pos2(canvas.left() + 20., canvas.center().y);
    h.drag_at(from);
    h.run();
    h.hover_at(to);
    h.run();
    h.drop_at(to);
    h.run();
    assert_eq!(h.state().views.active.panels[FILTERS].position(), Position::Left);
    assert_eq!(h.state().views.active.schematic.selection, selection);
    let from = h.get_by_label("Move Selection filters").rect().center();
    let center = h.get_by_label("Schematic canvas").rect().center();
    h.drag_at(from);
    h.run();
    h.hover_at(center);
    h.run();
    let id = h.state().views.active.id();
    h.state_mut().set_selection_filters(id, &json!({"open":false})).unwrap();
    h.drop_at(center);
    h.run();
    h.state_mut().set_selection_filters(id, &json!({"open":true})).unwrap();
    h.run();
    assert_eq!(h.state().views.active.panels[FILTERS].position(), Position::Left);
    assert_eq!(document(&h), before);
}

/// Moving the inspector retains an unapplied field draft instead of saving or discarding it.
#[test]
fn docking_properties_preserves_unapplied_fields() {
    let mut h = fixture(false);
    let before = document(&h);
    h.get_by_label("Value").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_label("Value").type_text("4.7k");
    h.run();
    click(&mut h, "Properties position");
    click(&mut h, "Float");
    assert_eq!(h.get_by_label("Value").value().as_deref(), Some("4.7k"));
    click(&mut h, "Properties position");
    click(&mut h, "Dock left");
    assert_eq!(h.get_by_label("Value").value().as_deref(), Some("4.7k"));
    assert_eq!(document(&h), before);
}

/// Placement survives forks, detach/dock, document retargeting and local serialization; old
/// records get the initial left/right positions and invalid coordinates fail validation.
#[test]
fn panel_layout_follows_views_and_round_trips_in_arrangements() {
    let mut h = fixture(false);
    let id = h.state().views.active.id();
    h.state_mut().set_panel_position(id, &json!({"panel":"properties","position":"floating","x":380,"y":80})).unwrap();
    let wanted = h.state().views.active.panels[PROPERTIES].saved_state();
    let app = h.state_mut();
    app.views.switch_workspace(id, Workspace::Board, &app.session);
    app.views.switch_workspace(id, Workspace::Schematic, &app.session);
    assert_eq!(app.views.active.panels[PROPERTIES].saved_state(), wanted);
    let state = app.views.active.saved_state(&app.session).unwrap();
    let mut encoded = serde_json::to_value(&state).unwrap();
    let decoded = serde_json::from_value(encoded.clone()).unwrap();
    let restored = EditorView::from_state(ViewId(900), &decoded, &app.session);
    assert_eq!(restored.panels[PROPERTIES].saved_state(), wanted);
    let child = app.views.open_beside(id).unwrap();
    let host = app.views.detach_view(child).unwrap();
    assert_eq!(app.views.hosts[&host].focused, child);
    app.views.dock_view(child);
    assert_eq!(app.views.view(child).unwrap().panels[PROPERTIES].saved_state(), wanted);
    encoded.as_object_mut().unwrap().remove("panels");
    let legacy = serde_json::from_value(encoded).unwrap();
    let legacy = EditorView::from_state(ViewId(901), &legacy, &app.session);
    assert_eq!(legacy.panels[LAYERS].position(), Position::Left);
    assert_eq!(legacy.panels[PROPERTIES].position(), Position::Right);
    let mut invalid = serde_json::to_value(state).unwrap();
    invalid["panels"]["layers"]["at"] = json!([1e10, 0]);
    let invalid: crate::editor_view::ViewState = serde_json::from_value(invalid).unwrap();
    assert!(!invalid.valid());
}

/// Out-of-window floating coordinates clamp on resize, while invalid live requests are atomic.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn invalid_panel_requests_and_compact_hosts_keep_controls_reachable() {
    let mut h = fixture(false);
    let ctx = h.ctx.clone();
    let id = h.state().views.active.id().0;
    let before = document(&h);
    let original = h.state().views.active.panels.each_ref().map(Panel::saved_state);
    for params in [
        json!({"view":id,"panel":"layers","position":"right"}),
        json!({"view":id,"panel":"properties","position":"floating","x":1}),
        json!({"view":id,"panel":"properties","position":"floating","x":100001,"y":0}),
        json!({"view":99999,"panel":"properties","position":"left"}),
    ] {
        assert!(h.state_mut().run_window_command(&ctx, "view.set_panel_position", &params).is_err());
        assert_eq!(h.state().views.active.panels.each_ref().map(Panel::saved_state), original);
    }
    h.state_mut()
        .run_window_command(&ctx, "view.set_panel_position", &json!({"view":id,"panel":"properties","position":"floating","x":99999,"y":99999}))
        .unwrap();
    h.set_size(egui::vec2(700., 700.));
    h.run();
    let canvas = h.get_by_label("Schematic canvas").rect();
    let header = h.get_by_label("Move Properties").rect();
    assert!(canvas.contains_rect(header));
    assert_eq!(document(&h), before);
}

/// Symbol and footprint authoring use the same draggable Properties card without changing
/// definition history or the selected primitive when the card changes its anchor.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn library_properties_use_the_same_docking_controls() {
    for kind in [pcbcraft_engine::authoring::Kind::Symbol, pcbcraft_engine::authoring::Kind::Footprint] {
        let mut h = harness();
        h.set_size(egui::vec2(1400., 900.));
        h.state_mut().new_library_item(kind);
        h.run();
        let before = h.state().library_editors.inspect();
        click(&mut h, "Properties position");
        click(&mut h, "Float");
        assert_eq!(h.state().views.active.panels[PROPERTIES].position(), Position::Floating);
        let ctx = h.ctx.clone();
        let id = h.state().views.active.id().0;
        h.state_mut().run_window_command(&ctx, "view.set_panel_position", &json!({"view":id,"panel":"properties","position":"left"})).unwrap();
        h.run();
        assert_eq!(h.state().views.active.panels[PROPERTIES].position(), Position::Left);
        let mut after = h.state().library_editors.inspect();
        let mut before = before;
        after["documents"][0].as_object_mut().unwrap().remove("view");
        before["documents"][0].as_object_mut().unwrap().remove("view");
        assert_eq!(after, before);
    }
}

/// The Net Inspector replaces Properties without inheriting a floating card's full-height
/// slot, and closing it restores the stored Properties placement.
#[test]
fn net_inspector_reserves_space_above_filters_without_moving_properties() {
    for position in [Position::Left, Position::Floating, Position::Right] {
        let mut h = fixture(false);
        h.state_mut().views.active.panels[PROPERTIES].set_position(position, None);
        let before = document(&h);
        h.state_mut().net_inspector.open = true;
        h.run();
        h.get_by_label("Search nets or pins");
        assert!(h.query_by_label("Move Properties").is_none());
        let inspector = h.state().net_inspector.rect;
        let filters = h.get_by_label("Move Selection filters").rect();
        assert!(inspector.is_positive());
        assert!(inspector.bottom() <= filters.top(), "{position:?}: {inspector:?} {filters:?}");
        h.state_mut().net_inspector.open = false;
        h.run();
        h.get_by_label("Move Properties");
        assert_eq!(h.state().views.active.panels[PROPERTIES].position(), position);
        assert_eq!(document(&h), before);
    }
}

/// Exhausting a short Compact column hides DRC without changing its open state or dock intent.
#[test]
fn compact_short_column_does_not_overlay_filters_and_drc() {
    let mut h = fixture(true);
    h.state_mut().preferences.panel_style = PanelStyle::Compact;
    h.state_mut().views.active.board.drc.open = true;
    let before = document(&h);
    let placement = h.state().views.active.panels[FILTERS].saved_state();
    for height in [500., 400., 320.] {
        h.set_size(egui::vec2(1400., height));
        h.run();
        let drc = h.state().views.active.board.drc.rect;
        if h.query_by_label("Move Selection filters").is_some() {
            let filters = card_rect(&h, "Move Selection filters");
            assert!(!drc.is_positive() || drc.top() >= filters.bottom() - 0.5, "height {height}: DRC {drc:?} must follow filters {filters:?}");
        }
        assert!(h.state().views.active.board.drc.open);
        assert_eq!(h.state().views.active.panels[FILTERS].saved_state(), placement);
        assert_eq!(document(&h), before);
    }
    h.set_size(egui::vec2(1400., 900.));
    h.run();
    assert!(h.state().views.active.board.drc.rect.is_positive(), "restoring room reveals the still-open DRC card");
}
