//! Revision-bound board layer/stackup forms over the existing shared engine commands.
use crate::{Dialog, PcbApp, ViewId, ViewInput, Workspace};
use egui::{Context, Ui};
use pcbcraft_chrome::board_setup::{self as form, Page};
use pcbcraft_engine::board::materials::{Construction, Material, MaterialRef, PrepregPly};
use std::collections::BTreeSet;
mod materials;
pub(crate) mod profile_io;
mod profiles;
use pcbcraft_chrome::{properties, units};
use pcbcraft_engine::board::{
    setup::{BoardSetup, CopperRole, LayerPurpose, Side, Stratum, groups::UserLayerGroup},
    view::BoardTarget,
};
use serde_json::json;

/// Preserve exact stored nanometers when a rounded display string has not been edited.
#[derive(Debug)]
struct Length {
    original: Option<i64>,
    initial: String,
    text: String,
}
impl Length {
    /// Unknown thickness starts blank; it is never replaced with a guessed default or zero.
    fn new(value: Option<i64>, unit: units::Unit) -> Self {
        let text = value.map(|n| units::label(n, unit)).unwrap_or_default();
        Self { original: value, initial: text.clone(), text }
    }
    /// Blank clears a specified thickness; unchanged display text retains exact source precision.
    fn value(&self, unit: units::Unit) -> Result<Option<i64>, String> {
        if self.text == self.initial {
            return Ok(self.original);
        }
        if self.text.trim().is_empty() {
            return Ok(None);
        }
        units::parse_length(&self.text, unit).map(Some).map_err(|e| e.to_string())
    }
}

/// Ply text fields retain partial input and exact unedited dimensions for the lifetime of a draft.
#[derive(Debug)]
struct PlyFields {
    resin: String,
    nominal: Length,
}
impl PlyFields {
    /// Stock thickness remains independent of finished dielectric thickness.
    fn new(ply: &PrepregPly, unit: units::Unit) -> Self {
        Self {
            resin: ply.resin_content_ppm.map(|n| format!("{:.4}", n as f64 / 10_000.)).unwrap_or_default(),
            nominal: Length::new(ply.nominal_thickness_nm, unit),
        }
    }
}

