//! Physical clipboard presentation owns no schematic widgets or camera; the engine owns linkage.
use crate::{Dialog, PcbApp, ViewId, ViewInput, Workspace, editor_view::BoardState};
use egui::{Context, Event, Key, Modifiers, Ui};
use pcbcraft_chrome::{properties, shell, units};
use pcbcraft_engine::{
    Project,
    board::clipboard::{self as engine, BoardFragment, BoardPasteOptions, BoardPastePreview, BoardSelection, SchematicDestination},
    schematic::{GRID_NM, Point},
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Context actions route through the same host handlers as keyboard and ribbon actions.
#[derive(Clone, Copy)]
pub(super) enum Action {
    Copy,
    Cut,
    Paste,
    Options,
    Duplicate,
    Delete,
    SelectAll,
}

/// One view-local physical ghost, cancelled by ordinary input-authority changes.
#[derive(Debug, Default)]
pub(super) struct State {
    paste: Option<Placement>,
}
impl State {
    /// Whether placement keys belong to a copied physical group.
    pub(super) fn active(&self) -> bool {
        self.paste.is_some()
    }
}
#[derive(Clone, Debug)]
struct Placement {
    source: ViewInput,
    text: String,
    fragment: BoardFragment,
    options: BoardPasteOptions,
    preview: BoardPastePreview,
}
#[derive(Debug)]
struct Target {
    source: Uuid,
    name: String,
    sheet: String,
    x: String,
    y: String,
}
/// Explicit ownership and layer choices stay with their initiating view and revision.
#[derive(Debug)]
pub(super) struct Options {
    source: ViewInput,
    text: String,
    fragment: BoardFragment,
    options: BoardPasteOptions,
    targets: Vec<Target>,
    layers: BTreeMap<Uuid, String>,
    original: bool,
    error: Option<String>,
}

/// Collect editing selections, never correspondence highlights. Unknown copper identities remain
/// in the request so validation fails rather than silently omitting a stale selected object.
pub(super) fn selection(state: &BoardState, project: &Project) -> BoardSelection {
    let vias: BTreeSet<_> = state.target.and_then(|t| project.board_view(t)).into_iter().flat_map(|b| b.vias().iter().map(|v| v.id)).collect();
    BoardSelection {
        footprints: state.selection.clone(),
        drawings: state.drawing.selection(),
        tracks: state.tracks.selected.difference(&vias).copied().collect(),
        vias: state.tracks.selected.intersection(&vias).copied().collect(),
    }
}
/// A canvas tool or preview retains exclusive ownership of input and its source selection.
pub(super) fn busy(state: &BoardState) -> bool {
    state.clipboard.active() || authored_tool(state)
}
/// Tools, including the view-only ruler, supersede a clipboard preview before canvas input runs.
fn authored_tool(state: &BoardState) -> bool {
    state.move_requested
        || state.rotate_requested != 0
        || state.flip_requested
        || state.draft.is_some()
        || state.reference_drag.is_some()
        || state.placement.is_some()
        || state.box_drag.is_some()
        || state.drawing.tool.is_some()
        || state.drawing.source().is_some()
        || state.tracks.busy()
        || state.measure.busy()
}

impl PcbApp {
    /// Context actions are applied only after the canvas releases its borrowed view snapshot.
    pub(super) fn board_clipboard_action(&mut self, ctx: &Context, action: Action) {
        match action {
            Action::Copy => self.copy_board_clipboard(ctx, false),
            Action::Cut => self.copy_board_clipboard(ctx, true),
            Action::Paste => self.request_clipboard(ctx, false),
            Action::Options => self.request_clipboard(ctx, true),
            Action::Duplicate => self.duplicate_board_clipboard(),
            Action::Delete => self.delete_board_items(),
            Action::SelectAll => self.select_all_board(),
        }
    }

    /// Export only a valid physical selection, publishing OS text after successful atomic Cut.
    pub(super) fn copy_board_clipboard(&mut self, ctx: &Context, cut: bool) {
        if self.views.active.workspace != Workspace::Board
            || self.views.active.definition.is_some()
            || self.home
            || !matches!(self.dialog, Dialog::None)
            || self.project_action_pending()
            || self.files.busy()
            || busy(&self.views.active.board)
        {
            return;
        }
        let Some(source) = self.views.active.input(&self.session) else { return };
        let Some(project) = self.session.project() else { return };
        let Some(target) = self.views.active.board.target else { return };
        let items = selection(&self.views.active.board, project);
        if items.is_empty() {
            return;
        }
        match self
            .session
            .execute(if cut { engine::CUT } else { engine::COPY }, &json!({"board":target.board,"items":items,"expected_revision":source.revision}))
        {
            Ok(result) => {
                if let Some(text) = result["clipboard"].as_str() {
                    ctx.copy_text(text.into());
                }
                self.views.sync(&self.session);
                self.message = None;
                self.repaint_editors(ctx);
            }
            Err(error) => self.message = Some(error.to_string()),
        }
    }
    /// Duplicate uses the same options/placement path without changing the system clipboard.
    pub(super) fn duplicate_board_clipboard(&mut self) {
        if self.views.active.workspace != Workspace::Board
            || self.views.active.definition.is_some()
            || self.home
            || !matches!(self.dialog, Dialog::None)
            || self.project_action_pending()
            || self.files.busy()
            || busy(&self.views.active.board)
        {
            return;
        }
        let result = self.session.project().ok_or_else(|| "No board is open.".to_owned()).and_then(|p| {
            BoardFragment::capture(p, &selection(&self.views.active.board, p), false).and_then(|f| f.to_text()).map_err(|e| e.to_string())
        });
        match result {
            Ok(text) => self.receive_board_clipboard(text, false),
            Err(error) => self.message = Some(error),
        }
    }
    /// External coordinate conversion uses the same native placement path. Keeping coordinates
    /// commits only a successfully prepared preview; failures retain correction state.
    pub(super) fn receive_prepared_board_clipboard(&mut self, text: String, original: bool) {
        self.receive_board_clipboard(text, false);
        if original {
            if let Some(placement) = self.views.active.board.clipboard.paste.clone() {
                if let Err(error) = self.commit_board_clipboard(&placement, placement.fragment.anchor()) {
                    self.message = Some(error);
                }
            } else if let Dialog::BoardPasteOptions(draft) = &mut self.dialog {
                draft.original = true;
            }
        }
    }

    /// Decode once before addressing the destination. Duplication always exposes schematic
    /// ownership; a same-project Cut can default to component reuse without creating symbols.
    pub(super) fn receive_board_clipboard(&mut self, text: String, special: bool) {
        let Some(source) = self.views.active.input(&self.session) else { return };
        let Some(project) = self.session.snapshot() else { return };
        let fragment = match BoardFragment::from_text(&text) {
            Ok(fragment) => fragment,
            Err(error) => {
                self.message = Some(error.to_string());
                return;
            }
        };
        let reuse = !fragment.circuits().is_empty() && fragment.can_reuse_in(&project);
        let options = BoardPasteOptions { reuse_components: reuse, ..Default::default() };
        let show_options = special || !fragment.circuits().is_empty() && !reuse || fragment.layer_mapping(&project, &options.layers).is_err();
        self.views.active.cancel_gesture();
        if show_options {
            self.dialog = Dialog::BoardPasteOptions(Box::new(Options::new(source, text, fragment, options, &project, self.preferences.units)));
            self.message = None;
        } else {
            match self.prepare_board_clipboard(source, text.clone(), fragment.clone(), options.clone()) {
                Ok(placement) => {
                    self.views.active.board.clipboard.paste = Some(placement);
                    self.message = None;
                }
                Err(error) if reuse => {
                    let mut draft = Options::new(source, text, fragment, options, &project, self.preferences.units);
                    draft.error = Some(error);
                    self.dialog = Dialog::BoardPasteOptions(Box::new(draft));
                }
                Err(error) => self.message = Some(error),
            }
        }
    }

    /// Cache schematic remapping and package acceptance once; mouse motion transforms only the
    /// copied physical geometry. The final command performs all checks again.
    fn prepare_board_clipboard(
        &self,
        source: ViewInput,
        text: String,
        fragment: BoardFragment,
        options: BoardPasteOptions,
    ) -> Result<Placement, String> {
        if !self.views.active.accepts(source, &self.session) {
            return Err("The destination changed. Paste again.".into());
        }
        let project = self.session.project().ok_or("No board is open.")?;
        let preview = fragment.preview(project, self.session.revision().saturating_add(1), &options).map_err(|e| e.to_string())?;
        Ok(Placement { source, text, fragment, options, preview })
    }
    /// Keep the options form in the physical view's own native host.
    pub(super) fn board_paste_options_host(&self) -> Option<crate::editor_layout::HostId> {
        match &self.dialog {
            Dialog::BoardPasteOptions(draft) => self.views.owner(draft.source.view()),
            _ => None,
        }
    }
    /// Let the person explicitly choose logical counterparts and physical layers before a cursor
    /// preview. Invalid choices retain the form and never create a partial circuit.
    pub(super) fn board_paste_options_dialog(&mut self, ctx: &Context, mut draft: Box<Options>) {
        let Some(project) = self.session.snapshot() else { return };
        let sheets: Vec<_> = project.sheets().map(|s| (s.target.id().to_string(), s.name.to_owned())).collect();
        let destination_layers = self.views.active.board.target.and_then(|t| project.board_view(t)).map(|b| b.layers()).unwrap_or_default();
        let mut apply = false;
        let mut cancel = false;
        let dismissed = shell::scrolling_dialog_width(ctx, "board_paste_options", "PCB Paste Options", 520., |ui| {
            let (fp, drawings, tracks, vias) = draft.fragment.counts();
            ui.label(format!(
                "{fp} footprint{} · {drawings} graphic{} · {tracks} track{} · {vias} via{}",
                if fp == 1 { "" } else { "s" },
                if drawings == 1 { "" } else { "s" },
                if tracks == 1 { "" } else { "s" },
                if vias == 1 { "" } else { "s" }
            ));
            if !draft.fragment.circuits().is_empty() {
                let original = draft.fragment.source_project() == project.id();
                ui.add_enabled(original, egui::Checkbox::new(&mut draft.options.reuse_components, "Reuse original unplaced components"));
                if !draft.options.reuse_components {
                    ui.label(shell::dim("Duplication creates linked schematic components and physical placements in one edit. Matching net names can join existing nets within their scope; add a suffix to keep the copied circuit separate."));
                    ui.checkbox(&mut draft.options.items.preserve_references, "Keep original references");
                    properties::text(ui, "Net-name suffix", &mut draft.options.items.net_name_suffix);
                    for target in &mut draft.targets {
                        ui.push_id(target.source, |ui| {
                            properties::section(ui, &target.name);
                            let mut choices = vec![("".to_owned(), "Choose sheet…".to_owned())];
                            choices.extend(sheets.clone());
                            let choices: Vec<_> = choices.iter().map(|(id, name)| (id.as_str(), name.clone())).collect();
                            properties::choice(ui, "Sheet", ui.scope_id().with("sheet"), &mut target.sheet, &choices);
                            properties::text(ui, "Schematic X", &mut target.x);
                            properties::text(ui, "Schematic Y", &mut target.y);
                        });
                    }
                }
            }
            if !draft.fragment.layers().is_empty() {
                properties::section(ui, "Layers");
                for layer in draft.fragment.layers() {
                    let mut choices = vec![("".to_owned(), "Choose layer…".to_owned())];
                    choices.extend(destination_layers.iter().filter(|l| l.purpose == layer.purpose).map(|l| (l.id.to_string(), l.name.clone())));
                    let choices: Vec<_> = choices.iter().map(|(id, name)| (id.as_str(), name.clone())).collect();
                    properties::mapping_choice(ui, &layer.name, ui.scope_id().with(layer.id), draft.layers.entry(layer.id).or_default(), &choices);
                }
            }
            ui.checkbox(&mut draft.original, "Paste at original board position");
            if let Some(error) = &draft.error {
                ui.label(shell::error(error));
            }
            ui.horizontal(|ui| {
                apply = ui.button("Paste").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if apply {
            let result = draft
                .policy(self.preferences.units)
                .and_then(|options| self.prepare_board_clipboard(draft.source, draft.text.clone(), draft.fragment.clone(), options));
            match result {
                Ok(placement) => {
                    if draft.original {
                        if let Err(error) = self.commit_board_clipboard(&placement, placement.fragment.anchor()) {
                            draft.error = Some(error);
                            self.dialog = Dialog::BoardPasteOptions(draft);
                        }
                    } else {
                        self.views.active.board.clipboard.paste = Some(placement);
                    }
                }
                Err(error) => {
                    draft.error = Some(error);
                    self.dialog = Dialog::BoardPasteOptions(draft);
                }
            }
        } else if !(cancel || dismissed) {
            self.dialog = Dialog::BoardPasteOptions(draft);
        }
    }
    /// Expire a ghost before allocating canvas layout, leaving a newly chosen authored tool
    /// intact for the ordinary renderer. Text/modal/focus changes cannot consume placement keys.
    pub(super) fn board_clipboard_ready(&mut self, ctx: &Context, id: ViewId, enabled: bool) -> bool {
        let Some(view) = self.views.view_mut(id) else { return false };
        let Some(placement) = view.board.clipboard.paste.as_ref() else { return false };
        let valid = enabled
            && !authored_tool(&view.board)
            && view.accepts(placement.source, &self.session)
            && !ctx.egui_wants_keyboard_input()
            && !ctx.input(|i| i.events.iter().any(|event| matches!(event, Event::WindowFocused(false))));
        if !valid || ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            view.board.clipboard = State::default();
            return false;
        }
        true
    }

    /// Paint a disposable candidate through the independent PCB renderer; navigation remains local
    /// and only a primary placement click can request a real transaction. The host has already
    /// granted this ghost the current canvas input through board_clipboard_ready.
    /// Authored DRC markers do not apply to this disposable candidate.
    /// R and Shift+R rotate counterclockwise and clockwise on screen, including mirrored views.
    pub(super) fn board_clipboard_canvas(&mut self, ui: &mut Ui, id: ViewId, label: &str) {
        let Some(project) = self.session.snapshot() else { return };
        let Some(view) = self.views.view_mut(id) else { return };
        let Some(mut placement) = view.board.clipboard.paste.clone() else { return };
        let rotation = if ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::R)) {
            270
        } else if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::R)) {
            90
        } else {
            0
        };
        if rotation != 0 {
            let turns = (view.camera.model_rotation(rotation) / 90) as u8;
            placement.options.items.quarter_turns = (placement.options.items.quarter_turns + turns) % 4;
            view.board.clipboard.paste = Some(placement.clone());
        }
        let rect = ui.available_rect_before_wrap();
        let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p));
        let at = pointer.map(|p| view.camera.snap(p, rect)).unwrap_or_else(|| placement.fragment.anchor());
        let candidate = placement.preview.at(at, placement.options.items.quarter_turns);
        let error = candidate.as_ref().err().map(ToString::to_string);
        let candidate = candidate.unwrap_or(project);
        let Some(board) = view.board.target.and_then(|t| candidate.board_view(t)) else { return };
        let selected = placement.preview.selection();
        let mut state = BoardState {
            target: view.board.target,
            active_layer: view.board.active_layer.clone(),
            hidden_layers: view.board.hidden_layers.clone(),
            selection: selected.footprints.clone(),
            ..Default::default()
        };
        state.tracks.selected = selected.tracks.union(&selected.vias).copied().collect();
        state.drawing.select_many(selected.drawings.clone());
        for layer in placement.options.layers.values() {
            state.hidden_layers.remove(&layer.to_string());
        }
        if let Ok(mapping) = placement.fragment.layer_mapping(&candidate, &placement.options.layers) {
            for layer in mapping.values() {
                state.hidden_layers.remove(&layer.to_string());
            }
        }
        let _ = crate::board_canvas::show(
            ui,
            board,
            &mut view.camera,
            &mut state,
            None,
            &Default::default(),
            view.filters,
            &self.board_appearance,
            true,
            self.preferences.navigation,
            self.preferences.grid_opacity_percent,
            self.preferences.board_grid_style,
            label,
            &[],
            self.preferences.units,
        );
        if pointer.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            if ui.input(|i| i.pointer.secondary_clicked()) {
                self.views.active.board.clipboard = State::default();
            } else if ui.input(|i| i.pointer.primary_clicked()) {
                self.message = error.or_else(|| self.commit_board_clipboard(&placement, at).err());
            }
        }
    }
    /// Submit one revision-checked physical/logical transaction, then select the actual newly
    /// allocated objects. Disposable preview UUIDs are never assumed to be committed identities.
    fn commit_board_clipboard(&mut self, placement: &Placement, at: Point) -> Result<(), String> {
        if !self.views.active.accepts(placement.source, &self.session) {
            return Err("The destination changed. Paste again.".into());
        }
        let target = self.views.active.board.target.ok_or("Create a board before pasting.")?;
        let before = self.session.project().and_then(|p| p.board_view(target)).ok_or("Board unavailable.")?;
        let footprints: BTreeSet<_> = before.footprints().map(|f| f.id).collect();
        let drawings: BTreeSet<_> = before.drawings().iter().map(|d| d.id).collect();
        let tracks: BTreeSet<_> = before.tracks().iter().map(|t| t.id).collect();
        let vias: BTreeSet<_> = before.vias().iter().map(|v| v.id).collect();
        let o = &placement.options;
        self.session.apply(engine::PASTE, &json!({"board":target.board,"expected_revision":placement.source.revision,"text":placement.text,"x_nm":at.x_nm,"y_nm":at.y_nm,
            "reuse_components":o.reuse_components,"schematic_targets":o.schematic_targets,"layers":o.layers,"rotation":u16::from(o.items.quarter_turns)*90,
            "references":if o.items.preserve_references {"preserve"}else{"renumber"},"net_name_suffix":o.items.net_name_suffix})).map_err(|e| e.to_string())?;
        self.views.active.board.clipboard = State::default();
        self.views.sync(&self.session);
        let board = self.session.project().and_then(|p| p.board_view(target)).ok_or("Board unavailable.")?;
        let selected = BoardSelection {
            footprints: board.footprints().map(|f| f.id).filter(|id| !footprints.contains(id)).collect(),
            drawings: board.drawings().iter().map(|d| d.id).filter(|id| !drawings.contains(id)).collect(),
            tracks: board.tracks().iter().map(|t| t.id).filter(|id| !tracks.contains(id)).collect(),
            vias: board.vias().iter().map(|v| v.id).filter(|id| !vias.contains(id)).collect(),
        };
        if let Ok(mapping) = placement.fragment.layer_mapping(self.session.project().ok_or("Project unavailable.")?, &o.layers) {
            for layer in mapping.values() {
                self.views.active.board.hidden_layers.remove(&layer.to_string());
            }
        }
        self.select_board_items(selected);
        self.message = None;
        Ok(())
    }
    /// Apply one mixed editing selection after correspondence handling has cleared old categories.
    pub(super) fn select_board_items(&mut self, selected: BoardSelection) {
        self.views.select_board(selected.footprints, &self.session, self.preferences.selection_follow);
        self.views.active.board.drawing.select_many(selected.drawings);
        self.views.active.board.tracks.selected = selected.tracks.union(&selected.vias).copied().collect();
    }
    /// Ctrl/Command+A selects visible supported geometry, respecting this view's filters.
    pub(super) fn select_all_board(&mut self) {
        let Some(project) = self.session.project() else { return };
        let Some(board) = self.views.active.board.target.and_then(|t| project.board_view(t)) else { return };
        let selected = crate::board_canvas::all_visible(board, &self.views.active.board, self.views.active.filters, &self.board_appearance);
        self.select_board_items(selected);
    }

    /// Delete a complete mixed selection atomically; component/package identities stay in place.
    pub(super) fn delete_board_items(&mut self) {
        if self.views.active.workspace != Workspace::Board
            || self.views.active.definition.is_some()
            || self.home
            || !matches!(self.dialog, Dialog::None)
            || self.project_action_pending()
            || self.files.busy()
            || busy(&self.views.active.board)
        {
            return;
        }
        if self.hide_selected_reference() {
            return;
        }
        let Some(source) = self.views.active.input(&self.session) else { return };
        let Some(project) = self.session.project() else { return };
        let Some(target) = self.views.active.board.target else { return };
        let items = selection(&self.views.active.board, project);
        if items.is_empty() {
            return;
        }
        self.run(engine::DELETE, &json!({"board":target.board,"items":items,"expected_revision":source.revision}));
        if self.message.is_none() {
            self.select_board_items(BoardSelection::default());
        }
    }
}

