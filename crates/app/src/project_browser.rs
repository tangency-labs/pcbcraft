//! One project navigator for schematic and board workspaces. Shared tree rows keep Sheets,
//! Boards and their object lists consistent. Navigation and lifecycle intents return to the
//! host; revision-checked engine commands own every authored sheet change.

use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

use egui::{Rect, Ui};
use pcbcraft_chrome::workspace::{self, ToolIcon, TreeIcon, TreeRow};
use pcbcraft_engine::{
    Project, Session,
    schematic::{Point, Terminal},
};

use crate::editor_view::DocumentTarget;
use pcbcraft_engine::{SheetTarget, board::view::BoardTarget, selection::Selection};

/// Folding uses document addresses, so names and row positions are never identities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Node {
    Project,
    Sheets,
    Sheet(SheetTarget),
    Symbols(SheetTarget),
    Nets(SheetTarget),
    Boards,
    Board(BoardTarget),
    Footprints(BoardTarget),
}

impl Node {
    /// Keep project/documents and footprint lists open initially; schematic lists start folded.
    fn open_by_default(self, active: DocumentTarget) -> bool {
        match self {
            Self::Sheet(sheet) => active == DocumentTarget::Sheet(sheet),
            Self::Board(board) => active == DocumentTarget::Board(board),
            Self::Symbols(_) | Self::Nets(_) => false,
            _ => true,
        }
    }
}

/// Revision-bound presentation intent; the host applies it after rendering and modal checks.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    OpenDocument(DocumentTarget),
    Schematic { sheet: SheetTarget, pick: Pick },
    Footprint { board: BoardTarget, id: String, at: Point },
    ManageSheet { command: &'static str, sheet: Option<SheetTarget> },
}

impl Action {
    /// Explicit navigation stays available in each row's context menu across workspaces.
    fn open_label(&self) -> Option<&'static str> {
        match self {
            Self::OpenDocument(DocumentTarget::Sheet(_)) => Some("Open in Schematic"),
            Self::OpenDocument(DocumentTarget::Board(_) | DocumentTarget::BoardWorkspace(_)) => Some("Open in PCB layout"),
            Self::Schematic { .. } => Some("Show in Schematic"),
            Self::Footprint { .. } => Some("Show in PCB layout"),
            Self::ManageSheet { .. } => None,
        }
    }

    /// The persistent document to reveal before selecting an object, independent of its label.
    pub(super) fn target(&self) -> Option<DocumentTarget> {
        match self {
            Self::OpenDocument(target) => Some(*target),
            Self::Schematic { sheet, .. } => Some(DocumentTarget::Sheet(*sheet)),
            Self::Footprint { board, .. } => Some(DocumentTarget::Board(*board)),
            Self::ManageSheet { .. } => None,
        }
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
    /// Hover text: a symbol's part name, a footprint definition or a net's pins.
    tip: String,
    /// The symbol or wire the row selects, with where it is. `None` for a row that can't be
    /// shown on the canvas, such as a net of free wires.
    target: Option<(String, Point)>,
}

/// Project and document lists, rebuilt when the immutable project snapshot changes.
#[derive(Debug)]
struct Lists {
    project: Arc<Project>,
    sheets: Vec<SheetItems>,
    boards: Vec<BoardItems>,
}

/// Cached object rows belong to exactly one persistent sheet address.
#[derive(Debug)]
struct SheetItems {
    target: SheetTarget,
    name: String,
    ancestors: Vec<SheetTarget>,
    symbols: Vec<Item>,
    nets: Vec<Item>,
}

/// A resolved board's labels and object targets, cached with the immutable project snapshot.
#[derive(Debug)]
struct BoardItems {
    target: BoardTarget,
    name: String,
    footprints: Vec<Item>,
}

impl Lists {
    /// List symbols by reference in natural order (R2 before R10), independent of the value
    /// displayed beside it. Nets use the inspector's resolved names and fallback numbering;
    /// a net row selects a wire belonging to its endpoint group. Each board's footprints also use natural reference
    /// order, and each footprint row targets its stable ID and board position.
    fn read(project: Arc<Project>) -> Self {
        let mut sheets = Vec::new();
        for root in project.sheets().filter(|s| project.parent_sheet(s.target).is_none()) {
            append_sheet_tree(&project, root.target, &[], &mut sheets);
        }
        let boards = project
            .boards()
            .map(|board| {
                let mut footprints = board
                    .footprints()
                    .map(|footprint| {
                        let reference = footprint.reference.to_owned();
                        let item = Item {
                            label: if footprint.value.is_empty() { reference.clone() } else { format!("{reference}  {}", footprint.value) },
                            tip: footprint.definition.to_owned(),
                            target: Some((footprint.id, footprint.position)),
                        };
                        (reference, item)
                    })
                    .collect::<Vec<_>>();
                footprints.sort_by(|a, b| natural(&a.0, &b.0));
                BoardItems { target: board.target, name: board.name().to_owned(), footprints: footprints.into_iter().map(|(_, item)| item).collect() }
            })
            .collect();
        Self { project, sheets, boards }
    }
}

