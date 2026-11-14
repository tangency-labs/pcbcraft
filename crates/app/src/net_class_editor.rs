//! Shared, revision-bound class authoring and net inspection for schematic and PCB views.
use crate::{Dialog, PcbApp, ViewInput, editor_view::DocumentTarget};
use pcbcraft_chrome::{board_setup as form, controls, properties, shell, units};
use pcbcraft_engine::{
    Project,
    net_classes::{self, Definition, Property, Resolution, Rules, Source},
    nets::{Anchor, ClassBinding, ClassTarget},
};
use serde_json::json;
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Classes,
    Net,
}

/// Text keeps its original integer even when the displayed unit rounds it.
#[derive(Debug)]
struct Field {
    property: Property,
    enabled: bool,
    original: i64,
    initial: String,
    text: String,
}

/// A disposable sparse definition never manufactures an identity before the create command.
#[derive(Debug)]
struct ClassDraft {
    id: Option<Uuid>,
    name: String,
    original_name: String,
    original_rules: Rules,
    fields: Vec<Field>,
}

/// One current chooser row has an exact typed target, independent of its editable display name.
#[derive(Debug)]
struct NetChoice {
    label: String,
    target: ClassTarget,
}

/// A modal owns one view/revision and immutable inspection snapshot. No idle-frame graph work.
#[derive(Debug)]
pub(super) struct Edit {
    source: ViewInput,
    project: Arc<Project>,
    unit: units::Unit,
    page: Page,
    selected_class: Option<Uuid>,
    draft: Option<ClassDraft>,
    nets: Vec<NetChoice>,
    net_index: Option<usize>,
    net_search: String,
    target: Option<ClassTarget>,
    classes: BTreeSet<Uuid>,
    original_classes: BTreeSet<Uuid>,
    bindings: Vec<ClassBinding>,
    selection_error: Option<String>,
    error: Option<String>,
}

/// Human dimensional labels are shared by the authored and effective presentations.
fn label(property: Property) -> &'static str {
    use Property::*;
    match property {
        ClearanceNm => "Copper clearance",
        MinTrackWidthNm => "Minimum track width",
        EdgeClearanceNm => "Board edge clearance",
        MinAnnularRingNm => "Minimum annular ring",
        MinViaDiameterNm => "Minimum via diameter",
        MaxViaDiameterNm => "Maximum via diameter",
        MinViaDrillNm => "Minimum via drill",
        MaxViaDrillNm => "Maximum via drill",
        PreferredTrackWidthNm => "Preferred track width",
        PreferredViaDiameterNm => "Preferred via diameter",
        PreferredViaDrillNm => "Preferred via drill",
    }
}

impl ClassDraft {
    /// Omitted opinions display the addressed baseline, without saving it into the class.
    fn new(class: Option<&Definition>, baseline: pcbcraft_engine::board::rules::Values, unit: units::Unit) -> Self {
        let rules = class.map(|class| class.rules.clone()).unwrap_or_default();
        let name = class.map(|class| class.name.clone()).unwrap_or_default();
        let fields = Property::ALL
            .into_iter()
            .map(|property| {
                let original = rules.get(&property).copied().unwrap_or_else(|| property.value(&baseline));
                let initial = units::edit_label(original, unit);
                Field { property, enabled: rules.contains_key(&property), original, initial: initial.clone(), text: initial }
            })
            .collect();
        Self { id: class.map(|class| class.id), name: name.clone(), original_name: name, original_rules: rules, fields }
    }
    /// Parse only changed text and ask the shared engine validator to check sparse relationships.
    fn rules(&self, unit: units::Unit) -> Result<Rules, String> {
        let rules = self
            .fields
            .iter()
            .filter(|field| field.enabled)
            .map(|field| {
                let value = if field.initial == field.text {
                    Ok(field.original)
                } else {
                    units::parse_length(&field.text, unit).map_err(|error| format!("{}: {error}", label(field.property)))
                }?;
                Ok((field.property, value))
            })
            .collect::<Result<Rules, String>>()?;
        net_classes::validate_draft(&self.name, &rules)?;
        Ok(rules)
    }
    /// Changed drafts cannot be replaced by a different page or definition without Apply/Cancel.
    fn changed(&self, unit: units::Unit) -> bool {
        self.id.is_none() || self.name != self.original_name || self.rules(unit).as_ref() != Ok(&self.original_rules)
    }
}

