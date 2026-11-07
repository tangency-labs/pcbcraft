//! The parts browser: a movable card floating over the canvas that lists embedded and loaded native
//! symbols as searchable thumbnails. Click a part to place it, or
//! drag parts onto the sheet one after another. Browsing never edits the project; placing goes through the same
//! `schematic.place_symbol` command as before. Adapted from PCBcraft Next's parts browser.

use std::sync::Arc;

use egui::{Rect, Ui};
use pcbcraft_chrome::workspace::catalog::{self, View};
use pcbcraft_engine::{library, schematic::SymbolDefinition};

use crate::{
    editor_view::ViewInput,
    schematic_canvas::{PartDrag, paint_preview},
};

#[derive(Debug, Default)]
pub(crate) struct PartsBrowser {
    pub(crate) open: bool,
    query: String,
    view: View,
    panel: catalog::Panel,
    rect: Option<Rect>,
    /// Focus the search field on the next frame, as after pressing A.
    focus: bool,
    /// Whether the search field had focus at the end of the last frame. egui drops focus on
    /// Escape before the frame's code runs, so this is how Escape knows it was meant here.
    search_focused: bool,
}

/// Remember catalog visibility and presentation, without search focus or drag payloads.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PartsState {
    open: bool,
    view: View,
    panel: catalog::PanelState,
}

impl PartsState {
    /// Validate saved placement before it reaches the chrome layout.
    pub(super) fn valid(&self) -> bool {
        self.panel.valid()
    }
}

impl PartsBrowser {
    /// Capture layout without retaining search text or an unfinished placement.
    pub(super) fn saved_state(&self) -> PartsState {
        PartsState { open: self.open, view: self.view, panel: self.panel.saved_state() }
    }

    /// Recreate presentation with fresh interaction ownership.
    pub(super) fn from_state(state: &PartsState) -> Self {
        Self { open: state.open, view: state.view, panel: catalog::Panel::from_state(&state.panel), ..Self::default() }
    }

    /// Release the shared search field when another editor takes input ownership. The query,
    /// view mode and panel position remain available to the newly focused pane.
    pub(super) fn release_focus(&mut self, ctx: &egui::Context) {
        self.focus = false;
        self.search_focused = false;
        ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
    }
    /// Open and focus the retained search, as Capture › Symbol and A do. A collapsed card expands first.
    pub(crate) fn focus_search(&mut self) {
        self.open = true;
        self.panel.expand();
        self.focus = true;
    }

    /// Hide the card without changing its search, view, position or the active placement tool.
    pub(crate) fn close(&mut self, ctx: &egui::Context) {
        self.open = false;
        self.focus = false;
        self.search_focused = false;
        self.panel.cancel_drag();
        egui::DragAndDrop::clear_payload(ctx);
        ctx.request_repaint();
    }

    /// Escape first cancels a panel drag or leaves search focus, keeping the library visible.
    /// Call after giving dialogs, popups and the assistant priority, but before tool cancellation.
    /// Outside a drag/search, Escape remains available to the canvas.
    pub(crate) fn take_escape(&mut self, ctx: &egui::Context) -> bool {
        if self.open && ctx.input(|i| i.key_pressed(egui::Key::Escape)) && (self.panel.cancel_drag() || self.search_focused) {
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            self.search_focused = false;
            ctx.request_repaint();
            return true;
        }
        false
    }

    /// The starter library's symbols whose name or ID contains the search text, or whose reference
    /// prefix is it. Matching ignores case; an empty search lists every symbol.
    #[cfg(test)]
    fn matches(&self) -> Vec<Arc<SymbolDefinition>> {
        matches(&self.query)
    }

    /// The current card rectangle, absent while hidden or before drawing. Used for fitting and pointer exclusion.
    pub(crate) fn rect(&self) -> Rect {
        if self.open { self.rect.unwrap_or(Rect::NOTHING) } else { Rect::NOTHING }
    }

    /// Reserve vertical room for a left-anchored library by limiting the Browser's card only.
    pub(crate) fn browser_area(&self, canvas: Rect) -> Rect {
        if self.open { self.panel.browser_area(canvas) } else { canvas }
    }

