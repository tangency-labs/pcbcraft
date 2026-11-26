//! Shared, revision-bound class authoring, net inspection and working-voltage requirements
//! for schematic and PCB views.
use crate::rule_diagram::label;
use crate::{Dialog, PcbApp, ViewInput, editor_view::DocumentTarget};
use pcbcraft_chrome::{controls, properties, rule_form as form, shell, units};
use pcbcraft_engine::{
    Project,
    board::view::BoardTarget,
    net_classes::{self, ContributionStatus, Definition, Property, Resolution, Rules, Source},
    nets::{Anchor, ClassBinding, ClassTarget, intent},
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Classes,
    Net,
    Voltage,
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

/// A saved net identity or class that can hold one working-voltage requirement. The target is
/// the exact `electrical_intent` selector; the label is display only. A historical owner is a
/// net that was deleted, merged or left ambiguous but still carries a declaration: it is listed
/// so the declaration can be reviewed and cleared, never redeclared.
#[derive(Debug)]
struct Owner {
    label: String,
    target: Value,
    net: Option<Uuid>,
    anchor: Option<Value>,
    historical: bool,
}

/// A voltage reference names Earth or a saved net identity, never a display name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reference {
    Earth,
    Net(Uuid),
}

/// Editable requirement text. The magnitude stays exact decimal text for the engine to check.
#[derive(Clone, Debug, PartialEq, Eq)]
struct VoltageFields {
    declared: bool,
    value: String,
    peak_ac: bool,
    reference: Reference,
}