/// Resolve actual selected object identities once when Rules opens. Mixed nets require a choice.
fn selected_target(app: &PcbApp, source: ViewInput) -> Result<Option<ClassTarget>, String> {
    let project = app.session.project().ok_or("No project open.")?;
    let view = app.views.view(source.view()).ok_or("The originating view closed.")?;
    let mut targets = Vec::new();
    let mut unsupported = false;
    match view.target() {
        Some(DocumentTarget::Sheet(sheet)) => {
            let own = project.for_sheet(sheet).ok_or("Sheet unavailable.")?;
            for wire in own.wires().iter().filter(|wire| view.schematic.selection.wires.contains(&wire.id())) {
                if let Some(target) = project.net_class_target_for_endpoint(sheet, wire.endpoints()[0]).map_err(|error| error.to_string())? {
                    targets.push(target);
                } else {
                    unsupported = true;
                }
            }
            for item in own.net_labels().iter().filter(|item| view.schematic.selection.net_labels.contains(&item.object_id())) {
                if let Some(target) = project
                    .net_class_target_for_endpoint(sheet, pcbcraft_engine::schematic::WireEndpoint::NetLabel(item.id))
                    .map_err(|error| error.to_string())?
                {
                    targets.push(target);
                } else {
                    unsupported = true;
                }
            }
        }
        Some(DocumentTarget::Board(board)) => {
            let board = project.board_view(board).ok_or("Board unavailable.")?;
            for id in &view.board.tracks.selected {
                if let Some(target) = board.net_class_target_for_copper(&id.to_string()).map_err(|error| error.to_string())? {
                    targets.push(target);
                } else {
                    unsupported = true;
                }
            }
        }
        _ => {}
    }
    if unsupported && !targets.is_empty() {
        return Err("Selection includes geometry without a supported current net.".into());
    }
    if targets.iter().any(|target| Some(target) != targets.first()) {
        return Err("Selection spans more than one supported net.".into());
    }
    Ok(targets.into_iter().next())
}

impl Edit {
    /// Capture fresh scoped chooser targets; opening never promotes a net or changes history.
    fn capture(app: &PcbApp, source: ViewInput, page: Page) -> Result<Self, String> {
        let project = app.session.snapshot().ok_or("No project open.")?;
        let registry = project.inspect_net_registry()?;
        let rows = registry["nets"].as_array().ok_or("Net inspection is unavailable.")?;
        let mut nets = Vec::new();
        for row in rows {
            let anchor: Anchor = serde_json::from_value(row["anchor"].clone()).map_err(|error| error.to_string())?;
            let names: Vec<String> = serde_json::from_value(row["names"].clone()).map_err(|error| error.to_string())?;
            let name = if names.is_empty() { format!("Unnamed · {} {}", anchor.object, anchor.terminal) } else { names.join(" / ") };
            let detail = if anchor.terminal.is_empty() { anchor.occurrence.to_string() } else { format!("{}:{}", anchor.object, anchor.terminal) };
            nets.push(NetChoice { label: format!("{name} · {detail}"), target: ClassTarget::Derived { anchor } });
        }
        let selected = selected_target(app, source);
        let net_index =
            selected.as_ref().ok().and_then(|selected| selected.as_ref()).and_then(|selected| nets.iter().position(|net| &net.target == selected));
        let mut edit = Self {
            source,
            project,
            unit: app.preferences.units,
            page,
            selected_class: None,
            draft: None,
            nets,
            net_index: None,
            net_search: String::new(),
            target: None,
            classes: BTreeSet::new(),
            original_classes: BTreeSet::new(),
            bindings: Vec::new(),
            selection_error: selected.err(),
            error: None,
        };
        if let Some(index) = net_index {
            edit.load_net(index)?;
        }
        Ok(edit)
    }
    /// The board baseline is explicit; schematic-only projects inherit released built-ins.
    fn baseline(&self) -> pcbcraft_engine::board::rules::Values {
        self.project
            .board_target()
            .and_then(|target| self.project.board_view(target))
            .map_or_else(Default::default, |board| board.effective_rules().values)
    }
    /// Load bindings through the same inspection contract MCP sees, normalizing saved targets.
    fn load_net(&mut self, index: usize) -> Result<(), String> {
        let net = self.nets.get(index).ok_or("Net no longer available.")?;
        let inspected = self.project.inspect_net_classes(&net.target).map_err(|error| error.to_string())?;
        let target = serde_json::from_value(inspected["target"].clone()).map_err(|error| error.to_string())?;
        let bindings: Vec<ClassBinding> = serde_json::from_value(inspected["bindings"].clone()).map_err(|error| error.to_string())?;
        self.classes = bindings.iter().map(|binding| binding.class).collect();
        self.original_classes = self.classes.clone();
        self.target = Some(target);
        self.bindings = bindings;
        self.net_index = Some(index);
        self.selection_error = None;
        self.error = None;
        Ok(())
    }
    /// Use exact class UUIDs for selection and retain all sparse source integers in the draft.
    fn load_class(&mut self, id: Option<Uuid>) {
        self.draft = Some(ClassDraft::new(id.and_then(|id| self.project.net_classes().get(id)), self.baseline(), self.unit));
        self.selected_class = id;
        self.error = None;
    }
    /// Only the active page owns pending edits; navigation cannot discard them implicitly.
    fn changed(&self) -> bool {
        match self.page {
            Page::Classes => self.draft.as_ref().is_some_and(|draft| draft.changed(self.unit)),
            Page::Net => self.classes != self.original_classes,
        }
    }
}

