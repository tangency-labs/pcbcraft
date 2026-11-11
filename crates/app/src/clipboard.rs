//! Clipboard ownership and placement belong to the focused view; portable fragments belong to the engine.
use crate::{Dialog, PcbApp, ViewInput, Workspace};
use egui::{Context, Event, Key, Modifiers, Ui};
use pcbcraft_engine::{
    clipboard::{self as engine, Fragment, PasteOptions, SchematicPastePreview},
    schematic::Point,
};
use serde_json::json;

/// Pending platform reads and previews expire with the view's opening, focus and revision.
#[derive(Debug, Default)]
pub(super) struct State {
    paste: Option<Placement>,
}

impl State {
    /// A clipboard preview owns placement keys until committed or cancelled.
    pub(super) fn active(&self) -> bool {
        self.paste.is_some()
    }
    /// The status hint advertises rotation only for model-supported flat fragments.
    pub(super) fn can_rotate(&self) -> bool {
        self.paste.as_ref().is_some_and(|p| !p.fragment.has_hierarchy())
    }
}

#[derive(Clone, Debug)]
struct Placement {
    source: ViewInput,
    text: String,
    fragment: Fragment,
    options: PasteOptions,
    preview: Option<SchematicPastePreview>,
}

/// Options retain captured destination authority; changing focus cannot retarget Apply.
#[derive(Debug)]
pub(super) struct Options {
    placement: Placement,
    original: bool,
    error: Option<String>,
}

impl PcbApp {
    /// Capture a delayed menu read before any text widget can receive its response. A modal,
    /// new tool, stale opening or changed focus invalidates the original canvas authority.
    pub(super) fn pending_clipboard_response(&mut self, ctx: &Context) {
        if self.clipboard_request.is_none() {
            return;
        }
        let text = ctx.input_mut(|input| {
            let mut text = None;
            input.events.retain(|event| {
                match event {
                    Event::Paste(value) => {
                        text = Some(value.clone());
                        false
                    }
                    Event::PasteImage(_) => {
                        // Image-only reads complete the text request without retaining its
                        // captured destination or converting bitmap pixels into project data.
                        text = Some(String::new());
                        false
                    }
                    _ => true,
                }
            });
            text
        });
        let Some(text) = text else { return };
        let Some((source, special)) = self.clipboard_request.take() else { return };
        if !self.views.active.accepts(source, &self.session)
            || self.home
            || !ctx.input(|i| i.focused)
            || self.project_action_pending()
            || self.views.active.definition.is_some()
            || !matches!(self.dialog, Dialog::None)
            || self.parts.is_modal()
            || egui::Popup::is_any_open(ctx)
            || ctx.egui_wants_keyboard_input()
            || (self.views.active.workspace == Workspace::Board && crate::board_clipboard::busy(&self.views.active.board))
            || self.views.active.schematic.tool.is_some()
            || self.views.active.schematic.text_edit.is_some()
        {
            self.message = Some("The clipboard destination changed. Paste again in the current view.".into());
            return;
        }
        self.receive_clipboard(text, special);
    }

