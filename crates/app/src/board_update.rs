//! Review one captured schematic-to-board change set without granting stale approval authority.
use crate::{Dialog, PcbApp, ViewId, ViewInput, editor_view::DocumentTarget};
use pcbcraft_chrome::{pcb_update as form, shell};
use pcbcraft_engine::{
    APPLY_UPDATE, CANCEL_UPDATE, PREVIEW_UPDATE, Project,
    board::{
        update::{Baseline, Blocker, BoardUpdatePreview, ComponentChange, NetState, Package},
        view::BoardTarget,
    },
};
use serde_json::json;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Create,
    Initialize,
    Review,
    Applied,
}

/// The originating view/opening and exact opaque plan remain fixed until explicit re-review.
#[derive(Debug)]
pub(super) struct Edit {
    pub(super) source: ViewInput,
    project: Uuid,
    board: Option<BoardTarget>,
    document: DocumentTarget,
    opening: u64,
    plan: Option<Uuid>,
    phase: Phase,
    stale: bool,
    error: Option<String>,
    place_new_footprints: bool,
    filters: form::Filters,
    report: Vec<(Option<u64>, form::Message)>,
}

#[derive(Default)]
enum Action {
    #[default]
    None,
    Create,
    Initialize,
    Review,
    Apply,
    Assign(u64),
    PlaceAll,
}

impl Edit {
    /// Block placement while the chooser is resolving a proposed update; only the accepted
    /// post-Apply list hands parts to placement without bypassing review.
    pub(super) fn allows_placement(&self) -> bool {
        self.phase == Phase::Applied
    }

    /// Changed chooser or project inputs invalidate a successful update result. Preserve the
    /// explicit creation/adoption steps, which capture their inputs when the user accepts them.
    pub(super) fn invalidate_review(&mut self) {
        if self.phase == Phase::Applied {
            self.phase = Phase::Review;
        }
        self.plan = None;
    }
}

impl PcbApp {
    /// Open a review from a real project pane. Opening alone changes no authored state;
    /// board creation and compatibility adoption each require their own explicit action.
    pub(super) fn begin_board_update(&mut self, view: ViewId) -> Result<(), String> {
        if self.home || self.files.busy() || self.project_action_pending() || !matches!(self.dialog, Dialog::None) {
            return Err("Finish the current dialog or project operation before updating the PCB.".into());
        }
        if self.views.owner(view).is_none() {
            return Err("The source pane is unavailable.".into());
        }
        let editor = self.views.view(view).ok_or("The source pane is unavailable.")?;
        let source = editor.input(&self.session).ok_or("Open a schematic or PCB project pane first.")?;
        let document = editor.target().ok_or("The source document is unavailable.")?;
        let project = self.session.project().ok_or("No project is open.")?;
        let mut edit = Edit {
            source,
            project: project.id(),
            board: project.board_target(),
            document,
            opening: self.session.history_root_revision(),
            plan: None,
            phase: update_phase(project),
            stale: false,
            error: None,
            place_new_footprints: true,
            filters: form::Filters::default(),
            report: Vec::new(),
        };
        if edit.phase == Phase::Review {
            edit.error = self.review_board_update(&mut edit).err();
        }
        self.dialog = Dialog::BoardUpdate(Box::new(edit));
        Ok(())
    }

    /// Keep re-review within the same opening, board and originating document; an equal
    /// persisted project ID after reopen is insufficient to revive approval authority.
    fn board_update_context(&self, edit: &Edit) -> bool {
        self.session.history_root_revision() == edit.opening
            && self.views.owner(edit.source.view()).is_some()
            && self.session.project().is_some_and(|project| project.id() == edit.project && project.board_target() == edit.board)
            && self.views.view(edit.source.view()).is_some_and(|view| view.target() == Some(edit.document) && view.input(&self.session).is_some())
    }

    /// Freeze a fresh engine-owned plan only after explicit opening or re-review. Another
    /// client's same-revision preview never becomes this dialog's plan automatically.
    fn review_board_update(&mut self, edit: &mut Edit) -> Result<(), String> {
        if !self.board_update_context(edit) {
            return Err("The project or source pane changed. Close this review and open Update PCB again.".into());
        }
        let board = edit.board.ok_or("Create a board before reviewing changes.")?;
        let source = self.views.view(edit.source.view()).and_then(|view| view.input(&self.session)).ok_or("The source pane is unavailable.")?;
        self.session.apply(PREVIEW_UPDATE, &json!({"board":board.board,"expected_revision":source.revision})).map_err(|error| error.to_string())?;
        let preview = self.session.cached_board_update().ok_or("The update preview is unavailable. Review again.")?;
        edit.plan = Some(preview.plan_token);
        edit.source = source;
        edit.report = report_entries(&preview);
        edit.phase = Phase::Review;
        Ok(())
    }