/// One owner's draft beside the inspected report it was loaded from.
#[derive(Debug)]
struct VoltageDraft {
    owner: usize,
    fields: VoltageFields,
    original: VoltageFields,
    report: Value,
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
    scope: Value,
    owners: Vec<Owner>,
    references: Vec<(Uuid, String)>,
    voltage: Option<VoltageDraft>,
    selection_error: Option<String>,
    error: Option<String>,
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
pub(super) fn selected_target(
    project: &Project,
    target: Option<DocumentTarget>,
    schematic: &pcbcraft_engine::selection::Selection,
    copper: &BTreeSet<Uuid>,
) -> Result<Option<ClassTarget>, String> {
    let mut targets = Vec::new();
    let mut unsupported = false;
    match target {
        Some(DocumentTarget::Sheet(sheet)) => {
            let own = project.for_sheet(sheet).ok_or("Sheet unavailable.")?;
            for wire in own.wires().iter().filter(|wire| schematic.wires.contains(&wire.id())) {
                if let Some(target) = project.net_class_target_for_endpoint(sheet, wire.endpoints()[0]).map_err(|error| error.to_string())? {
                    targets.push(target);
                } else {
                    unsupported = true;
                }
            }
            for item in own.net_labels().iter().filter(|item| schematic.net_labels.contains(&item.object_id())) {
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
            for id in copper {
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
    fn capture(
        project: Arc<Project>,
        source: ViewInput,
        page: Page,
        unit: units::Unit,
        selected: Result<Option<ClassTarget>, String>,
    ) -> Result<Self, String> {
        let registry = project.inspect_net_registry()?;
        let rows = registry["nets"].as_array().ok_or("Net inspection is unavailable.")?;
        let scope = registry["scope"].clone();
        let mut nets = Vec::new();
        let mut owners = Vec::new();
        let mut references = Vec::new();
        for row in rows {
            let anchor: Anchor = serde_json::from_value(row["anchor"].clone()).map_err(|error| error.to_string())?;
            let label = row["name"].as_str().unwrap_or("Unresolved net").to_owned();
            if let Some(id) = row["id"].as_str().and_then(|id| Uuid::parse_str(id).ok()) {
                let address = json!({"project":project.id(),"scope":scope,"net":id});
                owners.push(Owner {
                    label: format!("Net · {label}"),
                    target: json!({"kind":"net","address":address}),
                    net: Some(id),
                    anchor: Some(row["anchor"].clone()),
                    historical: false,
                });
                // Only a named net can be chosen as a new reference, matching the engine rule.
                if row["names"].as_array().is_some_and(|names| !names.is_empty()) {
                    references.push((id, label.clone()));
                }
            }
            nets.push(NetChoice { label, target: ClassTarget::Derived { anchor } });
        }
        for record in registry["records"].as_array().into_iter().flatten() {
            if record["state"]["kind"] == "active" || record["authored_intent"]["voltage"].is_null() {
                continue;
            }
            let Some(id) = record["id"].as_str().and_then(|id| Uuid::parse_str(id).ok()) else { continue };
            let address = json!({"project":project.id(),"scope":scope,"net":id});
            owners.push(Owner {
                label: format!("Former net · {}", &id.to_string()[..8]),
                target: json!({"kind":"net","address":address}),
                net: Some(id),
                anchor: None,
                historical: true,
            });
        }
        for class in &project.net_classes().definitions {
            let target = json!({"kind":"net_class","project":project.id(),"class":class.id});
            owners.push(Owner { label: format!("Class · {}", class_label(&project, class)), target, net: None, anchor: None, historical: false });
        }
        let selected_target = selected.as_ref().ok().and_then(|selected| selected.as_ref());
        let net_index = match selected.as_ref().ok().and_then(Option::as_ref) {
            Some(target @ ClassTarget::Derived { .. }) => nets.iter().position(|net| &net.target == target),
            Some(ClassTarget::Persistent { net, .. }) => {
                let id = net.to_string();
                rows.iter().position(|row| row["id"].as_str() == Some(&id))
            }
            None => None,
        };
        let owner_index = selected_target.and_then(|selected| {
            owners.iter().position(|owner| {
                !owner.historical
                    && match selected {
                        ClassTarget::Persistent { net, .. } => owner.net == Some(*net),
                        ClassTarget::Derived { anchor } => owner.anchor.is_some() && owner.anchor == serde_json::to_value(anchor).ok(),
                    }
            })
        });
        let mut edit = Self {
            source,
            project,
            unit,
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
            scope,
            owners,
            references,
            voltage: None,
            selection_error: selected.err(),
            error: None,
        };
        if let Some(index) = net_index {
            edit.load_net(index)?;
        }
        if let Some(index) = owner_index {
            edit.load_voltage(index)?;
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
    /// Read one owner's requirement through the same inspection path as `electrical_intent.inspect`.
    fn load_voltage(&mut self, index: usize) -> Result<(), String> {
        let owner = self.owners.get(index).ok_or("Net or class no longer available.")?;
        let report = self.project.inspect_electrical_intent(&owner.target).map_err(|error| error.to_string())?;
        let saved = &report["intent"]["working_voltage"];
        let fields = if saved.is_null() {
            VoltageFields { declared: false, value: String::new(), peak_ac: false, reference: Reference::Earth }
        } else {
            let reference = match saved["reference"]["address"]["net"].as_str() {
                Some(net) => Reference::Net(Uuid::parse_str(net).map_err(|error| error.to_string())?),
                None => Reference::Earth,
            };
            VoltageFields {
                declared: true,
                value: saved["magnitude"]["value"].as_str().unwrap_or_default().to_owned(),
                peak_ac: saved["basis"] == "peak_ac",
                reference,
            }
        };
        self.voltage = Some(VoltageDraft { owner: index, original: fields.clone(), fields, report });
        self.error = None;
        Ok(())
    }
    /// Build the `electrical_intent.set` replacement. An undeclared requirement clears it.
    fn voltage_intent(&self, fields: &VoltageFields) -> Value {
        if !fields.declared {
            return json!({"working_voltage": null});
        }
        let reference = match fields.reference {
            Reference::Earth => json!({"kind":"earth"}),
            Reference::Net(net) => json!({"kind":"net","address":{"project":self.project.id(),"scope":self.scope,"net":net}}),
        };
        json!({"working_voltage":{"magnitude":{"value":fields.value.trim(),"unit":"V"},"basis":if fields.peak_ac { "peak_ac" } else { "dc" },"reference":reference}})
    }
    /// Use exact class UUIDs for selection and retain all sparse source integers in the draft.
    fn load_class(&mut self, id: Option<Uuid>) {
        self.draft = Some(ClassDraft::new(id.and_then(|id| self.project.net_classes().get(id)), self.baseline(), self.unit));
        self.selected_class = id;
        self.error = None;
    }
    /// Only the active page owns pending edits; navigation cannot discard them implicitly.
    pub(super) fn changed(&self) -> bool {
        match self.page {
            Page::Classes => self.draft.as_ref().is_some_and(|draft| draft.changed(self.unit)),
            Page::Net => self.classes != self.original_classes,
            Page::Voltage => self.voltage.as_ref().is_some_and(|draft| draft.fields != draft.original),
        }
    }

    /// Only a saved, active identity can address the board's personal net palette.
    fn net_color_target(&self, board: BoardTarget) -> Option<(Uuid, &str)> {
        let ClassTarget::Persistent { project, net, .. } = self.target.as_ref()? else { return None };
        if *project != board.project || self.project.board_view(board)?.zone_net_state(*net) != "active" {
            return None;
        }
        Some((*net, &self.nets.get(self.net_index?)?.label))
    }
}

/// Render authored and inherited values in the same aligned form cells. Disabled fields retain
/// their source value and exact edit text, and the host owns each sparse opinion's checkbox.
fn class_fields(ui: &mut egui::Ui, draft: &mut ClassDraft) {
    ui.label("Class name");
    controls::text(ui, &mut draft.name).accessible_name("Class name");
    for preferred in [false, true] {
        {
            let mut fields: Vec<_> = draft.fields.iter_mut().filter(|field| field.property.is_preference() == preferred).collect();
            form::rows(
                ui,
                if preferred { "class_preferences" } else { "class_limits" },
                if preferred { "Routing preferences" } else { "Hard limits" },
                ["Class value", "Contribution"],
                &mut fields,
                |ui, field, cell| match cell {
                    form::Cell::Diagram => crate::rule_diagram::show(ui, field.property),
                    form::Cell::Rule => {
                        controls::checkbox(ui, &mut field.enabled, label(field.property));
                    }
                    form::Cell::Value => {
                        let mut inherited = "Not specified".to_owned();
                        let text = if field.enabled { &mut field.text } else { &mut inherited };
                        form::length_field(ui, text, field.enabled, format!("{} value", label(field.property)));
                    }
                    form::Cell::Source => {
                        ui.label(shell::dim(if !field.enabled {
                            "None"
                        } else if field.property.is_preference() {
                            "Preference"
                        } else if field.property.is_maximum() {
                            "Upper limit"
                        } else {
                            "Lower limit"
                        }));
                    }
                },
            );
        }
    }
}

/// Use human names in the main form; precise UUIDs/revisions remain in the source disclosure.
fn source_name(project: &Project, source: &Source) -> String {
    match source {
        Source::Builtin => "Built-in defaults".into(),
        Source::Request => "Request".into(),
        Source::Board { .. } => "Board defaults".into(),
        Source::Class { rule, .. } => project.net_classes().get(*rule).map_or_else(|| "Class".into(), |class| class_label(project, class)),
    }
}

/// Return a concise active-source summary and complete names for its tooltip. Equal hard
/// limits are a match, not an additive requirement; retain every contributor in the details.
/// An overridden baseline is never reported as an active preference merely because it matches.
fn effective_sources(project: &Project, result: &Resolution, property: Property) -> (String, String) {
    let active: Vec<_> = result
        .contribution_statuses(property)
        .filter(|(_, status)| *status == ContributionStatus::Active)
        .map(|(contribution, _)| contribution)
        .collect();
    let names = active
        .iter()
        .map(|contribution| source_name(project, &contribution.source))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ");
    let summary = if active.is_empty() {
        "No active source".into()
    } else if active.len() == 1 {
        names.clone()
    } else if property.is_preference() {
        "Agreed class preferences".into()
    } else {
        let bound = if property.is_maximum() { "maximum" } else { "minimum" };
        let baseline = active.iter().find_map(|contribution| match contribution.source {
            Source::Builtin => Some("built-in"),
            Source::Board { .. } => Some("board"),
            Source::Request => Some("requested"),
            Source::Class { .. } => None,
        });
        match baseline {
            Some(baseline) => format!("Matches {baseline} {bound}"),
            None => format!("Shared class {bound}"),
        }
    };
    (summary, names)
}

/// Detail labels describe participation without presenting inactive values as additional rules.
fn status_label(status: ContributionStatus) -> &'static str {
    match status {
        ContributionStatus::Active => "Active",
        ContributionStatus::Overridden => "Overridden",
        ContributionStatus::LessRestrictive => "Less restrictive",
        ContributionStatus::Conflicting => "Conflicting",
    }
}

/// Effective values use the same columns as authored dimensions. Surface conflicts before
/// the table while keeping technical source identities and assignment origins in a disclosure.
fn resolution(ui: &mut egui::Ui, project: &Project, result: &Resolution, unit: units::Unit, bindings: &[ClassBinding], preview: bool) {
    if preview {
        ui.label(shell::dim("Unsaved assignment preview"));
    }
    if !result.conflicts.is_empty() {
        form::conflict(ui, |ui| {
            ui.strong("Rule conflicts");
            for conflict in &result.conflicts {
                let properties = conflict.properties.iter().map(|property| label(*property)).collect::<Vec<_>>().join(" / ");
                ui.label(format!("{properties}: {}", conflict.message));
            }
        });
    }
    for preferred in [false, true] {
        {
            let mut fields: Vec<_> = Property::ALL.into_iter().filter(|property| property.is_preference() == preferred).collect();
            form::rows(
                ui,
                if preferred { "effective_preferences" } else { "effective_limits" },
                if preferred { "Routing preferences" } else { "Hard limits" },
                if preview { ["Preview value", "Preview source"] } else { ["Effective value", "Active source"] },
                &mut fields,
                |ui, property, cell| match cell {
                    form::Cell::Diagram => crate::rule_diagram::show(ui, *property),
                    form::Cell::Rule => {
                        ui.label(label(*property));
                    }
                    form::Cell::Value => {
                        if let Some(value) = result.values.get(property).copied().flatten() {
                            ui.label(units::label(value, unit));
                        } else {
                            ui.label(shell::error("Unresolved"));
                        }
                    }
                    form::Cell::Source => {
                        let (text, sources) = effective_sources(project, result, *property);
                        ui.add(egui::Label::new(&text).truncate())
                            .accessible_name(format!("{} {}: {text}", if preview { "Preview source" } else { "Active source" }, label(*property)))
                            .on_hover_text(sources);
                    }
                },
            );
        }
    }
    form::section(ui, "Details", |ui| {
        controls::collapsing(ui, "rule_sources", "Contributions and identities", false, None, |ui| {
            for property in Property::ALL {
                ui.strong(label(property));
                for (contribution, status) in result.contribution_statuses(property) {
                    let identity = match contribution.source {
                        Source::Class { rule, revision } => format!(" · {rule} · r{revision}"),
                        Source::Board { rule } => format!(" · {rule}"),
                        _ => String::new(),
                    };
                    let text = format!(
                        "{} · {}: {}{}",
                        status_label(status),
                        source_name(project, &contribution.source),
                        units::label(contribution.value_nm, unit),
                        identity
                    );
                    ui.add(
                        egui::Label::new(if status == ContributionStatus::Active {
                            egui::RichText::new(text)
                        } else if status == ContributionStatus::Conflicting {
                            shell::error(text)
                        } else {
                            shell::dim(text)
                        })
                        .wrap(),
                    );
                }
            }
        });
        controls::collapsing(ui, "class_origins", "Saved assignment origins", false, None, |ui| {
            for binding in bindings {
                ui.label(format!("Class {} · origin {}", binding.class, binding.origin));
            }
        });
    });
}

/// Designer-facing wording for the engine's requirement report; unknown codes stay legible.
fn review_reason(reason: &str) -> &str {
    match reason {
        "reference_retired" => "The reference net no longer exists in the design.",
        "reference_ambiguous" => "The reference net was split and no branch kept its identity.",
        "reference_merged" => "The reference net was merged into another net.",
        "reference_resolved" => "Resolving a split restored the original net identity. Choose that net again if it is still the reference.",
        "reference_split" => "The reference net was split; another branch now has its own identity.",
        "net_retired" => "This net no longer exists in the design.",
        "net_ambiguous" => "This net was split and no branch kept its identity.",
        "net_merged" => "This net was merged into another net.",
        "net_resolved" => "Resolving a split restored the original net identity. Re-declare the voltage on that net if it still applies.",
        "net_split" => "This net was split; another branch now has its own identity.",
        other => other,
    }
}

/// Whether every review reason is a split that kept the identities: saving the unchanged
/// declaration again then records the review and clears the flag.
fn split_review(report: &Value) -> bool {
    let reasons = report["requirement"]["reasons"].as_array();
    reasons.is_some_and(|reasons| !reasons.is_empty() && reasons.iter().all(|reason| reason.as_str().is_some_and(|r| r.ends_with("_split"))))
}

/// Author and inspect one working-voltage requirement through `electrical_intent`. The
/// status rows repeat the shared inspection report: where the value came from, that nothing
/// verifies it yet, and whether its owner or reference needs review. A retained reference
/// that is no longer current stays selectable so editing the value does not silently drop it.
/// Returns whether the designer asked to revert this page's draft.
fn voltage_page(ui: &mut egui::Ui, edit: &mut Edit, dirty: bool, pick_owner: &mut Option<usize>) -> bool {
    ui.add_enabled_ui(!dirty, |ui| {
        controls::Choice::from_id_salt("voltage_owner_choice")
            .width(ui.available_width())
            .selected_text(
                edit.voltage.as_ref().and_then(|draft| edit.owners.get(draft.owner)).map_or("Choose net or class", |owner| owner.label.as_str()),
            )
            .show_ui(ui, |ui| {
                for (index, owner) in edit.owners.iter().enumerate() {
                    ui.selectable_value(pick_owner, Some(index), &owner.label);
                }
            })
            .response
            .accessible_name("Declared on");
    });
    let discard = ui.add_enabled_ui(dirty, |ui| controls::button(ui, "Revert working voltage changes")).inner.clicked();
    let Some(draft) = &mut edit.voltage else {
        ui.label(shell::dim(if edit.owners.is_empty() { "No saved nets or classes" } else { "No net or class selected" }));
        return discard;
    };
    let own_net = edit.owners.get(draft.owner).and_then(|owner| owner.net);
    let historical = edit.owners.get(draft.owner).is_some_and(|owner| owner.historical);
    form::section(ui, "Requirement", |ui| {
        if historical {
            let removed =
                draft.report["requirement"]["reasons"].as_array().is_some_and(|reasons| reasons.iter().any(|reason| reason == "net_retired"));
            ui.label(shell::dim(if removed {
                "This net is no longer in the design. Its declaration can be reviewed or cleared, not changed."
            } else {
                "This net identity is no longer active. Its declaration can be reviewed or cleared, not changed."
            }));
        }
        controls::checkbox(ui, &mut draft.fields.declared, "Declare working voltage");
        ui.add_enabled_ui(draft.fields.declared && !historical, |ui| {
            properties::field(ui, "Value", |ui| {
                controls::text(ui, &mut draft.fields.value).accessible_name("Working voltage value");
                ui.label("V");
            });
            properties::field(ui, "Basis", |ui| {
                controls::Choice::from_id_salt("voltage_basis")
                    .width(160.)
                    .selected_text(if draft.fields.peak_ac { "Peak AC" } else { "DC" })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut draft.fields.peak_ac, false, "DC");
                        ui.selectable_value(&mut draft.fields.peak_ac, true, "Peak AC");
                    })
                    .response
                    .accessible_name("Voltage basis");
            });
            let retained = match draft.original.reference {
                Reference::Net(net) if !edit.references.iter().any(|(id, _)| *id == net) => Some(net),
                _ => None,
            };
            let reference_label = |reference: Reference| match reference {
                Reference::Earth => "Earth".to_owned(),
                Reference::Net(net) => edit
                    .references
                    .iter()
                    .find(|(id, _)| *id == net)
                    .map_or_else(|| format!("Retained net · {}", &net.to_string()[..8]), |(_, name)| name.clone()),
            };
            properties::field(ui, "Reference", |ui| {
                controls::Choice::from_id_salt("voltage_reference")
                    .width(ui.available_width())
                    .selected_text(reference_label(draft.fields.reference))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut draft.fields.reference, Reference::Earth, "Earth");
                        for (id, name) in edit.references.iter().filter(|(id, _)| Some(*id) != own_net) {
                            ui.selectable_value(&mut draft.fields.reference, Reference::Net(*id), name);
                        }
                        if let Some(net) = retained {
                            ui.selectable_value(&mut draft.fields.reference, Reference::Net(net), reference_label(Reference::Net(net)));
                        }
                    })
                    .response
                    .accessible_name("Voltage reference");
            });
        });
    });
    form::section(ui, "Status", |ui| {
        let requirement = &draft.report["requirement"];
        let source = match requirement["source"].as_str() {
            Some("authored_on_net") => "Entered on this net",
            Some("authored_on_net_class") => "Entered on this class",
            _ => "Not declared",
        };
        properties::field(ui, "Source", |ui| ui.label(source).accessible_name(format!("Requirement source: {source}")));
        if !requirement.is_null() {
            let verification = "Unverified: no ERC, DRC or solver checks this value yet";
            properties::field(ui, "Checked", |ui| ui.label(shell::dim(verification)).accessible_name(format!("Verification: {verification}")));
            let reasons: Vec<_> = requirement["reasons"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            if reasons.is_empty() {
                properties::field(ui, "Review", |ui| ui.label("Current").accessible_name("Review: Current"));
            } else {
                form::conflict(ui, |ui| {
                    ui.strong("Needs review");
                    for reason in reasons {
                        ui.label(review_reason(reason));
                    }
                    ui.label("The original declaration and reference are kept until you change them.");
                    if split_review(&draft.report) {
                        ui.label("After checking it, choose Save working voltage to record the review.");
                    }
                });
            }
        }
        if dirty {
            ui.label(shell::dim("Unsaved working voltage"));
        }
    });
    discard
}

/// Duplicate display names receive a short identity only when necessary to distinguish rows.
fn class_label(project: &Project, class: &Definition) -> String {
    if project.net_classes().definitions.iter().filter(|other| other.name == class.name).count() > 1 {
        format!("{} · {}", class.name, &class.id.to_string()[..8])
    } else {
        class.name.clone()
    }
}

/// Navigation intents produced by shared editor content; the containing host applies them.
#[derive(Debug)]
pub(super) struct Actions {
    page: Page,
    new_class: bool,
    discard: bool,
    pick_class: Option<Uuid>,
    pick_net: Option<usize>,
    pick_owner: Option<usize>,
    pub(super) board_defaults: bool,
    net_color: Option<(BoardTarget, Uuid, String)>,
}

impl Edit {
    /// Expose the captured view for host placement without acquiring new edit authority.
    pub(super) fn source(&self) -> ViewInput {
        self.source
    }
    /// The containing modal owns its footer; editor content supplies its action label.
    pub(super) fn primary(&self) -> &'static str {
        match self.page {
            Page::Classes => "Save class",
            Page::Net => "Assign classes",
            Page::Voltage => "Save working voltage",
        }
    }
    /// Validate only this editor's staged fields; other setup pages retain independent drafts.
    pub(super) fn can_apply(&self) -> bool {
        match self.page {
            Page::Classes => self.draft.as_ref().is_some_and(|d| d.rules(self.unit).is_ok()) && self.changed(),
            Page::Net => self.target.is_some() && self.changed(),
            Page::Voltage => self.voltage.as_ref().is_some_and(|draft| {
                let confirm = draft.fields.declared && split_review(&draft.report);
                (self.changed() || confirm) && (!draft.fields.declared || !draft.fields.value.trim().is_empty())
            }),
        }
    }
    /// A containing host may retain unsaved content while refreshing after its own validated
    /// command. External revisions must still expire the host and cannot use this method.
    pub(super) fn context_after_owned_command(&mut self, source: ViewInput, project: Arc<Project>) {
        self.source = source;
        self.project = project;
    }
    /// Render into the caller's scroll owner. A host with a board target can open its net palette.
    pub(super) fn show(&mut self, ui: &mut egui::Ui, current: bool, color_board: Option<BoardTarget>) -> Actions {
        let edit = self;
        let page = edit.page;
        let dirty = edit.changed();
        let rules = edit.draft.as_ref().map(|draft| draft.rules(edit.unit));
        let mut pick_class = edit.selected_class;
        let mut pick_net = edit.net_index;
        let mut pick_owner = edit.voltage.as_ref().map(|draft| draft.owner);
        let mut new_class = false;
        let mut board_defaults = false;
        let mut net_color = None;
        let discard;
        ui.add_enabled_ui(!dirty, |ui| {
            properties::tabs(
                ui,
                &mut edit.page,
                &[(Page::Classes, "Class definitions"), (Page::Net, "Effective net rules"), (Page::Voltage, "Working voltage")],
            )
        });
        if !current {
            ui.label(shell::error("The project or view changed. Close and reopen Rules to continue."));
        }
        match edit.page {
            Page::Classes => {
                ui.add_enabled_ui(!dirty, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        controls::Choice::from_id_salt("class_choice")
                            .width(240.)
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
                                        format!("{} · r{}", class_label(&edit.project, class), class.revision),
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
                discard = ui.add_enabled_ui(dirty, |ui| controls::button(ui, "Discard class changes")).inner.clicked();
                if let Some(draft) = &mut edit.draft {
                    // Revert actions may appear while typing; field identity must not depend on them.
                    ui.push_id("class_draft_fields", |ui| class_fields(ui, draft));
                }
            }
            Page::Net => {
                controls::search(ui, ui.scope_id().with("net_search"), &mut edit.net_search, "Search nets", "Search nets");
                ui.add_enabled_ui(!dirty, |ui| {
                    controls::Choice::from_id_salt("class_net_choice")
                        .width(ui.available_width())
                        .selected_text(edit.net_index.and_then(|index| edit.nets.get(index)).map_or("Choose net", |net| net.label.as_str()))
                        .show_ui(ui, |ui| {
                            for (index, net) in
                                edit.nets.iter().enumerate().filter(|(_, net)| net.label.to_lowercase().contains(&edit.net_search.to_lowercase()))
                            {
                                ui.selectable_value(&mut pick_net, Some(index), &net.label);
                            }
                        })
                        .response
                        .accessible_name("Net");
                });
                if let Some(board) = color_board
                    && let Some((net, label)) = edit.net_color_target(board)
                    && controls::button(ui, "Net color…").clicked()
                {
                    net_color = Some((board, net, label.to_owned()));
                }
                discard = ui.add_enabled_ui(dirty, |ui| controls::button(ui, "Revert assignment changes")).inner.clicked();
                if let Some(error) = &edit.selection_error {
                    ui.label(shell::error(error));
                }
                if edit.target.is_some() {
                    form::section(ui, "Assigned classes", |ui| {
                        if edit.project.net_classes().definitions.is_empty() {
                            ui.label(shell::dim("No class definitions"));
                        }
                        for class in &edit.project.net_classes().definitions {
                            let mut selected = edit.classes.contains(&class.id);
                            let name = class_label(&edit.project, class);
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
                    });
                    match edit.project.resolve_net_classes(&edit.classes.iter().copied().collect::<Vec<_>>(), edit.project.board_target()) {
                        Ok(result) => resolution(ui, &edit.project, &result, edit.unit, &edit.bindings, edit.classes != edit.original_classes),
                        Err(error) => {
                            ui.label(shell::error(error.to_string()));
                        }
                    }
                } else if edit.selection_error.is_none() {
                    ui.label(shell::dim("No supported net selected"));
                }
            }
            Page::Voltage => discard = voltage_page(ui, edit, dirty, &mut pick_owner),
        }
        if let Some(error) = &edit.error {
            ui.label(shell::error(error));
        }
        if edit.page == Page::Classes
            && let Some(Err(error)) = &rules
        {
            ui.label(shell::error(error));
        }

        Actions { page, new_class, discard, pick_class, pick_net, pick_owner, board_defaults, net_color }
    }
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
        self.open_rules_from(source, net);
    }
    /// Validate a navigation handoff before capturing the same revision-bound shared form.
    fn open_rules_from(&mut self, source: ViewInput, net: bool) {
        if !matches!(self.dialog, Dialog::None)
            || self.files.busy()
            || self.project_action_pending()
            || !self.views.view(source.view()).is_some_and(|view| view.accepts(source, &self.session))
        {
            return;
        }
        match self.capture_rules(source, if net { Page::Net } else { Page::Classes }, self.preferences.units) {
            Ok(edit) => self.dialog = Dialog::Rules(Box::new(edit)),
            Err(error) => self.message = Some(error),
        }
    }
    /// Capture class content for an existing host without opening or replacing any modal.
    pub(super) fn capture_class_rules(&self, source: ViewInput, unit: units::Unit) -> Result<Edit, String> {
        self.capture_rules(source, Page::Classes, unit)
    }
    /// Assemble source-view inputs at the shell boundary; reusable draft/selection helpers need
    /// only the immutable project, explicit selection and captured display/authority data.
    fn capture_rules(&self, source: ViewInput, page: Page, unit: units::Unit) -> Result<Edit, String> {
        let view = self.views.view(source.view()).filter(|view| view.accepts(source, &self.session)).ok_or("The originating view changed.")?;
        let project = self.session.snapshot().ok_or("No project open.")?;
        let selected = selected_target(&project, view.target(), &view.schematic.selection, &view.board.tracks.selected);
        Edit::capture(project, source, page, unit, selected)
    }
    /// Host reusable Rules content in its standalone modal. The same content and command path
    /// are embedded in Board Setup without replacing that containing modal.
    pub(super) fn rules_dialog(&mut self, ctx: &egui::Context, mut edit: Box<Edit>) {
        let Some(view) = self.views.view(edit.source.view()) else { return };
        let current = view.accepts(edit.source, &self.session);
        let color_board = view.target().filter(|_| current).and_then(|target| match target {
            DocumentTarget::Board(board) => Some(board),
            DocumentTarget::Sheet(_) => edit.project.board_target(),
            DocumentTarget::BoardWorkspace(_) => None,
        });
        let primary = edit.primary();
        let valid = edit.can_apply();
        let mut actions = None;
        let action = pcbcraft_chrome::catalog::settings_form_width(ctx, "Rules", 800., primary, current && valid, |ui| {
            form::body(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("class_rule_body").auto_shrink([false, false]).show(ui, |ui| {
                    pcbcraft_chrome::workspace::describe_scroll_viewport(ui);
                    actions = Some(edit.show(ui, current, color_board));
                });
            });
        });
        if action.close {
            return;
        }
        if let Some(mut actions) = actions {
            if actions.board_defaults && current {
                self.begin_board_rules_from(edit.source);
                return;
            }
            if let Some((board, net, label)) = actions.net_color.take() {
                self.begin_net_color_for(edit.source, board, net, label, None, Some(edit));
                return;
            }
            self.apply_rule_actions(ctx, &mut edit, actions, action.primary);
        }
        self.dialog = Dialog::Rules(edit);
    }
    /// Apply one editor's typed actions through shared commands. A true result means this host
    /// successfully submitted its own command; errors retain the draft and stale sources refuse.
    pub(super) fn apply_rule_actions(&mut self, ctx: &egui::Context, edit: &mut Edit, actions: Actions, submit: bool) -> bool {
        let Actions { page, new_class, discard, pick_class, pick_net, pick_owner, .. } = actions;
        if discard {
            match edit.page {
                Page::Classes => {
                    if edit.selected_class.is_some() {
                        edit.load_class(edit.selected_class);
                    } else {
                        edit.draft = None;
                        edit.error = None;
                    }
                }
                Page::Net => {
                    edit.classes = edit.original_classes.clone();
                    edit.error = None;
                }
                Page::Voltage => {
                    if let Some(draft) = &mut edit.voltage {
                        draft.fields = draft.original.clone();
                    }
                    edit.error = None;
                }
            }
            return false;
        }
        let current = self.views.view(edit.source.view()).is_some_and(|view| view.accepts(edit.source, &self.session));
        let valid = edit.can_apply();
        let mut saved = false;
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
        if pick_owner != edit.voltage.as_ref().map(|draft| draft.owner)
            && let Some(index) = pick_owner
            && let Err(error) = edit.load_voltage(index)
        {
            edit.error = Some(error);
        }
        if submit && current && valid && edit.page == page {
            let old_ids: BTreeSet<_> = edit.project.net_classes().definitions.iter().map(|class| class.id).collect();
            let selected = edit.selected_class;
            let chosen_net = edit.net_index.and_then(|index| edit.nets.get(index)).map(|net| net.target.clone());
            let chosen_owner = edit.voltage.as_ref().and_then(|draft| edit.owners.get(draft.owner)).map(|owner| owner.target.clone());
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
                                return false;
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
                Page::Voltage => {
                    if let (Some(draft), Some(target)) = (&edit.voltage, &chosen_owner) {
                        let params = json!({"version":intent::CONTRACT_VERSION,"expected_revision":edit.source.revision,"target":target,
                            "intent":edit.voltage_intent(&draft.fields)});
                        self.run(intent::SET, &params);
                    }
                }
            }
            if let Some(error) = self.message.take() {
                edit.error = Some(error);
            } else {
                let source = self.views.view(edit.source.view()).and_then(|view| view.input(&self.session));
                if let Some(source) = source {
                    match self.capture_rules(source, edit.page, edit.unit) {
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
                            if let Some(target) = chosen_owner
                                && let Some(index) = refreshed.owners.iter().position(|owner| owner.target == target)
                                && let Err(error) = refreshed.load_voltage(index)
                            {
                                refreshed.error = Some(error);
                            }
                            *edit = refreshed;
                        }
                        Err(error) => edit.error = Some(error),
                    }
                }
                self.repaint_editors(ctx);
                saved = true;
            }
        }
        saved
    }
}

#[cfg(test)]
mod tests;
