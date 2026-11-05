//! Present the same committed connectivity that the engine exposes to agents.

use std::sync::Arc;

use egui::Ui;
use pcbcraft_chrome::{shell::dim, workspace};
use pcbcraft_engine::{Project, Session, schematic::Terminal};

#[derive(Debug, Default)]
pub(super) struct NetInspector {
    pub open: bool,
    project: Option<Arc<Project>>,
    snapshot: Option<Snapshot>,
}

#[derive(Debug)]
struct Snapshot {
    nets: Vec<String>,
    unconnected: String,
}

impl Snapshot {
    /// Format the typed groups for display using the project's stable terminal IDs.
    fn read(project: &Project) -> Self {
        let pin_labels = |terminals: &[Terminal]| {
            terminals
                .iter()
                .filter_map(|&pin| {
                    let reference = project.terminal_symbol(pin)?.reference();
                    Some(format!("{reference}.{}", pin.number()))
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        let connectivity = project.connectivity();
        Self { nets: connectivity.nets.iter().map(|net| pin_labels(net)).collect(), unconnected: pin_labels(&connectivity.unconnected) }
    }

    /// List connected groups and isolated pins independently. Net numbers are local to the view.
    fn show(&self, ui: &mut Ui) {
        ui.strong("Connected nets");
        if self.nets.is_empty() {
            ui.label(dim("No connected nets"));
        }
        for (index, pins) in self.nets.iter().enumerate() {
            ui.label(format!("Net {}", index + 1));
            ui.label(dim(pins.as_str()));
        }
        ui.separator();
        ui.strong("Unconnected pins");
        if self.unconnected.is_empty() {
            ui.label(dim("No unconnected pins"));
        } else {
            ui.label(dim(self.unconnected.as_str()));
        }
    }
}

impl NetInspector {
    /// Dock a read-only view next to the schematic. Refresh on revisions, including undo and redo.
    pub(super) fn show(&mut self, ui: &mut Ui, session: &Session) {
        if !self.open {
            return;
        }
        if let Some(project) = session.project()
            && !self.project.as_ref().is_some_and(|previous| previous.same_connectivity(project))
        {
            self.snapshot = Some(Snapshot::read(project));
            self.project = session.snapshot();
        }

        let close = workspace::right_card(ui, "net_inspector", "Net Inspector", |ui| {
            if let Some(snapshot) = &self.snapshot {
                snapshot.show(ui);
            } else {
                ui.label(dim("No project open"));
            }
        });
        self.open = !close;
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::{Harness, kittest::Queryable};
    use pcbcraft_engine::{CONNECT_PINS, PLACE_RESISTOR};
    use serde_json::json;

    use crate::PcbApp;

    /// Open the real View menu. Choosing an item exercises the shared desktop and browser frame.
    fn choose(harness: &mut Harness<'_, PcbApp>, menu: &str, item: &str) {
        harness.get_by_label(menu).click();
        harness.run();
        harness.get_by_label(item).click();
        harness.run();
    }

    #[test]
    fn net_inspector_tracks_wiring_and_history_without_editing_the_project() {
        let mut app = PcbApp::default();
        app.session.execute("project.new", &json!({"name": "wiring"})).unwrap();
        for (x, revision) in [(0, 1), (20_000_000, 2)] {
            app.session.execute(PLACE_RESISTOR, &json!({"x_nm": x, "y_nm": 0, "expected_revision": revision})).unwrap();
        }
        let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| app.frame(ui), app);
        harness.run();
        choose(&mut harness, "View", "Show Net Inspector");
        harness.get_by_label("No connected nets");
        harness.get_by_label("R1.1, R1.2, R2.1, R2.2");
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
        harness.get_by_label("R1.2, R2.1");
        harness.get_by_label("R1.1, R2.2");

        choose(&mut harness, "Edit", "Undo");
        harness.get_by_label("No connected nets");
        harness.get_by_label("R1.1, R1.2, R2.1, R2.2");
        choose(&mut harness, "Edit", "Redo");
        harness.get_by_label("R1.2, R2.1");

        choose(&mut harness, "View", "Hide Net Inspector");
        assert!(harness.query_by_label("R1.2, R2.1").is_none());
        assert_eq!(harness.state().session.revision(), 6);
    }
}