    /// Cancel only this dialog's capability. A replaced or stale token is harmless and cannot
    /// revoke a newer client's preview; no document edit or undo entry is created.
    fn cancel_board_update_review(&mut self, edit: &mut Edit) {
        if let Some(plan) = edit.plan.take() {
            let _ = self.session.apply(CANCEL_UPDATE, &json!({"plan_token":plan}));
        }
    }

    /// Return from package/placement work to the captured update context. Package changes need
    /// a new explicit review; the post-Apply list can refresh without approving anything else.
    pub(super) fn resume_board_update(&mut self, mut edit: Box<Edit>) {
        if !self.board_update_context(&edit) {
            self.cancel_board_update_review(&mut edit);
            self.message = Some("The project or source pane changed. Open Update PCB again to continue.".into());
            return;
        }
        edit.plan = None;
        edit.error = None;
        if let Some(source) = self.views.view(edit.source.view()).and_then(|view| view.input(&self.session)) {
            edit.source = source;
        }
        if edit.phase == Phase::Review {
            edit.error = self.review_board_update(&mut edit).err();
        }
        self.dialog = Dialog::BoardUpdate(edit);
    }

    /// Show complete old/new component and connection groups, keeping authoring behind the
    /// shared engine commands. Stale context, blockers and no-op plans cannot enable Apply.
    pub(super) fn board_update_dialog(&mut self, ctx: &egui::Context, mut edit: Box<Edit>) {
        let context = self.board_update_context(&edit);
        let source_current = self.views.view(edit.source.view()).and_then(|view| view.input(&self.session)) == Some(edit.source);
        let preview = self.session.cached_board_update().filter(|preview| Some(preview.plan_token) == edit.plan);
        let current_plan = context && source_current && preview.is_some();
        let can_apply = current_plan && preview.as_ref().is_some_and(|preview| preview.can_apply && !preview.changes.is_noop());
        let stale = !context || !source_current || (edit.phase == Phase::Review && !current_plan);
        if stale != edit.stale {
            edit.stale = stale;
            // External edits/previews can change this centered modal's height without an
            // authored revision. Settle its new geometry before the next pointer gesture.
            self.repaint_editors(ctx);
        }
        let project = self.session.snapshot();
        let mut action = Action::None;
        let missing_legacy_package = project.as_ref().is_some_and(|project| {
            let choices = Baseline::current_choices(project);
            project.components().any(|component| {
                component_number(&component.id()).is_some_and(|id| project.component_is_board_eligible(id) && !choices.contains_key(&id))
            })
        });
        let (primary, enabled) = match edit.phase {
            Phase::Create => ("Create board", context && source_current),
            Phase::Initialize => ("Use current state", context && source_current && !missing_legacy_package),
            _ => ("Update PCB", can_apply),
        };
        let has_report = matches!(edit.phase, Phase::Review | Phase::Applied) && context;
        let response = form::form(ctx, primary, enabled, has_report, |ui| {
            if !context {
                ui.label("The project or source pane changed. Close this review and open Update PCB again.");
            } else {
                if let Some(board) = edit.board.and_then(|target| project.as_ref().and_then(|project| project.board_view(target))) {
                    ui.strong(format!("{} · All schematic sheets", board.name()));
                } else {
                    ui.strong("All schematic sheets");
                }
                match edit.phase {
                    Phase::Create => {
                        ui.label("Create a PCB for this schematic.");
                    }
                    Phase::Initialize => {
                        ui.label("Existing footprints have no recorded accepted state. Choosing “Use current state” records today's schematic, footprint choices and connections as its starting point; earlier accepted history is unavailable.");
                        if let Some(project) = project.as_ref() {
                            let choices = Baseline::current_choices(project);
                            for component in project.components() {
                                if let Some(id) = component_number(&component.id())
                                    && project.component_is_board_eligible(id)
                                    && !choices.contains_key(&id)
                                {
                                    let label = format!("Assign footprint to {}", component.reference());
                                    if pcbcraft_chrome::controls::button(ui, label).clicked() {
                                        action = Action::Assign(id);
                                    }
                                }
                            }
                        }
                    }
                    Phase::Review => {
                        if let Some(preview) = preview.as_ref().filter(|_| current_plan) {
                            form::options(ui, &mut edit.place_new_footprints);
                            draw_report(ui, &edit.report, &mut edit.filters, &mut action, preview.changes.is_noop());
                            if preview.changes.is_noop()
                                && let Some(project) = project.as_ref()
                            {
                                draw_unplaced(ui, project, &mut action);
                            }
                        } else {
                            ui.label("The design, footprint choices or review changed. Review again before applying.");
                        }
                    }
                    Phase::Applied => {
                        ui.add_enabled_ui(false, |ui| form::options(ui, &mut edit.place_new_footprints));
                        ui.label("PCB updated.");
                        draw_report(ui, &edit.report, &mut edit.filters, &mut action, true);
                        if source_current {
                            if let Some(project) = project.as_ref() {
                                draw_unplaced(ui, project, &mut action);
                            }
                        } else {
                            ui.label("The design changed after this update. Review again before continuing.");
                        }
                    }
                }
                if (!source_current || (edit.phase == Phase::Review && !current_plan))
                    && pcbcraft_chrome::controls::button(ui, "Review again").clicked()
                {
                    action = Action::Review;
                }
            }
            if let Some(error) = &edit.error {
                ui.label(shell::error(error));
            }
        });
        if response.primary {
            action = match edit.phase {
                Phase::Create => Action::Create,
                Phase::Initialize => Action::Initialize,
                _ => Action::Apply,
            };
        }
        if response.close {
            self.cancel_board_update_review(&mut edit);
            self.repaint_editors(ctx);
            return;
        }
        if !context || matches!(action, Action::None) {
            self.dialog = Dialog::BoardUpdate(edit);
            return;
        }
        let result = match action {
            Action::Create if source_current => self
                .session
                .apply("pcb.create_board", &json!({"expected_revision":edit.source.revision}))
                .map_err(|error| error.to_string())
                .and_then(|_| {
                    self.views.sync(&self.session);
                    edit.board = self.session.project().and_then(|project| project.board_target());
                    edit.document = self.views.view(edit.source.view()).and_then(|view| view.target()).ok_or("The source pane is unavailable.")?;
                    edit.phase = Phase::Review;
                    self.review_board_update(&mut edit)
                }),
            Action::Initialize if source_current => self
                .session
                .apply(
                    pcbcraft_engine::board::update::INITIALIZE_BASELINE,
                    &json!({"board":edit.board.map(|board|board.board),"expected_revision":edit.source.revision}),
                )
                .map_err(|error| error.to_string())
                .and_then(|_| {
                    edit.phase = Phase::Review;
                    self.review_board_update(&mut edit)
                }),
            Action::Review => {
                self.cancel_board_update_review(&mut edit);
                edit.phase = self.session.project().map_or(Phase::Review, update_phase);
                if let Some(source) = self.views.view(edit.source.view()).and_then(|view| view.input(&self.session)) {
                    edit.source = source;
                }
                if edit.phase == Phase::Review { self.review_board_update(&mut edit) } else { Ok(()) }
            }
            Action::Apply if can_apply => self
                .session
                .apply(
                    APPLY_UPDATE,
                    &json!({"board":edit.board.map(|board|board.board),"plan_token":edit.plan,"expected_revision":edit.source.revision}),
                )
                .map_err(|error| error.to_string())
                .map(|_| {
                    edit.plan = None;
                    edit.phase = Phase::Applied;
                    self.views.sync(&self.session);
                    if let Some(source) = self.views.view(edit.source.view()).and_then(|view| view.input(&self.session)) {
                        edit.source = source;
                    }
                }),
            Action::PlaceAll => {
                self.cancel_board_update_review(&mut edit);
                if let Err(error) = self.begin_updated_footprints(&edit, ctx) {
                    edit.error = Some(error);
                    self.dialog = Dialog::BoardUpdate(edit);
                }
                self.repaint_editors(ctx);
                return;
            }
            Action::Assign(component) => {
                self.cancel_board_update_review(&mut edit);
                let previous = self.views.active.id();
                self.views.bind(edit.source.view());
                self.begin_footprint_assignment(Some(format!("component-{component}")));
                self.views.bind(previous);
                let review_slot = match &mut self.dialog {
                    Dialog::Footprint(chooser) => Some(&mut chooser.update_review),
                    Dialog::NativePart(chooser) => Some(&mut chooser.update_review),
                    _ => None,
                };
                if let Some(slot) = review_slot {
                    *slot = Some(edit);
                    self.repaint_editors(ctx);
                    return;
                }
                Err("The footprint chooser is unavailable. Reopen Update PCB to continue.".into())
            }
            _ => Ok(()),
        };
        self.views.sync(&self.session);
        self.repaint_editors(ctx);
        edit.error = result.err();
        if matches!(action, Action::Apply) && edit.error.is_none() && edit.place_new_footprints {
            if let Err(error) = self.begin_updated_footprints(&edit, ctx) {
                edit.error = Some(error);
                self.dialog = Dialog::BoardUpdate(edit);
            }
        } else {
            self.dialog = Dialog::BoardUpdate(edit);
        }
    }
    /// Switch only the originating pane after successful acceptance and capture the remaining
    /// accepted assignments for a disposable group placement. Cancelling placement retains them.
    fn begin_updated_footprints(&mut self, edit: &Edit, ctx: &egui::Context) -> Result<(), String> {
        let project = self.session.project().ok_or("No project is open.")?;
        let components: Vec<_> = project
            .footprint_assignments()
            .into_iter()
            .filter(|a| a["placed"] == false && a["pending_update"] != true && a["pending_removal"] != true)
            .filter_map(|a| a["component"].as_str().map(str::to_owned))
            .filter(|id| component_number(id).is_some_and(|id| project.component_is_board_eligible(id)))
            .collect();
        let board = edit.board.ok_or("The board is unavailable.")?;
        let candidate = if components.is_empty() {
            None
        } else {
            Some(
                self.session
                    .preview_footprint_placement(
                        &json!({"board":board.board,"components":components,"x_nm":0,"y_nm":0,"expected_revision":self.session.revision()}),
                    )
                    .map_err(|error| error.to_string())?,
            )
        };
        let owner = self.views.owner(edit.source.view()).ok_or("The source pane is unavailable.")?;
        let host = self.views.host;
        self.bind_editor_host(owner);
        self.activate_editor(ctx, edit.source.view());
        let view = self.views.view_mut(edit.source.view()).ok_or("The source pane is unavailable.")?;
        view.open_document(DocumentTarget::Board(board), &self.session);
        if let Some(candidate) = candidate {
            let source = view.input(&self.session).ok_or("The source pane is unavailable.")?;
            view.board.placement = Some(crate::board_canvas::Placement {
                source,
                components,
                candidate,
                handoff: true,
                position: pcbcraft_engine::schematic::Point { x_nm: 0, y_nm: 0 },
                rotation: 0,
            });
        }
        self.message = None;
        self.bind_editor_host(host);
        ctx.send_viewport_cmd_to(crate::editor_windows::viewport(owner), egui::ViewportCommand::Focus);
        Ok(())
    }
}