/// Render sparse opinions with the existing Craft checkbox/field primitives. A disabled opinion
/// shows the actual inherited baseline rather than a stale, previously authored string.
fn class_fields(ui: &mut egui::Ui, draft: &mut ClassDraft, baseline: pcbcraft_engine::board::rules::Values, unit: units::Unit, board: bool) {
    if ui.available_width() < 450. {
        ui.label("Class name");
        controls::text(ui, &mut draft.name).accessible_name("Class name");
    } else {
        form::text(ui, "Class name", &mut draft.name);
    }
    for preferred in [false, true] {
        form::heading(ui, if preferred { "Routing preferences" } else { "Hard limits" });
        let narrow = ui.available_width() < 450.;
        let label_width = if narrow { ui.available_width() } else { 250. };
        egui::Grid::new(("class_fields", preferred)).num_columns(if narrow { 1 } else { 3 }).spacing(egui::vec2(12., 6.)).show(ui, |ui| {
            for field in draft.fields.iter_mut().filter(|field| field.property.is_preference() == preferred) {
                let name = label(field.property);
                ui.allocate_ui_with_layout(egui::vec2(label_width, 28.), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    controls::checkbox(ui, &mut field.enabled, name);
                });
                if narrow {
                    ui.end_row();
                }
                if field.enabled {
                    ui.add_sized(egui::vec2(140., 28.), properties::input(&mut field.text)).accessible_name(format!("{name} value"));
                } else {
                    ui.label(units::label(field.property.value(&baseline), unit));
                }
                if narrow {
                    ui.end_row();
                }
                ui.label(shell::dim(if field.enabled {
                    "Class"
                } else if board {
                    "Board"
                } else {
                    "Built-in"
                }));
                ui.end_row();
            }
        });
    }
}

/// Effective values and every contribution stay distinct from editable class opinions.
fn resolution(ui: &mut egui::Ui, project: &Project, result: &Resolution, unit: units::Unit) {
    for preferred in [false, true] {
        form::heading(ui, if preferred { "Routing preferences" } else { "Hard limits" });
        egui::Grid::new(("effective_class_rules", preferred)).num_columns(2).show(ui, |ui| {
            for property in Property::ALL.into_iter().filter(|property| property.is_preference() == preferred) {
                ui.label(label(property));
                ui.label(
                    result.values.get(&property).copied().flatten().map(|value| units::label(value, unit)).unwrap_or_else(|| "Unresolved".into()),
                );
                ui.end_row();
            }
        });
    }
    for conflict in &result.conflicts {
        ui.label(shell::error(&conflict.message));
    }
    properties::section(ui, "Rule sources");
    egui::CollapsingHeader::new("Contributions and identities").show(ui, |ui| {
        for property in Property::ALL {
            ui.label(label(property));
            for contribution in result.contributions.get(&property).into_iter().flatten() {
                let source = match contribution.source {
                    Source::Builtin => "Built-in".to_owned(),
                    Source::Request => "Request".to_owned(),
                    Source::Board { rule } => format!("Board · {rule}"),
                    Source::Class { rule, revision } => {
                        format!("{} · {rule} · r{revision}", project.net_classes().get(rule).map_or("Class", |class| class.name.as_str()))
                    }
                };
                ui.label(format!("{source}: {}", units::label(contribution.value_nm, unit)));
            }
        }
    });
}

