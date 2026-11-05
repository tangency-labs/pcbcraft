//! The PCBCraft window, the same on the desktop and in the browser.
//!
//! Every change to the project goes through [`Session::execute`], with commands from the engine's
//! catalog, the same ones MCP runs. Preferences belong to the person, not the project, so they live
//! here and stay out of the command set.

#[cfg(not(target_arch = "wasm32"))]
mod assistant;
mod home;
#[cfg(not(target_arch = "wasm32"))]
mod live_control;
mod net_inspector;
mod preferences;
mod project_files;
mod schematic_canvas;

use schematic_canvas::{Click, Tool};

use eframe::Frame;
use egui::{Key, KeyboardShortcut, Modifiers, RichText, Ui};
use pcbcraft_chrome::{
    MenuItem, Tokens, fonts,
    menu::{self, MenuIcon},
    shell::{self, dim},
    theme, workspace,
};
use pcbcraft_engine::{
    CONNECT_PINS, MOVE_SYMBOL, PLACE_SYMBOL, Session, command,
    persistence::Encoding,
    schematic::{BUILTIN_SYMBOLS, SymbolDefinition},
};
use serde_json::{Value, json};

pub use preferences::{Preferences, ThemeChoice};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const BUG_REPORT_URL: &str = "https://github.com/tangency-labs/pcbcraft/issues/new";
const PREFERENCES_KEY: &str = "pcbcraft.preferences";
/// App bar width below which About moves to the Home page.
const COMPACT_APP_BAR: f32 = 380.0;
#[cfg(not(target_arch = "wasm32"))]
const ASSISTANT_AGENT_KEY: &str = "pcbcraft.assistant.agent";
#[cfg(not(target_arch = "wasm32"))]
const ASSISTANT_LAYOUT_KEY: &str = "pcbcraft.assistant.layout";
#[cfg(not(target_arch = "wasm32"))]
const ASSISTANT_SETTINGS_KEY: &str = "pcbcraft.assistant.settings";

const COMMAND_Z: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
const COMMAND_Y: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
const COMMAND_O: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const COMMAND_S: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const SAVE_AS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
const COMMAND_N: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
const COMMAND_Q: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Q);
const ADD_SYMBOL: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::A);
const ADD_WIRE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::W);
const COMMAND_COMMA: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);

/// A browser keeps Ctrl+N and Ctrl+Q for itself, and has no window of ours to quit.
const IN_BROWSER: bool = cfg!(target_arch = "wasm32");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PreferencesPage {
    #[default]
    Appearance,
    #[cfg(not(target_arch = "wasm32"))]
    Ai,
}

#[derive(Debug, Default)]
enum Dialog {
    #[default]
    None,
    /// Asks for the name that `project.new` or `project.rename` takes.
    Name {
        command: &'static str,
        text: String,
        error: Option<String>,
        focused: bool,
    },
    ChooseSymbol,
    ConfirmOpen,
    SaveEncoding,
    #[cfg(not(target_arch = "wasm32"))]
    LocalMcp {
        error: Option<String>,
        copied: bool,
    },
    Preferences(PreferencesPage),
    #[cfg(not(target_arch = "wasm32"))]
    Agent {
        form: assistant::AgentForm,
        return_to_ai: bool,
    },
    About,
}

#[derive(Debug, Default)]
pub struct PcbApp {
    integrated_titlebar: bool,
    #[cfg(not(target_arch = "wasm32"))]
    assistant: assistant::Assistant,
    session: Session,
    #[cfg(not(target_arch = "wasm32"))]
    live_control: Option<live_control::LiveControl>,
    files: project_files::ProjectFiles,
    file_notice: Option<String>,
    camera: schematic_canvas::Camera,
    preferences: Preferences,
    net_inspector: net_inspector::NetInspector,
    dialog: Dialog,
    /// Whether the Home page replaces the ribbon and canvas. The project stays open behind it.
    home: bool,
    tool: Option<Tool>,
    /// Why the last command failed, until the next one runs.
    message: Option<String>,
    /// The palette and zoom last applied, so they are set only when they change.
    applied: Option<(bool, f32)>,
}

