//! Revision-bound form for authored differential relationships, hosted by the owning editor.

use crate::{Dialog, PcbApp, board_drc::jobs, editor_view::ViewInput};
use pcbcraft_chrome::{catalog, controls, properties, rule_form as form, shell};
use pcbcraft_engine::{Project, nets};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};
use uuid::Uuid;

/// Immutable chooser data and disposable form fields; member names are never command inputs.
#[derive(Debug)]
pub(super) struct Edit {
    source: ViewInput,
    project: Arc<Project>,
    members: BTreeMap<Uuid, String>,
    active_members: BTreeSet<Uuid>,
    member_order: Vec<Uuid>,
    pairs: Vec<(Uuid, String)>,
    selected: Option<Uuid>,
    positive: Option<Uuid>,
    negative: Option<Uuid>,
    name: String,
    search: String,
    error: Option<String>,
    preparation: jobs::Queue<nets::PairEditJob>,
}

impl Edit {
    /// Keep the modal on the view that opened Rules, including detached editor windows.
    pub(super) fn source(&self) -> ViewInput {
        self.source
    }

    /// Capture current display names once. Retained members with no live name remain visible
    /// and can be preserved on edit, without guessing their successor or changing review intent.
    fn capture(source: ViewInput, project: Arc<Project>) -> Result<Self, String> {
        let registry = project.inspect_net_registry()?;
        let mut members: BTreeMap<_, _> =
            crate::board_net_choice::net_choices(&registry).into_iter().filter_map(|choice| choice.id.map(|id| (id, choice.label))).collect();
        let active_members: BTreeSet<_> = members.keys().copied().collect();
        for pair in project.differential_pairs() {
            for id in [pair.positive, pair.negative] {
                members.entry(id).or_insert_with(|| "Retained net".into());
            }
        }
        for (id, name) in &mut members {
            *name = choice_label(name, *id);
        }
        let mut member_order: Vec<_> = members.keys().copied().collect();
        member_order.sort_by(|a, b| members[a].cmp(&members[b]));
        let mut pairs: Vec<_> = project.differential_pairs().iter().map(|pair| (pair.id, choice_label(&pair.name, pair.id))).collect();
        pairs.sort_by(|a, b| a.1.cmp(&b.1));
        Ok(Self {
            source,
            project,
            members,
            active_members,
            member_order,
            pairs,
            selected: None,
            positive: None,
            negative: None,
            name: String::new(),
            search: String::new(),
            error: None,
            preparation: jobs::Queue::default(),
        })
    }

    /// Load the unchanged state of the selected record, or clear fields for a new relationship.
    fn select(&mut self, selected: Option<Uuid>) {
        self.selected = selected;
        let pair = self.current();
        let positive = pair.map(|pair| pair.positive);
        let negative = pair.map(|pair| pair.negative);
        self.name = pair.map_or_else(String::new, |pair| pair.name.clone());
        self.positive = positive;
        self.negative = negative;
        self.error = None;
    }

    /// Disable unfilled and same-net submissions; the engine owns all authoritative validation.
    fn complete(&self) -> bool {
        !self.name.trim().is_empty() && self.positive.is_some() && self.negative.is_some() && self.positive != self.negative
    }

    /// Do not switch records over an unsaved draft; Cancel discards it explicitly.
    fn changed(&self) -> bool {
        self.current().map_or_else(
            || !self.name.is_empty() || self.positive.is_some() || self.negative.is_some(),
            |pair| self.name != pair.name || self.positive != Some(pair.positive) || self.negative != Some(pair.negative),
        )
    }

    /// Get the stored record for the selected UUID, independently of draft values.
    fn current(&self) -> Option<&nets::Pair> {
        let id = self.selected?;
        let pairs = self.project.differential_pairs();
        let index = pairs.binary_search_by_key(&id, |pair| pair.id).ok()?;
        Some(pairs[index].as_ref())
    }
}