/// A draft retains every material/construction field while exposing names and finished thickness.
#[derive(Debug)]
struct Candidate {
    setup: BoardSetup,
    names: Vec<String>,
    thickness: Vec<Length>,
    material_labels: Vec<String>,
    enabled: Vec<bool>,
    constructions: Vec<Option<Construction>>,
    roles: Vec<Option<CopperRole>>,
    groups: Vec<UserLayerGroup>,
    tolerance: Vec<Length>,
    materials: Vec<Material>,
    plies: std::collections::BTreeMap<String, PlyFields>,
}
impl Candidate {
    /// Seed fields in the current display unit without adopting any project changes.
    fn new(setup: BoardSetup, unit: units::Unit) -> Self {
        let names: Vec<String> = setup.layers().iter().map(|l| l.name.clone()).collect();
        let thickness = setup.stackup().strata.iter().map(|s| Length::new(s.thickness_nm(), unit)).collect();
        let material_labels = setup
            .stackup()
            .strata
            .iter()
            .map(|s| match s {
                Stratum::Dielectric { material, .. } => material.clone().unwrap_or_default(),
                _ => String::new(),
            })
            .collect();
        let enabled = vec![true; names.len()];
        let constructions = setup
            .stackup()
            .strata
            .iter()
            .map(|s| if let Stratum::Dielectric { construction, .. } = s { construction.clone() } else { None })
            .collect();
        let tolerance = setup
            .stackup()
            .strata
            .iter()
            .map(|s| Length::new(if let Stratum::Dielectric { thickness_tolerance_nm, .. } = s { *thickness_tolerance_nm } else { None }, unit))
            .collect();
        let materials = setup.materials().to_vec();
        let plies = setup
            .stackup()
            .strata
            .iter()
            .filter_map(|s| if let Stratum::Dielectric { construction: Some(Construction::Prepreg { plies }), .. } = s { Some(plies) } else { None })
            .flatten()
            .map(|ply| (ply.id.to_string(), PlyFields::new(ply, unit)))
            .collect();
        let roles = setup.stackup().strata.iter().map(|s| if let Stratum::Copper { role, .. } = s { *role } else { None }).collect();
        let groups = setup.stackup().layer_groups.clone();
        Self { setup, names, thickness, material_labels, enabled, constructions, roles, groups, tolerance, materials, plies }
    }
    /// Build and validate the complete candidate, preserving layer identities, pinned materials,
    /// construction, tolerances and fields not edited by this first graphical form.
    fn value(&self, unit: units::Unit) -> Result<BoardSetup, String> {
        let mut value = serde_json::to_value(&self.setup).map_err(|e| e.to_string())?;
        for (i, name) in self.names.iter().enumerate() {
            value["layers"][i]["name"] = json!(name);
        }
        let layers = value["layers"].as_array_mut().ok_or("Invalid layer draft")?;
        let mut index = 0;
        layers.retain(|_| {
            let keep = self.enabled[index];
            index += 1;
            keep
        });
        value["materials"] = json!(self.materials);
        value["stackup"]["layer_groups"] = json!(self.groups);
        for (i, length) in self.thickness.iter().enumerate() {
            let entry = value["stackup"]["strata"][i].as_object_mut().ok_or("Invalid stackup draft")?;
            match length.value(unit)? {
                Some(n) => {
                    entry.insert("thickness_nm".into(), json!(n));
                }
                None => {
                    entry.remove("thickness_nm");
                }
            }
            if matches!(self.setup.stackup().strata[i], Stratum::Copper { .. }) {
                if let Some(role) = self.roles[i] {
                    entry.insert("role".into(), json!(role));
                } else {
                    entry.remove("role");
                }
            }
            if matches!(self.setup.stackup().strata[i], Stratum::Dielectric { .. }) {
                match &self.constructions[i] {
                    Some(c) => {
                        let mut c = c.clone();
                        if let Construction::Prepreg { plies } = &mut c {
                            for ply in plies {
                                if let Some(fields) = self.plies.get(&ply.id.to_string()) {
                                    ply.nominal_thickness_nm = fields.nominal.value(unit)?;
                                    ply.resin_content_ppm = materials::decimal(&fields.resin, 4)?
                                        .map(u32::try_from)
                                        .transpose()
                                        .map_err(|_| "Resin content is out of range")?;
                                }
                            }
                        }
                        entry.insert("construction".into(), json!(c));
                    }
                    None => {
                        entry.remove("construction");
                    }
                }
                match self.tolerance[i].value(unit)? {
                    Some(n) => {
                        entry.insert("thickness_tolerance_nm".into(), json!(n));
                    }
                    None => {
                        entry.remove("thickness_tolerance_nm");
                    }
                }
                let material = &self.material_labels[i];
                if material.is_empty() {
                    entry.remove("material");
                } else {
                    entry.insert("material".into(), json!(material));
                }
            }
        }
        serde_json::from_value(value).map_err(|e| e.to_string())
    }
}

/// Captured board and view authority prevents an old form from editing a reopened project.
#[derive(Debug)]
pub(super) struct Edit {
    source: ViewInput,
    board: Option<BoardTarget>,
    unit: units::Unit,
    populated: bool,
    original_copper: u64,
    rules: Option<Box<crate::board_rules::Edit>>,
    classes: Option<Box<crate::net_class_editor::Edit>>,
    silkscreen: [Length; 3],
    candidate: Candidate,
    initializing: bool,
    page: Page,
    locked: BTreeSet<String>,
    target_thickness: String,
    selected_stratum: usize,
    material_edit: Option<materials::Edit>,
    profile_name: String,
    profile_ticket: Option<u64>,
    profile_notice: Option<String>,
    new_name: String,
    new_purpose: LayerPurpose,
    new_side: Option<Side>,
    error: Option<String>,
}

impl Edit {
    /// Keep the containing window bound to its captured view across every setup page.
    pub(super) fn source(&self) -> ViewInput {
        self.source
    }
    /// A setup Apply cannot close over unsaved content on another rule page.
    fn pending_rule_pages(&self) -> bool {
        self.rules.as_ref().is_some_and(|edit| edit.changed()) || self.classes.as_ref().is_some_and(|edit| edit.changed())
    }
}

