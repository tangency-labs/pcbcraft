//! The Browser: a card floating over the left of the canvas with the project's tree, Project › Sheets ›
//! sheet › Symbols / Nets. Folding and picking rows never edit the project; a pick selects the
//! symbol or wire on the canvas and centers the view on it. Follows PCBcraft Next's Browser.

use std::{cmp::Ordering, collections::BTreeSet, sync::Arc};

use egui::{Id, Rect, Ui};
use pcbcraft_chrome::{
    shell::dim,
    workspace::{self, ToolIcon, TreeIcon, TreeRow},
};
use pcbcraft_engine::{
    Project, Session,
    schematic::{Point, Terminal},
};

/// The sheet every project has today. It becomes a sheet ID once projects hold several sheets.
const MAIN_SHEET: &str = "Main";

/// A row that folds. Each starts open or closed as [`Node::open_by_default`] says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Node {
    Project,
    Sheets,
    Sheet,
    Symbols,
    Nets,
}

impl Node {
    /// The project, its sheets and the open sheet start open; the item lists start folded.
    fn open_by_default(self) -> bool {
        matches!(self, Self::Project | Self::Sheets | Self::Sheet)
    }
}

/// What a click on a Browser row asks the canvas to show.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pick {
    /// Select this symbol, or this wire when `wire` is true.
    pub(crate) id: String,
    pub(crate) wire: bool,
    /// Center the view here.
    pub(crate) at: Point,
}

/// One item row, built once per project snapshot.
#[derive(Debug)]
struct Item {
    label: String,
    /// Hover text: a symbol's part name, or a net's pins.
    tip: String,
    /// The symbol or wire the row selects, with where it is. `None` for a row that can't be
    /// shown on the canvas, such as a net of free wires.
    target: Option<(String, Point)>,
}

/// The sheet-local lists, rebuilt when the project snapshot changes.
#[derive(Debug)]
struct Lists {
    project: Arc<Project>,
    symbols: Vec<Item>,
    nets: Vec<Item>,
}

impl Lists {
    /// List symbols by reference in natural order (R2 before R10), independent of the value
    /// displayed beside it. Nets use the Net Inspector's order and numbering; a net row selects
    /// a wire attached to one of its pins.
    fn read(project: Arc<Project>) -> Self {
        let mut symbols: Vec<_> = project
            .symbols()
            .iter()
            .map(|symbol| {
                let reference = project.reference(symbol);
                let value = project.value(symbol);
                let reference = if reference.is_empty() { symbol.id() } else { reference };
                // Reference and value, two spaces apart, as PCBcraft Next lists them.
                let item = Item {
                    label: if value.is_empty() { reference.clone() } else { format!("{reference}  {value}") },
                    tip: symbol.definition.name.clone(),
                    target: Some((symbol.id(), symbol.position)),
                };
                (reference, item)
            })
            .collect();
        symbols.sort_by(|a, b| natural(&a.0, &b.0));
        let symbols = symbols.into_iter().map(|(_, item)| item).collect();

        let connectivity = project.connectivity();
        let mut wire_for_net: Vec<Option<(String, Point)>> = vec![None; connectivity.nets.len()];
        for wire in project.wires() {
            let Some(index) = wire.terminals().into_iter().find_map(|pin| net_index(&connectivity.nets, pin)) else { continue };
            if wire_for_net[index].is_none()
                && let Some(at) = project.endpoint_position(wire.endpoints()[0])
            {
                wire_for_net[index] = Some((wire.id(), at));
            }
        }
        let nets = connectivity
            .nets
            .iter()
            .zip(wire_for_net)
            .enumerate()
            .map(|(index, (pins, target))| Item {
                label: format!("Net {}", index + 1),
                tip: if pins.is_empty() { "Free wire".to_owned() } else { format!("{} pins · {}", pins.len(), pin_labels(&project, pins)) },
                target,
            })
            .collect();
        Self { project, symbols, nets }
    }
}

/// Find the net holding a pin by its position in the connectivity's net list.
fn net_index(nets: &[Vec<Terminal>], pin: Terminal) -> Option<usize> {
    nets.iter().position(|net| net.contains(&pin))
}