    /// Consume native Copy/Cut/Paste events only when the canvas owns input. Text editors and
    /// assistant fields keep their normal clipboard behavior; no app-local fallback masks OS data.
    /// The board's standalone-via chord takes precedence over the generic Cut shortcut.
    pub(super) fn clipboard_shortcuts(&mut self, ctx: &Context, assistant_keys: bool) -> bool {
        if self.home
            || assistant_keys
            || self.views.active.definition.is_some()
            || ctx.egui_wants_keyboard_input()
            || !matches!(self.dialog, Dialog::None)
            || egui::Popup::is_any_open(ctx)
            || self.parts.is_modal()
        {
            return false;
        }
        if self.views.active.workspace == Workspace::Board && ctx.input_mut(|input| input.consume_shortcut(&crate::PLACE_VIA)) {
            self.start_via();
            return true;
        }
        let events = ctx.input_mut(|input| {
            let mut clipboard = Vec::new();
            input.events.retain(|event| {
                if matches!(event, Event::PasteImage(_)) {
                    clipboard.push(Event::Paste(String::new()));
                    false
                } else if matches!(event, Event::Copy | Event::Cut | Event::Paste(_)) {
                    clipboard.push(event.clone());
                    false
                } else {
                    true
                }
            });
            clipboard
        });
        let mut handled = false;
        for event in events {
            handled = true;
            match event {
                Event::Copy => self.copy_clipboard(ctx, false),
                Event::Cut => self.copy_clipboard(ctx, true),
                Event::Paste(text) => {
                    let request = self.clipboard_request.take();
                    if let Some((source, _)) = request
                        && !self.views.active.accepts(source, &self.session)
                    {
                        self.message = Some("The clipboard destination changed. Paste again in the current view.".into());
                        continue;
                    }
                    let special = request.is_some_and(|(_, special)| special) || ctx.input(|i| i.modifiers.shift);
                    self.receive_clipboard(text, special);
                }
                _ => {}
            }
        }
        if handled {
            return true;
        }
        let action = ctx.input_mut(|i| {
            if i.consume_key(Modifiers::COMMAND, Key::C) {
                Some(0)
            } else if i.consume_key(Modifiers::COMMAND, Key::X) {
                Some(1)
            } else if i.consume_key(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::V) {
                Some(3)
            } else if i.consume_key(Modifiers::COMMAND, Key::V) {
                Some(2)
            } else if i.consume_key(Modifiers::COMMAND, Key::D) {
                Some(4)
            } else {
                None
            }
        });
        match action {
            Some(0) => self.copy_clipboard(ctx, false),
            Some(1) => self.copy_clipboard(ctx, true),
            Some(2 | 3) => self.request_clipboard(ctx, action == Some(3)),
            Some(4) => self.duplicate_clipboard(ctx),
            _ => {}
        }
        action.is_some()
    }

    /// Route portable payloads by the destination editor; neither editor depends on the other's widgets.
    pub(super) fn receive_clipboard(&mut self, text: String, special: bool) {
        if text.is_empty() {
            self.message = Some("The clipboard contains no text.".into());
            return;
        }
        let start = text.trim_start_matches('\u{feff}').trim_start();
        if !start.starts_with(['{', '[']) && !text.contains("pcbcraft-clipboard/") {
            self.begin_external_clipboard(text);
            return;
        }
        match self.views.active.workspace {
            Workspace::Schematic => self.receive_schematic_clipboard(text, special),
            Workspace::Board => self.receive_board_clipboard(text, special),
        }
    }
    /// Native platform events and menus share the same focused-editor copy/cut path.
    pub(super) fn copy_clipboard(&mut self, ctx: &Context, cut: bool) {
        match self.views.active.workspace {
            Workspace::Schematic => self.copy_schematic_clipboard(ctx, cut),
            Workspace::Board => self.copy_board_clipboard(ctx, cut),
        }
    }
    /// Duplication uses the current editor's fragment path without touching OS clipboard text.
    pub(super) fn duplicate_clipboard(&mut self, ctx: &Context) {
        match self.views.active.workspace {
            Workspace::Schematic => self.duplicate_schematic_clipboard_from(Some(ctx)),
            Workspace::Board => self.duplicate_board_clipboard(),
        }
    }