/// Select only the workflow phase required by current physical ownership and compatibility state.
fn update_phase(project: &Project) -> Phase {
    match project.board_target().and_then(|target| project.board_view(target)) {
        None => Phase::Create,
        Some(board) if board.needs_update_initialization() => Phase::Initialize,
        Some(_) => Phase::Review,
    }
}

/// Parse a canonical component address returned by the engine, never a displayed reference.
fn component_number(component: &str) -> Option<u64> {
    component.strip_prefix("component-")?.parse().ok()
}

/// Resolve review labels from the captured input so removed parts keep their original annotation.
fn component_label(preview: &BoardUpdatePreview<'_>, component: u64) -> String {
    preview.component(component).map_or_else(|| "Part".into(), |part| part.reference.clone())
}

/// Keep package identity and revision visible without exposing runtime capabilities or source hashes.
fn package_label(package: Option<&Package>) -> String {
    package.map_or_else(
        || "Unassigned".into(),
        |package| format!("{} / {} · r{}", package.library.as_deref().unwrap_or("Legacy marker"), package.definition, package.revision),
    )
}

/// Show terminal mappings and explicit unconnected pads in both current and previous review summaries.
fn mapping_label(package: Option<&Package>) -> String {
    package.map_or_else(
        || "Unassigned".into(),
        |package| {
            let mut entries: Vec<_> = package.mapping.iter().map(|(terminal, pad)| format!("Pin {terminal} → pad {pad}")).collect();
            if !package.unconnected_pads.is_empty() {
                entries.push(format!("unconnected pads {}", package.unconnected_pads.join(", ")));
            }
            entries.join(", ")
        },
    )
}