impl PcbApp {
    /// Open the same Rules workflow from either editor, without requiring a physical board.
    pub(super) fn begin_rules(&mut self) {
        self.open_rules(false);
    }
    /// Selected-net Properties enters the same modal with its exact current net preselected.
    pub(super) fn begin_net_rules(&mut self) {
        self.open_rules(true);
    }
    /// Modal/file ownership and view authority follow the existing board-default form boundary.
    fn open_rules(&mut self, net: bool) {
        if !matches!(self.dialog, Dialog::None) || self.files.busy() || self.project_action_pending() {
            return;
        }
        let Some(source) = self.views.active.input(&self.session) else { return };
        match Edit::capture(self, source, if net { Page::Net } else { Page::Classes }) {
            Ok(edit) => self.dialog = Dialog::Rules(Box::new(edit)),
            Err(error) => self.message = Some(error),
        }
    }
    /// One Apply emits one shared native command. Stale drafts remain inspectable but cannot
    /// write; cancellation changes no document, and successful edits refresh the originating view.
    pub(super) fn rules_dialog(&mut self, ctx: &egui::Context, mut edit: Box<Edit>) {
        if self.views.view(edit.source.view()).is_none() {
            return;
        }
        let current = self.views.view(edit.source.view()).is_some_and(|view| view.accepts(edit.source, &self.session));
        let page = edit.page;
        let dirty = edit.changed();
        let rules = edit.draft.as_ref().map(|draft| draft.rules(edit.unit));
        let valid = match edit.page {
            Page::Classes => rules.as_ref().is_some_and(Result::is_ok) && dirty,
            Page::Net => edit.target.is_some() && dirty,
        };
        let mut board_defaults = false;
        let mut pick_class = edit.selected_class;
        let mut new_class = false;
        let mut pick_net = edit.net_index;
        let primary = if edit.page == Page::Classes { "Save class" } else { "Assign classes" };
        let action = pcbcraft_chrome::catalog::settings_form(ctx, "Rules", primary, current && valid, |ui| {
            ui.add_enabled_ui(!dirty, |ui| properties::tabs(ui, &mut edit.page, &[(Page::Classes, "Classes"), (Page::Net, "Selected net")]));
            if !current {
                ui.label(shell::error("The project or view changed. Close and reopen Rules to continue."));
            }
            egui::ScrollArea::vertical().id_salt("class_rule_body").auto_shrink([false, false]).show(ui, |ui| {
                match edit.page {
                    Page::Classes => {
                        ui.add_enabled_ui(!dirty, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                controls::Choice::from_id_salt("class_choice")
                                    .selected_text(
                                        edit.selected_class
                                            .and_then(|id| edit.project.net_classes().get(id))
                                            .map_or("Choose class", |class| class.name.as_str()),
                                    )
                                    .show_ui(ui, |ui| {
                                        for class in &edit.project.net_classes().definitions {
                                            ui.selectable_value(
                                                &mut pick_class,
                                                Some(class.id),
                                                format!("{} · r{} · {}", class.name, class.revision, &class.id.to_string()[..8]),
                                            );
                                        }
                                    })
                                    .response
                                    .accessible_name("Class");
                                new_class = controls::button(ui, "New class").clicked();
                                board_defaults = ui
                                    .add_enabled(edit.project.board_target().is_some(), pcbcraft_chrome::catalog::button("Board defaults…", false))
                                    .clicked();
                            })
                        });
                        let base = edit.baseline();
                        let board = edit.project.board_target().is_some();
                        if let Some(draft) = &mut edit.draft {
                            class_fields(ui, draft, base, edit.unit, board);
                        }
                    }
                    Page::Net => {
                        controls::search(ui, ui.scope_id().with("net_search"), &mut edit.net_search, "Search nets", "Search nets");
                        ui.add_enabled_ui(!dirty, |ui| {
                            controls::Choice::from_id_salt("class_net_choice")
                                .selected_text(edit.net_index.and_then(|index| edit.nets.get(index)).map_or("Choose net", |net| net.label.as_str()))
                                .show_ui(ui, |ui| {
                                    for (index, net) in edit
                                        .nets
                                        .iter()
                                        .enumerate()
                                        .filter(|(_, net)| net.label.to_lowercase().contains(&edit.net_search.to_lowercase()))
                                    {
                                        ui.selectable_value(&mut pick_net, Some(index), &net.label);
                                    }
                                })
                                .response
                                .accessible_name("Net");
                        });
                        if let Some(error) = &edit.selection_error {
                            ui.label(shell::error(error));
                        }
                        if edit.target.is_some() {
                            properties::section(ui, "Assigned classes");
                            if edit.project.net_classes().definitions.is_empty() {
                                ui.label(shell::dim("No class definitions"));
                            }
                            for class in &edit.project.net_classes().definitions {
                                let mut selected = edit.classes.contains(&class.id);
                                let name = format!("{} · {}", class.name, &class.id.to_string()[..8]);
                                if ui
                                    .push_id(class.id, |ui| controls::checkbox(ui, &mut selected, &name))
                                    .inner
                                    .on_hover_text(class.id.to_string())
                                    .changed()
                                {
                                    if selected {
                                        edit.classes.insert(class.id);
                                    } else {
                                        edit.classes.remove(&class.id);
                                    }
                                }
                            }
                            match edit.project.resolve_net_classes(&edit.classes.iter().copied().collect::<Vec<_>>(), edit.project.board_target()) {
                                Ok(result) => resolution(ui, &edit.project, &result, edit.unit),
                                Err(error) => {
                                    ui.label(shell::error(error.to_string()));
                                }
                            }
                            egui::CollapsingHeader::new("Assignment origins").show(ui, |ui| {
                                for binding in &edit.bindings {
                                    ui.label(format!("Class {} · origin {}", binding.class, binding.origin));
                                }
                            });
                        } else if edit.selection_error.is_none() {
                            ui.label(shell::dim("No supported net selected"));
                        }
                    }
                }
                if let Some(error) = &edit.error {
                    ui.label(shell::error(error));
                }
                if edit.page == Page::Classes
                    && let Some(Err(error)) = &rules
                {
                    ui.label(shell::error(error));
                }
            });
        });
        if action.close {
            return;
        }
        if board_defaults && current {
            self.begin_board_rules_from(edit.source);
            return;
        }
        if new_class {
            edit.load_class(None);
        } else if pick_class != edit.selected_class {
            edit.load_class(pick_class);
        }
        if pick_net != edit.net_index
            && let Some(index) = pick_net
            && let Err(error) = edit.load_net(index)
        {
            edit.error = Some(error);
        }
        if action.primary && current && valid && edit.page == page {
            let old_ids: BTreeSet<_> = edit.project.net_classes().definitions.iter().map(|class| class.id).collect();
            let selected = edit.selected_class;
            let chosen_net = edit.net_index.and_then(|index| edit.nets.get(index)).map(|net| net.target.clone());
            match edit.page {
                Page::Classes => {
                    if let Some(draft) = &edit.draft {
                        match draft.rules(edit.unit) {
                            Ok(rules) => {
                                let mut params = json!({"name":draft.name,"rules":rules,"expected_revision":edit.source.revision});
                                if let Some(id) = draft.id {
                                    params["class"] = json!(id);
                                }
                                self.run(if draft.id.is_some() { net_classes::UPDATE } else { net_classes::CREATE }, &params);
                            }
                            Err(error) => {
                                edit.error = Some(error);
                                self.dialog = Dialog::Rules(edit);
                                return;
                            }
                        }
                    }
                }
                Page::Net => {
                    if let Some(target) = &edit.target {
                        self.run(
                            pcbcraft_engine::nets::SET_CLASSES,
                            &json!({"target":target,"classes":edit.classes,"expected_revision":edit.source.revision}),
                        );
                    }
                }
            }
            if let Some(error) = self.message.take() {
                edit.error = Some(error);
            } else {
                let source = self.views.view(edit.source.view()).and_then(|view| view.input(&self.session));
                if let Some(source) = source {
                    match Edit::capture(self, source, edit.page) {
                        Ok(mut refreshed) => {
                            if edit.page == Page::Classes {
                                let id = selected.or_else(|| {
                                    refreshed
                                        .project
                                        .net_classes()
                                        .definitions
                                        .iter()
                                        .find(|class| !old_ids.contains(&class.id))
                                        .map(|class| class.id)
                                });
                                refreshed.load_class(id);
                            }
                            if let Some(target) = chosen_net
                                && let Some(index) = refreshed.nets.iter().position(|net| net.target == target)
                                && let Err(error) = refreshed.load_net(index)
                            {
                                refreshed.error = Some(error);
                            }
                            edit = Box::new(refreshed);
                        }
                        Err(error) => edit.error = Some(error),
                    }
                }
                self.repaint_editors(ctx);
            }
        }
        self.dialog = Dialog::Rules(edit);
    }
}

#[cfg(test)]
mod tests;
