//! Inspect the engine's resolved nets using the same card and tree primitives as Properties/Browser.
use egui::{Rect, Ui};
use pcbcraft_chrome::{properties, shell, workspace};
use pcbcraft_engine::{
    Project,
    nets::{SheetOverview, SheetOverviewBasis, Via},
    schematic::Terminal,
};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Debug)]
pub(super) struct NetInspector {
    pub open: bool,
    pub(super) rect: Rect,
    collapsed: bool,
    query: String,
    folded: BTreeSet<usize>,
    project: Option<Arc<Project>>,
    snapshot: Option<Snapshot>,
    /// Cross-sheet nets folded by the user, by net name, kept while the project changes.
    folded_sheets: BTreeSet<String>,
    /// Snapshot the inter-sheet overview was last checked against.
    overview_project: Option<Arc<Project>>,
    /// What the shown overview was derived from; anchors are resolved again only when it changes.
    overview_basis: Option<SheetOverviewBasis>,
    overview: Option<Result<SheetOverview, String>>,
}
impl Default for NetInspector {
    /// Match the Properties card's initial state without reserving a sidebar in the workspace.
    fn default() -> Self {
        Self {
            open: false,
            rect: Rect::NOTHING,
            collapsed: false,
            query: String::new(),
            folded: BTreeSet::new(),
            project: None,
            snapshot: None,
            folded_sheets: BTreeSet::new(),
            overview_project: None,
            overview_basis: None,
            overview: None,
        }
    }
}
#[derive(Debug)]
struct Net {
    name: String,
    aliases: Vec<String>,
    pins: Vec<String>,
}
#[derive(Debug)]
struct Snapshot {
    nets: Vec<Net>,
    unconnected: Vec<String>,
}
impl Snapshot {
    /// Resolve displayed references from the same typed endpoints used by command inspection.
    fn read(project: &Project) -> Self {
        let pins = |pins: &[Terminal]| {
            pins.iter()
                .filter_map(|&pin| {
                    Some(format!(
                        "{}.{}",
                        project.terminal_symbol(pin)?.display_reference(&project.reference(project.terminal_symbol(pin)?)),
                        pin.number()
                    ))
                })
                .collect::<Vec<_>>()
        };
        let graph = project.connectivity();
        Self {
            nets: graph
                .nets
                .iter()
                .enumerate()
                .map(|(i, net)| Net { name: graph.net_name(i), aliases: graph.net_names(i).to_vec(), pins: pins(net) })
                .collect(),
            unconnected: pins(&graph.unconnected),
        }
    }
}
/// Tree rows use the Browser's actual metrics, disclosure control and typography.
fn row(ui: &mut Ui, key: impl std::hash::Hash + std::fmt::Debug, label: &str, depth: usize, open: Option<bool>, tip: &str) -> bool {
    let accessible = open.is_some().then(|| format!("Net {label}"));
    tree_row(ui, key, label, accessible.as_deref(), depth, open, tip, false)
}

/// One Browser-style tree row; returns whether it was clicked or its disclosure toggled.
#[allow(clippy::too_many_arguments)]
fn tree_row(
    ui: &mut Ui,
    key: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    accessibility_label: Option<&str>,
    depth: usize,
    open: Option<bool>,
    tip: &str,
    selected: bool,
) -> bool {
    let response = workspace::tree_row(
        ui,
        ui.scope_id().with(key),
        &workspace::TreeRow { depth, fold: open, icon: None, label, accessibility_label, tip, selected },
    );
    response.fold || response.clicked
}

/// Sheet row tooltip: full sheet name, all cross-sheet connection types, and part-pin count.
fn sheet_tip(page: usize, name: &str, via: &[Via], terminals: usize) -> String {
    let pins = match terminals {
        0 => "No part pins on this sheet".into(),
        1 => "1 part pin on this sheet".into(),
        count => format!("{count} part pins on this sheet"),
    };
    format!("{} · {pins}", sheet_line(page, name, via))
}