/// Project typed changes into report text without creating another mutable update plan. Missing
/// package rows carry canonical component IDs; report filters cannot change approval or mutations.
fn report_entries(preview: &BoardUpdatePreview<'_>) -> Vec<(Option<u64>, form::Message)> {
    let mut rows = Vec::new();
    for Blocker::MissingPackage { component } in &preview.changes.blockers {
        let label = component_label(preview, *component);
        rows.push((
            Some(*component),
            form::Message {
                kind: form::Kind::Error,
                text: format!("{label}: no footprint assigned."),
                action: Some(format!("Assign footprint to {label}")),
            },
        ));
    }
    for change in &preview.changes.components {
        let (id, verb) = match change {
            ComponentChange::Added { component } => (*component, "Add footprint"),
            ComponentChange::Removed { component } => (*component, "Remove footprint"),
            ComponentChange::Definition { component } => (*component, "Update symbol definition for"),
            ComponentChange::Package { component } => (*component, "Replace footprint for"),
            ComponentChange::Mapping { component } => (*component, "Update pin mapping for"),
        };
        let label = component_label(preview, id);
        let old = preview.accepted_component(id);
        let new = preview.proposed_component(id);
        let package = new.or(old).and_then(|part| part.package.as_ref());
        rows.push((None, form::Message { kind: form::Kind::Action, text: format!("{verb} {label}: {}.", package_label(package)), action: None }));
        if matches!(change, ComponentChange::Removed { .. }) {
            rows.push((None, form::Message { kind: form::Kind::Warning, text: format!("{label} will be removed from the PCB."), action: None }));
        } else {
            let mapping = new.and_then(|part| part.package.as_ref());
            if mapping.is_some() {
                rows.push((
                    None,
                    form::Message { kind: form::Kind::Info, text: format!("{label} pin mapping: {}.", mapping_label(mapping)), action: None },
                ));
            }
            if let Some(old) = old {
                rows.push((
                    None,
                    form::Message {
                        kind: form::Kind::Info,
                        text: format!(
                            "Previous footprint for {label}: {}. Previous mapping: {}.",
                            package_label(old.package.as_ref()),
                            mapping_label(old.package.as_ref())
                        ),
                        action: None,
                    },
                ));
            }
            if matches!(change, ComponentChange::Definition { .. }) {
                let symbol =
                    |part: &pcbcraft_engine::board::update::ComponentState| format!("{} / {} · r{}", part.library, part.definition, part.revision);
                rows.push((
                    None,
                    form::Message {
                        kind: form::Kind::Info,
                        text: format!(
                            "{label} symbol: {} → {}.",
                            old.map_or_else(|| "None".into(), symbol),
                            new.map_or_else(|| "None".into(), symbol)
                        ),
                        action: None,
                    },
                ));
            }
        }
    }
    for (nets, verb) in [(&preview.changes.removed_nets, "Remove previous connections"), (&preview.changes.added_nets, "Update connections")] {
        for net in nets {
            rows.push((None, form::Message { kind: form::Kind::Info, text: format!("{verb} — {}.", net_label(preview, net)), action: None }));
        }
    }
    if rows.is_empty() {
        rows.push((None, form::Message { kind: form::Kind::Info, text: "PCB is up to date.".into(), action: None }));
    }
    rows
}