/// Name a net's pins as REFERENCE.PIN, as the Net Inspector does.
fn pin_labels(project: &Project, pins: &[Terminal]) -> String {
    pins.iter()
        .filter_map(|&pin| Some(format!("{}.{}", project.reference(project.terminal_symbol(pin)?), pin.number())))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Compare references so that runs of digits order by value: C1, C2, C10, R1.
fn natural(a: &str, b: &str) -> Ordering {
    let split = |text: &str| {
        let digits = text.len() - text.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        let (prefix, number) = text.split_at(text.len() - digits);
        (prefix.to_owned(), number.parse::<u64>().ok())
    };
    split(a).cmp(&split(b)).then_with(|| a.cmp(b))
}

#[derive(Debug)]
pub(crate) struct ProjectBrowser {
    pub(crate) open: bool,
    /// Rows folded or unfolded away from their default.
    toggled: BTreeSet<Node>,
    lists: Option<Lists>,
    /// Whether the card is folded to its header.
    collapsed: bool,
    rect: Rect,
}

impl Default for ProjectBrowser {
    /// The Browser starts open, as KiCad's hierarchy navigator and PCBcraft Next's Browser do.
    fn default() -> Self {
        Self { open: true, toggled: BTreeSet::new(), lists: None, collapsed: false, rect: Rect::NOTHING }
    }
}

impl ProjectBrowser {
    /// Whether a folding row is open now.
    fn is_open(&self, node: Node) -> bool {
        node.open_by_default() != self.toggled.contains(&node)
    }

    /// Fold or unfold a row.
    fn toggle(&mut self, node: Node) {
        if !self.toggled.remove(&node) {
            self.toggled.insert(node);
        }
    }

    /// The actual visible card, including its collapsed header, or no rectangle while hidden.
    pub(crate) fn rect(&self) -> Rect {
        if self.open { self.rect } else { Rect::NOTHING }
    }

    /// Float the Browser over the top-left of `canvas` while a project is open, leaving the
    /// canvas its full size. Selected symbol and wire rows are highlighted. Returns the row picked this frame. The
    /// header folds the card to itself; View › Browser hides it.
    pub(crate) fn show(&mut self, ui: &Ui, canvas: Rect, session: &Session, selected: &pcbcraft_engine::selection::Selection) -> Option<Pick> {
        self.rect = Rect::NOTHING;
        if !self.open {
            return None;
        }
        let snapshot = session.snapshot()?;
        if !self.lists.as_ref().is_some_and(|lists| Arc::ptr_eq(&lists.project, &snapshot)) {
            self.lists = Some(Lists::read(snapshot));
        }
        let mut pick = None;
        let mut fold = None;
        let (card, toggle) = workspace::floating_card(ui, "project_browser", "Browser", canvas, self.collapsed, |ui| {
            let Some(lists) = &self.lists else { return };
            let mut folding = |ui: &mut Ui, node: Node, depth, icon, label: &str, selected| {
                let row = TreeRow { depth, fold: Some(self.is_open(node)), icon: Some(icon), label, tip: "", selected };
                let response = workspace::tree_row(ui, Id::new(("project_browser", node)), &row);
                if response.fold || (response.clicked && !matches!(node, Node::Sheet)) {
                    fold = Some(node);
                }
            };
            folding(ui, Node::Project, 0, TreeIcon::Folder, &lists.project.name, false);
            if !self.is_open(Node::Project) {
                return;
            }
            folding(ui, Node::Sheets, 1, TreeIcon::Folder, "Sheets", false);
            if !self.is_open(Node::Sheets) {
                return;
            }
            // The only sheet is always the one shown, so it is highlighted as the active sheet.
            folding(ui, Node::Sheet, 2, TreeIcon::Sheet, MAIN_SHEET, true);
            if !self.is_open(Node::Sheet) {
                return;
            }
            for (node, icon, title, items, current) in [
                (Node::Symbols, ToolIcon::Symbol, "Symbols", &lists.symbols, &selected.symbols),
                (Node::Nets, ToolIcon::Wire, "Nets", &lists.nets, &selected.wires),
            ] {
                folding(ui, node, 3, TreeIcon::Tool(icon), title, false);
                if !self.is_open(node) {
                    continue;
                }
                if items.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(4.0 * 16.0 + 18.0);
                        ui.label(dim(if node == Node::Symbols { "No symbols placed" } else { "No connected nets" }));
                    });
                }
                for (index, item) in items.iter().enumerate() {
                    let id = item.target.as_ref().map(|(id, _)| id.as_str());
                    let row = TreeRow {
                        depth: 4,
                        fold: None,
                        icon: Some(TreeIcon::Tool(icon)),
                        label: &item.label,
                        tip: &item.tip,
                        selected: id.is_some_and(|id| current.contains(id)),
                    };
                    if workspace::tree_row(ui, Id::new(("project_browser", node, index)), &row).clicked
                        && let Some((id, at)) = &item.target
                    {
                        pick = Some(Pick { id: id.clone(), wire: node == Node::Nets, at: *at });
                    }
                }
            }
        });
        if let Some(node) = fold {
            self.toggle(node);
        }
        if toggle {
            self.collapsed = !self.collapsed;
        }
        self.rect = card;
        pick
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use egui_kittest::{Harness, kittest::Queryable};
    use pcbcraft_engine::{CONNECT_PINS, PLACE_RESISTOR, SET_SYMBOL_FIELDS};
    use serde_json::json;

    use super::{Lists, natural};
    use crate::{PcbApp, tests::click};

    #[test]
    fn references_sort_by_number() {
        let mut references = vec!["R10", "C1", "R2", "R1", "LED"];
        references.sort_by(|a, b| natural(a, b));
        assert_eq!(references, ["C1", "LED", "R1", "R2", "R10"]);
        assert_eq!(natural("R01", "R1"), Ordering::Less);
    }

    /// Real Browser lists order by reference whether the displayed value has a suffix or
    /// ends in digits. Sorting never reorders the project or changes row targets.
    #[test]
    fn symbol_lists_sort_references_independently_of_values() {
        let mut app = wired_app();
        for value in ["1k", "1000"] {
            for (symbol, reference) in [("symbol-2", "R10"), ("symbol-3", "R2")] {
                app.session
                    .execute(
                        SET_SYMBOL_FIELDS,
                        &json!({"symbol":symbol,"reference":reference,"value":value,"expected_revision":app.session.revision()}),
                    )
                    .unwrap();
            }
            let before = app.session.inspect();
            let lists = Lists::read(app.session.snapshot().unwrap());
            let expected = [format!("R2  {value}"), format!("R10  {value}")];
            assert_eq!(lists.symbols.iter().map(|item| &item.label).collect::<Vec<_>>(), expected.iter().collect::<Vec<_>>());
            assert_eq!(lists.symbols[0].target.as_ref().unwrap().0, "symbol-3");
            assert_eq!(lists.symbols[1].target.as_ref().unwrap().0, "symbol-2");
            assert_eq!(app.session.inspect(), before);
        }
    }

    /// Two connected resistors, R1 at the origin and R2 to its right.
    fn wired_app() -> PcbApp {
        let mut app = PcbApp::default();
        app.session.execute("project.new", &json!({"name": "Divider"})).unwrap();
        for (x, revision) in [(0, 1), (20_000_000, 2)] {
            app.session.execute(PLACE_RESISTOR, &json!({"x_nm": x, "y_nm": 0, "expected_revision": revision})).unwrap();
        }
        app.session
            .execute(
                CONNECT_PINS,
                &json!({"from": {"symbol": "symbol-2", "terminal": "2"}, "to": {"symbol": "symbol-3", "terminal": "1"}, "expected_revision": 3}),
            )
            .unwrap();
        app
    }

    /// The tree opens on the project's sheet; Symbols and Nets unfold to sheet-local lists, and
    /// picking a row selects that item without editing the project.
    #[test]
    fn browser_lists_and_selects_sheet_items_without_editing() {
        let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app| app.frame(ui), wired_app());
        harness.run();
        harness.get_by_label("Collapse Divider");
        harness.get_by_label("Main");
        assert!(harness.query_by_label("R1  1k").is_none(), "item lists start folded");

        click(&mut harness, "Expand Symbols");
        click(&mut harness, "R2  1k");
        assert_eq!(harness.state().views.active.schematic.selection.single_symbol(), Some("symbol-3"));
        assert_eq!(harness.state().views.active.schematic.selection.single_wire(), None);

        click(&mut harness, "Expand Nets");
        click(&mut harness, "Net 1");
        assert_eq!(harness.state().views.active.schematic.selection.single_symbol(), None);
        assert_eq!(harness.state().views.active.schematic.selection.single_wire(), Some("wire-4"));
        assert_eq!(harness.state().session.revision(), 4);

        click(&mut harness, "Collapse Symbols");
        assert!(harness.query_by_label("R2  1k").is_none());
        harness.get_by_label("Net 1");
    }

    /// The Browser floats over the canvas: showing or hiding it leaves the canvas's size alone,
    /// and Fit keeps the drawing clear of the card.
    #[test]
    fn browser_floats_over_the_canvas_without_resizing_it() {
        let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app| app.frame(ui), wired_app());
        harness.run();
        let canvas = harness.get_by_label("Schematic canvas").rect();
        let card = harness.get_by_label("Collapse Browser").rect();
        assert!(canvas.contains_rect(card), "card {card:?} floats inside canvas {canvas:?}");

        click(&mut harness, "Fit schematic");
        let app = harness.state();
        let project = app.session.project().unwrap();
        let covered = app.browser.rect().right() - canvas.left();
        assert!(covered > 0.0);
        for symbol in project.symbols() {
            let at = app.views.active.camera.screen(symbol.position, canvas);
            assert!(at.x > canvas.left() + covered, "{} at {at:?} is under the Browser", symbol.id());
        }

        click(&mut harness, "View");
        click(&mut harness, "Toggle Browser");
        assert_eq!(harness.get_by_label("Schematic canvas").rect(), canvas);
        assert_eq!(harness.state().browser.rect(), egui::Rect::NOTHING);
    }

    /// Clicking a sheet name never folds it; only its arrow does. The header folds the card,
    /// and View › Browser hides it and brings it back.
    #[test]
    fn browser_folds_by_arrow_and_reopens_from_the_view_tab() {
        let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(|ui, app| app.frame(ui), wired_app());
        harness.run();
        click(&mut harness, "Main");
        harness.get_by_label("Symbols");
        click(&mut harness, "Collapse Main");
        assert!(harness.query_by_label("Symbols").is_none());

        click(&mut harness, "Collapse Browser");
        assert!(harness.query_by_label("Main").is_none());
        click(&mut harness, "Expand Browser");
        harness.get_by_label("Main");
        click(&mut harness, "View");
        click(&mut harness, "Toggle Browser");
        assert!(harness.query_by_label("Expand Browser").is_none() && harness.query_by_label("Collapse Browser").is_none());
        click(&mut harness, "Toggle Browser");
        harness.get_by_label("Main");
        assert_eq!(harness.state().session.revision(), 4);
    }
}