/// Sheet row text: page number, sheet name and how the net arrives there.
fn sheet_line(page: usize, name: &str, via: &[Via]) -> String {
    if via.is_empty() {
        format!("{page} · {name}")
    } else {
        format!("{page} · {name} — {}", via.iter().map(|via| via.label()).collect::<Vec<_>>().join(", "))
    }
}
impl NetInspector {
    /// Read the inter-sheet overview of a changed snapshot. Edits that keep every connection,
    /// such as moves, leave the basis equal and skip resolving net anchors again.
    fn refresh_overview(&mut self, project: &Project) {
        match project.sheet_overview_basis() {
            Ok(basis) => {
                if self.overview_basis.as_ref() != Some(&basis) || self.overview.is_none() {
                    self.overview = Some(basis.overview(project, None));
                    self.overview_basis = Some(basis);
                }
            }
            Err(error) => {
                self.overview = Some(Err(error));
                self.overview_basis = None;
            }
        }
    }

    /// Float a stable Craft card inside its pane, sharing the Properties inspector
    /// footprint without resizing the canvas or changing document history.
    pub(super) fn show(&mut self, ui: &Ui, project: Option<Arc<Project>>, area: Rect) {
        if !self.open {
            self.rect = Rect::NOTHING;
            return;
        }
        let active = project.as_ref().map(|project| project.sheet_target().id());
        if let Some(project) = project {
            if !self.overview_project.as_ref().is_some_and(|p| p.same_sheet_overview(&project)) {
                self.refresh_overview(&project);
                self.overview_project = Some(project.clone());
            }
            if !self.project.as_ref().is_some_and(|p| p.same_connectivity(&project)) {
                self.snapshot = Some(Snapshot::read(&project));
                self.project = Some(project);
                self.folded.clear();
            }
        } else {
            self.project = None;
            self.snapshot = None;
            self.overview_project = None;
            self.overview_basis = None;
            self.overview = None;
        }
        let width = if area.width() < 600. { area.width() } else { 260. };
        let (rect, collapse) =
            workspace::floating_card_width(ui, ui.scope_id().with("net_inspector"), "Net Inspector", area, self.collapsed, true, width, |ui| {
                properties::inspector_body(ui);
                pcbcraft_chrome::controls::search(
                    ui,
                    ui.make_persistent_id("net_search"),
                    &mut self.query,
                    "Search nets or pins",
                    "Search nets or pins",
                );
                let Some(snapshot) = &self.snapshot else {
                    ui.label(shell::dim("No project open"));
                    return;
                };
                let query = self.query.trim().to_lowercase();
                pcbcraft_chrome::library_sources::heading(ui, "Connected nets", snapshot.nets.len());
                if snapshot.nets.is_empty() {
                    ui.label(shell::dim("No connected nets"));
                }
                for (index, net) in snapshot.nets.iter().enumerate().filter(|(_, net)| {
                    query.is_empty() || net.name.to_lowercase().contains(&query) || net.pins.iter().any(|pin| pin.to_lowercase().contains(&query))
                }) {
                    let expanded = !self.folded.contains(&index) || !query.is_empty();
                    let tip = if net.aliases.len() > 1 {
                        format!("{} pins; electrical names: {}", net.pins.len(), net.aliases.join(", "))
                    } else {
                        format!("{} pins", net.pins.len())
                    };
                    if row(ui, ("net", index), &net.name, 0, (!net.pins.is_empty()).then_some(expanded), &tip) && !self.folded.remove(&index) {
                        self.folded.insert(index);
                    }
                    if expanded {
                        for pin in &net.pins {
                            row(ui, ("net_pin", index, pin), pin, 1, None, "");
                        }
                    }
                }
                match &self.overview {
                    Some(Err(error)) => {
                        pcbcraft_chrome::library_sources::heading(ui, "Across sheets", 0);
                        ui.label(shell::dim(format!("Can't list nets across sheets: {error}")));
                    }
                    Some(Ok(overview)) => {
                        pcbcraft_chrome::library_sources::heading(ui, "Across sheets", overview.nets.len());
                        if overview.nets.is_empty() {
                            ui.label(shell::dim("No nets cross sheets"));
                        }
                        for (index, net) in overview.nets.iter().enumerate().filter(|(_, net)| {
                            query.is_empty()
                                || net.name.to_lowercase().contains(&query)
                                || net.sheets.iter().any(|sheet| sheet.name.to_lowercase().contains(&query))
                        }) {
                            let expanded = !self.folded_sheets.contains(&net.name) || !query.is_empty();
                            let accessible = format!("Cross-sheet net {}", net.name);
                            let tip = if net.names.len() > 1 {
                                format!("{} sheets; electrical names: {}", net.sheets.len(), net.names.join(", "))
                            } else {
                                format!("{} sheets", net.sheets.len())
                            };
                            if tree_row(ui, ("sheet_net", index), &net.name, Some(&accessible), 0, Some(expanded), &tip, false)
                                && !self.folded_sheets.remove(&net.name)
                            {
                                self.folded_sheets.insert(net.name.clone());
                            }
                            if expanded {
                                for sheet in &net.sheets {
                                    let line = sheet_line(sheet.page, &sheet.name, &sheet.via);
                                    let tip = sheet_tip(sheet.page, &sheet.name, &sheet.via, sheet.terminals);
                                    tree_row(ui, ("sheet_net_page", index, sheet.sheet), &line, None, 1, None, &tip, active == Some(sheet.sheet));
                                }
                            }
                        }
                    }
                    None => {}
                }
                properties::section(ui, "Unconnected pins");
                if snapshot.unconnected.is_empty() {
                    ui.label(shell::dim("No unconnected pins"));
                }
                for pin in snapshot.unconnected.iter().filter(|pin| query.is_empty() || pin.to_lowercase().contains(&query)) {
                    row(ui, ("unconnected", pin), pin, 0, None, "");
                }
            });
        self.rect = rect;
        if collapse {
            self.collapsed = !self.collapsed;
        }
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    use pcbcraft_engine::{CONNECT_PINS, PLACE_RESISTOR};
    use serde_json::json;

    use crate::{PcbApp, tests::click};

    /// A three-sheet project with a CAN_H global label on pages 1 and 3 only.
    fn three_sheets() -> PcbApp {
        use pcbcraft_engine::{net_labels, sheets};
        let mut app = PcbApp::default();
        app.session.execute("project.new", &json!({"name": "bus"})).unwrap();
        for name in ["Bus", "Radio"] {
            let revision = app.session.revision();
            app.session.execute(sheets::CREATE_SHEET, &json!({"name": name, "expected_revision": revision})).unwrap();
        }
        let pages = app.session.project().unwrap().page_order();
        for sheet in [pages[0], pages[2]] {
            let revision = app.session.revision();
            app.session.execute(PLACE_RESISTOR, &json!({"sheet": sheet.id(), "x_nm": 0, "y_nm": 0, "expected_revision": revision})).unwrap();
            let projection = app.session.project().unwrap().for_sheet(sheet).unwrap();
            let at = projection.terminal_position(projection.symbols()[0].terminals()[0]).unwrap();
            let revision = app.session.revision();
            let label =
                json!({"sheet": sheet.id(), "name": "CAN_H", "style": "global", "x_nm": at.x_nm, "y_nm": at.y_nm, "expected_revision": revision});
            app.session.execute(net_labels::ADD, &label).unwrap();
        }
        app
    }

    /// The Across sheets section lists CAN_H on pages 1 and 3, marks the active sheet, filters by
    /// sheet name, folds by mouse and keyboard, and never edits the project.
    #[test]
    fn net_inspector_lists_nets_across_sheets() {
        use egui::accesskit::Toggled;
        let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| app.frame(ui), three_sheets());
        harness.run();
        let revision = harness.state().session.revision();
        click(&mut harness, "Toggle Net Inspector");
        harness.get_by_label("Cross-sheet net CAN_H");
        let main = "1 · Main — global label";
        let radio = "3 · Radio — global label";
        assert_eq!(harness.get_by_label(main).accesskit_node().toggled(), Some(Toggled::True));
        assert_eq!(harness.get_by_label(radio).accesskit_node().toggled(), Some(Toggled::False));
        assert!(harness.query_by_label_contains("2 · Bus").is_none());

        click(&mut harness, "Cross-sheet net CAN_H");
        assert!(harness.query_by_label(radio).is_none());
        harness.get_by_label("Cross-sheet net CAN_H").focus();
        harness.run();
        assert!(harness.get_by_label("Cross-sheet net CAN_H").is_focused());
        harness.key_press(egui::Key::Enter);
        harness.run();
        harness.get_by_label(radio);

        click(&mut harness, "Search nets or pins");
        harness.get_by_label("Search nets or pins").type_text("radio");
        harness.run();
        harness.get_by_label("Cross-sheet net CAN_H");
        harness.get_by_label(radio);
        harness.get_by_label("Search nets or pins").type_text("x");
        harness.run();
        assert!(harness.query_by_label("Cross-sheet net CAN_H").is_none());
        assert_eq!(harness.state().session.revision(), revision);
    }