    /// Draw the floating library when open; clicking a part or pressing Enter releases search
    /// focus and starts placement while keeping the query and card visible.
    /// Drag payloads retain the originating view, sheet and revision so intervening edits cancel placement. A short
    /// dock temporarily uses List while preserving the preferred view for a taller card.
    pub(crate) fn show(
        &mut self,
        ui: &Ui,
        canvas: Rect,
        browser: Rect,
        placing: Option<&Arc<SymbolDefinition>>,
        source: ViewInput,
        libraries: &library::sources::Catalog,
    ) -> Option<Arc<SymbolDefinition>> {
        if !self.open {
            self.rect = None;
            return None;
        }
        let mut picked = None;
        let count = matches_sources(&self.query, libraries).len();
        let Self { panel, query, view, focus, search_focused, .. } = self;
        let (rect, closed) = panel.show(ui, canvas, browser, |ui, compact| {
            catalog::source(ui, "Symbols", count);
            let search = catalog::search(ui, query);
            if std::mem::take(focus) {
                search.request_focus();
            }
            *search_focused = search.has_focus();
            let enter = search.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let view = catalog::view_picker(ui, view, compact);
            let parts = matches_sources(query, libraries);
            if enter {
                picked = parts.first().cloned();
            }
            catalog::grid(ui, view, &parts, |ui, definition, width| {
                let identity =
                    (definition.library != "starter").then(|| format!("{}/{} r{}", definition.library, definition.id, definition.revision()));
                let detail = identity.as_ref().map_or_else(
                    || definition.default_value().to_owned(),
                    |_| format!("{} · {} r{}", definition.default_value(), definition.library, definition.revision()),
                );
                let item = catalog::Item {
                    name: &definition.name,
                    value: &detail,
                    identity: identity.as_deref(),
                    selected: placing.is_some_and(|placing| Arc::ptr_eq(placing, definition)),
                };
                let response = catalog::tile(ui, view, width, item, |painter, rect, ink| paint_preview(painter, definition, rect, ink));
                let response = response.on_hover_text(format!(
                    "{}/{} · revision {}{}",
                    definition.library,
                    definition.id,
                    definition.revision(),
                    definition.provenance.as_ref().map(|p| format!("\n{}", p.license)).unwrap_or_default()
                ));
                response.dnd_set_drag_payload(PartDrag { definition: definition.clone(), source });
                if response.clicked() {
                    picked = Some(definition.clone());
                }
            });
            if picked.is_some() {
                search.surrender_focus();
                *search_focused = false;
            }
        });
        if panel.is_collapsed() {
            *search_focused = false;
        }
        self.rect = Some(rect);
        if closed {
            self.close(ui.ctx());
        }
        if picked.is_some() {
            self.search_focused = false;
            egui::DragAndDrop::clear_payload(ui.ctx());
            ui.ctx().request_repaint();
        }
        picked
    }
}

/// Find symbols by name, ID or reference prefix. Familiar starter parts lead in the prototype's
/// order; any additional definitions retain their library order after them.
fn matches(query: &str) -> Vec<Arc<SymbolDefinition>> {
    let query = query.trim().to_lowercase();
    let mut parts: Vec<_> = library::starter()
        .symbols()
        .iter()
        .filter(|definition| {
            query.is_empty()
                || definition.name.to_lowercase().contains(&query)
                || definition.id.contains(&query)
                || definition.reference_prefix().eq_ignore_ascii_case(&query)
        })
        .cloned()
        .collect();
    let order = ["resistor", "capacitor", "led", "connector"];
    parts.sort_by_key(|part| order.iter().position(|id| *id == part.id).unwrap_or(order.len()));
    parts
}

