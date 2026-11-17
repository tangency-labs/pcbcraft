//! Shared typed net choices for board intent forms; display names never grant authority.
use pcbcraft_engine::nets::ClassTarget;
use serde_json::Value;
/// A revision-qualified derived/persistent target and display label; names never grant authority.
#[derive(Clone, Debug)]
pub(super) struct NetChoice {
    pub(super) label: String,
    pub(super) target: ClassTarget,
    pub(super) id: Option<uuid::Uuid>,
}
/// Build picker presentation once from an engine inspection; unnamed nets remain distinguishable.
pub(super) fn net_choices(rows: &Value) -> Vec<NetChoice> {
    rows["nets"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, row)| {
            let anchor: pcbcraft_engine::nets::Anchor = serde_json::from_value(row["anchor"].clone()).ok()?;
            let label = row["name"].as_str().filter(|name| !name.is_empty()).map_or_else(|| format!("Net {}", index + 1), str::to_owned);
            let id = serde_json::from_value(row["id"].clone()).ok();
            let target = if let Some(net) = id {
                ClassTarget::Persistent { project: anchor.project, scope: anchor.scope, net }
            } else {
                ClassTarget::Derived { anchor }
            };
            Some(NetChoice { label, target, id })
        })
        .collect()
}

/// Show the current association as a shared action button, without enumerating net rows.
/// The owner opens a captured searchable chooser after rendering its form or ribbon field.
pub(super) fn trigger(ui: &mut egui::Ui, label: &str, current: &str) -> egui::Response {
    let response = pcbcraft_chrome::controls::bounded_button(ui, label, &format!("{current}…"), ui.available_width(), true);
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label);
        info.current_text_value = Some(current.to_owned());
        info
    });
    pcbcraft_chrome::automation::publish(ui, &response, format!("field:{label}"));
    response.on_hover_text(current)
}

use crate::{Dialog, PcbApp, editor_layout::HostId, editor_view::ViewInput};
use pcbcraft_chrome::{
    controls,
    library_workspace::{self, ItemAction, TableEntry},
};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Render at most this many matching nets, independently of the board registry size.
const PAGE_SIZE: usize = 50;
/// The assignment's owner stays explicit while the searchable form is open.
#[derive(Debug)]
pub(super) enum Destination {
    Placement,
    Vias { ids: Vec<Uuid>, selection: BTreeSet<Uuid> },
}
/// Search/selection are disposable; only confirmation submits the captured destination.
#[derive(Debug)]
pub(super) struct Edit {
    source: ViewInput,
    host: HostId,
    destination: Destination,
    choices: Vec<NetChoice>,
    query: String,
    filtered_query: String,
    matches: Vec<usize>,
    /// Zero is Automatic; later indices address the immutable captured choices.
    selected: Option<usize>,
    page: usize,
    focus: bool,
    reveal: bool,
}
impl Edit {
    /// Keep modal presentation in its initiating host, including detached editor windows.
    pub(super) fn host(&self) -> HostId {
        self.host
    }

    /// Query changes filter captured metadata once, reset pagination and discard hidden selection.
    fn filter(&mut self) {
        let query = self.query.trim().to_lowercase();
        if query == self.filtered_query {
            return;
        }
        self.matches = self.choices.iter().enumerate().filter(|(_, c)| c.label.to_lowercase().contains(&query)).map(|(i, _)| i).collect();
        self.filtered_query = query;
        self.page = 0;
        self.selected = None;
        self.reveal = true;
    }

    /// Construct only one page of shared data rows; Automatic remains available during searches.
    fn rows(&self) -> Vec<TableEntry> {
        let start = self.page * PAGE_SIZE;
        let mut rows = vec![TableEntry {
            key: "Choose automatic net".into(),
            cells: vec!["Automatic".into()],
            selected: self.selected == Some(0),
            branch: None,
            nested: false,
        }];
        rows.extend(self.matches.iter().skip(start).take(PAGE_SIZE).map(|i| {
            let choice = &self.choices[*i];
            TableEntry {
                key: format!("Choose net {}", choice.label),
                cells: vec![choice.label.clone()],
                selected: self.selected == Some(i + 1),
                branch: None,
                nested: false,
            }
        }));
        rows
    }

