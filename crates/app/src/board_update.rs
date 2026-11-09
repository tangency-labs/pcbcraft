//! Review one captured schematic-to-board change set without granting stale approval authority.
use crate::{Dialog, PcbApp, ViewId, ViewInput, editor_view::DocumentTarget};
use pcbcraft_chrome::{catalog, shell};
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
    Place(u64),
    Close,
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
        let closing = shell::scrolling_dialog_width(ctx, "board_update", "Update PCB from schematic", 760.0, |ui| {
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
                        ui.label("This project has no PCB. Creating its first board is a separate undoable edit.");
                        if ui.add_enabled(source_current, catalog::button("Create board", true)).clicked() {
                            action = Action::Create;
                        }
                    }
                    Phase::Initialize => {
                        ui.label("Existing footprints have no recorded accepted state. Choosing “Use current state” records today's schematic, footprint choices and connections as its starting point; earlier accepted history is unavailable.");
                        let mut missing = false;
                        if let Some(project) = project.as_ref() {
                            let choices = Baseline::current_choices(project);
                            for component in project.components() {
                                if let Some(id) = component_number(&component.id())
                                    && project.component_is_board_eligible(id)
                                    && !choices.contains_key(&id)
                                {
                                    missing = true;
                                    let label = format!("Assign footprint to {}", component.reference());
                                    if ui.button(label).clicked() {
                                        action = Action::Assign(id);
                                    }
                                }
                            }
                        }
                        if ui.add_enabled(source_current && !missing, catalog::button("Use current state", true)).clicked() {
                            action = Action::Initialize;
                        }
                    }
                    Phase::Review => {
                        if let Some(preview) = preview.as_ref().filter(|_| current_plan) {
                            draw_preview(ui, preview, &mut action);
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
                        ui.label("PCB updated.");
                        if source_current {
                            if let Some(project) = project.as_ref() {
                                draw_unplaced(ui, project, &mut action);
                            }
                        } else {
                            ui.label("The design changed after this update. Review again before continuing.");
                        }
                    }
                }
                if (!source_current || (edit.phase == Phase::Review && !current_plan)) && ui.button("Review again").clicked() {
                    action = Action::Review;
                }
            }
            if let Some(error) = &edit.error {
                ui.label(shell::error(error));
            }
            ui.separator();
            ui.horizontal(|ui| {
                let apply = edit.phase == Phase::Review
                    && ui.push_id(edit.plan, |ui| ui.add_enabled(can_apply, catalog::button("Apply update", true)).clicked()).inner;
                if apply {
                    action = Action::Apply;
                }
                let close = if edit.phase == Phase::Applied || !context { "Close" } else { "Cancel" };
                if ui.button(close).clicked() {
                    action = Action::Close;
                }
            });
        });
        if closing || matches!(action, Action::Close) {
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
            Action::Assign(component) | Action::Place(component) => {
                self.cancel_board_update_review(&mut edit);
                if matches!(action, Action::Place(_))
                    && let Some(board) = edit.board
                    && let Some(view) = self.views.view_mut(edit.source.view())
                {
                    edit.phase = Phase::Applied;
                    view.open_document(DocumentTarget::Board(board), &self.session);
                    edit.document = DocumentTarget::Board(board);
                    if let Some(source) = view.input(&self.session) {
                        edit.source = source;
                    }
                }
                let previous = self.views.active.id();
                self.views.bind(edit.source.view());
                self.begin_footprint_assignment(Some(format!("component-{component}")));
                self.views.bind(previous);
                if let Dialog::Footprint(chooser) = &mut self.dialog {
                    chooser.update_review = Some(edit);
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
        self.dialog = Dialog::BoardUpdate(edit);
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
    preview.component(component).map_or_else(|| "Component".into(), |part| part.reference.clone())
}

/// Keep package identity and revision visible without exposing runtime capabilities or source hashes.
fn package_label(package: Option<&Package>) -> String {
    package.map_or_else(
        || "Unassigned".into(),
        |package| format!("{} / {} · r{}", package.library.as_deref().unwrap_or("Legacy marker"), package.definition, package.revision),
    )
}

/// Show an explicit mapping in terminal order without implying that pin text determines identity.
fn mapping_label(package: Option<&Package>) -> String {
    package.map_or_else(
        || "Unassigned".into(),
        |package| package.mapping.iter().map(|(terminal, pad)| format!("Pin {terminal} → pad {pad}")).collect::<Vec<_>>().join(", "),
    )
}

/// Present complete typed changes and blockers without creating a second mutable update plan.
fn draw_preview(ui: &mut egui::Ui, preview: &BoardUpdatePreview<'_>, action: &mut Action) {
    if preview.changes.is_noop() {
        ui.label("PCB is up to date.");
        return;
    }
    for Blocker::MissingPackage { component } in &preview.changes.blockers {
        if ui.button(format!("Assign footprint to {}", component_label(preview, *component))).clicked() {
            *action = Action::Assign(*component);
        }
    }
    for change in &preview.changes.components {
        let (component, kind) = match change {
            ComponentChange::Added { component } => (*component, "Add"),
            ComponentChange::Removed { component } => (*component, "Remove"),
            ComponentChange::Definition { component } => (*component, "Change symbol"),
            ComponentChange::Package { component } => (*component, "Replace footprint"),
            ComponentChange::Mapping { component } => (*component, "Change pin mapping"),
        };
        ui.collapsing(format!("{kind} {}", component_label(preview, component)), |ui| {
            let old = preview.accepted_component(component);
            let new = preview.proposed_component(component);
            ui.label(format!("Previous footprint: {}", old.map_or_else(|| "Not on PCB".into(), |part| package_label(part.package.as_ref()))));
            ui.label(format!("Proposed footprint: {}", new.map_or_else(|| "Removed".into(), |part| package_label(part.package.as_ref()))));
            ui.label(format!("Previous mapping: {}", old.map_or_else(|| "Not on PCB".into(), |part| mapping_label(part.package.as_ref()))));
            ui.label(format!("Proposed mapping: {}", new.map_or_else(|| "Removed".into(), |part| mapping_label(part.package.as_ref()))));
            if matches!(change, ComponentChange::Definition { .. }) {
                let symbol =
                    |part: &pcbcraft_engine::board::update::ComponentState| format!("{} / {} · r{}", part.library, part.definition, part.revision);
                ui.label(format!("Previous symbol: {}", old.map_or_else(|| "Not on PCB".into(), symbol)));
                ui.label(format!("Proposed symbol: {}", new.map_or_else(|| "Removed".into(), symbol)));
            }
        });
    }
    if !preview.changes.removed_nets.is_empty() {
        ui.strong("Previous connections");
        for net in &preview.changes.removed_nets {
            ui.label(net_label(preview, net));
        }
    }
    if !preview.changes.added_nets.is_empty() {
        ui.strong("Updated connections");
        for net in &preview.changes.added_nets {
            ui.label(net_label(preview, net));
        }
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
    ui.strong(format!("Unplaced footprints ({})", unplaced.len()));
    for assignment in unplaced {
        if let Some(component) = assignment["component"].as_str()
            && let Some(part) = project.components().find(|part| part.id() == component)
            && let Some(id) = component_number(component)
            && ui.button(format!("Place {}…", part.reference())).clicked()
        {
            *action = Action::Place(id);
        }
    }
}
