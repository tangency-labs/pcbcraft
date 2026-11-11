//! Inspect the engine's resolved nets using the same card and tree primitives as Properties/Browser.
use egui::{Rect, Ui};
use pcbcraft_chrome::{properties, shell, workspace};
use pcbcraft_engine::{Project, schematic::Terminal};
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
}
impl Default for NetInspector {
    /// Match the Properties card's initial state without reserving a sidebar in the workspace.
    fn default() -> Self {
        Self { open: false, rect: Rect::NOTHING, collapsed: false, query: String::new(), folded: BTreeSet::new(), project: None, snapshot: None }
    }
}
#[derive(Debug)]
struct Net {
    name: String,
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
            pins.iter().filter_map(|&pin| Some(format!("{}.{}", project.reference(project.terminal_symbol(pin)?), pin.number()))).collect::<Vec<_>>()
        };
        let graph = project.connectivity();
        Self {
            nets: graph.nets.iter().enumerate().map(|(i, net)| Net { name: graph.net_name(i), pins: pins(net) }).collect(),
            unconnected: pins(&graph.unconnected),
        }
    }
}
/// Tree rows use the Browser's actual metrics, disclosure control and typography.
fn row(ui: &mut Ui, key: impl std::hash::Hash + std::fmt::Debug, label: &str, depth: usize, open: Option<bool>, tip: &str) -> bool {
    let response = workspace::tree_row(
        ui,
        ui.scope_id().with(key),
        &workspace::TreeRow { depth, fold: open, icon: None, label, accessibility_label: None, tip, selected: false },
    );
    response.fold || response.clicked
}
impl NetInspector {
    /// Float a stable Craft card inside its pane, sharing the Properties inspector
    /// footprint without resizing the canvas or changing document history.
    pub(super) fn show(&mut self, ui: &Ui, project: Option<Arc<Project>>, area: Rect) {
        if !self.open {
            self.rect = Rect::NOTHING;
            return;
        }
        if let Some(project) = project {
            if !self.project.as_ref().is_some_and(|p| p.same_connectivity(&project)) {
                self.snapshot = Some(Snapshot::read(&project));
                self.project = Some(project);
                self.folded.clear();
            }
        } else {
            self.project = None;
            self.snapshot = None;
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
                    let tip = format!("{} pins", net.pins.len());
                    if row(ui, ("net", index), &net.name, 0, (!net.pins.is_empty()).then_some(expanded), &tip) && !self.folded.remove(&index) {
                        self.folded.insert(index);
                    }
                    if expanded {
                        for pin in &net.pins {
                            row(ui, ("net_pin", index, pin), pin, 1, None, "");
                        }
                    }
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
    use egui_kittest::{Harness, kittest::Queryable};
    use pcbcraft_engine::{CONNECT_PINS, PLACE_RESISTOR};
    use serde_json::json;

    use crate::{PcbApp, tests::click};

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
        harness.get_by_label("Net 1");
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
        harness.get_by_label("Net 1");
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