/// Friendly purpose names are presentation only; command inputs retain typed layer roles.
fn purpose_name(purpose: LayerPurpose) -> &'static str {
    match purpose {
        LayerPurpose::Copper => "Copper",
        LayerPurpose::Mask => "Solder mask",
        LayerPurpose::Paste => "Solder paste",
        LayerPurpose::Silkscreen => "Silkscreen",
        LayerPurpose::Adhesive => "Adhesive",
        LayerPurpose::Fabrication => "Fabrication",
        LayerPurpose::Courtyard => "Courtyard",
        LayerPurpose::Outline => "Board Outline",
        LayerPurpose::Mechanical => "Mechanical",
        LayerPurpose::User => "User",
    }
}

mod pages;

impl PcbApp {
    /// Rule/setup dialogs stay in the window containing their captured source view.
    pub(super) fn rule_dialog_source(&self) -> Option<ViewInput> {
        match &self.dialog {
            Dialog::BoardSetup(edit) => Some(edit.source()),
            Dialog::BoardRules(edit) => Some(edit.source()),
            Dialog::Rules(edit) => Some(edit.source()),
            _ => None,
        }
    }
    /// Placement follows the owning view, independently of whichever window currently has focus.
    pub(super) fn rule_dialog_host(&self) -> Option<crate::editor_layout::HostId> {
        self.rule_dialog_source().and_then(|source| self.views.owner(source.view()))
    }
    /// Open an addressed board's disposable draft, or stage the first board from its project
    /// workspace. Neither path edits the session until Apply; a missing real target is refused.
    pub(super) fn begin_board_setup(&mut self, view: ViewId) {
        if !matches!(self.dialog, Dialog::None) || self.files.busy() || self.project_action_pending() {
            return;
        }
        let Some(editor) = self.views.view(view).filter(|v| v.workspace == Workspace::Board && v.definition.is_none()) else { return };
        let Some(source) = editor.input(&self.session) else { return };
        let Some(project) = self.session.project() else { return };
        let board = editor.board.target.and_then(|target| project.board_view(target));
        if board.is_none()
            && (project.board_target().is_some() || editor.target() != Some(crate::editor_view::DocumentTarget::BoardWorkspace(project.id())))
        {
            return;
        }
        let initializing = board.is_none_or(|board| board.setup().is_none());
        let setup = match board.map_or_else(|| BoardSetup::new_board_draft(2), |board| board.setup_draft(2)).and_then(|setup| {
            if initializing { setup.with_starting_stackup(pcbcraft_engine::board::setup::StackupPreset::Fr4OneOz) } else { Ok(setup) }
        }) {
            Ok(setup) => setup,
            Err(error) => {
                self.message = Some(error.to_string());
                return;
            }
        };
        let silkscreen = board.map_or_else(pcbcraft_engine::board::silkscreen::Settings::default, |board| board.silkscreen());
        self.dialog = Dialog::BoardSetup(Box::new(Edit {
            source,
            board: board.map(|board| board.target),
            unit: self.preferences.units,
            populated: board.is_some_and(|board| board.footprints().next().is_some()),
            original_copper: setup.stackup().strata.len().div_ceil(2) as u64,
            rules: None,
            classes: None,
            candidate: Candidate::new(setup, self.preferences.units),
            silkscreen: [silkscreen.text_height_nm, silkscreen.stroke_width_nm, silkscreen.pad_clearance_nm]
                .map(|n| Length::new(Some(n), self.preferences.units)),
            initializing,
            page: Page::Layers,
            locked: BTreeSet::new(),
            target_thickness: String::new(),
            selected_stratum: 1,
            material_edit: None,
            profile_name: "Board setup".into(),
            profile_ticket: None,
            profile_notice: None,
            new_name: String::new(),
            new_purpose: LayerPurpose::User,
            new_side: None,
            error: None,
        }));
    }

    /// Rebind retained drafts only after this setup host's own validated command. Never adopt
    /// an external edit or a changed view/target/epoch; retained fields remain inspectable on error.
    fn refresh_setup_rule_context(&self, edit: &mut Edit) -> bool {
        let previous = edit.source;
        let mut source = previous;
        source.revision = self.session.revision();
        if !self.views.view(source.view()).is_some_and(|view| view.accepts(source, &self.session)) {
            edit.error = Some("The originating view changed. Close and reopen Board setup.".into());
            return false;
        }
        let Some(project) = self.session.snapshot() else {
            return false;
        };
        edit.source = source;
        if let Some(rules) = &mut edit.rules
            && rules.source() == previous
        {
            rules.context_after_owned_command(source);
        }
        // The class command handler refreshes its own saved draft. If that refresh failed,
        // retain its old source rather than blessing an unrefreshed create/update as current.
        if edit.page != Page::NetClasses
            && let Some(classes) = &mut edit.classes
            && classes.source() == previous
        {
            classes.context_after_owned_command(source, project);
        }
        edit.error = None;
        true
    }