/// Combine embedded symbols with unambiguous current native revisions from the session catalog.
fn matches_sources(query: &str, catalog: &library::sources::Catalog) -> Vec<Arc<SymbolDefinition>> {
    let mut parts = matches(query);
    parts.extend(catalog.search(query, Some(library::sources::Kind::Symbol)).into_iter().filter_map(|(_, item)| item.symbol()));
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PcbApp, tests::click};
    use egui::{Key, Modifiers, Vec2, pos2, vec2};
    use egui_kittest::{Harness, kittest::Queryable};
    use pcbcraft_engine::{PLACE_RESISTOR, schematic::Point};
    use serde_json::json;

    /// Open a project with two separated symbols, so Fit has real extents to protect.
    fn harness() -> Harness<'static, PcbApp> {
        let mut app = PcbApp::default();
        app.session.execute("project.new", &json!({"name":"Library layout"})).unwrap();
        for (x, revision) in [(0, 1), (60_960_000, 2)] {
            app.session.execute(PLACE_RESISTOR, &json!({"x_nm":x,"y_nm":0,"expected_revision":revision})).unwrap();
        }
        let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_max_steps(20).build_ui_state(|ui, app| app.frame(ui), app);
        h.run();
        h
    }

    /// Use the accessible position menu, allowing geometry to settle after each choice.
    fn position(h: &mut Harness<'_, PcbApp>, label: &str) {
        click(h, "Parts library position");
        click(h, label);
    }

    /// Both edge anchors and free placement keep the full canvas. Fit and Browser picks target
    /// clear space, and these UI operations leave the session unchanged.
    #[test]
    fn floating_library_preserves_canvas_and_navigation_avoids_both_cards() {
        let mut h = harness();
        let canvas = h.get_by_label("Schematic canvas").rect();
        let origin = h.state().views.active.camera.screen(Point { x_nm: 0, y_nm: 0 }, canvas);
        let before = h.state().session.inspect();
        h.key_press(Key::A);
        h.run();
        assert_eq!(h.state().views.active.camera.screen(Point { x_nm: 0, y_nm: 0 }, canvas), origin, "opening must not pan");
        for anchor in ["Dock left", "Dock right", "Float"] {
            position(&mut h, anchor);
            assert_eq!(h.get_by_label("Schematic canvas").rect(), canvas);
            let cards = [h.state().browser.rect(), h.state().parts.rect()];
            assert!(canvas.contains_rect(cards[1]));
            assert!(!cards[0].intersects(cards[1]));
            click(&mut h, "Fit schematic");
            let clear = catalog::uncovered(canvas, &cards);
            for symbol in h.state().session.project().unwrap().symbols() {
                let at = h.state().views.active.camera.screen(symbol.position, canvas);
                assert!(clear.contains(at), "{anchor}: {at:?} not inside {clear:?}");
            }
        }
        click(&mut h, "Expand Symbols");
        click(&mut h, "R2  1k");
        let app = h.state();
        let clear = catalog::uncovered(canvas, &[app.browser.rect(), app.parts.rect()]);
        let at = app.views.active.camera.screen(app.session.project().unwrap().symbols()[1].position, canvas);
        assert!(at.distance(clear.center()) < 0.1, "pick must use this frame's card bounds");
        assert_eq!(app.session.inspect(), before);
    }

    /// The Assistant's floating drawer participates in the same Fit target as Browser and Parts.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn fit_also_avoids_the_assistant() {
        let mut h = harness();
        h.key_press(Key::A);
        h.run();
        click(&mut h, "Assistant");
        let canvas = h.get_by_label("Schematic canvas").rect();
        let assistant = h.state().assistant.covered_rect(canvas);
        assert!(assistant.is_positive());
        click(&mut h, "Fit schematic");
        let clear = catalog::uncovered(canvas, &[h.state().browser.rect(), h.state().parts.rect(), assistant]);
        for symbol in h.state().session.project().unwrap().symbols() {
            assert!(clear.contains(h.state().views.active.camera.screen(symbol.position, canvas)));
        }
    }

    /// A short left dock uses a scrollable list; returning to Float restores the chosen density.
    /// Narrow windows keep the card's header on the canvas, including after an anchor change.
    #[test]
    fn short_dock_uses_list_and_resizing_keeps_controls_reachable() {
        let mut h = harness();
        h.key_press(Key::A);
        h.run();
        click(&mut h, "Large");
        h.set_size(vec2(1000.0, 700.0));
        h.run();
        position(&mut h, "Dock left");
        assert!(h.query_by_label("Large").is_none());
        assert!((h.get_by_label("Resistor").rect().height() - 58.0).abs() < 0.1);
        h.get_by_label("4-pin header").scroll_to_me();
        h.run();
        let last = h.get_by_label("4-pin header").rect();
        assert!(h.state().parts.rect().contains_rect(last));
        position(&mut h, "Float");
        assert_eq!(h.state().parts.view, View::Large);
        assert!((h.get_by_label("Resistor").rect().height() - 180.0).abs() < 0.1);
        h.set_size(vec2(420.0, 520.0));
        h.run();
        let canvas = h.get_by_label("Schematic canvas").rect();
        assert!(canvas.contains_rect(h.state().parts.rect()));
        assert!(canvas.contains_rect(h.get_by_label("Collapse Parts library").rect()));
        position(&mut h, "Dock right");
        assert!(canvas.contains_rect(h.state().parts.rect()));
    }

    /// Collapsing releases canvas space at every anchor without losing the query or view.
    /// Fit uses the smaller header, and A reveals and focuses the retained search.
    #[test]
    fn collapse_preserves_catalog_state_and_a_expands_search() {
        let mut h = harness();
        h.key_press(Key::A);
        h.run();
        click(&mut h, "Large");
        h.get_by_role(egui::accesskit::Role::TextInput).click();
        h.run();
        h.get_by_role(egui::accesskit::Role::TextInput).type_text("cap");
        h.run();
        let before = h.state().session.inspect();
        let canvas = h.get_by_label("Schematic canvas").rect();
        for anchor in ["Float", "Dock left", "Dock right"] {
            position(&mut h, anchor);
            let expanded = h.state().parts.rect();
            let origin = h.state().views.active.camera.screen(Point { x_nm: 0, y_nm: 0 }, canvas);
            click(&mut h, "Collapse Parts library");
            let collapsed = h.state().parts.rect();
            h.get_by_label("Move Parts library");
            assert!(collapsed.height() < 60.0);
            assert_eq!(collapsed.min, expanded.min);
            assert_eq!(collapsed.width(), expanded.width());
            assert_eq!(h.get_by_label("Schematic canvas").rect(), canvas);
            assert_eq!(h.state().views.active.camera.screen(Point { x_nm: 0, y_nm: 0 }, canvas), origin);
            assert!(h.query_by_label("Capacitor").is_none());
            assert!(h.query_by_label("Large").is_none());
            click(&mut h, "Fit schematic");
            let clear = catalog::uncovered(canvas, &[h.state().browser.rect(), collapsed]);
            for symbol in h.state().session.project().unwrap().symbols() {
                assert!(clear.contains(h.state().views.active.camera.screen(symbol.position, canvas)));
            }
            click(&mut h, "Expand Parts library");
            assert_eq!(h.state().parts.rect(), expanded);
            h.get_by_label("Capacitor");
            assert_eq!(h.state().parts.view, View::Large);
            assert_eq!(h.state().parts.query, "cap");
            click(&mut h, "Collapse Parts library");
            h.key_press(Key::A);
            h.run();
            assert_eq!(h.state().parts.rect(), expanded);
            assert!(h.state().parts.search_focused);
            assert_eq!(h.state().parts.query, "cap");
        }
        assert_eq!(h.state().session.inspect(), before);
    }

    /// Closing hides all coverage while retaining search, density and dock position. A and the
    /// View toggle reopen the card, and these window actions never change project history.
    #[test]
    fn close_and_reopen_preserve_catalog_state() {
        let mut h = harness();
        let before = h.state().session.inspect();
        let canvas = h.get_by_label("Schematic canvas").rect();
        let origin = h.state().views.active.camera.screen(Point { x_nm: 0, y_nm: 0 }, canvas);
        h.key_press(Key::A);
        h.run();
        click(&mut h, "Large");
        h.get_by_role(egui::accesskit::Role::TextInput).click();
        h.run();
        h.get_by_role(egui::accesskit::Role::TextInput).type_text("cap");
        h.run();
        position(&mut h, "Dock right");
        let card = h.state().parts.rect();
        click(&mut h, "Close Parts library");
        assert!(h.query_by_label("Move Parts library").is_none());
        assert!(!h.state().parts.rect().is_positive());
        assert_eq!(h.state().parts.browser_area(canvas), canvas);
        h.key_press(Key::A);
        h.run();
        assert_eq!(h.state().parts.rect(), card);
        assert_eq!(h.state().parts.query, "cap");
        assert_eq!(h.state().parts.view, View::Large);
        assert!(h.state().parts.search_focused);
        click(&mut h, "View");
        click(&mut h, "Toggle Parts");
        assert!(h.query_by_label("Move Parts library").is_none());
        click(&mut h, "Toggle Parts");
        assert_eq!(h.state().parts.rect(), card);
        assert_eq!(h.state().parts.query, "cap");
        assert_eq!(h.get_by_label("Schematic canvas").rect(), canvas);
        assert_eq!(h.state().views.active.camera.screen(Point { x_nm: 0, y_nm: 0 }, canvas), origin);
        assert_eq!(h.state().session.inspect(), before);
    }

    /// A collapsed library still moves, snaps and handles its menu/Escape, while wheel input
    /// over its former body reaches the sheet instead of an invisible catalog.
    #[test]
    fn collapsed_card_keeps_controls_and_releases_pointer_input() {
        let mut h = harness();
        h.key_press(Key::A);
        h.run();
        let body = h.state().parts.rect().center();
        click(&mut h, "Collapse Parts library");
        h.hover_at(body);
        h.run();
        let zoom = h.state().views.active.camera.zoom();
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -50.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.run();
        assert_ne!(h.state().views.active.camera.zoom(), zoom);
        let from = h.get_by_label("Move Parts library").rect().center();
        let canvas = h.get_by_label("Schematic canvas").rect();
        let to = pos2(canvas.right() - 20.0, canvas.center().y);
        h.drag_at(from);
        h.run();
        h.hover_at(to);
        h.run();
        h.drop_at(to);
        h.run();
        assert!((canvas.right() - h.state().parts.rect().right() - 12.0).abs() < 0.1);
        assert!(h.state().parts.rect().height() < 60.0);
        position(&mut h, "Dock left");
        assert!(h.state().parts.rect().height() < 60.0);
        h.set_size(vec2(420.0, 520.0));
        h.run();
        assert!(h.get_by_label("Schematic canvas").rect().contains_rect(h.state().parts.rect()));
        click(&mut h, "Parts library position");
        h.key_press(Key::Escape);
        h.run();
        h.get_by_label("Move Parts library");
        h.key_press(Key::Escape);
        h.run();
        h.get_by_label("Move Parts library");
        h.key_press(Key::A);
        h.run();
        h.get_by_label("Collapse Parts library");
        assert!(h.state().parts.search_focused);
    }

    /// A title drag snaps on release; Escape restores its origin while keeping the library visible.
    #[test]
    fn header_drags_snap_to_edges_and_escape_restores_the_previous_position() {
        let mut h = harness();
        h.key_press(Key::A);
        h.run();
        let canvas = h.get_by_label("Schematic canvas").rect();
        let original = h.state().parts.rect();
        let from = h.get_by_label("Move Parts library").rect().center();
        h.drag_at(from);
        h.run();
        h.hover_at(from + vec2(100.0, 80.0));
        h.run();
        assert_ne!(h.state().parts.rect(), original);
        h.key_press(Key::Escape);
        h.run();
        assert_eq!(h.state().parts.rect(), original);
        h.drop_at(from + vec2(100.0, 80.0));
        h.run();
        h.get_by_label("Move Parts library");
        for right in [true, false] {
            let from = h.get_by_label("Move Parts library").rect().center();
            let to = pos2(if right { canvas.right() - 20.0 } else { canvas.left() + 20.0 }, canvas.center().y);
            h.drag_at(from);
            h.run();
            for i in 1..=4 {
                h.hover_at(from + (to - from) * (i as f32 / 4.0));
                h.run();
            }
            h.drop_at(to);
            h.run();
            let rect = h.state().parts.rect();
            assert!(canvas.contains_rect(rect));
            let gap = if right { canvas.right() - rect.right() } else { rect.left() - canvas.left() };
            assert!((gap - 12.0).abs() < 0.1, "edge anchor: {rect:?}");
        }
    }

    /// Library scrolling does not zoom the sheet, part drops over a card do not place, and the
    /// position popup consumes Escape before the library does.
    #[test]
    fn library_takes_pointer_input_and_leaves_escape_to_its_menu() {
        let mut h = harness();
        h.key_press(Key::A);
        h.run();
        let before = h.state().session.inspect();
        click(&mut h, "Medium");
        let from = h.get_by_label("Capacitor").rect().center();
        h.hover_at(from);
        h.run();
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -50.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.run();
        assert_eq!(h.state().views.active.camera.zoom(), 1.0);
        h.get_by_label("Capacitor").scroll_to_me();
        h.run();
        let from = h.get_by_label("Capacitor").rect().center();
        let to = h.state().browser.rect().center();
        h.drag_at(from);
        h.run();
        h.hover_at(from + Vec2::splat(10.0));
        h.run();
        h.hover_at(to);
        h.run();
        h.drop_at(to);
        h.run();
        assert_eq!(h.state().session.inspect(), before);
        click(&mut h, "Parts library position");
        h.key_press(Key::Escape);
        h.run();
        assert!(h.query_by_label("Dock left").is_none());
        h.get_by_label("Move Parts library");
        h.key_press(Key::Escape);
        h.run();
        h.get_by_label("Move Parts library");
    }

    #[test]
    fn search_matches_names_ids_and_prefixes_ignoring_case() {
        let mut browser = PartsBrowser::default();
        let names = |browser: &PartsBrowser| browser.matches().into_iter().map(|definition| definition.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&browser).len(), library::starter().symbols().len());
        browser.query = "CAP".to_owned();
        assert_eq!(names(&browser), ["Capacitor"]);
        browser.query = "j".to_owned();
        assert_eq!(names(&browser), ["2-pin connector", "4-pin header"], "J is the connectors' prefix");
        browser.query = "zzz".to_owned();
        assert!(names(&browser).is_empty());
    }
}