impl PcbApp {
    /// The app as the desktop and browser builds start it.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let preferences = cc.storage.and_then(|s| s.get_string(PREFERENCES_KEY)).map(|text| Preferences::parse(&text)).unwrap_or_default();
        Self {
            preferences,
            #[cfg(not(target_arch = "wasm32"))]
            assistant: assistant::Assistant::from_setting(
                cc.storage.and_then(|s| s.get_string(ASSISTANT_AGENT_KEY)).as_deref(),
                cc.storage.and_then(|s| s.get_string(ASSISTANT_LAYOUT_KEY)).as_deref(),
                cc.storage.and_then(|s| s.get_string(ASSISTANT_SETTINGS_KEY)).as_deref(),
            ),
            ..Self::default()
        }
    }

    /// Enable window controls in the app bar when the desktop launcher removes OS decorations.
    pub fn with_integrated_titlebar(mut self, enabled: bool) -> Self {
        self.integrated_titlebar = enabled;
        self
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn preferences(&self) -> &Preferences {
        &self.preferences
    }

    /// Run a command for the person at the keyboard. A failure shows in the status bar.
    fn run(&mut self, id: &str, params: &Value) {
        self.file_notice = None;
        self.message = self.session.apply(id, params).err().map(|e| e.to_string());
    }

    /// Start a symbol preview at the current project revision.
    fn begin_placement(&mut self, definition: &'static SymbolDefinition) {
        self.message = None;
        self.tool = Some(Tool::PlaceSymbol { definition, revision: self.session.revision() });
    }

    /// Wait for the first pin at the current project revision.
    fn begin_wire(&mut self) {
        self.message = None;
        self.tool = Some(Tool::Wire { revision: self.session.revision(), start: None });
    }

    fn ask_name(&mut self, command: &'static str) {
        let text = match command {
            "project.rename" => self.session.project().map(|p| p.name.clone()).unwrap_or_default(),
            _ => String::new(),
        };
        self.dialog = Dialog::Name { command, text, error: None, focused: false };
    }

    /// Protect unsaved edits before opening a file that replaces the current session.
    fn open_project_file(&mut self, ctx: &egui::Context) {
        if self.files.dirty(&self.session) {
            self.dialog = Dialog::ConfirmOpen;
        } else {
            self.message = self.files.open(self.session.revision(), ctx).err();
            self.file_notice = None;
        }
    }

    /// Start a save or download for the current snapshot and report setup failures.
    fn save_project_file(&mut self, encoding: Option<Encoding>, ctx: &egui::Context) {
        self.message = self.files.save(&self.session, encoding, ctx).err();
        self.file_notice = None;
    }

    /// Draw an engine command's catalog title and use the session to decide whether it is available.
    /// Return true on activation so the caller can open the command dialog.
    fn command_row(&self, ui: &mut Ui, id: &str, suffix: &str, shortcut: &str, icon: MenuIcon) -> bool {
        let title = command(id).map_or(id, |c| c.title);
        let label = format!("{title}{suffix}");
        MenuItem::new(&label).icon(icon).shortcut(shortcut).enabled(self.session.available(id)).show(ui)
    }

    /// Show the File commands and route them through the existing project dialogs and file jobs.
    /// Open and save actions are disabled while a file job is in flight. The browser offers
    /// Download, while desktop Save As opens the encoding chooser.
    fn file_menu(&mut self, ui: &mut Ui) {
        let new_shortcut = if IN_BROWSER { String::new() } else { ui.ctx().format_shortcut(&COMMAND_N) };
        if self.command_row(ui, "project.new", "…", &new_shortcut, MenuIcon::New) {
            self.ask_name("project.new");
        }
        let file_ready = !self.files.busy();
        let open_shortcut = if IN_BROWSER { String::new() } else { ui.ctx().format_shortcut(&COMMAND_O) };
        if MenuItem::new("Open…").icon(MenuIcon::Open).shortcut(&open_shortcut).enabled(file_ready).show(ui) {
            self.open_project_file(ui.ctx());
        }

        menu::separator(ui);
        let can_save = self.can_save_project();
        if IN_BROWSER {
            if MenuItem::new("Download Project…").icon(MenuIcon::Download).enabled(can_save).show(ui) {
                self.primary_save(ui.ctx());
            }
        } else {
            let save_shortcut = ui.ctx().format_shortcut(&COMMAND_S);
            if MenuItem::new("Save").icon(MenuIcon::Save).shortcut(&save_shortcut).enabled(can_save).show(ui) {
                self.primary_save(ui.ctx());
            }
            let save_as_shortcut = ui.ctx().format_shortcut(&SAVE_AS);
            if MenuItem::new("Save As…").icon(MenuIcon::Save).shortcut(&save_as_shortcut).enabled(can_save).show(ui) {
                self.dialog = Dialog::SaveEncoding;
            }
        }

        menu::separator(ui);
        if self.command_row(ui, "project.rename", "…", "", MenuIcon::Rename) {
            self.ask_name("project.rename");
        }
        #[cfg(not(target_arch = "wasm32"))]
        if MenuItem::new("Local MCP…").icon(MenuIcon::Settings).show(ui) {
            self.dialog = Dialog::LocalMcp { error: None, copied: false };
        }
        if !IN_BROWSER {
            let quit_shortcut = ui.ctx().format_shortcut(&COMMAND_Q);
            if MenuItem::new("Quit").icon(MenuIcon::Close).shortcut(&quit_shortcut).show(ui) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Whether the current project can start a save or download.
    fn can_save_project(&self) -> bool {
        !self.files.busy() && self.session.project().is_some()
    }

    /// Save the current desktop file or choose the encoding for a browser download.
    fn primary_save(&mut self, ctx: &egui::Context) {
        if IN_BROWSER {
            self.dialog = Dialog::SaveEncoding;
        } else {
            self.save_project_file(None, ctx);
        }
    }

    /// Show the project's title and unsaved state beside the Home, file and history actions.
    /// Undo and Redo dispatch through the same engine commands as their keyboard shortcuts.
    /// The version and bug report are shown when the app bar has space, and About from 380 points.
    fn app_bar(&mut self, ui: &mut Ui) {
        let project = self.session.project().map(|project| (project.name.clone(), self.files.dirty(&self.session)));
        let title = project.as_ref().map(|(name, dirty)| (name.as_str(), *dirty));
        let mut preferences = false;
        let mut about = false;
        let mut home = false;
        #[cfg(not(target_arch = "wasm32"))]
        let mut assistant_clicked = false;
        #[cfg(not(target_arch = "wasm32"))]
        let (assistant_open, assistant_ready) = self.assistant.toolbar_state();
        #[cfg(not(target_arch = "wasm32"))]
        let assistant_open = assistant_open && !self.home;
        #[cfg(not(target_arch = "wasm32"))]
        let assistant_visible = self.assistant.toolbar_visible();
        shell::app_bar(
            ui,
            title,
            self.integrated_titlebar,
            |ui| home = menu::quick_action(ui, MenuIcon::Home, "Home", "", true).clicked(),
            |ui| {
                menu::file(ui, |ui| self.file_menu(ui));
                let (save_icon, save_label, save_key) = if IN_BROWSER {
                    (MenuIcon::Download, "Download Project", String::new())
                } else {
                    (MenuIcon::Save, "Save Project", ui.ctx().format_shortcut(&COMMAND_S))
                };
                if menu::quick_action(ui, save_icon, save_label, &save_key, self.can_save_project()).clicked() {
                    self.primary_save(ui.ctx());
                }
                for (command, icon, label, key) in
                    [("edit.undo", MenuIcon::Undo, "Undo", COMMAND_Z), ("edit.redo", MenuIcon::Redo, "Redo", COMMAND_Y)]
                {
                    let shortcut = ui.ctx().format_shortcut(&key);
                    if menu::quick_action(ui, icon, label, &shortcut, self.session.available(command)).clicked() {
                        self.run(command, &Value::Null);
                    }
                }
            },
            |ui, width| {
                if width >= 560.0 {
                    ui.label(dim(format!("v{VERSION}")));
                    ui.add_space(Tokens::get().space_8);
                    if menu::labeled_action(ui, MenuIcon::Bug, "Report a bug").clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(BUG_REPORT_URL));
                    }
                }
                // Narrow bars leave About to the Home page so Home fits beside the history actions.
                if width >= COMPACT_APP_BAR {
                    about = menu::quick_action(ui, MenuIcon::Help, "About PCBCraft", "", true).clicked();
                }
                let shortcut = ui.ctx().format_shortcut(&COMMAND_COMMA);
                preferences = menu::quick_action(ui, MenuIcon::Settings, "Preferences", &shortcut, true).clicked();
                #[cfg(not(target_arch = "wasm32"))]
                if assistant_visible {
                    let shortcut = ui.ctx().format_shortcut(&assistant::TOGGLE);
                    assistant_clicked = pcbcraft_chrome::assistant::toolbar(ui, assistant_open, assistant_ready, &shortcut);
                }
            },
        );
        #[cfg(not(target_arch = "wasm32"))]
        if assistant_clicked {
            self.toggle_assistant(ui.ctx());
        }
        if preferences {
            self.dialog = Dialog::Preferences(PreferencesPage::default());
        } else if about {
            self.dialog = Dialog::About;
        } else if home {
            self.show_home();
        }
    }

    /// Open the Assistant from Home in the workspace. In the workspace, toggle its visibility.
    #[cfg(not(target_arch = "wasm32"))]
    fn toggle_assistant(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.home) {
            self.assistant.open(ctx);
        } else {
            self.assistant.toggle(ctx);
        }
    }

    /// Route the Assistant shortcut globally and give its chat first claim on editing keys.
    #[cfg(not(target_arch = "wasm32"))]
    fn assistant_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if !matches!(self.dialog, Dialog::None) || !self.assistant.enabled() {
            return false;
        }
        if ctx.input_mut(|input| input.consume_shortcut(&assistant::TOGGLE)) {
            self.toggle_assistant(ctx);
            return true;
        }
        !self.home && self.assistant.shortcuts(ctx)
    }

    /// Show placement, wiring, inspection, and fit actions in the Capture ribbon.
    fn ribbon(&mut self, ui: &mut Ui) {
        use workspace::ToolIcon;

        let has_project = self.session.project().is_some();
        workspace::ribbon(ui, "Schematic", "Capture", |ui| {
            workspace::ribbon_group(ui, "Place", 1, |ui| {
                let placing = matches!(self.tool, Some(Tool::PlaceSymbol { .. }));
                if workspace::tool(ui, ToolIcon::Symbol, "Symbol", "Add symbol (A)", self.session.available(PLACE_SYMBOL), placing).clicked() {
                    self.dialog = Dialog::ChooseSymbol;
                }
            });
            ui.separator();
            workspace::ribbon_group(ui, "Connect", 1, |ui| {
                let wiring = matches!(self.tool, Some(Tool::Wire { .. }));
                if workspace::tool(ui, ToolIcon::Wire, "Wire", "Draw wire (W)", self.session.available(CONNECT_PINS), wiring).clicked() {
                    self.begin_wire();
                }
            });
            ui.separator();
            workspace::ribbon_group(ui, "View", 2, |ui| {
                if workspace::tool(ui, ToolIcon::Nets, "Nets", "Toggle Net Inspector", has_project, has_project && self.net_inspector.open).clicked()
                {
                    self.net_inspector.open = !self.net_inspector.open;
                }
                if workspace::tool(ui, ToolIcon::Fit, "Fit", "Fit schematic", has_project, false).clicked() {
                    self.camera.request_fit();
                }
            });
        });
    }

    /// Show the command error or placement hint alongside the project name.
    fn status(&self, ui: &mut Ui) {
        let name = self.session.project().map_or("No project open", |p| p.name.as_str()).to_owned();
        let message = self.message.clone();
        shell::status_bar(
            ui,
            |ui| {
                ui.label(dim(name));
                if let Some(message) = message {
                    ui.label(shell::error(message));
                } else if let Some(tool) = self.tool {
                    ui.label(match tool {
                        Tool::PlaceSymbol { definition, .. } => dim(format!("Place {} · Esc to cancel", definition.name)),
                        Tool::Move { .. } => dim("Move symbol · Esc to cancel"),
                        Tool::Wire { start: None, .. } => dim("Wire · Click a pin"),
                        Tool::Wire { start: Some(_), .. } => dim("Wire · Click the destination pin"),
                    });
                } else if self.files.busy() {
                    ui.label(dim("File operation in progress"));
                } else if let Some(notice) = &self.file_notice {
                    ui.label(dim(notice.as_str()));
                }
            },
            |_ui| {
                #[cfg(not(target_arch = "wasm32"))]
                if self.live_control.is_some() {
                    _ui.label(dim("Local MCP enabled"));
                }
            },
        );
    }

    /// Draw the schematic and send completed edits through the shared command catalog. Cancel only clears the local preview.
    fn canvas(&mut self, ui: &mut Ui) {
        let input_enabled = matches!(self.dialog, Dialog::None) && !egui::Popup::is_any_open(ui.ctx());
        let click = shell::canvas(ui, |ui| {
            if let Some(project) = self.session.project() {
                return schematic_canvas::show(ui, project, &mut self.camera, self.tool, input_enabled);
            }
            let t = Tokens::get();
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("No project open. Choose File, then New Project.").size(t.font_button).color(t.canvas_ink));
            });
            None
        });
        match (self.tool, click) {
            (Some(Tool::PlaceSymbol { definition, revision }), Some(Click::Place(point))) => {
                self.tool = None;
                self.run(PLACE_SYMBOL, &json!({"definition": definition.id, "x_nm": point.x_nm, "y_nm": point.y_nm, "expected_revision": revision}));
            }
            (Some(Tool::Wire { revision, start: None }), Some(Click::Pin(pin))) => {
                self.tool = Some(Tool::Wire { revision, start: Some(pin) });
            }
            (Some(Tool::Wire { revision, start: Some(from) }), Some(Click::Pin(to))) if from != to => {
                self.run(CONNECT_PINS, &json!({"from": from.inspect(), "to": to.inspect(), "expected_revision": revision}));
                if self.message.is_none() {
                    self.tool = None;
                }
            }
            (None, Some(Click::Grab(symbol, offset))) => {
                self.tool = Some(Tool::Move { symbol, offset, revision: self.session.revision() });
            }
            (Some(Tool::Move { symbol, revision, .. }), Some(Click::FinishMove(position))) => {
                self.tool = None;
                if let Some(point) = position {
                    self.run(
                        MOVE_SYMBOL,
                        &json!({
                            "symbol": symbol.id(), "x_nm": point.x_nm, "y_nm": point.y_nm, "expected_revision": revision,
                        }),
                    );
                }
            }
            (Some(Tool::Wire { .. }), Some(Click::CancelWire)) => {
                self.tool = None;
                self.message = None;
            }
            _ => {}
        }
    }

    /// Handle editor keys outside text fields and dialogs. Placement keys apply only while menus are closed.
    /// On Home, Escape returns to the workspace and the canvas keys do nothing.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || !matches!(self.dialog, Dialog::None) {
            return;
        }
        let pressed = |shortcut: &KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(shortcut));
        if self.home {
            if !egui::Popup::is_any_open(ctx) && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                self.home = false;
            }
        } else if !egui::Popup::is_any_open(ctx) {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) && self.tool.take().is_some() {
                self.message = None;
            }
            if pressed(&ADD_SYMBOL) && self.session.available(PLACE_SYMBOL) {
                self.dialog = Dialog::ChooseSymbol;
            }
            if pressed(&ADD_WIRE) && self.session.available(CONNECT_PINS) {
                self.begin_wire();
            }
        }
        if pressed(&COMMAND_Z) && self.session.available("edit.undo") {
            self.run("edit.undo", &Value::Null);
        }
        if pressed(&COMMAND_Y) && self.session.available("edit.redo") {
            self.run("edit.redo", &Value::Null);
        }
        if pressed(&COMMAND_COMMA) {
            self.dialog = Dialog::Preferences(PreferencesPage::default());
        }
        if !IN_BROWSER {
            if !self.files.busy() {
                if pressed(&COMMAND_O) {
                    self.open_project_file(ctx);
                }
                if self.session.project().is_some() {
                    if pressed(&SAVE_AS) {
                        self.dialog = Dialog::SaveEncoding;
                    } else if pressed(&COMMAND_S) {
                        self.save_project_file(None, ctx);
                    }
                }
            }
            if pressed(&COMMAND_N) {
                self.ask_name("project.new");
            }
            if pressed(&COMMAND_Q) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Draw the open dialog and retain it until the person finishes or dismisses it.
    fn dialogs(&mut self, ctx: &egui::Context) {
        match std::mem::take(&mut self.dialog) {
            Dialog::None => {}
            Dialog::Name { command: id, mut text, mut error, mut focused } => {
                let title = command(id).map_or(id, |c| c.title);
                let (mut submit, mut cancel) = (false, false);
                let closing = shell::dialog(ctx, "name_dialog", title, |ui| {
                    ui.label("Name");
                    let field = ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY));
                    if !focused {
                        field.request_focus();
                        focused = true;
                    }
                    submit = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    if let Some(error) = &error {
                        ui.label(shell::error(error.as_str()));
                    }
                    ui.horizontal(|ui| {
                        submit |= ui.button("OK").clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
                if submit {
                    // A bad name stays in the dialog to fix, so the person sees why.
                    match self.session.apply(id, &json!({"name": text})) {
                        Ok(_) => {
                            self.file_notice = None;
                            self.message = None;
                            if id == "project.new" {
                                self.camera = schematic_canvas::Camera::default();
                                self.home = false;
                            }
                        }
                        Err(e) => {
                            error = Some(e.to_string());
                            self.dialog = Dialog::Name { command: id, text, error, focused };
                        }
                    }
                } else if !(closing || cancel) {
                    self.dialog = Dialog::Name { command: id, text, error, focused };
                }
            }
            Dialog::ConfirmOpen => {
                let (mut proceed, mut cancel) = (false, false);
                let closing = shell::dialog(ctx, "confirm_open", "Open Project", |ui| {
                    ui.label("Opening another project will discard unsaved changes.");
                    ui.horizontal(|ui| {
                        proceed = ui.button("Discard and open…").clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
                if proceed {
                    self.message = self.files.open(self.session.revision(), ctx).err();
                    self.file_notice = None;
                } else if !(cancel || closing) {
                    self.dialog = Dialog::ConfirmOpen;
                }
            }
            Dialog::SaveEncoding => {
                let mut selected = None;
                let mut cancel = false;
                let title = if IN_BROWSER { "Download Project" } else { "Save Project As" };
                let closing = shell::dialog(ctx, "save_encoding", title, |ui| {
                    ui.label("File format");
                    ui.horizontal(|ui| {
                        if ui.button("Plain JSON").clicked() {
                            selected = Some(Encoding::Json);
                        }
                        if ui.button("ZIP archive").clicked() {
                            selected = Some(Encoding::Zip);
                        }
                        cancel = ui.button("Cancel").clicked();
                    });
                });
                if let Some(encoding) = selected {
                    self.save_project_file(Some(encoding), ctx);
                } else if !(closing || cancel) {
                    self.dialog = Dialog::SaveEncoding;
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Dialog::LocalMcp { error, copied } => self.local_mcp_dialog(ctx, error, copied),
            Dialog::ChooseSymbol => {
                let mut selected = None;
                let mut cancel = false;
                let closing = shell::dialog(ctx, "symbol_picker", "Place Symbol", |ui| {
                    let row = [ui.available_width(), Tokens::get().control_height];
                    for &definition in BUILTIN_SYMBOLS {
                        if ui.add_sized(row, egui::Button::new(definition.name)).clicked() {
                            selected = Some(definition);
                        }
                    }
                    cancel = ui.button("Cancel").clicked();
                });
                if let Some(definition) = selected {
                    self.begin_placement(definition);
                } else if !(closing || cancel) {
                    self.dialog = Dialog::ChooseSymbol;
                }
            }
            Dialog::Preferences(page) => {
                #[cfg(not(target_arch = "wasm32"))]
                let mut page = page;
                let mut close = false;
                #[cfg(not(target_arch = "wasm32"))]
                let mut request = None;
                let closing = shell::dialog_width(ctx, "preferences_dialog", "Preferences", 490.0, |ui| {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut page, PreferencesPage::Appearance, "Appearance");
                            ui.selectable_value(&mut page, PreferencesPage::Ai, "AI");
                        });
                        ui.separator();
                    }
                    let height = (ui.ctx().content_rect().height() - 140.0).clamp(120.0, 510.0);
                    egui::ScrollArea::vertical()
                        .id_salt("preferences_page")
                        .max_height(height)
                        .min_scrolled_height(height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| match page {
                            PreferencesPage::Appearance => self.preferences.show(ui),
                            #[cfg(not(target_arch = "wasm32"))]
                            PreferencesPage::Ai => request = self.assistant.settings_page(ui),
                        });
                    close = ui.button("Close").clicked();
                });
                if closing || close {
                    ctx.request_repaint();
                } else {
                    self.dialog = Dialog::Preferences(page);
                }
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(assistant::SettingsRequest::Add(form)) = request {
                    self.dialog = Dialog::Agent { form, return_to_ai: true };
                    ctx.request_repaint();
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Dialog::Agent { mut form, return_to_ai } => {
                if self.assistant.agent_dialog(ctx, &mut form) {
                    if return_to_ai {
                        self.dialog = Dialog::Preferences(PreferencesPage::Ai);
                    }
                    ctx.request_repaint();
                } else {
                    self.dialog = Dialog::Agent { form, return_to_ai };
                }
            }
            Dialog::About => {
                let mut close = false;
                let closing = shell::dialog(ctx, "about_dialog", "About PCBCraft", |ui| {
                    ui.label(format!("PCBCraft {VERSION}"));
                    ui.label("Electronics design that feels like KiCad, built for people and agents to work on the same board.");
                    ui.label(dim("MIT OR Apache-2.0. Third-party notices are in ATTRIBUTION.md."));
                    ui.hyperlink_to("Report a bug", BUG_REPORT_URL);
                    close = ui.button("Close").clicked();
                });
                if !(closing || close) {
                    self.dialog = Dialog::About;
                }
            }
        }
    }

    fn sync_style(&mut self, ctx: &egui::Context) {
        let dark = self.preferences.is_dark(ctx.system_theme());
        let zoom = self.preferences.zoom();
        if self.applied.is_none() {
            fonts::install(ctx);
        }
        if self.applied != Some((dark, zoom)) {
            theme::apply(ctx, dark);
            ctx.set_zoom_factor(zoom);
            self.applied = Some((dark, zoom));
        }
    }

    /// Enable or revoke a connection for this window and copy its client configuration on request.
    #[cfg(not(target_arch = "wasm32"))]
    fn local_mcp_dialog(&mut self, ctx: &egui::Context, mut error: Option<String>, mut copied: bool) {
        let mut close = false;
        let dismissed = shell::dialog(ctx, "local_mcp", "Local MCP", |ui| {
            if let Some(control) = &self.live_control {
                ui.label("Clients with this configuration can inspect this window and move symbols.");
                let disable = ui
                    .horizontal(|ui| {
                        if ui.button(if copied { "Configuration copied" } else { "Copy configuration" }).clicked() {
                            ctx.copy_text(control.client_configuration());
                            copied = true;
                        }
                        ui.button("Disable connection").clicked()
                    })
                    .inner;
                if disable {
                    self.live_control = None;
                    copied = false;
                }
            } else {
                ui.label("Let a local MCP client inspect and move symbols in this editor.");
                if ui.button("Enable local connection").clicked() {
                    match live_control::LiveControl::start(ctx) {
                        Ok(control) => {
                            self.live_control = Some(control);
                            error = None;
                        }
                        Err(reason) => error = Some(format!("Could not start the local connection: {reason}")),
                    }
                }
            }
            if let Some(error) = &error {
                ui.label(shell::error(error.as_str()));
            }
            close = ui.button("Close").clicked();
        });
        if !(close || dismissed) {
            self.dialog = Dialog::LocalMcp { error, copied };
        }
    }

    /// Apply a live request after completed file jobs and local keyboard shortcuts.
    /// A successful move advances the same revision used by the canvas and save status.
    #[cfg(not(target_arch = "wasm32"))]
    fn poll_live_control(&mut self) {
        let Some(control) = &self.live_control else { return };
        let can_edit = matches!(self.dialog, Dialog::None) && !self.files.busy();
        let revision = self.session.revision();
        if let Err(error) = control.poll(&mut self.session, can_edit) {
            self.live_control = None;
            self.message = Some(error);
        } else if self.session.revision() != revision {
            self.message = None;
            self.file_notice = Some("Symbol moved through MCP".to_owned());
        }
    }

    /// Draw one frame. Drop the preview if another action changed the project or opened a dialog.
    pub fn frame(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.sync_style(&ctx);
        if let Some(update) = self.files.poll(&mut self.session, &ctx) {
            match update {
                project_files::Update::Opened => {
                    self.camera = schematic_canvas::Camera::default();
                    self.camera.request_fit();
                    self.tool = None;
                    self.home = false;
                    self.dialog = Dialog::None;
                    self.message = None;
                    self.file_notice = Some("Project opened".to_owned());
                }
                project_files::Update::Message(text) => self.file_notice = Some(text),
                project_files::Update::Error(error) => self.message = Some(error),
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let assistant_keys = self.assistant_shortcuts(&ctx);
        #[cfg(target_arch = "wasm32")]
        let assistant_keys = false;
        if !assistant_keys {
            self.shortcuts(&ctx);
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.poll_live_control();
        self.app_bar(ui);
        if self.home {
            self.status(ui);
            self.home_page(ui);
            #[cfg(not(target_arch = "wasm32"))]
            self.assistant.show(&ctx, None, &mut self.session, false);
            if self.integrated_titlebar {
                shell::resize_window_edges(ui);
            }
            self.dialogs(&ctx);
            return;
        }
        self.ribbon(ui);
        if self.tool.is_some_and(|tool| tool.revision() != self.session.revision()) || !matches!(self.dialog, Dialog::None) {
            self.tool = None;
        }
        self.status(ui);
        #[cfg(not(target_arch = "wasm32"))]
        let workspace = ui.available_rect_before_wrap();
        if let Some(project) = self.session.project() {
            workspace::sheet_header(ui, &format!("{} / Main", project.name));
            #[cfg(not(target_arch = "wasm32"))]
            let clearance = self.assistant.rail_clearance();
            #[cfg(target_arch = "wasm32")]
            let clearance = 0.0;
            self.net_inspector.show(ui, &self.session, clearance);
        }
        let revision = self.session.revision();
        self.canvas(ui);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let can_edit = matches!(self.dialog, Dialog::None) && !self.files.busy();
            self.assistant.show(&ctx, Some(workspace), &mut self.session, can_edit);
            if let Some(request) = self.assistant.take_settings_request() {
                self.dialog = match request {
                    assistant::SettingsRequest::Settings => Dialog::Preferences(PreferencesPage::Ai),
                    assistant::SettingsRequest::Add(form) => Dialog::Agent { form, return_to_ai: false },
                };
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.request_repaint();
            }
        }
        if self.integrated_titlebar {
            shell::resize_window_edges(ui);
        }
        self.dialogs(&ctx);
        if self.net_inspector.open && revision != self.session.revision() {
            ctx.request_repaint();
        }
    }
}

impl eframe::App for PcbApp {
    fn ui(&mut self, ui: &mut Ui, frame: &mut Frame) {
        let before = self.preferences;
        self.frame(ui);
        let changed = self.preferences != before;
        #[cfg(not(target_arch = "wasm32"))]
        let changed = self.assistant.take_settings_changed() || changed;
        // eframe's own save runs every 30 seconds, too late if the page reloads right after a change.
        if changed && let Some(storage) = frame.storage_mut() {
            self.save(storage);
            storage.flush();
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(PREFERENCES_KEY, self.preferences.to_text());
        #[cfg(not(target_arch = "wasm32"))]
        {
            storage.set_string(ASSISTANT_SETTINGS_KEY, self.assistant.settings_text());
            storage.set_string(ASSISTANT_LAYOUT_KEY, self.assistant.layout_setting());
        }
    }
}

#[cfg(test)]
mod tests;