    /// Host physical setup and shared rule pages in one window with independent drafts.
    /// Each Save/Apply uses its existing command and retains other pending pages; external
    /// revisions expire the host. First-board setup publishes ownership and settings together;
    /// Cancel discards its draft without an edit. Existing boards retain independent rule saves.
    /// Only this form's pending profile job blocks publication; abandoned jobs retain their slot.
    pub(super) fn board_setup_dialog(&mut self, ctx: &Context, mut edit: Box<Edit>) {
        if self.views.view(edit.source.view()).is_none() {
            return;
        }
        let valid = self.views.view(edit.source.view()).is_some_and(|v| v.accepts(edit.source, &self.session));
        if !valid {
            edit.error = Some("The project or view changed. Close Board setup and reopen it to edit the current board.".into());
        }
        self.poll_setup_profile(&mut edit, valid);
        let can_apply = valid
            && edit.profile_ticket.is_none()
            && match edit.page {
                Page::BoardDefaults => {
                    edit.material_edit.is_none() && edit.rules.as_ref().is_some_and(|rules| rules.source() == edit.source && rules.can_apply())
                }
                Page::NetClasses => {
                    edit.material_edit.is_none()
                        && edit.classes.as_ref().is_some_and(|classes| classes.source() == edit.source && classes.can_apply())
                }
                _ => true,
            };
        let profile_busy = self.board_profile_files.busy();
        let mut profile_action = None;
        let apply_label = match edit.page {
            Page::BoardDefaults => "Apply board defaults",
            Page::NetClasses => edit.classes.as_ref().map_or("Save class", |classes| classes.primary()),
            _ if edit.material_edit.is_some() => "Save material to draft",
            _ => "Apply board setup",
        };
        let previous_page = edit.page;
        let mut class_actions = None;
        let error = edit.error.clone();
        let mut page = edit.page;
        let action = pcbcraft_chrome::catalog::settings_form(ctx, "Board setup", apply_label, can_apply, |ui| {
            form::pages(ui, &mut page, error.as_deref(), edit.board.is_some(), |ui, page| {
                if edit.initializing && edit.populated && !matches!(page, Page::BoardDefaults | Page::NetClasses) {
                    ui.label("Applying setup resolves existing surface-mount pads to front copper and through-hole pads across the stack. Placements and package assignments are retained.");
                }
                match page {
                    Page::Layers => edit.layers(ui),
                    Page::Stackup => edit.stackup(ui),
                    Page::Materials => edit.materials(ui),
                    Page::Silkscreen => edit.silkscreen(ui),
                    Page::Profiles => profile_action = edit.profiles(ui, profile_busy),
                    Page::BoardDefaults => {
                        if edit.rules.is_none() && valid {
                            edit.rules = self.session.snapshot().and_then(|project| {
                                edit.board
                                    .and_then(|target| project.board_view(target))
                                    .map(|board| Box::new(crate::board_rules::Edit::capture(edit.source, board, edit.unit)))
                            });
                        }
                        if let Some(rules) = &mut edit.rules {
                            rules.show(ui, valid);
                        }
                    }
                    Page::NetClasses => {
                        if edit.classes.is_none() && valid && edit.board.is_some() {
                            match self.capture_class_rules(edit.source, edit.unit) {
                                Ok(classes) => edit.classes = Some(Box::new(classes)),
                                Err(error) => edit.error = Some(error),
                            }
                        }
                        if let Some(classes) = &mut edit.classes {
                            class_actions = Some(classes.show(ui, valid));
                        }
                    }
                }
            });
        });
        if action.close {
            return;
        }
        edit.page = page;
        if previous_page != page && valid {
            edit.error = None;
        }
        if page == Page::NetClasses {
            if let (Some(classes), Some(actions)) = (&mut edit.classes, class_actions) {
                let defaults = actions.board_defaults;
                if self.apply_rule_actions(ctx, classes, actions, action.primary && can_apply && page == previous_page) {
                    self.refresh_setup_rule_context(&mut edit);
                }
                if defaults && valid {
                    edit.page = Page::BoardDefaults;
                    ctx.request_repaint();
                }
            }
            self.dialog = Dialog::BoardSetup(edit);
            return;
        }
        if page == Page::BoardDefaults {
            let Some(board) = edit.board else {
                self.dialog = Dialog::BoardSetup(edit);
                return;
            };
            if action.primary && can_apply && page == previous_page {
                let rules = edit.rules.as_ref().ok_or_else(|| "Board defaults are unavailable.".to_owned()).and_then(|rules| rules.rules());
                match rules {
                    Ok(rules) => {
                        if let Err(error) = self.apply_board_rule_values(edit.source, board, rules) {
                            edit.error = Some(error);
                        } else if self.refresh_setup_rule_context(&mut edit) {
                            edit.rules = self.session.snapshot().and_then(|project| {
                                project.board_view(board).map(|board| Box::new(crate::board_rules::Edit::capture(edit.source, board, edit.unit)))
                            });
                            self.repaint_editors(ctx);
                        }
                    }
                    Err(error) => edit.error = Some(error),
                }
            }
            self.dialog = Dialog::BoardSetup(edit);
            return;
        }
        if let Some(action) = profile_action {
            self.start_setup_profile(ctx, &mut edit, action);
        }
        let apply = action.primary && page == previous_page;
        if apply && valid && edit.profile_ticket.is_none() {
            if edit.material_edit.is_some() {
                edit.error = edit.save_material().err();
                self.dialog = Dialog::BoardSetup(edit);
                return;
            }
            let command = if edit.board.is_none() {
                pcbcraft_engine::board::identity::CREATE_BOARD
            } else if edit.initializing {
                pcbcraft_engine::INITIALIZE_BOARD_SETUP
            } else {
                pcbcraft_engine::SET_BOARD_SETUP
            };
            let result = edit.candidate.value(edit.unit).and_then(|setup| {
                let silkscreen = edit.silk_style()?;
                let setup = serde_json::to_value(setup).map_err(|e| e.to_string())?;
                Ok(json!({"setup":setup,"silkscreen":silkscreen}))
            });
            match result {
                Ok(mut params) => {
                    params["expected_revision"] = json!(edit.source.revision);
                    if let Some(board) = edit.board {
                        params["board"] = json!(board.board);
                        params["resolve_existing_footprints"] = json!(edit.populated);
                        if edit.initializing {
                            params["copper_layers"] = json!(edit.candidate.setup.stackup().strata.len().div_ceil(2));
                        }
                    }
                    self.run(command, &params);
                    if let Some(error) = self.message.take() {
                        edit.error = Some(error);
                    } else {
                        if edit.board.is_none() {
                            if let Some(target) = self.session.project().and_then(|project| project.board_target())
                                && let Some(view) = self.views.view_mut(edit.source.view())
                            {
                                view.open_document(crate::editor_view::DocumentTarget::Board(target), &self.session);
                            }
                            self.repaint_editors(ctx);
                            return;
                        }
                        self.repaint_editors(ctx);
                        if !edit.pending_rule_pages() {
                            return;
                        }
                        if self.refresh_setup_rule_context(&mut edit) {
                            // Keep the containing host open over remaining unsaved rule content.
                            if let Some(board) = self.session.project().and_then(|project| edit.board.and_then(|target| project.board_view(target)))
                                && let Some(setup) = board.setup()
                            {
                                edit.candidate = Candidate::new(setup.clone(), edit.unit);
                                edit.original_copper = setup.stackup().strata.len().div_ceil(2) as u64;
                                edit.silkscreen =
                                    [board.silkscreen().text_height_nm, board.silkscreen().stroke_width_nm, board.silkscreen().pad_clearance_nm]
                                        .map(|value| Length::new(Some(value), edit.unit));
                                edit.initializing = false;
                                edit.locked.clear();
                                edit.target_thickness.clear();
                            }
                        }
                    }
                }
                Err(error) => edit.error = Some(error),
            }
        }
        self.dialog = Dialog::BoardSetup(edit);
    }
}

#[cfg(test)]
mod tests;