    /// Keyboard navigation follows the complete matching set and reveals its bounded page.
    fn step(&mut self, forward: bool) {
        let mut options = Vec::with_capacity(self.matches.len() + 1);
        if self.filtered_query.is_empty() {
            options.push(0);
        }
        options.extend(self.matches.iter().map(|i| i + 1));
        if options.is_empty() {
            return;
        }
        let current = self.selected.and_then(|s| options.iter().position(|i| *i == s));
        let index = match (current, forward) {
            (Some(i), true) => (i + 1).min(options.len() - 1),
            (Some(i), false) => i.saturating_sub(1),
            (None, true) => 0,
            (None, false) => options.len() - 1,
        };
        self.selected = Some(options[index]);
        self.page = self.selected.and_then(|s| s.checked_sub(1)).and_then(|i| self.matches.iter().position(|n| *n == i)).unwrap_or(0) / PAGE_SIZE;
        self.reveal = true;
    }
}
impl PcbApp {
    /// Modal input cancels previews, while the net chooser retains only its idle Via policy.
    /// Taking the cached choices avoids copying the registry on every modal frame.
    pub(super) fn cancel_dialog_gestures(&mut self) {
        let source = match &self.dialog {
            Dialog::ViaNet(edit) if matches!(edit.destination, Destination::Placement) && self.via_net_choice_current(edit) => Some(edit.source),
            _ => None,
        };
        let retained = source.and_then(|source| {
            let tracks = &mut self.views.view_mut(source.view())?.board.tracks;
            Some((source.view(), tracks.via_target.take(), tracks.via_choices.take()))
        });
        self.views.cancel_gestures();
        if let Some((id, target, choices)) = retained
            && let Some(view) = self.views.view_mut(id)
        {
            view.board.tracks.via_tool = true;
            view.board.tracks.via_target = target;
            view.board.tracks.via_choices = choices;
        }
    }

    /// Capture the current board, exact destination and typed choices without changing project history.
    pub(super) fn begin_via_net_choice(&mut self, source: ViewInput, destination: Destination) {
        if !matches!(self.dialog, Dialog::None)
            || !self.views.view(source.view()).is_some_and(|v| v.accepts(source, &self.session) && !v.board.tracks.committing())
        {
            return;
        }
        let Some(snapshot) = self.session.snapshot() else { return };
        let Some(view) = self.views.view_mut(source.view()) else { return };
        let Some(board) = view.board.target.and_then(|t| snapshot.board_view(t)) else { return };
        view.board.tracks.refresh_net_choices(board, source.revision);
        let choices = view.board.tracks.via_choices.as_ref().map(|(_, _, c)| c.clone()).unwrap_or_default();
        let current = match &destination {
            Destination::Placement => Some(view.board.tracks.via_target.clone()),
            Destination::Vias { ids, .. } => {
                let vias: Vec<_> = board.vias().iter().filter(|v| ids.contains(&v.id)).collect();
                let Some(first) = vias.first() else { return };
                if vias.len() != ids.len() || vias.iter().any(|v| v.locked) {
                    return;
                }
                if vias.iter().any(|v| v.net_assignment != first.net_assignment) {
                    None
                } else {
                    Some(first.net_assignment.map(|a| ClassTarget::Persistent { project: a.project, scope: a.scope, net: a.net }))
                }
            }
        };
        let selected = match current {
            Some(None) => Some(0),
            Some(Some(target)) => choices.iter().position(|c| c.target == target).map(|i| i + 1),
            None => None,
        };
        let matches = (0..choices.len()).collect::<Vec<_>>();
        let page = selected.and_then(|s| s.checked_sub(1)).unwrap_or(0) / PAGE_SIZE;
        self.dialog = Dialog::ViaNet(Box::new(Edit {
            source,
            host: self.views.host,
            destination,
            choices,
            query: String::new(),
            filtered_query: String::new(),
            matches,
            selected,
            page,
            focus: true,
            reveal: true,
        }));
    }

    /// Refuse revision, opening, focus, selection or tool replacement before applying a chooser result.
    fn via_net_choice_current(&self, edit: &Edit) -> bool {
        self.views.view(edit.source.view()).is_some_and(|v| {
            v.accepts(edit.source, &self.session)
                && match &edit.destination {
                    Destination::Placement => v.board.tracks.via_tool,
                    Destination::Vias { selection, .. } => &v.board.tracks.selected == selection,
                }
        })
    }