impl SheetItems {
    /// Build naturally ordered symbols and local net targets from an explicitly scoped snapshot.
    fn read(project: &Project) -> Self {
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
            let Some(index) = wire.endpoints().into_iter().find_map(|end| connectivity.net_index_for_endpoint(end)) else { continue };
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
                label: connectivity.net_name(index),
                tip: if pins.is_empty() {
                    if connectivity.net_names(index).is_empty() { "Free wire".into() } else { "No component pins".into() }
                } else {
                    format!("{} pins · {}", pins.len(), pin_labels(project, pins))
                },
                target,
            })
            .collect();
        Self { target: project.sheet_target(), name: project.sheet_name().to_owned(), ancestors: Vec::new(), symbols, nets }
    }
}

/// Cache a bounded pre-order traversal, so sibling order remains the project's authored order.
fn append_sheet_tree(project: &Project, target: SheetTarget, ancestors: &[SheetTarget], out: &mut Vec<SheetItems>) {
    let Some(projection) = project.for_sheet(target) else {
        return;
    };
    let mut row = SheetItems::read(&projection);
    row.ancestors = ancestors.to_vec();
    out.push(row);
    let mut path = ancestors.to_vec();
    path.push(target);
    for child in project.sheets().filter(|s| project.parent_sheet(s.target) == Some(target)) {
        append_sheet_tree(project, child.target, &path, out);
    }
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
    toggled: HashMap<Node, bool>,
    lists: Option<Lists>,
    /// Whether the card is folded to its header.
    collapsed: bool,
    rect: Rect,
}

impl Default for ProjectBrowser {
    /// The Browser starts open, as KiCad's hierarchy navigator and PCBcraft Next's Browser do.
    fn default() -> Self {
        Self { open: true, toggled: HashMap::new(), lists: None, collapsed: false, rect: Rect::NOTHING }
    }
}

impl ProjectBrowser {
    /// Remember visibility and header collapse without cached project objects or selection.
    pub(super) fn saved_state(&self) -> [bool; 2] {
        [self.open, self.collapsed]
    }

    /// Restore layout with a fresh document cache.
    pub(super) fn from_state([open, collapsed]: [bool; 2]) -> Self {
        Self { open, collapsed, ..Self::default() }
    }

    /// The actual visible card, including its collapsed header, or no rectangle while hidden.
    pub(crate) fn rect(&self) -> Rect {
        if self.open { self.rect } else { Rect::NOTHING }
    }

    /// Float the common project tree over the schematic canvas without changing its geometry.
    /// Board hosts use the same contents in their bounded Browser area above Layers.
    pub(crate) fn show(
        &mut self,
        ui: &Ui,
        canvas: Rect,
        session: &Session,
        active: DocumentTarget,
        selected: &Selection,
        footprints: &BTreeSet<String>,
    ) -> Option<Action> {
        self.rect = Rect::NOTHING;
        if !self.open {
            return None;
        }
        let snapshot = session.snapshot()?;
        let mut action = None;
        let (card, toggle) = workspace::floating_card(ui, "project_browser", "Browser", canvas, self.collapsed, |ui| {
            action = self.tree(ui, snapshot, active, selected, footprints);
        });
        if toggle {
            self.collapsed = !self.collapsed;
        }
        self.rect = card;
        action
    }