/// Keep report selection and clipboard export separate from domain changes and plan authority.
fn draw_report(ui: &mut egui::Ui, rows: &[(Option<u64>, form::Message)], filters: &mut form::Filters, action: &mut Action, trailing_actions: bool) {
    let messages: Vec<_> = rows.iter().map(|(_, message)| message).collect();
    let selected = form::report(ui, &messages, filters, trailing_actions);
    if let Some(component) = selected.row.and_then(|index| rows.get(index)).and_then(|row| row.0) {
        *action = Action::Assign(component);
    }
    if selected.copy {
        ui.ctx().copy_text(rows.iter().map(|(_, message)| message.text.as_str()).collect::<Vec<_>>().join("\n"));
    }
}

/// Display terminal membership in a captured connection group using board annotations.
fn net_label(preview: &BoardUpdatePreview<'_>, net: &NetState) -> String {
    let name = if net.names.is_empty() { "Unnamed net".into() } else { net.names.join(", ") };
    let terminals = net
        .terminals
        .iter()
        .map(|terminal| format!("{}.{}", component_label(preview, terminal.component), terminal.terminal))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{name}: {terminals}")
}

/// New accepted packages remain unplaced; each action enters the existing explicit placement tool.
fn draw_unplaced(ui: &mut egui::Ui, project: &Project, action: &mut Action) {
    let assignments = project.footprint_assignments();
    let unplaced: Vec<_> = assignments
        .iter()
        .filter(|assignment| {
            assignment["placed"] == false
                && assignment["pending_removal"] != true
                && assignment["component"].as_str().and_then(component_number).is_some_and(|id| project.component_is_board_eligible(id))
        })
        .collect();
    if unplaced.is_empty() {
        ui.label("No unplaced footprints.");
        return;
    }
    ui.label(if unplaced.len() == 1 { "1 footprint ready to place".into() } else { format!("{} footprints ready to place", unplaced.len()) });
    if pcbcraft_chrome::controls::button(ui, "Place footprints").clicked() {
        *action = Action::PlaceAll;
    }
}