    /// One sheet has nothing to cross; undoing the label that joined two sheets empties the list.
    #[test]
    fn net_inspector_across_sheets_empty_and_after_undo() {
        let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| app.frame(ui), three_sheets());
        harness.run();
        click(&mut harness, "Toggle Net Inspector");
        harness.get_by_label("Cross-sheet net CAN_H");
        click(&mut harness, "Undo");
        harness.get_by_label("No nets cross sheets");
        assert!(harness.query_by_label("Cross-sheet net CAN_H").is_none());
        click(&mut harness, "Redo");
        harness.get_by_label("Cross-sheet net CAN_H");
    }

    /// Sheet row tooltips include the full sheet name, all connection types, and the part-pin count.
    #[test]
    fn sheet_row_tooltips_count_part_pins() {
        use pcbcraft_engine::nets::Via;
        assert_eq!(super::sheet_tip(1, "Main", &[], 0), "1 · Main · No part pins on this sheet");
        assert_eq!(
            super::sheet_tip(2, "Radio Front End and RF Interface", &[Via::GlobalLabel, Via::HierarchicalLabel], 1),
            "2 · Radio Front End and RF Interface — global label, hierarchical label · 1 part pin on this sheet"
        );
        assert_eq!(super::sheet_tip(3, "Power", &[Via::SheetPin], 2), "3 · Power — sheet pin · 2 part pins on this sheet");
    }

    /// Hovering a truncated long sheet row in the Net Inspector shows the full sheet name,
    /// all cross-sheet connection types, and the part-pin count in its tooltip.
    #[test]
    fn net_inspector_long_sheet_name_tooltip_shows_full_name_via_and_pins() {
        use pcbcraft_engine::{DRAW_WIRE, WireEndpoint, hierarchy, net_labels, schematic::Point};
        let mut app = PcbApp::default();
        app.session.execute("project.new", &json!({"name": "radio"})).unwrap();
        let parent = app.session.project().unwrap().sheet_target();
        let long_name = "Radio Front End and RF Interface";
        let rev = app.session.revision();
        app.session.execute(hierarchy::CREATE_CHILD, &json!({"name": long_name, "x_nm": 0, "y_nm": 0, "expected_revision": rev})).unwrap();
        let child = app.session.project().unwrap().sheets().nth(1).unwrap().target;
        let rev = app.session.revision();
        app.session
            .execute(
                hierarchy::ADD_LABEL,
                &json!({"sheet": child.id(), "name": "SIGNAL", "direction": "input", "x_nm": 0, "y_nm": 0, "expected_revision": rev}),
            )
            .unwrap();
        let label_id = app.session.project().unwrap().for_sheet(child).unwrap().hierarchical_labels()[0].id;
        for sheet in [parent, child] {
            let rev = app.session.revision();
            app.session.execute(PLACE_RESISTOR, &json!({"sheet": sheet.id(), "x_nm": 10_000_000, "y_nm": 0, "expected_revision": rev})).unwrap();
            let projection = app.session.project().unwrap().for_sheet(sheet).unwrap();
            let terminal = projection.symbols()[0].terminals()[0];
            let endpoint = if sheet == parent { WireEndpoint::SheetPin(label_id) } else { WireEndpoint::Label(label_id) };
            let from = projection.terminal_position(terminal).unwrap();
            let to = projection.endpoint_position(endpoint).unwrap();
            let bends = if from.x_nm != to.x_nm && from.y_nm != to.y_nm { vec![Point { x_nm: from.x_nm, y_nm: to.y_nm }] } else { vec![] };
            let rev = app.session.revision();
            app.session
                .execute(
                    DRAW_WIRE,
                    &json!({"sheet": sheet.id(), "from": terminal.inspect(), "to": endpoint.inspect(), "bends": bends, "auto_heal": false, "expected_revision": rev}),
                )
                .unwrap();
            let rev = app.session.revision();
            app.session
                .execute(
                    net_labels::ADD,
                    &json!({"sheet": sheet.id(), "name": "RF_BUS", "style": "global", "x_nm": from.x_nm, "y_nm": from.y_nm, "expected_revision": rev}),
                )
                .unwrap();
        }

        let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| app.frame(ui), app);
        harness.ctx.global_style_mut(|style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.show_tooltips_only_when_still = false;
        });
        harness.run();
        click(&mut harness, "Toggle Net Inspector");
        let row_label = format!("2 · {long_name} — global label, hierarchical label");
        let row = harness.get_by_label(row_label.as_str()).rect();
        harness.hover_at(row.center());
        harness.run_steps(3);
        let expected_tip = format!("{row_label} · 1 part pin on this sheet");
        assert!(
            harness.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(glyphs) if glyphs.galley.job.text == expected_tip)),
            "expected tooltip {expected_tip:?}"
        );
    }

    /// The three-sheet project with the Net Inspector open on page 1.
    fn across_sheets_open() -> Harness<'static, PcbApp> {
        let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| app.frame(ui), three_sheets());
        harness.run();
        click(&mut harness, "Toggle Net Inspector");
        harness
    }

    /// Visual state `filled`: CAN_H expanded with pages 1 and 3, page 1 highlighted.
    #[test]
    fn across_sheets_shot_filled() {
        let harness = across_sheets_open();
        harness.get_by_label("Cross-sheet net CAN_H");
        harness.get_by_label("1 · Main — global label");
        harness.get_by_label("3 · Radio — global label");
    }

    /// Visual state `empty`: a one-sheet project has no nets that cross sheets.
    #[test]
    fn across_sheets_shot_empty() {
        let mut app = PcbApp::default();
        app.session.execute("project.new", &json!({"name": "single"})).unwrap();
        app.session.execute(PLACE_RESISTOR, &json!({"x_nm": 0, "y_nm": 0, "expected_revision": 1})).unwrap();
        let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| app.frame(ui), app);
        harness.run();
        click(&mut harness, "Toggle Net Inspector");
        harness.get_by_label("No nets cross sheets");
        assert!(harness.query_by_label_contains("Cross-sheet net").is_none());
    }

    /// Visual state `search`: a sheet name in the search box keeps the net and its sheets.
    #[test]
    fn across_sheets_shot_search() {
        let mut harness = across_sheets_open();
        click(&mut harness, "Search nets or pins");
        harness.get_by_label("Search nets or pins").type_text("radio");
        harness.run();
        harness.get_by_label("Cross-sheet net CAN_H");
        harness.get_by_label("3 · Radio — global label");
    }

    /// Visual state `folded`: CAN_H collapsed to its heading row.
    #[test]
    fn across_sheets_shot_folded() {
        let mut harness = across_sheets_open();
        click(&mut harness, "Cross-sheet net CAN_H");
        harness.get_by_label("Cross-sheet net CAN_H");
        assert!(harness.query_by_label("1 · Main — global label").is_none());
    }

    /// Visual state `keyboard-focus`: keyboard focus on the CAN_H row.
    #[test]
    fn across_sheets_shot_keyboard_focus() {
        let mut harness = across_sheets_open();
        harness.get_by_label("Cross-sheet net CAN_H").focus();
        harness.run();
        assert!(harness.get_by_label("Cross-sheet net CAN_H").is_focused());
        harness.get_by_label("3 · Radio — global label");
    }

    /// Visual state `error`: the overview could not be read, so the section says why and
    /// keeps the rest of the inspector.
    #[test]
    fn across_sheets_shot_error() {
        let mut harness = across_sheets_open();
        harness.state_mut().net_inspector.overview = Some(Err("Missing net anchor while building the sheet overview.".into()));
        harness.run();
        harness.get_by_label("Can't list nets across sheets: Missing net anchor while building the sheet overview.");
        assert!(harness.query_by_label("Cross-sheet net CAN_H").is_none());
        harness.get_by_label("Unconnected pins");
    }

    /// The ribbon toggle leaves the project alone while app bar history updates the displayed nets.
    #[test]
    fn net_inspector_tracks_wiring_and_history_without_editing_the_project() {
        let mut app = PcbApp::default();
        app.session.execute("project.new", &json!({"name": "wiring"})).unwrap();
        for (x, revision) in [(0, 1), (20_000_000, 2)] {
            app.session.execute(PLACE_RESISTOR, &json!({"x_nm": x, "y_nm": 0, "expected_revision": revision})).unwrap();
        }
        let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| app.frame(ui), app);
        harness.run();
        let canvas = harness.get_by_label("Schematic canvas").rect();
        click(&mut harness, "Toggle Net Inspector");
        assert_eq!(harness.get_by_label("Schematic canvas").rect(), canvas);
        harness.get_by_label("No connected nets");
        for pin in ["R1.1", "R1.2", "R2.1", "R2.2"] {
            harness.get_by_label(pin);
        }
        assert_eq!(harness.state().session.revision(), 3);

        harness
            .state_mut()
            .session
            .execute(
                CONNECT_PINS,
                &json!({"from": {"symbol": "symbol-2", "terminal": "2"}, "to": {"symbol": "symbol-3", "terminal": "1"}, "expected_revision": 3}),
            )
            .unwrap();
        harness.run();
        harness.get_by_label("Net Net-0001");
        harness.get_by_label("R1.2");
        harness.get_by_label("R2.1");
        harness.get_by_label("R1.1");
        harness.get_by_label("R2.2");

        click(&mut harness, "Undo");
        harness.get_by_label("No connected nets");
        for pin in ["R1.1", "R1.2", "R2.1", "R2.2"] {
            harness.get_by_label(pin);
        }
        click(&mut harness, "Redo");
        harness.get_by_label("R1.2");
        harness.get_by_label("R2.1");

        let revision = harness.state().session.revision();
        click(&mut harness, "Search nets or pins");
        harness.get_by_label("Search nets or pins").type_text("R1.2");
        harness.run();
        harness.get_by_label("Net Net-0001");
        harness.get_by_label("R1.2");
        assert!(harness.query_by_label("R1.1").is_none());
        assert_eq!(harness.state().session.revision(), revision);
        click(&mut harness, "Collapse Net Inspector");
        assert!(harness.query_by_label("R1.2").is_none());
        click(&mut harness, "Expand Net Inspector");
        harness.get_by_label("R1.2");
        click(&mut harness, "Toggle Net Inspector");
        assert!(harness.query_by_label("R1.2").is_none());
        assert_eq!(harness.state().session.revision(), 6);
    }
}