    /// Compose the existing search, table and fixed modal footer; browsing and cancellation are read-only.
    pub(super) fn via_net_dialog(&mut self, ctx: &egui::Context, mut edit: Edit) {
        if !self.via_net_choice_current(&edit) {
            self.message = Some("View or project changed; reopen the net picker.".into());
            return;
        }
        let mut enter = false;
        let action = pcbcraft_chrome::catalog::property_form_enabled(
            ctx,
            "via_net_picker",
            "Choose via net",
            620.,
            "Use net",
            edit.selected.is_some(),
            |ui| {
                let search = controls::search(ui, ui.scope_id().with("net_search"), &mut edit.query, "Search nets", "Search nets");
                if edit.focus {
                    search.request_focus();
                    edit.focus = false;
                }
                edit.filter();
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)) {
                    edit.step(true);
                }
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
                    edit.step(false);
                }
                enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                ui.horizontal_wrapped(|ui| {
                    let start = edit.page * PAGE_SIZE;
                    let end = (start + PAGE_SIZE).min(edit.matches.len());
                    ui.label(if edit.matches.is_empty() {
                        "0 nets".into()
                    } else {
                        format!("{}–{} of {} nets", start + 1, end, edit.matches.len())
                    });
                    if edit.matches.len() > PAGE_SIZE {
                        if controls::enabled_button(ui, edit.page > 0, "Previous page").clicked() {
                            edit.page = edit.page.saturating_sub(1);
                            edit.selected = None;
                            edit.reveal = true;
                        }
                        if controls::enabled_button(ui, end < edit.matches.len(), "Next page").clicked() {
                            edit.page += 1;
                            edit.selected = None;
                            edit.reveal = true;
                        }
                    }
                });
                let rows = edit.rows();
                let t = pcbcraft_chrome::Tokens::get();
                let reserved = t.control_height * 5. + t.space_16 * 6. + t.form_footer_height;
                let height = (ctx.content_rect().height() - reserved).clamp(t.control_height * 4., t.control_height * 12.);
                let mut picked = None;
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.push_id(edit.page, |ui| {
                        library_workspace::table_with_reveal(ui, &["Net"], &rows, std::mem::take(&mut edit.reveal), |action| {
                            if let ItemAction::Select(i) = action {
                                picked = Some(i);
                            }
                        })
                    });
                });
                if let Some(row) = picked {
                    edit.selected = if row == 0 { Some(0) } else { edit.matches.get(edit.page * PAGE_SIZE + row - 1).map(|i| i + 1) };
                }
            },
        );
        if action.close {
            return;
        }
        if (action.primary || enter)
            && self.via_net_choice_current(&edit)
            && let Some(selected) = edit.selected
        {
            let choice = selected.checked_sub(1).and_then(|i| edit.choices.get(i)).map(|c| c.target.clone());
            match &edit.destination {
                Destination::Placement => {
                    if let Some(v) = self.views.view_mut(edit.source.view()) {
                        v.board.tracks.via_target = choice;
                    }
                }
                Destination::Vias { ids, .. } => {
                    let mut params = serde_json::json!({"vias":ids,"expected_revision":edit.source.revision});
                    if let Some(target) = choice {
                        params["target"] = serde_json::json!(target);
                    } else {
                        params["automatic"] = serde_json::json!(true);
                    }
                    if let Some(board) = self.views.view(edit.source.view()).and_then(|v| v.board.target) {
                        params["board"] = serde_json::json!(board.board);
                    }
                    self.prepare_canvas_copper(crate::board_drawing::Command::new(edit.source, pcbcraft_engine::SET_VIA_NET, params));
                }
            }
            self.repaint_editors(ctx);
            return;
        }
        self.dialog = Dialog::ViaNet(Box::new(edit));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        editor_view::Workspace,
        tests::{click, harness},
    };
    use egui_kittest::{Harness, kittest::Queryable};
    use pcbcraft_engine::persistence::encode_documents;
    use serde_json::json;

    /// Open the picker from a real command-generated board and one actual named net.
    fn fixture() -> Harness<'static, PcbApp> {
        let mut h = harness();
        h.set_size(egui::vec2(1100., 800.));
        h.state_mut().run("project.new", &json!({"create_board":true}));
        for (command, mut params) in [
            (pcbcraft_engine::INITIALIZE_BOARD_SETUP, json!({"copper_layers":2})),
            (pcbcraft_engine::net_labels::ADD, json!({"name":"GND","style":"global","x_nm":0,"y_nm":0})),
        ] {
            params["expected_revision"] = json!(h.state().session.revision());
            h.state_mut().run(command, &params);
            assert_eq!(h.state().message, None);
        }
        let app = h.state_mut();
        app.views.active.switch(Workspace::Board, &app.session);
        h.run();
        h.state_mut().start_via();
        h.run();
        click(&mut h, "Via placement net");
        h
    }

    /// Thousands of captured metadata rows stay page-bounded; typing changes results without model edits.
    #[test]
    fn thousands_of_net_choices_are_searchable_bounded_and_cancelable() {
        let mut h = fixture();
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        let Dialog::ViaNet(edit) = &mut h.state_mut().dialog else { panic!() };
        // This is a presentation fixture: all names retain the actual source target and are never committed.
        let original = edit.choices[0].clone();
        edit.choices = (0..4096).map(|i| NetChoice { label: format!("Signal {i:04}"), ..original.clone() }).collect();
        edit.matches = (0..edit.choices.len()).collect();
        edit.selected = Some(PAGE_SIZE);
        assert_eq!(edit.rows().len(), PAGE_SIZE + 1);
        h.run();
        h.key_press(egui::Key::ArrowDown);
        h.run();
        let Dialog::ViaNet(edit) = &h.state().dialog else { panic!() };
        assert_eq!(edit.page, 1);
        assert_eq!(edit.selected, Some(PAGE_SIZE + 1));
        assert!(h.query_by_label("Choose net Signal 0050").is_some());
        assert_eq!(edit.rows().len(), PAGE_SIZE + 1);
        click(&mut h, "Next page");
        click(&mut h, "Search nets");
        h.get_by_label("Search nets").type_text("sIgNaL 4095");
        h.run();
        let Dialog::ViaNet(edit) = &h.state().dialog else { panic!() };
        assert_eq!(edit.page, 0);
        assert_eq!(edit.matches, vec![4095]);
        assert_eq!(edit.rows().len(), 2);
        assert!(h.query_by_label("Choose net Signal 4095").is_some());
        assert!(h.query_by_label("Choose net Signal 0000").is_none());
        h.key_press(egui::Key::ArrowDown);
        h.run();
        let Dialog::ViaNet(edit) = &h.state().dialog else { panic!() };
        assert_eq!(edit.selected, Some(4096));
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::None));
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    }

    /// Revision changes invalidate an open chooser before any selection can author intent.
    #[test]
    fn stale_search_form_cannot_set_the_placement_net() {
        let mut h = fixture();
        h.get_by_label("Search nets").type_text("GND");
        h.run();
        click(&mut h, "Choose net GND");
        h.state_mut().session.apply("project.rename", &json!({"name":"Peer revision"})).unwrap();
        let before = h.state().session.snapshot().unwrap();
        h.run();
        assert!(matches!(h.state().dialog, Dialog::None));
        assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.snapshot().unwrap()));
        assert!(h.state().views.active.board.tracks.via_target.is_none());
    }

    /// A search with no matches cannot reuse a previously selected hidden target on Enter.
    #[test]
    fn empty_results_discard_the_previous_choice() {
        let mut h = fixture();
        click(&mut h, "Choose net GND");
        click(&mut h, "Search nets");
        h.get_by_label("Search nets").type_text("No such net");
        h.run();
        let Dialog::ViaNet(edit) = &h.state().dialog else { panic!() };
        assert!(edit.matches.is_empty());
        assert_eq!(edit.selected, None);
        h.key_press(egui::Key::Enter);
        h.run();
        assert!(matches!(h.state().dialog, Dialog::ViaNet(_)));
        click(&mut h, "Choose automatic net");
        click(&mut h, "Use net");
        assert!(h.state().views.active.board.tracks.via_target.is_none());
    }

    /// Cancel preserves a prior placement policy; ordinary focus invalidation still disarms it.
    #[test]
    fn picker_cancel_retains_intent_but_focus_loss_does_not() {
        let mut h = fixture();
        click(&mut h, "Choose net GND");
        click(&mut h, "Use net");
        let target = h.state().views.active.board.tracks.via_target.clone();
        assert!(target.is_some());
        click(&mut h, "Via placement net");
        click(&mut h, "Choose automatic net");
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(h.state().views.active.board.tracks.via_tool);
        assert_eq!(h.state().views.active.board.tracks.via_target, target);
        click(&mut h, "Via placement net");
        h.state_mut().views.active.release_input();
        h.run();
        assert!(matches!(h.state().dialog, Dialog::None));
        assert!(!h.state().views.active.board.tracks.via_tool);
        assert!(h.state().views.active.board.tracks.via_target.is_none());
    }
}