/// Include the full stable ID so distinct rows with equal display names remain addressable.
fn choice_label(name: &str, id: Uuid) -> String {
    format!("{name} · {id}")
}

/// Filter captured choices only while the popup is open and bound the number of rendered rows.
fn chooser<'a>(ui: &mut egui::Ui, title: &str, label: &str, search: &str, rows: impl Iterator<Item = (Uuid, &'a str)>, selected: &mut Option<Uuid>) {
    controls::Choice::from_id_salt(title)
        .width(ui.available_width().min(500.))
        .selected_text(label)
        .show_ui(ui, |ui| {
            let search = search.trim().to_lowercase();
            let mut matches = rows.filter(|(_, label)| search.is_empty() || label.to_lowercase().contains(&search));
            for (id, label) in matches.by_ref().take(50) {
                ui.selectable_value(selected, Some(id), label);
            }
            if matches.next().is_some() {
                ui.label("Use Filter choices to narrow this list.");
            }
        })
        .response
        .accessible_name(title);
}

impl PcbApp {
    /// Navigate from Rules to a disposable pair editor only while its captured source is current.
    pub(super) fn begin_pairs_from(&mut self, source: ViewInput) -> Result<(), String> {
        if !self.views.view(source.view()).is_some_and(|view| view.accepts(source, &self.session)) {
            return Err("The originating view changed.".into());
        }
        let project = self.session.snapshot().ok_or("No project open.")?;
        self.dialog = Dialog::Pairs(Box::new(Edit::capture(source, project)?));
        Ok(())
    }