impl Options {
    /// Propose typed choices without inferring sheet identity from its name or list order. A
    /// single destination sheet is unambiguous; several sheets require an explicit selection.
    fn new(source: ViewInput, text: String, fragment: BoardFragment, options: BoardPasteOptions, project: &Project, unit: units::Unit) -> Self {
        let sheets: Vec<_> = project.sheets().collect();
        let mut next_y = BTreeMap::new();
        let targets = fragment
            .circuits()
            .iter()
            .map(|c| {
                let sheet = if fragment.source_project() == project.id() { sheets.iter().find(|s| s.target.id() == c.source_sheet) } else { None }
                    .or_else(|| (sheets.len() == 1).then(|| &sheets[0]));
                let target = sheet.map(|s| s.target);
                let bounds = c.bounds();
                let base = target.and_then(|t| project.for_sheet(t)).map(|p| content_bottom(&p)).unwrap_or(0);
                let y = next_y.entry(target.map(|t| t.id())).or_insert(base.saturating_add(10 * GRID_NM));
                let at = Point {
                    x_nm: c.anchor().x_nm.saturating_sub(bounds[0].x_nm),
                    y_nm: y.saturating_add(c.anchor().y_nm.saturating_sub(bounds[0].y_nm)),
                };
                *y = y.saturating_add(bounds[1].y_nm.saturating_sub(bounds[0].y_nm)).saturating_add(10 * GRID_NM);
                Target {
                    source: c.source_sheet,
                    name: c.name.clone(),
                    sheet: target.map(|t| t.id().to_string()).unwrap_or_default(),
                    x: units::edit_label(at.x_nm, unit),
                    y: units::edit_label(at.y_nm, unit),
                }
            })
            .collect();
        let mapping = fragment.layer_mapping(project, &BTreeMap::new()).unwrap_or_default();
        let layers = fragment.layers().iter().map(|l| (l.id, mapping.get(&l.id).map(ToString::to_string).unwrap_or_default())).collect();
        Self { source, text, fragment, options, targets, layers, original: false, error: None }
    }
    /// Parse all form fields together, leaving the engine to validate chosen identities and scope.
    fn policy(&self, unit: units::Unit) -> Result<BoardPasteOptions, String> {
        let mut options = self.options.clone();
        options.schematic_targets.clear();
        if !options.reuse_components {
            for target in &self.targets {
                let sheet = target.sheet.parse().map_err(|_| format!("Choose a sheet for {}.", target.name))?;
                let at = Point {
                    x_nm: units::parse_length(&target.x, unit).map_err(|e| e.to_string())?,
                    y_nm: units::parse_length(&target.y, unit).map_err(|e| e.to_string())?,
                };
                options.schematic_targets.insert(target.source, SchematicDestination { sheet, at });
            }
        } else {
            options.items.net_name_suffix.clear();
            options.items.preserve_references = false;
        }
        options.layers = self
            .layers
            .iter()
            .map(|(source, dest)| dest.parse().map(|dest| (*source, dest)).map_err(|_| "Choose a destination for every copied layer.".into()))
            .collect::<Result<_, String>>()?;
        Ok(options)
    }
}
/// Model-only extents propose a gap below existing content, including hierarchical block bodies.
fn content_bottom(project: &Project) -> i64 {
    project
        .symbols()
        .iter()
        .flat_map(|s| std::iter::once(s.position).chain(s.pins()))
        .chain(project.wires().iter().flat_map(|w| project.wire_path(w)))
        .chain(project.net_labels().iter().map(|l| l.at))
        .chain(project.hierarchical_labels().iter().map(|l| l.at))
        .chain(project.sheet_symbols().iter().map(|s| Point { x_nm: s.at.x_nm, y_nm: s.at.y_nm.saturating_add(project.sheet_symbol_size(s).y_nm) }))
        .map(|p| p.y_nm)
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;

#[cfg(all(test, any(target_os = "linux", target_os = "windows")))]
impl PcbApp {
    /// Complete the already-presented, single-sheet native regression form through the same
    /// policy/preview/commit handlers exercised by canvas tests; no test mutates project fields.
    pub(super) fn finish_native_board_clipboard(&mut self) -> Result<bool, String> {
        let dialog = std::mem::take(&mut self.dialog);
        if let Dialog::BoardPasteOptions(draft) = dialog {
            let options = draft.policy(self.preferences.units)?;
            self.views.active.board.clipboard.paste = Some(self.prepare_board_clipboard(draft.source, draft.text, draft.fragment, options)?);
        } else {
            self.dialog = dialog;
        }
        let Some(placement) = self.views.active.board.clipboard.paste.clone() else { return Ok(false) };
        self.commit_board_clipboard(&placement, Point { x_nm: 12_700_000, y_nm: 12_700_000 })?;
        Ok(true)
    }
}