    /// Render identical project/document rows in either workspace. Cached labels rebuild after
    /// edits; a different project resets folding. Item selection belongs only to the active
    /// document. Ordinary clicks stay in the current workspace; context
    /// menu actions explicitly open a document or object in another workspace.
    pub(super) fn tree(
        &mut self,
        ui: &mut Ui,
        snapshot: Arc<Project>,
        active: DocumentTarget,
        selected: &Selection,
        footprints: &BTreeSet<String>,
    ) -> Option<Action> {
        if !self.lists.as_ref().is_some_and(|lists| Arc::ptr_eq(&lists.project, &snapshot)) {
            if self.lists.as_ref().is_some_and(|lists| lists.project.sheet_target() != snapshot.sheet_target()) {
                self.toggled.clear();
            }
            self.lists = Some(Lists::read(snapshot));
        }
        let lists = self.lists.as_ref()?;
        let mut tree = Tree {
            ancestors: lists.sheets.iter().find(|s| active == DocumentTarget::Sheet(s.target)).map_or(&[][..], |s| s.ancestors.as_slice()),
            toggled: &self.toggled,
            active,
            sheet_count: lists.sheets.len(),
            folded: None,
            reset: None,
            picked: None,
        };
        if tree.branch(ui, Node::Project, 0, TreeIcon::Folder, &lists.project.name, false, None) {
            if tree.branch(ui, Node::Sheets, 1, TreeIcon::Folder, "Sheets", false, None) {
                for items in &lists.sheets {
                    if items.ancestors.iter().any(|parent| !tree.open(Node::Sheet(*parent))) {
                        continue;
                    }
                    let depth = items.ancestors.len();
                    let sheet = items.target;
                    if !tree.branch(
                        ui,
                        Node::Sheet(sheet),
                        2 + depth,
                        TreeIcon::Sheet,
                        &items.name,
                        active == DocumentTarget::Sheet(sheet),
                        Some(Action::OpenDocument(DocumentTarget::Sheet(sheet))),
                    ) {
                        continue;
                    }
                    for (node, icon, title, items, current) in [
                        (Node::Symbols(sheet), ToolIcon::Symbol, "Symbols", &items.symbols, &selected.symbols),
                        (Node::Nets(sheet), ToolIcon::Wire, "Nets", &items.nets, &selected.wires),
                    ] {
                        if !tree.branch(ui, node, 3 + depth, TreeIcon::Tool(icon), title, false, None) {
                            continue;
                        }
                        if items.is_empty() {
                            empty_row(ui, 4 + depth, if matches!(node, Node::Symbols(_)) { "No symbols placed" } else { "No connected nets" });
                        }
                        for (index, item) in items.iter().enumerate() {
                            let id = item.target.as_ref().map(|(id, _)| id.as_str());
                            let row = TreeRow {
                                depth: 4 + depth,
                                fold: None,
                                icon: Some(TreeIcon::Tool(icon)),
                                label: &item.label,
                                accessibility_label: None,
                                tip: &item.tip,
                                selected: active == DocumentTarget::Sheet(sheet) && id.is_some_and(|id| current.contains(id)),
                            };
                            let widget = id.map_or_else(|| ui.make_persistent_id((node, index)), |id| ui.make_persistent_id((node, id)));
                            let response = workspace::tree_row(ui, widget, &row);
                            if let Some((id, at)) = &item.target {
                                tree.item(
                                    response,
                                    Action::Schematic { sheet, pick: Pick { id: id.clone(), wire: matches!(node, Node::Nets(_)), at: *at } },
                                );
                            }
                        }
                    }
                }
            }
            if tree.branch(ui, Node::Boards, 1, TreeIcon::Folder, "Boards", false, None) {
                for board in &lists.boards {
                    if !tree.branch(
                        ui,
                        Node::Board(board.target),
                        2,
                        TreeIcon::Board,
                        &board.name,
                        active == DocumentTarget::Board(board.target),
                        Some(Action::OpenDocument(DocumentTarget::Board(board.target))),
                    ) {
                        continue;
                    }
                    let node = Node::Footprints(board.target);
                    if !tree.branch(ui, node, 3, TreeIcon::Board, "Footprints", false, None) {
                        continue;
                    }
                    if board.footprints.is_empty() {
                        empty_row(ui, 4, "No footprints placed");
                    }
                    for item in &board.footprints {
                        let Some((id, at)) = &item.target else {
                            continue;
                        };
                        let access = format!("Footprint {}", item.label);
                        let row = TreeRow {
                            depth: 4,
                            fold: None,
                            icon: Some(TreeIcon::Board),
                            label: &item.label,
                            accessibility_label: Some(&access),
                            tip: &item.tip,
                            selected: active == DocumentTarget::Board(board.target) && footprints.contains(id),
                        };
                        let response = workspace::tree_row(ui, ui.make_persistent_id((node, id)), &row);
                        tree.item(response, Action::Footprint { board: board.target, id: id.clone(), at: *at });
                    }
                }
            }
        }
        let Tree { picked, folded, reset, .. } = tree;
        if let Some((node, open)) = folded {
            self.toggled.insert(node, open);
        }
        if let Some(node) = reset {
            self.toggled.remove(&node);
        }
        picked
    }
}

/// Only transient fold and click intents are collected while snapshot data is borrowed.
struct Tree<'a> {
    toggled: &'a HashMap<Node, bool>,
    ancestors: &'a [SheetTarget],
    active: DocumentTarget,
    sheet_count: usize,
    folded: Option<(Node, bool)>,
    reset: Option<Node>,
    picked: Option<Action>,
}