    /// Retain invalid drafts with an error; stale forms cannot submit into a changed view or
    /// revision. One successful action closes the form and leaves history to the shared command.
    pub(super) fn pair_dialog(&mut self, ctx: &egui::Context, mut edit: Box<Edit>) {
        let Some(view) = self.views.view(edit.source.view()) else { return };
        let current =
            view.accepts(edit.source, &self.session) && self.session.project().is_some_and(|project| std::ptr::eq(project, edit.project.as_ref()));
        let preparing = edit.preparation.busy();
        let dirty = edit.changed();
        let mut selected = edit.selected;
        let mut remove = false;
        let mut mark_reviewed = false;
        let mut discard = false;
        let other_paired: BTreeSet<Uuid> = edit
            .project
            .differential_pairs()
            .iter()
            .filter(|pair| Some(pair.id) != edit.selected)
            .flat_map(|pair| [pair.positive, pair.negative])
            .collect();
        let current_pair = edit.current().cloned();
        let action = catalog::settings_form(ctx, "Differential pairs", "Save pair", current && !preparing && dirty && edit.complete(), |ui| {
            egui::ScrollArea::vertical().id_salt("differential_pair_form").auto_shrink([false, false]).show(ui, |ui| {
                pcbcraft_chrome::workspace::describe_scroll_viewport(ui);
                if !current {
                    ui.label(shell::error("The project or view changed. Close and reopen Differential pairs."));
                }
                if let Some(error) = &edit.error {
                    ui.label(shell::error(error));
                }
                ui.label("Pairs record intent only; they do not change copper or routing rules.");
                if preparing && current {
                    ui.label("Checking native document capacity…");
                }
                ui.add_enabled_ui(current && !preparing, |ui| {
                    controls::search(ui, ui.scope_id().with("pair_filter"), &mut edit.search, "Filter pairs or nets by name or ID", "Filter choices");
                    ui.add_enabled_ui(!dirty, |ui| {
                        let label = edit.current().map_or_else(|| "Choose pair".into(), |pair| choice_label(&pair.name, pair.id));
                        chooser(ui, "Existing pair", &label, &edit.search, edit.pairs.iter().map(|(id, label)| (*id, label.as_str())), &mut selected);
                        if edit.selected.is_some() && controls::button(ui, "New pair").clicked() {
                            selected = None;
                        }
                        if edit.selected.is_some() {
                            remove = controls::button(ui, "Remove pair")
                                .on_hover_text("Remove the relationship, not its nets. Undo restores it.")
                                .clicked();
                        }
                    });
                    if edit.members.len() < 2 {
                        ui.label("At least two persistent nets are needed to create a pair.");
                    }
                    if let Some(pair) = current_pair.as_ref()
                        && let nets::PairState::NeedsReview { reason } = &pair.state
                    {
                        form::conflict(ui, |ui| {
                            ui.label(format!("Needs review: {reason}"));
                            let both_active = edit.active_members.contains(&pair.positive) && edit.active_members.contains(&pair.negative);
                            mark_reviewed = ui
                                .add_enabled_ui(!dirty && both_active, |ui| controls::button(ui, "Mark reviewed"))
                                .inner
                                .on_hover_text("Confirm the current live endpoints and clear review status.")
                                .clicked();
                        });
                    }
                    properties::text(ui, "Pair name", &mut edit.name);
                    for (title, field, retained) in [
                        ("Positive net", &mut edit.positive, current_pair.as_ref().map(|pair| pair.positive)),
                        ("Negative net", &mut edit.negative, current_pair.as_ref().map(|pair| pair.negative)),
                    ] {
                        ui.label(title);
                        let label = field.and_then(|id| edit.members.get(&id)).map_or("Choose net", String::as_str);
                        let rows = edit
                            .member_order
                            .iter()
                            .filter(|id| (edit.active_members.contains(id) && !other_paired.contains(id)) || retained == Some(**id))
                            .map(|id| (*id, edit.members[id].as_str()));
                        chooser(ui, title, label, &edit.search, rows, field);
                    }
                    discard = ui.add_enabled_ui(dirty, |ui| controls::button(ui, "Discard pair changes")).inner.clicked();
                });
            });
        });
        if action.close {
            return;
        }
        if preparing {
            if !current {
                edit.preparation.cancel();
            }
            if let Some(done) = edit.preparation.poll()
                && current
                && done.source.is_current(&self.session)
            {
                let result = done.report.and_then(|prepared| {
                    let prepared = prepared.ok_or_else(|| "Pair preparation canceled".to_owned())?.map_err(|error| error.to_string())?;
                    self.session.complete_pair_edit(prepared, false).map_err(|error| error.to_string())
                });
                match result {
                    Ok(_) => {
                        self.file_notice = None;
                        self.message = None;
                        self.views.sync(&self.session);
                        self.repaint_editors(ctx);
                        return;
                    }
                    Err(error) => edit.error = Some(error),
                }
            }
        } else if current && (discard || selected != edit.selected) {
            edit.select(selected);
        } else if current && (remove || mark_reviewed || action.primary && dirty && edit.complete()) {
            match self.prepare_pair(&edit, remove, mark_reviewed) {
                Ok(job) => {
                    edit.error = None;
                    edit.preparation.submit(jobs::Request { id: 0, job });
                }
                Err(error) => edit.error = Some(error),
            }
        }
        if edit.preparation.busy() {
            ctx.request_repaint_after(Duration::from_millis(8));
        }
        self.dialog = Dialog::Pairs(edit);
    }

    /// Capture one shared revision-checked command, with a fresh ID only on creation.
    fn prepare_pair(&self, edit: &Edit, remove: bool, reviewed: bool) -> Result<nets::PairEditJob, String> {
        let pair = edit.selected.map_or_else(nets::new_pair_id, Ok).map_err(|error| error.to_string())?;
        let mut params = json!({"pair":pair, "expected_revision":edit.source.revision});
        let command = if remove {
            nets::REMOVE_PAIR
        } else {
            params["name"] = json!(edit.name);
            params["positive"] = json!(edit.positive);
            params["negative"] = json!(edit.negative);
            if reviewed {
                params["reviewed"] = json!(true);
            }
            if edit.selected.is_some() { nets::UPDATE_PAIR } else { nets::CREATE_PAIR }
        };
        self.session.prepare_pair_edit(command, &params).map_err(|error| error.to_string())
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