    /// Publish only a successfully exported fragment. Cut uses the same engine transaction as MCP;
    /// failed cuts leave both the existing OS clipboard output and project unchanged.
    pub(super) fn copy_schematic_clipboard(&mut self, ctx: &Context, cut: bool) {
        if self.views.active.schematic.clipboard.active() || self.views.active.schematic.tool.is_some() {
            return;
        }
        let Some(source) = self.views.active.input(&self.session) else {
            return;
        };
        let Some(project) = self.views.active.snapshot(&self.session) else {
            return;
        };
        let items = self.views.active.schematic.selection.clone();
        let objects = self.schematic_clipboard_objects(Some(ctx));
        if items.is_empty() && objects.is_empty() {
            return;
        }
        let params = json!({"items":items,"objects":objects,"sheet":project.sheet_target().id(),"expected_revision":source.revision});
        match self.session.execute(if cut { engine::CUT } else { engine::COPY }, &params) {
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

    /// Resolve hierarchy editing targets by stable IDs; a current explicit selection wins over
    /// hover, and ordinary flat selections never collect unrelated objects beneath the pointer.
    /// Hover requires the current pointer on the canvas background, excluding floating panels;
    /// the preceding frame's cached model point grants no targeting authority.
    fn schematic_clipboard_objects(&self, ctx: Option<&Context>) -> std::collections::BTreeSet<String> {
        if let Some(id) = self.views.active.schematic.selected_sheet_symbol {
            return [format!("sheet-symbol-{id}")].into();
        }
        if !self.views.active.schematic.selection.is_empty() {
            return Default::default();
        }
        let Some(ctx) = ctx else { return Default::default() };
        if !self.views.active.camera.schematic_rect().is_some_and(|rect| ctx.rect_contains_pointer(egui::LayerId::background(), rect)) {
            return Default::default();
        }
        let Some(project) = self.views.active.snapshot(&self.session) else { return Default::default() };
        crate::hierarchy::clipboard_hover(&project, &self.views.active.camera, ctx, self.views.active.filters).into_iter().collect()
    }

    /// Hierarchy context menus address their own hovered object, independently of an old flat
    /// selection. Copy/Cut publish portable OS text; Duplicate keeps the system clipboard intact.
    pub(super) fn hierarchy_clipboard_action(
        &mut self,
        ctx: &Context,
        kind: crate::hierarchy::Kind,
        id: u64,
        action: crate::schematic_canvas::ToolAction,
    ) {
        let Some(source) = self.views.active.input(&self.session) else { return };
        let Some(project) = self.views.active.snapshot(&self.session) else { return };
        let objects: std::collections::BTreeSet<_> =
            [format!("{}-{id}", if kind == crate::hierarchy::Kind::Sheet { "sheet-symbol" } else { "label" })].into();
        let items = pcbcraft_engine::selection::Selection::default();
        if matches!(action, crate::schematic_canvas::ToolAction::Duplicate) {
            match Fragment::capture_with_hierarchy(&project, &items, &objects, false).and_then(|f| f.to_text()) {
                Ok(text) => self.receive_schematic_clipboard(text, false),
                Err(e) => self.message = Some(e.to_string()),
            }
        } else if matches!(action, crate::schematic_canvas::ToolAction::Copy | crate::schematic_canvas::ToolAction::Cut) {
            let params = json!({"sheet":project.sheet_target().id(),"items":items,"objects":objects,"expected_revision":source.revision});
            match self.session.execute(if matches!(action, crate::schematic_canvas::ToolAction::Cut) { engine::CUT } else { engine::COPY }, &params) {
                Ok(result) => {
                    if let Some(text) = result["clipboard"].as_str() {
                        ctx.copy_text(text.into());
                    }
                    self.views.sync(&self.session);
                    self.message = None;
                }
                Err(error) => self.message = Some(error.to_string()),
            }
        }
    }

    /// Request the platform clipboard for menu actions. Native keyboard paste arrives directly
    /// as Event::Paste; the browser menu explains the keyboard gesture required by its host.
    pub(super) fn request_clipboard(&mut self, ctx: &Context, special: bool) {
        self.views.active.cancel_gesture();
        if let Some(source) = self.views.active.input(&self.session) {
            self.clipboard_request = Some((source, special));
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            ctx.request_repaint();
            if crate::IN_BROWSER {
                self.message = Some("Press Ctrl+V (⌘V on macOS) to paste the system clipboard.".into());
            }
        }
    }

    /// Duplicate shares the portable placement path while preserving the person's system clipboard.
    pub(super) fn duplicate_schematic_clipboard(&mut self) {
        self.duplicate_schematic_clipboard_from(None);
    }

    /// Keyboard duplication may target a hovered interface when no editing selection exists.
    fn duplicate_schematic_clipboard_from(&mut self, ctx: Option<&Context>) {
        if self.views.active.schematic.clipboard.active() || self.views.active.schematic.tool.is_some() {
            return;
        }
        let result = self.views.active.snapshot(&self.session).ok_or_else(|| "No schematic is open.".to_owned()).and_then(|project| {
            Fragment::capture_with_hierarchy(&project, &self.views.active.schematic.selection, &self.schematic_clipboard_objects(ctx), false)
                .and_then(|f| f.to_text())
                .map_err(|e| e.to_string())
        });
        match result {
            Ok(text) => self.receive_schematic_clipboard(text, false),
            Err(error) => self.message = Some(error),
        }
    }

    /// Decode clipboard text once, before entering a local preview; malformed data never creates
    /// a tool or modifies the project. Special paste shows reference and named-net choices;
    /// eligible Cut metadata opens the form but never enables live-sheet reuse automatically.
    fn receive_schematic_clipboard(&mut self, text: String, special: bool) {
        if text.is_empty() {
            self.message = Some("The clipboard contains no text.".into());
            return;
        }
        let Some(source) = self.views.active.input(&self.session) else {
            return;
        };
        match Fragment::from_text(&text) {
            Ok(fragment) => {
                let offers_reuse = self.views.active.snapshot(&self.session).is_some_and(|p| fragment.offers_sheet_reuse(&p));
                let options = PasteOptions::default();
                let mut placement = Placement { source, text, fragment, options, preview: None };
                self.views.active.cancel_gesture();
                if special || offers_reuse || placement.fragment.hierarchy_counts().1 > 0 {
                    self.dialog = Dialog::PasteOptions(Box::new(Options { placement, original: false, error: None }));
                    self.message = None;
                } else if let Err(error) = self.prepare_schematic_clipboard(&mut placement) {
                    if placement.fragment.has_hierarchy() {
                        self.dialog = Dialog::PasteOptions(Box::new(Options { placement, original: false, error: Some(error) }));
                    } else {
                        self.message = Some(error);
                    }
                } else {
                    self.views.active.schematic.clipboard.paste = Some(placement);
                    self.message = None;
                }
            }
            Err(error) => self.message = Some(error.to_string()),
        }
    }

    /// Cache logical/child-sheet work once. Cursor movement carries stable transient identities;
    /// the original text and captured revision still govern the authoritative commit.
    fn prepare_schematic_clipboard(&self, placement: &mut Placement) -> Result<(), String> {
        if !self.views.active.accepts(placement.source, &self.session) {
            return Err("The destination changed. Paste again.".into());
        }
        let project = self.views.active.snapshot(&self.session).ok_or("No schematic is open.")?;
        placement.preview =
            Some(placement.fragment.preview(&project, self.session.revision().saturating_add(1), &placement.options).map_err(|e| e.to_string())?);
        Ok(())
    }

    /// Paste forms stay in their initiating host so Apply validates the same view and a
    /// cursor preview never needs to cross an unrelated window's focus epoch.
    pub(super) fn paste_options_host(&self) -> Option<crate::editor_layout::HostId> {
        match &self.dialog {
            Dialog::PasteOptions(draft) => self.views.owner(draft.placement.source.view()),
            _ => None,
        }
    }

    /// Configure the captured batch without editing its source or destination. Original position
    /// applies once; ordinary paste enters a cursor preview and can still be cancelled with Escape.
    pub(super) fn paste_options_dialog(&mut self, ctx: &Context, mut draft: Box<Options>) {
        let mut apply = false;
        let mut cancel = false;
        let dismissed = pcbcraft_chrome::shell::dialog(ctx, "paste_options", "Paste Options", |ui| {
            let (symbols, wires, labels) = draft.placement.fragment.counts();
            ui.label(format!(
                "{symbols} symbol{} · {wires} wire{} · {labels} net label{}",
                if symbols == 1 { "" } else { "s" },
                if wires == 1 { "" } else { "s" },
                if labels == 1 { "" } else { "s" }
            ));
            let (blocks, ports, sheets) = draft.placement.fragment.hierarchy_counts();
            if blocks + ports + sheets > 0 {
                ui.label(format!(
                    "{blocks} sheet symbol{} · {ports} root interface{} · {sheets} child sheet{}",
                    if blocks == 1 { "" } else { "s" },
                    if ports == 1 { "" } else { "s" },
                    if sheets == 1 { "" } else { "s" }
                ));
            }
            if blocks > 0 {
                let eligible = self.views.active.snapshot(&self.session).is_some_and(|p| draft.placement.fragment.offers_sheet_reuse(&p));
                ui.add_enabled(eligible, egui::Checkbox::new(&mut draft.placement.options.reuse_sheets, "Reuse original unparented sheets"));
                if draft.placement.options.reuse_sheets {
                    draft.placement.options.net_name_suffix.clear();
                    ui.label(pcbcraft_chrome::shell::dim("Relocation retains the child sheets' current content, references and net names."));
                }
            }
            pcbcraft_chrome::controls::checkbox(ui, &mut draft.placement.options.preserve_references, "Keep original references");
            ui.add_enabled_ui(!draft.placement.options.reuse_sheets, |ui| {
                pcbcraft_chrome::properties::text(ui, "Net-name suffix", &mut draft.placement.options.net_name_suffix);
            });
            ui.label(pcbcraft_chrome::shell::dim(
                "Named nets retain their names and may join existing nets. Add a suffix to keep the copied circuit separate.",
            ));
            pcbcraft_chrome::controls::checkbox(ui, &mut draft.original, "Paste at original position");
            if let Some(error) = &draft.error {
                ui.label(pcbcraft_chrome::shell::error(error));
            }
            ui.horizontal(|ui| {
                apply = pcbcraft_chrome::controls::button(ui, "Paste").clicked();
                cancel = pcbcraft_chrome::controls::button(ui, "Cancel").clicked();
            });
        });
        if apply {
            if !self.views.active.accepts(draft.placement.source, &self.session) {
                draft.error = Some("The destination changed. Close this form and paste again.".into());
                self.dialog = Dialog::PasteOptions(draft);
                return;
            }
            if draft.original {
                let at = draft.placement.fragment.anchor();
                if let Err(error) = self.commit_schematic_clipboard(&draft.placement, at) {
                    draft.error = Some(error);
                    self.dialog = Dialog::PasteOptions(draft);
                }
            } else if let Err(error) = self.prepare_schematic_clipboard(&mut draft.placement) {
                draft.error = Some(error);
                self.dialog = Dialog::PasteOptions(draft);
            } else {
                self.views.active.schematic.clipboard.paste = Some(draft.placement);
            }
        } else if !(cancel || dismissed) {
            self.dialog = Dialog::PasteOptions(draft);
        }
    }

    /// Paint the actual candidate through the schematic renderer. Preview IDs are disposable;
    /// Escape/right-click, focus loss or a competing edit cannot publish them.
    pub(super) fn schematic_clipboard_canvas(&mut self, ui: &mut Ui, project: &pcbcraft_engine::Project, label: &str, input_enabled: bool) -> bool {
        let Some(mut placement) = self.views.active.schematic.clipboard.paste.clone() else {
            return false;
        };
        if !self.views.active.accepts(placement.source, &self.session) {
            self.views.active.schematic.clipboard.paste = None;
            return false;
        }
        if !input_enabled
            || self.views.active.schematic.tool.is_some()
            || ui.ctx().egui_wants_keyboard_input()
            || ui.input(|i| !i.focused || i.events.iter().any(|event| matches!(event, Event::WindowFocused(false))))
        {
            self.views.active.schematic.clipboard.paste = None;
            return false;
        }
        let rect = ui.available_rect_before_wrap();
        if input_enabled && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.views.active.schematic.clipboard.paste = None;
            return false;
        }
        if input_enabled && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::R)) {
            if placement.fragment.has_hierarchy() {
                self.message = Some("Hierarchical sheet symbols and interfaces retain their orientation.".into());
            } else {
                placement.options.quarter_turns = (placement.options.quarter_turns + 1) % 4;
                self.views.active.schematic.clipboard.paste = Some(placement.clone());
            }
        }
        let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|pos| rect.contains(*pos));
        let at = pointer.map(|pos| self.views.active.camera.snap(pos, rect)).unwrap_or_else(|| placement.fragment.anchor());
        let Some(preview) = &placement.preview else {
            self.views.active.schematic.clipboard.paste = None;
            return false;
        };
        let (candidate, selection, block, error) = match preview.at(at, placement.options.quarter_turns) {
            Ok(p) => (p, preview.selection().clone(), preview.sheet_symbol(), None),
            Err(error) => (std::sync::Arc::new(project.clone()), Default::default(), None, Some(error.to_string())),
        };
        crate::schematic_canvas::show(
            ui,
            &candidate,
            self.session.revision(),
            &mut self.views.active.camera,
            None,
            &selection,
            None,
            false,
            block,
            &Default::default(),
            self.views.active.filters,
            false,
            true,
            self.preferences.navigation,
            self.preferences.grid_opacity(false),
            false,
            false,
            self.preferences.units,
            &Default::default(),
            label,
            None,
        );
        if input_enabled && pointer.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            if ui.input(|i| i.pointer.secondary_clicked()) {
                self.views.active.schematic.clipboard.paste = None;
            } else if ui.input(|i| i.pointer.primary_clicked()) {
                if let Some(error) = error {
                    self.message = Some(error);
                } else if let Err(error) = self.commit_schematic_clipboard(&placement, at) {
                    self.message = Some(error);
                }
            }
        }
        true
    }

    /// Submit a captured revision and explicit sheet, then select only the newly created objects.
    /// The engine performs the authoritative whole-project validation and creates one history entry.
    fn commit_schematic_clipboard(&mut self, placement: &Placement, at: Point) -> Result<(), String> {
        if !self.views.active.accepts(placement.source, &self.session) {
            return Err("Paste destination changed; paste again.".into());
        }
        let project = self.views.active.snapshot(&self.session).ok_or("No schematic is open.")?;
        let before = pcbcraft_engine::selection::Selection {
            symbols: project.symbols().iter().map(|s| s.id()).collect(),
            wires: project.wires().iter().map(|w| w.id()).collect(),
            net_labels: project.net_labels().iter().map(|l| l.object_id()).collect(),
        };
        let blocks: std::collections::BTreeSet<_> = project.sheet_symbols().iter().map(|s| s.id).collect();
        let params = json!({"text":placement.text,"x_nm":at.x_nm,"y_nm":at.y_nm,"sheet":project.sheet_target().id(),
            "expected_revision":placement.source.revision,"reuse_sheets":placement.options.reuse_sheets,"rotation":u16::from(placement.options.quarter_turns)*90,
            "references":if placement.options.preserve_references {"preserve"}else{"renumber"},"net_name_suffix":placement.options.net_name_suffix});
        self.session.apply(engine::PASTE, &params).map_err(|e| e.to_string())?;
        self.views.active.schematic.clipboard = State::default();
        self.views.sync(&self.session);
        let project = self.views.active.snapshot(&self.session).ok_or("Schematic unavailable after paste.")?;
        let selected = pcbcraft_engine::selection::Selection {
            symbols: project.symbols().iter().map(|s| s.id()).filter(|id| !before.symbols.contains(id)).collect(),
            wires: project.wires().iter().map(|w| w.id()).filter(|id| !before.wires.contains(id)).collect(),
            net_labels: project.net_labels().iter().map(|l| l.object_id()).filter(|id| !before.net_labels.contains(id)).collect(),
        };
        let block = project.sheet_symbols().iter().find(|s| !blocks.contains(&s.id)).map(|s| s.id);
        let flat = selected.is_empty();
        self.select(selected);
        if flat {
            self.views.active.schematic.selected_sheet_symbol = block;
        }
        self.message = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, any(target_os = "linux", target_os = "windows")))]
mod native_tests;

#[cfg(test)]
mod hierarchy_tests;