impl Tree<'_> {
    /// A single click may navigate sheets or boards within the current workspace, never across it.
    fn same_workspace(&self, action: &Action) -> bool {
        matches!(
            (self.active, action.target()),
            (DocumentTarget::Sheet(_), Some(DocumentTarget::Sheet(_))) | (DocumentTarget::Board(_), Some(DocumentTarget::Board(_)))
        )
    }

    /// Cross-workspace opening is an explicit menu action, shared by document and object rows.
    fn open_menu(&mut self, ui: &mut Ui, action: &Action) -> bool {
        let Some(label) = action.open_label() else {
            return false;
        };
        let clicked = ui.button(label).clicked();
        if clicked {
            self.picked = Some(action.clone());
            ui.close();
        }
        clicked
    }

    /// Select objects on ordinary clicks only in their workspace; explicit opens retain the
    /// existing revision-checked navigation, selection and centering path.
    fn item(&mut self, response: workspace::TreeResponse, action: Action) {
        response.response.context_menu(|ui| {
            self.open_menu(ui, &action);
        });
        if response.clicked && self.same_workspace(&action) {
            self.picked = Some(action);
        }
    }

    /// Explicit folds win; otherwise unfold the path to the active child document.
    fn open(&self, node: Node) -> bool {
        self.toggled
            .get(&node)
            .copied()
            .unwrap_or_else(|| node.open_by_default(self.active) || matches!(node,Node::Sheet(sheet) if self.ancestors.contains(&sheet)))
    }

    /// Body clicks navigate within the current workspace or fold other branches. Context menus
    /// explicitly cross workspaces; sheet lifecycle actions retain their bounds.
    #[allow(clippy::too_many_arguments)]
    fn branch(&mut self, ui: &mut Ui, node: Node, depth: usize, icon: TreeIcon, label: &str, selected: bool, pick: Option<Action>) -> bool {
        let open = self.open(node);
        let access = matches!(node, Node::Board(_)).then(|| format!("Board {label}"));
        let row = TreeRow { depth, fold: Some(open), icon: Some(icon), label, accessibility_label: access.as_deref(), tip: "", selected };
        let response = workspace::tree_row(ui, ui.make_persistent_id(("project_browser", node)), &row);
        if pick.is_some() || matches!(node, Node::Sheets | Node::Sheet(_)) {
            response.response.context_menu(|ui| {
                if let Some(pick) = &pick
                    && self.open_menu(ui, pick)
                {
                    self.reset = Some(node);
                }
                if matches!(node, Node::Sheets | Node::Sheet(_))
                    && ui.add_enabled(self.sheet_count < pcbcraft_engine::sheets::MAX_SHEETS, egui::Button::new("New Sheet…")).clicked()
                {
                    self.picked = Some(Action::ManageSheet { command: pcbcraft_engine::sheets::CREATE_SHEET, sheet: None });
                    ui.close();
                }
                if let Node::Sheet(sheet) = node {
                    if ui.button("Rename Sheet…").clicked() {
                        self.picked = Some(Action::ManageSheet { command: pcbcraft_engine::sheets::RENAME_SHEET, sheet: Some(sheet) });
                        ui.close();
                    }
                    if ui.add_enabled(self.sheet_count > 1, egui::Button::new("Delete Sheet…")).clicked() {
                        self.picked = Some(Action::ManageSheet { command: pcbcraft_engine::sheets::DELETE_SHEET, sheet: Some(sheet) });
                        ui.close();
                    }
                }
            });
        }
        if response.fold {
            self.folded = Some((node, !open));
        }
        if response.clicked {
            if let Some(pick) = pick.filter(|pick| self.same_workspace(pick)) {
                self.reset = Some(node);
                self.picked = Some(pick);
            } else {
                self.folded = Some((node, !open));
            }
        }
        open
    }
}

/// Empty lists use the shared row metrics, aligned with their future item labels.
fn empty_row(ui: &mut Ui, depth: usize, label: &str) {
    workspace::tree_row(
        ui,
        ui.make_persistent_id(("browser_empty", label, depth)),
        &TreeRow { depth, fold: None, icon: None, label, accessibility_label: None, tip: "", selected: false },
    );
}

#[cfg(test)]
mod tests {
    mod navigation;
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
            assert_eq!(lists.sheets[0].symbols.iter().map(|item| &item.label).collect::<Vec<_>>(), expected.iter().collect::<Vec<_>>());
            assert_eq!(lists.sheets[0].symbols[0].target.as_ref().unwrap().0, "symbol-3");
            assert_eq!(lists.sheets[0].symbols[1].target.as_ref().unwrap().0, "symbol-2");
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
