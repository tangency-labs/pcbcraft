//! The PCBCraft window, the same on the desktop and in the browser.
//!
//! Every change to the project goes through [`Session::execute`], with commands from the engine's
//! catalog, the same ones MCP runs. Preferences belong to the person, not the project, so they live
//! here and stay out of the command set.

use std::sync::Arc;

mod arrangements;
#[cfg(not(target_arch = "wasm32"))]
mod assistant;
mod board_canvas;
mod board_panels;
mod board_workspace;
mod canvas_camera;
mod editor_layout;
mod editor_view;
mod editor_windows;
mod editor_workspace;
mod footprint_assignment;
mod hierarchy;
mod home;
#[cfg(not(target_arch = "wasm32"))]
mod live_control;
mod net_inspector;
mod parts_browser;
mod preferences;
mod project_browser;
mod project_files;
#[cfg(not(target_arch = "wasm32"))]
mod recent_projects;
mod ribbon;
mod schematic_canvas;
mod sheet_editing;
mod unsaved;
mod window_commands;

use editor_view::{EditorView, ViewId, ViewInput, Workspace};
use editor_windows::EditorWindows;
use editor_workspace::EditorViews;
use schematic_canvas::{Click, PartDrag, SymbolAction, Tool};
use window_commands::Panel;

use eframe::Frame;
use egui::{Key, KeyboardShortcut, Modifiers, RichText, Ui};
use pcbcraft_chrome::{
    MenuItem, Tokens, fonts, grid,
    menu::{self, MenuIcon},
    shell::{self, dim},
    theme, units, workspace,
};
use pcbcraft_engine::selection::Selection;
use pcbcraft_engine::{
    CONNECT_PINS, DELETE_SYMBOL, MOVE_SYMBOL, PLACE_SYMBOL, ROTATE_SYMBOL, SET_SYMBOL_FIELDS, Session, command, schematic::SymbolDefinition,
};
use serde_json::{Value, json};

pub use preferences::{Navigation, Preferences, ScrollAction, ThemeChoice, ZoomAround};
use project_files::SaveFormat;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const BUG_REPORT_URL: &str = "https://github.com/tangency-labs/pcbcraft/issues/new";
const PREFERENCES_KEY: &str = "pcbcraft.preferences";
#[cfg(not(target_arch = "wasm32"))]
const RECENT_PROJECTS_KEY: &str = "pcbcraft.recent_projects";
/// App bar width below which the Help menu gives way, leaving About and Connect AI assistants
/// to the Home page.
const COMPACT_APP_BAR: f32 = 380.0;
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
const ADD_SHEET: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::S);
const ADD_HIERARCHICAL_LABEL: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::H);
const ADD_WIRE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::W);
const ROTATE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::R);
/// KiCad's keys for the next and previous grid: N for finer, Shift+N for coarser.
const FINER_GRID: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::N);
const COARSER_GRID: KeyboardShortcut = KeyboardShortcut::new(Modifiers::SHIFT, Key::N);
const EDIT_FIELDS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::E);
const COMMAND_COMMA: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);
/// KiCad's key for switching the display units.
const SWITCH_UNITS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::U);

/// A browser keeps Ctrl+N and Ctrl+Q for itself, and has no window of ours to quit.
const IN_BROWSER: bool = cfg!(target_arch = "wasm32");

/// The Help rows, in SolidCraft's order. Returns the row picked this frame.
/// Connect AI assistants is desktop only: the browser can't host a local connection.
fn help_menu(ui: &mut Ui) -> Option<Help> {
    let mut pick = None;
    if MenuItem::new("About PCBCraft").icon(MenuIcon::Help).show(ui) {
        pick = Some(Help::About);
    }
    if MenuItem::new("Keyboard Shortcuts").icon(MenuIcon::Keyboard).show(ui) {
        pick = Some(Help::Shortcuts);
    }
    if MenuItem::new("Report an Issue…").icon(MenuIcon::Bug).show(ui) {
        pick = Some(Help::Report);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        menu::separator(ui);
        if MenuItem::new("Connect AI assistants…").icon(MenuIcon::Assistant).show(ui) {
            pick = Some(Help::Connect);
        }
    }
    pick
}

/// A row in the Help menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Help {
    About,
    Shortcuts,
    Report,
    #[cfg(not(target_arch = "wasm32"))]
    Connect,
}

#[derive(Debug, Default)]
enum Dialog {
    #[default]
    None,
    /// Asks for the name that `project.rename` takes.
    Name {
        command: &'static str,
        text: String,
        error: Option<String>,
        focused: bool,
    },
    /// Names and manages a captured arrangement in the window that opened the form.
    Arrangement(arrangements::Edit),
    /// Revision-bound create/rename/delete forms for an explicitly addressed flat sheet.
    Sheet(sheet_editing::Edit),
    Hierarchy(hierarchy::Edit),
    /// Chooses and pins a package independently of board placement.
    Footprint(footprint_assignment::Edit),
    /// Edit a symbol's reference and value through `schematic.set_symbol_fields`.
    EditFields {
        source: ViewInput,
        symbol: String,
        reference: String,
        value: String,
        error: Option<String>,
        focused: bool,
    },
    /// Asks whether to save unsaved changes before closing, New Project or Open.
    SaveChanges(unsaved::Then),
    OpenLocation,
    SaveEncoding,
    #[cfg(not(target_arch = "wasm32"))]
    LocalMcp {
        error: Option<String>,
        copied: bool,
    },
    Preferences(preferences::PreferencesWindow),
    /// Add an agent. `return_to` is the Preferences window to go back to, when it opened from there.
    #[cfg(not(target_arch = "wasm32"))]
    Agent {
        form: Box<assistant::AgentForm>,
        return_to: Option<preferences::PreferencesWindow>,
    },
    About,
}

/// The project host owns one session, file lifecycle and services. Its schematic view holds
/// navigation and unfinished input separately; split panes share the project authority.
#[derive(Debug, Default)]
pub struct PcbApp {
    arrangements: arrangements::Arrangements,
    views: EditorViews,
    windows: EditorWindows,
    integrated_titlebar: bool,
    #[cfg(not(target_arch = "wasm32"))]
    assistant: assistant::Assistant,
    session: Session,
    #[cfg(not(target_arch = "wasm32"))]
    live_control: Option<live_control::LiveControl>,
    files: project_files::ProjectFiles,
    file_notice: Option<String>,
    preferences: Preferences,
    net_inspector: net_inspector::NetInspector,
    parts: parts_browser::PartsBrowser,
    browser: project_browser::ProjectBrowser,
    /// The ribbon tab shown, an index into [`RIBBON_TABS`].
    ribbon_tab: usize,
    dialog: Dialog,
    /// Whether the Home page replaces the ribbon and canvas. The project stays open behind it.
    home: bool,
    /// Why the last command failed, until the next one runs.
    message: Option<String>,
    /// Set once closing the window has been answered for, so the next close request passes.
    close_allowed: bool,
    /// Close, New Project or Open held back until the running file operation finishes.
    waiting: Option<unsaved::Waiting>,
    /// The snapped point under the pointer on a schematic canvas in the last frame, for the
    /// status bar's coordinate readout. `None` when the pointer is off every canvas.
    pointer: Option<pcbcraft_engine::schematic::Point>,
    /// The point being found this frame; it becomes `pointer` at the start of the next.
    pointer_next: Option<pcbcraft_engine::schematic::Point>,
    /// Whether the browser's leave-page warning is on, once it has been set.
    #[cfg(target_arch = "wasm32")]
    leave_warning: Option<bool>,
    /// The palette and zoom last applied, so they are set only when they change.
    applied: Option<(bool, f32)>,
}

/// The Schematic workspace's ribbon tabs, in order.
const RIBBON_TABS: [&str; 5] = ["Capture", "Analyze", "Libraries", "Manage", "View"];
const VIEW_TAB: usize = 1;

/// Built-in workspace contributions. The picker handles presentation only; each document
/// renderer and input handler retain their own state over the shared session.
const WORKSPACES: workspace::Workspaces<'static> = workspace::Workspaces {
    entries: &[
        workspace::Workspace {
            title: "Schematic",
            description: "Capture circuits, place symbols and connect pins",
            tabs: Some("Capture · Analyze · Libraries · Manage · View"),
            icon: workspace::WorkspaceIcon::Schematic,
            current: true,
        },
        workspace::Workspace {
            title: "PCB layout",
            description: "Inspect footprint placements and linked schematic components",
            tabs: Some("Layout · View"),
            icon: workspace::WorkspaceIcon::Layout,
            current: false,
        },
        workspace::Workspace {
            title: "3D render",
            description: "Inspect the assembled board, layers and components",
            tabs: None,
            icon: workspace::WorkspaceIcon::Render,
            current: false,
        },
        workspace::Workspace {
            title: "Simulate",
            description: "Set up circuit models, analyses and waveform probes",
            tabs: None,
            icon: workspace::WorkspaceIcon::Simulate,
            current: false,
        },
    ],
    footer: "One project across workspaces. Board placement and routing tools are still in development.",
    native_windows: !IN_BROWSER,
};

impl PcbApp {
    /// Start on Home with no project loaded, restoring settings and desktop recent paths.
    /// Restoring history does not read project files.
    /// Successful New/Open actions enter the workspace; cancelling them leaves Home visible.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Home uses custom fonts on its first frame; installing during that frame is too late.
        fonts::install(&cc.egui_ctx);
        let preferences = cc.storage.and_then(|s| s.get_string(PREFERENCES_KEY)).map(|text| Preferences::parse(&text)).unwrap_or_default();
        let mut views = EditorViews::default();
        views.set_grid_everywhere(preferences.schematic_grid_nm);
        Self {
            home: true,
            arrangements: arrangements::Arrangements::from_storage(cc.storage.and_then(|storage| storage.get_string(arrangements::STORAGE_KEY))),
            preferences,
            views,
            #[cfg(not(target_arch = "wasm32"))]
            files: project_files::ProjectFiles::with_recent(cc.storage.and_then(|storage| storage.get_string(RECENT_PROJECTS_KEY)).as_deref()),
            #[cfg(not(target_arch = "wasm32"))]
            assistant: assistant::Assistant::from_setting(
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

    /// Run a command through the shared session, then discard view state invalidated by it.
    /// A failure shows in the status bar and leaves the document and surviving view state intact.
    /// Commands that support automatic wire cleanup default to the person's current preference;
    /// the explicit Heal command always cleans up, without changing that preference.
    fn run(&mut self, id: &str, params: &Value) {
        self.file_notice = None;
        let mut params = params.clone();
        if pcbcraft_engine::command(id).is_some_and(|c| c.params.iter().any(|p| p.name == "auto_heal"))
            && let Some(params) = params.as_object_mut()
        {
            params.entry("auto_heal").or_insert(json!(self.preferences.auto_heal_wires));
        }
        if pcbcraft_engine::sheets::scoped_command(id)
            && let Some(object) = params.as_object_mut()
            && !object.contains_key("sheet")
            && let Some(crate::editor_view::DocumentTarget::Sheet(sheet)) = self.views.active.target()
        {
            object.insert("sheet".into(), json!(sheet.id()));
        }
        self.message = self.session.apply(id, &params).err().map(|e| e.to_string());
        self.views.sync(&self.session);
    }

    /// Start a symbol preview at the current project revision.
    fn begin_placement(&mut self, definition: Arc<SymbolDefinition>) {
        if self.views.active.workspace != Workspace::Schematic {
            return;
        }
        self.views.sync(&self.session);
        self.message = None;
        self.views.active.schematic.tool = Some(Tool::PlaceSymbol { definition, revision: self.session.revision(), turns: 0 });
    }

    /// Start a fresh wire draft at the current revision, accepting empty space or existing endpoints.
    fn begin_wire(&mut self) {
        if self.views.active.workspace != Workspace::Schematic {
            return;
        }
        self.views.sync(&self.session);
        self.message = None;
        self.views.active.schematic.wire_draft = schematic_canvas::Draft::default();
        self.views.active.schematic.tool = Some(Tool::Wire { revision: self.session.revision(), start: None });
    }

    /// Start a new project, first asking whether to save any unsaved changes it would replace.
    fn new_project(&mut self, ctx: &egui::Context) {
        self.unless_unsaved(unsaved::Then::New, ctx);
    }

    /// Open an editable Untitled project through `project.new`, leaving naming and a destination
    /// to the first save, and enter the workspace. The open project is replaced as an undoable
    /// step, as for an agent; a failure shows in the status bar and leaves Home as it was.
    fn start_new_project(&mut self) {
        self.start_new_project_in(Workspace::Schematic);
    }

    /// Create through the shared command, then explicitly target the new project's sheet or
    /// board in the focused pane. A failed command preserves Home and the existing view target.
    fn start_new_project_in(&mut self, workspace: Workspace) {
        self.checkpoint_arrangements();
        self.run("project.new", &Value::Null);
        if self.message.is_none() {
            if let Some(project) = self.session.project() {
                let target = match workspace {
                    Workspace::Schematic => editor_view::DocumentTarget::Sheet(project.sheet_target()),
                    Workspace::Board => editor_view::DocumentTarget::Board(project.board_target()),
                };
                self.views.active.open_document(target, &self.session);
            }
            self.home = false;
        }
    }

    /// Open the name dialog for `command`, starting from the current name when renaming.
    fn ask_name(&mut self, command: &'static str) {
        let text = match command {
            "project.rename" => self.session.project().map(|p| p.name.clone()).unwrap_or_default(),
            _ => String::new(),
        };
        self.dialog = Dialog::Name { command, text, error: None, focused: false };
    }

    /// Open a file that replaces the current session, first asking whether to save unsaved changes.
    fn open_project_file(&mut self, ctx: &egui::Context) {
        self.unless_unsaved(unsaved::Then::Open, ctx);
    }

    /// Desktop users may choose an entry point, archive or folder; browsers pick an upload.
    fn choose_open_location(&mut self, ctx: &egui::Context) {
        if IN_BROWSER {
            self.message = self.files.open(self.session.revision(), ctx, false).err();
        } else {
            self.dialog = Dialog::OpenLocation;
        }
        self.file_notice = None;
    }

    /// Start a save or download for the current snapshot and report setup failures.
    fn save_project_file(&mut self, encoding: Option<SaveFormat>, ctx: &egui::Context) {
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
    /// Download, while desktop Save As opens the encoding chooser. Preferences and Keyboard
    /// Shortcuts sit above Quit, as in SolidCraft.
    fn file_menu(&mut self, ui: &mut Ui) {
        let new_shortcut = if IN_BROWSER { String::new() } else { ui.ctx().format_shortcut(&COMMAND_N) };
        if self.command_row(ui, "project.new", "", &new_shortcut, MenuIcon::New) {
            self.new_project(ui.ctx());
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

        // SolidCraft ends File with Preferences and Keyboard Shortcuts, above Quit.
        menu::separator(ui);
        let preferences_shortcut = ui.ctx().format_shortcut(&COMMAND_COMMA);
        if MenuItem::new("Preferences…").icon(MenuIcon::Settings).shortcut(&preferences_shortcut).show(ui) {
            self.open_preferences();
        }
        if MenuItem::new("Keyboard Shortcuts…").icon(MenuIcon::Keyboard).show(ui) {
            self.dialog = Dialog::Preferences(preferences::PreferencesWindow::at_shortcuts(self.preferences));
        }
        if !IN_BROWSER {
            let quit_shortcut = ui.ctx().format_shortcut(&COMMAND_Q);
            if MenuItem::new("Quit").icon(MenuIcon::Close).shortcut(&quit_shortcut).show(ui) {
                self.request_project_close(ui.ctx());
            }
        }
    }

    /// The selected symbol's shown reference, if it still exists.
    fn selected_reference(&self) -> Option<String> {
        let project = self.views.active.snapshot(&self.session)?;
        let id = self.views.active.schematic.selection.single_symbol()?;
        project.symbols().iter().find(|symbol| symbol.id() == id).map(|symbol| project.reference(symbol))
    }

    /// Rotate, delete, edit fields or pick up a symbol for disconnected movement.
    /// Pickup is view-only until placement; deleting clears selection and failures show in status.
    fn edit_selected(&mut self, action: SymbolAction) {
        if self.views.active.workspace != Workspace::Schematic {
            return;
        }
        self.views.sync(&self.session);
        let Some(id) = self.views.active.schematic.selection.single_symbol().map(str::to_owned) else { return };
        let revision = self.session.revision();
        match action {
            SymbolAction::DisconnectMove => {
                if let Some(symbol) =
                    self.views.active.snapshot(&self.session).and_then(|project| project.symbols().iter().find(|symbol| symbol.id() == id).cloned())
                {
                    self.views.active.schematic.tool =
                        Some(Tool::Move { symbol: symbol.clone(), offset: egui::Vec2::ZERO, revision, disconnect: true });
                }
            }
            SymbolAction::Rotate => self.run(ROTATE_SYMBOL, &json!({"symbol": id, "expected_revision": revision})),
            SymbolAction::Delete => {
                self.run(DELETE_SYMBOL, &json!({"symbol": id, "expected_revision": revision}));
                if self.message.is_none() {
                    self.select(Selection::default());
                }
            }
            SymbolAction::AssignFootprint => {
                let component = self
                    .views
                    .active
                    .snapshot(&self.session)
                    .and_then(|p| p.symbols().iter().find(|s| s.id() == id).and_then(|s| p.component(s)).map(|c| c.id()));
                self.begin_footprint_assignment(component);
            }
            SymbolAction::EditFields => {
                let Some(project) = self.views.active.snapshot(&self.session) else { return };
                let Some(symbol) = project.symbols().iter().find(|symbol| symbol.id() == id) else { return };
                let (reference, value) = (project.reference(symbol), project.value(symbol).to_owned());
                let Some(source) = self.views.active.input(&self.session) else { return };
                self.dialog = Dialog::EditFields { source, symbol: id, reference, value, error: None, focused: false };
            }
        }
    }

    /// Run a wire menu row through the shared commands at the revision the menu opened on, so an
    /// edit made meanwhile (by an agent, say) makes it fail visibly instead of hitting the wrong
    /// leg. A successful edit clears the selection, whose wire may no longer exist. Select Wire
    /// only widens the local selection and is ignored if its captured revision is stale.
    fn edit_wire(&mut self, action: schematic_canvas::WireAction, leg: &schematic_canvas::LegTarget) {
        use schematic_canvas::WireAction;
        let (command, at) = match action {
            WireAction::Heal => {
                self.run(pcbcraft_engine::HEAL_WIRES, &json!({"items":Selection::one_wire(leg.wire.clone()),"expected_revision":leg.revision}));
                if self.message.is_none() {
                    self.select(Selection::default());
                }
                return;
            }
            WireAction::SelectWire => {
                if leg.revision == self.session.revision() {
                    self.select(Selection::one_wire(leg.wire.clone()));
                    self.views.active.schematic.whole_wire_drag = true;
                }
                return;
            }
            WireAction::Slice => match leg.split {
                Some(point) => (pcbcraft_engine::SLICE_WIRE, Some(point)),
                None => return,
            },
            WireAction::Break => (pcbcraft_engine::BREAK_WIRE, Some(leg.inside())),
            WireAction::DeleteSegment => (pcbcraft_engine::DELETE_WIRE_SEGMENT, Some(leg.inside())),
            WireAction::Delete => (pcbcraft_engine::DELETE_WIRE, None),
        };
        let mut params = json!({"wire": leg.wire, "expected_revision": leg.revision});
        if let Some(at) = at {
            params["at"] = json!(at);
        }
        self.run(command, &params);
        if self.message.is_none() {
            self.select(Selection::default());
        }
    }

    /// Commit a finished reshape drag through the matching shared command at the revision the
    /// drag began on, so an edit made meanwhile makes it fail visibly. Segments rebind to the
    /// new geometry when unambiguous, otherwise clear; corner/junction edits keep the wire selected.
    /// Dropping a junction on an explicit endpoint attaches it; free positions only reshape it.
    fn reshape_wire(&mut self, drag: &schematic_canvas::ReshapeDrag, to: pcbcraft_engine::wire::Anchor, revision: u64) {
        use pcbcraft_engine::wire::{Anchor, Reshape};
        let (command, params) = match (&drag.edit, to) {
            (Reshape::Segment { wire, at }, Anchor::Free(to)) => (
                pcbcraft_engine::MOVE_WIRE_SEGMENT,
                json!({"wire": wire, "at": at, "to": to,
                "connect_ends":drag.connection_ends().iter().map(|id|format!("junction-{id}")).collect::<Vec<_>>()}),
            ),
            (Reshape::Corner { wire, at }, Anchor::Free(to)) => (pcbcraft_engine::MOVE_WIRE_CORNER, json!({"wire": wire, "at": at, "to": to})),
            (Reshape::Junction(id), Anchor::Free(to)) => (pcbcraft_engine::MOVE_JUNCTION, json!({"junction": format!("junction-{id}"), "to": to})),
            (Reshape::Junction(id), Anchor::Existing(_) | Anchor::Split { .. }) => {
                (pcbcraft_engine::ATTACH_JUNCTION, json!({"junction": format!("junction-{id}"), "to": to.inspect()}))
            }
            _ => return,
        };
        let mut params = params;
        params["expected_revision"] = json!(revision);
        let selected_segment = self.views.active.schematic.selected_leg.is_some();
        self.run(command, &params);
        if self.message.is_none() && selected_segment {
            if let (Reshape::Segment { wire, .. }, Anchor::Free(to)) = (&drag.edit, to)
                && let Some(leg) = self
                    .views
                    .active
                    .snapshot(&self.session)
                    .and_then(|project| schematic_canvas::LegTarget::at_model(&project, wire, to, self.session.revision()))
            {
                self.select_leg(leg);
            } else {
                self.select(Selection::default());
            }
        }
    }

    /// Delete the selected segment, or the complete item selection when there is no segment.
    /// Both paths use revision-checked shared commands; failures retain the valid selection.
    fn delete_selection(&mut self) {
        if self.views.active.workspace != Workspace::Schematic {
            return;
        }
        if let Some(leg) = self.views.active.schematic.selected_leg.clone() {
            self.edit_wire(schematic_canvas::WireAction::DeleteSegment, &leg);
            return;
        }
        self.run(pcbcraft_engine::DELETE_ITEMS, &json!({"items":self.views.active.schematic.selection,"expected_revision":self.session.revision()}));
        if self.message.is_none() {
            self.select(Selection::default());
        }
    }

    /// Open Preferences on its first page, remembering the current settings for Cancel.
    fn open_preferences(&mut self) {
        self.dialog = Dialog::Preferences(preferences::PreferencesWindow::new(self.preferences));
    }

    /// Run a Help row: a dialog, Preferences on its Shortcuts page, or the issue form.
    fn help_action(&mut self, ctx: &egui::Context, pick: Help) {
        match pick {
            Help::About => self.dialog = Dialog::About,
            Help::Shortcuts => self.dialog = Dialog::Preferences(preferences::PreferencesWindow::at_shortcuts(self.preferences)),
            Help::Report => ctx.open_url(egui::OpenUrl::new_tab(BUG_REPORT_URL)),
            #[cfg(not(target_arch = "wasm32"))]
            Help::Connect => self.dialog = Dialog::LocalMcp { error: None, copied: false },
        }
    }

    /// Save/download availability belongs to the whole open project, even if the active view's
    /// sheet was deleted. A pending file operation still prevents starting another save.
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
    /// Disabling application actions leaves the integrated native window controls available.
    fn app_bar(&mut self, ui: &mut Ui, actions_enabled: bool) {
        let tabs = if self.home { Vec::new() } else { self.arrangements.tabs() };
        let active_arrangement = self.arrangements.active();
        let mut arrangement_action = None;
        let project = self.session.project().map(|project| (project.name.clone(), self.files.dirty(&self.session)));
        let title = project.as_ref().map(|(name, dirty)| (name.as_str(), *dirty));
        let mut preferences = false;
        let mut help = None;
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
            actions_enabled,
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
            |ui| arrangement_action = pcbcraft_chrome::arrangements::tabs(ui, &tabs, active_arrangement),
            |ui, width| {
                if width >= 950.0 {
                    ui.label(dim(format!("v{VERSION}")));
                    ui.add_space(Tokens::get().space_8);
                    if menu::labeled_action(ui, MenuIcon::Bug, "Report a bug").clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(BUG_REPORT_URL));
                    }
                }
                // Narrow bars leave Help's rows to the Home page so Home fits beside the history actions.
                if width >= COMPACT_APP_BAR {
                    help = menu::help(ui, help_menu).flatten();
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
        if let Some(intent) = arrangement_action {
            self.arrangement_intent(ui.ctx(), intent);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if assistant_clicked {
            self.toggle_assistant(ui.ctx());
        }
        if preferences {
            self.open_preferences();
        } else if let Some(pick) = help {
            self.help_action(ui.ctx(), pick);
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

    /// Route the Assistant toggle and report whether its chat owns the canvas keys.
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

    /// Render the workspace toolbar using ported schematic groups and upstream PCB/View controls.
    /// Saved tab slots stay stable while visual ordering follows Next; quick actions use the focused view.
    fn ribbon(&mut self, ui: &mut Ui) {
        use workspace::{RibbonTool, ToolIcon};
        let current = self.views.active.workspace;
        let board = current == Workspace::Board;
        let mut tab = if board {
            usize::from(self.ribbon_tab == VIEW_TAB)
        } else {
            ribbon::TAB_ORDER.iter().position(|slot| *slot == self.ribbon_tab).unwrap_or(0)
        };
        let mut entries = WORKSPACES.entries.to_vec();
        entries[0].current = current == Workspace::Schematic && !self.views.active.unavailable();
        entries[1].current = board && !self.views.active.unavailable();
        let tabs: &[&str] = if board { &["Layout", "View"] } else { &RIBBON_TABS };
        let has_project = self.session.project().is_some();
        let mut quick = Vec::new();
        if self.ribbon_tab != VIEW_TAB {
            if !board {
                quick.push(RibbonTool {
                    icon: ToolIcon::Nets,
                    label: "Nets",
                    description: "Toggle Net Inspector",
                    shortcut: "",
                    enabled: has_project,
                    active: has_project && self.net_inspector.open,
                    toggle: true,
                });
            }
            quick.push(RibbonTool {
                icon: ToolIcon::Fit,
                label: "Fit",
                description: if board { "Fit board" } else { "Fit schematic" },
                shortcut: "",
                enabled: has_project,
                active: false,
                toggle: false,
            });
        }
        let response =
            workspace::ribbon(ui, current.title(), tabs, &mut tab, workspace::Workspaces { entries: &entries, ..WORKSPACES }, &quick, |ui, shown| {
                let slot = if board { shown } else { ribbon::TAB_ORDER[shown] };
                if slot == VIEW_TAB {
                    self.view_tab(ui);
                } else if board {
                    self.board_tab(ui);
                } else {
                    self.ported_ribbon_tab(ui, slot);
                }
            });
        self.ribbon_tab = if board { tab } else { ribbon::TAB_ORDER[tab] };
        if let Some(action) = response.quick {
            if !board && action == 0 {
                self.set_panel(ui.ctx(), Panel::Nets, !self.net_inspector.open);
            } else {
                self.fit_view(self.views.active.id());
            }
        }
        let picked = response.workspace;
        if let Some(pick) = picked {
            let target = match pick.index {
                0 => Some(Workspace::Schematic),
                1 => Some(Workspace::Board),
                _ => None,
            };
            if let Some(target) = target {
                self.open_workspace(target, pick.placement, ui.ctx());
            }
        }
    }

    /// The View tab: show or hide contextual panels, fit the active view, and toggle the split.
    /// Each group's caption also lists its actions in a menu, with check marks for the panels.
    fn view_tab(&mut self, ui: &mut Ui) {
        use workspace::{RibbonTool, ToolIcon};

        let has_project = self.session.project().is_some();
        let schematic = self.views.active.workspace == Workspace::Schematic;
        let parts = schematic && self.session.available(PLACE_SYMBOL);
        let panels = [
            RibbonTool {
                icon: ToolIcon::Browser,
                label: "Browser",
                description: "Toggle Browser",
                shortcut: "",
                enabled: has_project,
                active: has_project && self.browser.open,
                toggle: true,
            },
            RibbonTool {
                icon: ToolIcon::Symbol,
                label: "Parts",
                description: "Toggle Parts",
                shortcut: "",
                enabled: parts,
                active: parts && self.parts.open,
                toggle: true,
            },
            RibbonTool {
                icon: ToolIcon::Nets,
                label: "Nets",
                description: "Toggle Net Inspector",
                shortcut: "",
                enabled: has_project && schematic,
                active: has_project && self.net_inspector.open,
                toggle: true,
            },
        ];
        let toggled = match workspace::ribbon_group(ui, "Panels", &panels) {
            Some(0) => Some(Panel::Browser),
            Some(1) => Some(Panel::Parts),
            Some(2) => Some(Panel::Nets),
            _ => None,
        };
        if let Some(panel) = toggled {
            self.set_panel(ui.ctx(), panel, !self.panel_open(panel));
        }
        workspace::ribbon_separator(ui);
        let navigate = [RibbonTool {
            icon: ToolIcon::Fit,
            label: "Fit",
            description: if schematic { "Fit schematic" } else { "Fit board" },
            shortcut: "",
            enabled: has_project,
            active: false,
            toggle: false,
        }];
        if workspace::ribbon_group(ui, "Navigate", &navigate).is_some() {
            self.fit_view(self.views.active.id());
        }
        workspace::ribbon_separator(ui);
        let mut editors = vec![
            RibbonTool {
                icon: ToolIcon::SplitView,
                label: "Split",
                description: "Split editor views",
                shortcut: "",
                enabled: has_project,
                active: self.views.is_split(),
                toggle: true,
            },
            RibbonTool {
                icon: ToolIcon::Settings,
                label: "Settings",
                description: "View settings",
                shortcut: "",
                enabled: true,
                active: false,
                toggle: false,
            },
        ];
        if !IN_BROWSER {
            editors.push(RibbonTool {
                icon: ToolIcon::EditorWindow,
                label: if self.views.host != self.views.main { "Dock" } else { "Detach" },
                description: if self.views.host != self.views.main { "Dock editor view" } else { "Detach active editor view" },
                shortcut: "",
                enabled: has_project,
                active: false,
                toggle: false,
            });
            editors.push(RibbonTool {
                icon: ToolIcon::EditorWindow,
                label: "New window",
                description: "Open another editor window",
                shortcut: "",
                enabled: has_project,
                active: false,
                toggle: false,
            });
        }
        match workspace::ribbon_group(ui, "Editors", &editors) {
            Some(0) => self.split_editors(ui.ctx(), !self.views.is_split()),
            Some(1) => self.dialog = Dialog::Preferences(preferences::PreferencesWindow::at_views(self.preferences)),
            Some(2) => self.toggle_editor_window(ui.ctx()),
            Some(3) => self.open_editor_window(ui.ctx()),
            _ => {}
        }
    }

    /// Select and center a Browser pick only in its originating view and revision. Selection is
    /// view state, so the project and its revision are unchanged; stale picks do not move a camera.
    fn show_pick(&mut self, source: ViewInput, pick: project_browser::Pick) {
        if self.views.active.workspace != Workspace::Schematic
            || self.home
            || !matches!(self.dialog, Dialog::None)
            || !self.views.active.accepts(source, &self.session)
        {
            return;
        }
        self.select(if pick.wire { Selection::one_wire(pick.id) } else { Selection::one_symbol(pick.id) });
        self.views.active.camera.center_on(pick.at);
    }

    /// Apply a Browser intent after deferred view validation. Only the originating pane
    /// navigates; object picks then use the existing selection coordinator and camera methods.
    /// Invalid targets select nothing, and native render slots return to the main host.
    fn apply_browser_action(&mut self, source: ViewId, action: project_browser::Action) {
        use project_browser::Action;
        if let Action::ManageSheet { command, sheet } = action {
            self.begin_sheet_edit(source, command, sheet);
            return;
        }
        let Some(project) = self.session.project() else {
            return;
        };
        let valid = match &action {
            Action::ManageSheet { .. } => false,
            Action::OpenDocument(_) => true,
            Action::Schematic { sheet, pick } => project.for_sheet(*sheet).is_some_and(|project| {
                if pick.wire {
                    project.wires().iter().any(|wire| wire.id() == pick.id)
                } else {
                    project.symbols().iter().any(|symbol| symbol.id() == pick.id)
                }
            }),
            Action::Footprint { board, id, .. } => {
                project.board_view(*board).is_some_and(|board| board.footprints().any(|footprint| footprint.id == *id))
            }
        };
        if !valid {
            return;
        }
        let Some(view) = action.target().and_then(|target| self.views.open_document(source, target, &self.session)) else {
            return;
        };
        self.views.bind(view);
        match action {
            Action::OpenDocument(_) | Action::ManageSheet { .. } => {}
            Action::Schematic { pick, .. } => {
                if let Some(source) = self.views.active.input(&self.session) {
                    let at = pick.at;
                    self.show_pick(source, pick);
                    self.views.active.camera.request_center(at);
                }
            }
            Action::Footprint { id, at, .. } => {
                self.views.select_board(std::collections::BTreeSet::from([id]), &self.session, self.preferences.selection_follow);
                self.views.active.camera.request_center(at);
            }
        }
        self.views.bind_main();
    }

    /// Show the command error or placement hint alongside the project name, and at the right
    /// the focused view's grid spacing, which can be changed there. Home has no canvas, so it
    /// shows no grid. Density annotations follow the visible grid style, independently of snapping.
    fn status(&mut self, ui: &mut Ui) {
        let name = self.session.project().map_or("No project open", |p| p.name.as_str()).to_owned();
        let message = self.message.clone();
        let grid_view = (!self.home && self.session.project().is_some()).then(|| self.views.active.id());
        let mut spacing = self.views.active.camera.snap_nm();
        let every = grid::shown_every(self.views.active.camera.snap_step()).unwrap_or(1);
        let grid_style = if self.preferences.grid_opacity_percent == 0 {
            grid::Style::Off
        } else if self.views.active.workspace == Workspace::Board {
            self.preferences.board_grid_style
        } else {
            grid::Style::Dots
        };
        let mut grid_changed = false;
        let mut unit = self.preferences.units;
        let pointer = self.pointer.filter(|_| grid_view.is_some());
        let units_shortcut = ui.ctx().format_shortcut(&SWITCH_UNITS);
        shell::status_bar(
            ui,
            |ui| {
                ui.label(dim(name));
                if let Some(message) = message {
                    ui.label(shell::error(message));
                } else if self.views.active.workspace == Workspace::Board {
                    ui.label(dim("Board"));
                } else if self.views.active.schematic.tool.is_none() && self.views.active.schematic.selection.len() > 1 {
                    ui.label(dim(format!("{} items selected · Drag to move · Del delete", self.views.active.schematic.selection.len())));
                } else if let (None, Some(reference)) = (&self.views.active.schematic.tool, self.selected_reference()) {
                    ui.label(dim(format!("{reference} selected · R rotate · E edit fields · Del delete")));
                } else if let Some(tool) = &self.views.active.schematic.tool {
                    ui.label(match tool {
                        Tool::SheetGesture { drag, .. } => {
                            dim(if drag.corner.is_some() { "Resize sheet · Esc to cancel" } else { "Move sheet · Esc to cancel" })
                        }
                        Tool::Hierarchy { .. } => dim("Hierarchical sheet or label · Esc to cancel"),
                        Tool::PlaceSymbol { definition, .. } => dim(format!("Place {} · R rotate · Esc to cancel", definition.name)),
                        Tool::Move { disconnect: true, .. } => dim("Disconnect and move symbol · Esc to cancel"),
                        Tool::Move { .. } => dim("Move symbol · Esc to cancel"),
                        Tool::MoveItems { .. } => dim("Move selection · Esc to cancel"),
                        Tool::Reshape { .. } => dim("Reshape wire · Esc to cancel"),
                        Tool::BoxSelect { .. } => dim("Select items · Esc to cancel"),
                        Tool::Wire { start: None, .. } => dim("Wire · Click a start point"),
                        Tool::Wire { start: Some(_), .. } => dim("Wire · Click corners · Enter finish · Space flip · Esc cancel"),
                    });
                } else if self.views.active.schematic.selected_leg.is_some() {
                    ui.label(dim("Wire segment selected · Del delete"));
                } else if self.views.active.schematic.selection.single_wire().is_some() {
                    ui.label(dim(if self.views.active.schematic.whole_wire_drag {
                        "Whole wire selected · Drag to move · Del delete"
                    } else {
                        "Wire selected · Drag a segment, corner or free end to reshape · Del delete"
                    }));
                } else if self.files.busy() {
                    ui.label(dim("File operation in progress"));
                } else if let Some(notice) = &self.file_notice {
                    ui.label(dim(notice.as_str()));
                }
            },
            |ui| {
                // Right to left: units, then grid, then the pointer's coordinates. With no
                // schematic showing there is nothing to measure, so none of them appear.
                if grid_view.is_some() {
                    units::toggle(ui, &mut unit, &units_shortcut);
                    grid_changed = grid::spacing_menu(
                        ui,
                        &mut spacing,
                        &grid::SCHEMATIC_SPACING_PRESETS_NM,
                        grid::SCHEMATIC_SPACING_DEFAULT_NM,
                        every,
                        unit,
                        grid_style,
                    );
                }
                if let Some(point) = pointer {
                    let text = format!("X {}  Y {}", units::readout(point.x_nm, unit), units::readout(point.y_nm, unit));
                    ui.label(dim(text)).on_hover_text("Pointer position on the grid");
                }
                #[cfg(not(target_arch = "wasm32"))]
                if self.live_control.is_some() {
                    ui.label(dim("Local MCP enabled"));
                }
            },
        );
        if let Some(id) = grid_view.filter(|_| grid_changed) {
            self.set_view_grid(id, spacing);
        }
        self.preferences.units = unit;
    }

    /// Draw the schematic and send completed edits through the shared command catalog using each
    /// gesture's starting revision to reject stale actions. M picks up the selected symbol or,
    /// without a selection, the hovered symbol for disconnected movement. Cancel clears only
    /// the local preview, and another active tool cannot be replaced by a queued shortcut.
    fn canvas(&mut self, ui: &mut Ui) {
        let snapshot = self.views.active.snapshot(&self.session);
        let label = if self.views.is_split() { format!("Schematic canvas {}", self.views.active.id().0 + 1) } else { "Schematic canvas".to_owned() };
        let source = self.views.active.input(&self.session);
        // Web wheel events reach the canvas before it gains keyboard focus. Native windows
        // still require OS focus so an inactive host cannot act on canvas input.
        let input_enabled = ui.is_enabled()
            && (IN_BROWSER || ui.input(|input| input.focused))
            && !self.home
            && !self.project_action_pending()
            && matches!(self.dialog, Dialog::None)
            && !egui::Popup::is_any_open(ui.ctx());
        let click = ui
            .push_id(self.views.active.widget_id(), |ui| {
                shell::canvas(ui, |ui| {
                    if snapshot.as_deref().is_some()
                        && let Some(point) = pointer_on_canvas(ui, &self.views.active.camera)
                    {
                        self.pointer_next = Some(point);
                    }
                    let moving = std::mem::take(&mut self.views.active.schematic.move_requested);
                    let rotating = std::mem::take(&mut self.views.active.schematic.rotate_requested);
                    if (moving || rotating)
                        && input_enabled
                        && self.views.active.schematic.tool.is_none()
                        && self.views.active.schematic.selection.len() <= 1
                    {
                        let rect = ui.available_rect_before_wrap();
                        let hovered = snapshot.as_deref().and_then(|project| {
                            let pointer = ui.input(|input| input.pointer.hover_pos())?;
                            rect.contains(pointer)
                                .then(|| schematic_canvas::symbol_at(project, &self.views.active.camera, rect, pointer))
                                .flatten()
                                .map(|(symbol, _)| symbol.id())
                        });
                        let selected = self.views.active.schematic.selection.single_symbol().map(str::to_owned);
                        let target = if moving { selected.or(hovered) } else { hovered.or(selected) };
                        if let Some(symbol) = target {
                            self.select(Selection::one_symbol(symbol));
                            self.edit_selected(if moving { SymbolAction::DisconnectMove } else { SymbolAction::Rotate });
                        }
                    }
                    if let Some(project) = snapshot.as_deref() {
                        let highlight = self.views.highlight_for(self.views.active.id(), &self.session);
                        return schematic_canvas::show(
                            ui,
                            project,
                            self.session.revision(),
                            &mut self.views.active.camera,
                            self.views.active.schematic.tool.clone(),
                            &self.views.active.schematic.selection,
                            self.views.active.schematic.selected_leg.as_ref(),
                            self.views.active.schematic.whole_wire_drag,
                            self.views.active.schematic.selected_sheet_symbol,
                            &highlight,
                            input_enabled,
                            self.preferences.navigation,
                            self.preferences.grid_opacity_percent,
                            self.preferences.auto_heal_wires,
                            &self.views.active.schematic.wire_draft,
                            &label,
                        );
                    }
                    let t = Tokens::get();
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new("No project open. Choose File, then New Project.").size(t.font_button).color(t.canvas_ink));
                    });
                    None
                })
            })
            .inner;
        self.views.active.schematic.wire_draft.finish_requested = false;
        self.views.active.schematic.wire_draft.start_requested = false;
        if let (Some(source), Some(click)) = (source, click) {
            self.view_click(source, click);
            if matches!(self.dialog, Dialog::Hierarchy(_) | Dialog::Sheet(_)) {
                ui.ctx().send_viewport_cmd_to(editor_windows::viewport(self.views.main), egui::ViewportCommand::Focus);
            }
            // Selection can change after the other pane was painted, without a document edit.
            self.repaint_editors(ui.ctx());
        }
    }

    /// Apply canvas input only to its originating view, sheet and revision. The shared session
    /// remains the sole mutation authority; stale input cannot alter selection or a new preview.
    fn view_click(&mut self, source: ViewInput, click: Click) {
        if self.views.active.workspace != Workspace::Schematic
            || self.home
            || !matches!(self.dialog, Dialog::None)
            || !self.views.active.accepts(source, &self.session)
        {
            return;
        }
        match (self.views.active.schematic.tool.clone(), click) {
            (None, Click::SelectSheetSymbol(id)) => {
                self.select(Selection::default());
                self.views.active.schematic.selected_sheet_symbol = Some(id);
            }
            (None, Click::GrabSheetGesture(drag)) => {
                self.select(Selection::default());
                self.views.active.schematic.selected_sheet_symbol = Some(drag.block.id);
                self.views.active.schematic.tool = Some(Tool::SheetGesture { drag, revision: source.revision });
            }
            (Some(Tool::SheetGesture { drag, revision }), Click::FinishSheetGesture(geometry)) => {
                self.views.active.schematic.tool = None;
                if let Some(geometry) = geometry {
                    let mut params = json!({"id":format!("sheet-symbol-{}",drag.block.id),"x_nm":geometry.at.x_nm,"y_nm":geometry.at.y_nm,"expected_revision":revision});
                    if drag.corner.is_some() {
                        params["width_nm"] = json!(geometry.size.x_nm);
                        params["height_nm"] = json!(geometry.size.y_nm);
                    }
                    self.run(
                        if drag.corner.is_some() { pcbcraft_engine::hierarchy::RESIZE_SHEET } else { pcbcraft_engine::hierarchy::MOVE_SHEET },
                        &params,
                    );
                }
            }
            (Some(Tool::Hierarchy { kind, id, .. }), Click::HierarchyPlace(at)) => self.hierarchy_place(source, kind, id, at),
            (None, Click::HierarchyAction(kind, id, action)) => self.hierarchy_action(source, kind, id, action),
            (Some(Tool::PlaceSymbol { definition, revision, turns }), Click::Place(point)) => {
                self.views.active.schematic.tool = None;
                self.run(PLACE_SYMBOL, &json!({"definition": definition.id, "x_nm": point.x_nm, "y_nm": point.y_nm, "expected_revision": revision, "rotation": u16::from(turns) * 90}));
            }
            (Some(Tool::Wire { revision, start: None }), Click::WirePoint(anchor, _)) => {
                self.views.active.schematic.tool = Some(Tool::Wire { revision, start: Some(anchor) });
            }
            (None, Click::StartWire(anchor)) => {
                self.begin_wire();
                self.views.active.schematic.tool = Some(Tool::Wire { revision: self.session.revision(), start: Some(anchor) });
            }
            (Some(Tool::Wire { revision, start: Some(from) }), Click::WirePoint(to, finish)) => {
                if let Some(project) = self.views.active.snapshot(&self.session)
                    && let (Some(a), Some(b)) = (from.position(&project), to.position(&project))
                    && a != b
                {
                    if finish || !matches!(to, pcbcraft_engine::wire::Anchor::Free(_)) {
                        let Some(path) = self.views.active.schematic.wire_draft.path(a, b, self.preferences.auto_heal_wires) else {
                            self.message = Some("Wire preview exceeds the supported geometry limits.".to_owned());
                            return;
                        };
                        self.run(
                            pcbcraft_engine::DRAW_WIRE,
                            &json!({"from": from.inspect(), "to": to.inspect(), "bends": &path[1..path.len()-1], "expected_revision": revision}),
                        );
                        if self.message.is_none() {
                            self.views.active.schematic.tool = None;
                        }
                    } else if !self.views.active.schematic.wire_draft.corner(a, b, self.preferences.auto_heal_wires) {
                        self.message = Some("Wire has too many corners; finish it before continuing.".to_owned());
                    }
                }
            }
            (None, Click::Grab(symbol, offset)) => {
                self.select(Selection::one_symbol(symbol.id()));
                self.views.active.schematic.tool = Some(Tool::Move { symbol, offset, revision: self.session.revision(), disconnect: false });
            }
            (None, Click::StartBox(drag)) => {
                self.views.active.schematic.tool = Some(Tool::BoxSelect { drag, revision: self.session.revision() });
            }
            (None | Some(Tool::BoxSelect { .. }), Click::SelectItems(items)) => {
                self.views.active.schematic.tool = None;
                self.select(items);
            }
            (None, Click::Select(symbol)) => {
                self.select(symbol.map(Selection::one_symbol).unwrap_or_default());
            }
            (None, Click::SelectLeg(leg)) => self.select_leg(leg),
            (None, Click::GrabItems(items, origin)) => {
                let whole_wire_drag = self.views.active.schematic.whole_wire_drag && items.single_wire().is_some();
                self.select(items.clone());
                self.views.active.schematic.whole_wire_drag = whole_wire_drag;
                self.views.active.schematic.tool = Some(Tool::MoveItems { items, origin, revision: self.session.revision() });
            }
            (Some(Tool::MoveItems { items, revision, .. }), Click::FinishItems(delta)) => {
                self.views.active.schematic.tool = None;
                if let Some(delta) = delta {
                    self.run(pcbcraft_engine::MOVE_ITEMS, &json!({"items":items,"dx_nm":delta.x_nm,"dy_nm":delta.y_nm,"expected_revision":revision}));
                }
            }
            (None, Click::GrabReshape(drag)) => {
                if let pcbcraft_engine::wire::Reshape::Segment { wire, at } = &drag.edit {
                    if let Some(leg) = self
                        .views
                        .active
                        .snapshot(&self.session)
                        .and_then(|project| schematic_canvas::LegTarget::at_model(&project, wire, *at, self.session.revision()))
                    {
                        self.select_leg(leg);
                    } else {
                        return;
                    }
                } else {
                    self.select(Selection::one_wire(drag.wire.clone()));
                }
                self.views.active.schematic.tool = Some(Tool::Reshape { drag, revision: self.session.revision() });
            }
            (Some(Tool::Reshape { drag, revision }), Click::FinishReshape(to)) => {
                self.views.active.schematic.tool = None;
                if let Some(to) = to {
                    self.reshape_wire(&drag, to, revision);
                }
            }
            (None, Click::DeleteItems) => self.delete_selection(),
            (_, Click::Drop(PartDrag { definition, source: drag_source }, point)) => {
                if !self.views.active.accepts(drag_source, &self.session) {
                    return;
                }
                let revision = drag_source.revision;
                self.views.active.schematic.tool = None;
                self.run(PLACE_SYMBOL, &json!({"definition": definition.id, "x_nm": point.x_nm, "y_nm": point.y_nm, "expected_revision": revision}));
            }
            (None, Click::WireEdit(action, leg)) => self.edit_wire(action, &leg),
            (None, Click::Edit(action, symbol)) => {
                self.select(Selection::one_symbol(symbol));
                self.edit_selected(action);
            }
            (Some(Tool::Move { symbol, revision, disconnect, .. }), Click::FinishMove(position)) => {
                self.views.active.schematic.tool = None;
                if let Some(point) = position {
                    self.run(
                        MOVE_SYMBOL,
                        &json!({
                            "symbol": symbol.id(), "x_nm": point.x_nm, "y_nm": point.y_nm, "expected_revision": revision, "disconnect":disconnect,
                        }),
                    );
                }
            }
            (Some(Tool::Wire { .. }), Click::CancelWire) => {
                self.views.active.schematic.tool = None;
                self.message = None;
            }
            _ => {}
        }
    }

    /// Route shortcuts only from the addressed view, outside text fields and dialogs. A different
    /// view leaves the keys unconsumed. Assistant focus suppresses canvas tools.
    /// On Home, Escape returns to the workspace and the canvas keys do nothing. In the workspace,
    /// Escape leaves parts search before cancelling a tool, after overlays have priority.
    fn shortcuts(&mut self, ctx: &egui::Context, assistant_keys: bool, view: ViewId) {
        if view != self.views.active.id() || !ctx.input(|input| input.focused) || self.project_action_pending() {
            return;
        }
        self.views.active.schematic.rotate_requested = false;
        self.views.active.schematic.move_requested = false;
        self.views.active.board.move_requested = false;
        self.views.active.board.rotate_requested = 0;
        if !self.home
            && self.views.active.workspace == Workspace::Schematic
            && !assistant_keys
            && matches!(self.dialog, Dialog::None)
            && !egui::Popup::is_any_open(ctx)
            && self.parts.take_escape(ctx)
        {
            return;
        }
        if ctx.egui_wants_keyboard_input() || !matches!(self.dialog, Dialog::None) {
            return;
        }
        let pressed = |shortcut: &KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(shortcut));
        if self.home {
            if !egui::Popup::is_any_open(ctx) && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                self.home = false;
            }
        } else if !assistant_keys && !egui::Popup::is_any_open(ctx) && self.views.active.workspace == Workspace::Board {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) && self.views.active.board.draft.take().is_none() {
                self.views.select_board(Default::default(), &self.session, self.preferences.selection_follow);
            }
            self.views.active.board.move_requested = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::M));
            self.views.active.board.rotate_requested = if ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::R)) {
                270
            } else if pressed(&ROTATE) {
                90
            } else {
                0
            };
            if pressed(&COARSER_GRID) {
                self.cycle_grid(false);
            } else if pressed(&FINER_GRID) {
                self.cycle_grid(true);
            }
        } else if !assistant_keys && !egui::Popup::is_any_open(ctx) {
            if ctx.input_mut(|i| i.consume_key(Modifiers::ALT, Key::Backspace)) {
                self.leave_sheet();
                return;
            }
            if pressed(&ADD_HIERARCHICAL_LABEL) {
                self.begin_hierarchy(hierarchy::Kind::Label);
            }
            if pressed(&ADD_SHEET) {
                self.begin_hierarchy(hierarchy::Kind::Sheet);
            }
            let hierarchy_hovered = ctx.input(|i| [Key::E, Key::Delete, Key::Backspace, Key::R, Key::M].into_iter().any(|key| i.key_pressed(key)))
                && self.views.active.snapshot(&self.session).is_some_and(|project| {
                    ctx.input(|i| i.pointer.hover_pos()).is_some_and(|pointer| hierarchy::hovered(&project, &self.views.active.camera, pointer))
                });
            // Ctrl temporarily bypasses the wire grid without disabling tool controls.
            let wire_keys = Modifiers {
                ctrl: matches!(self.views.active.schematic.tool, Some(Tool::Wire { .. })) && ctx.input(|i| i.modifiers.ctrl),
                ..Modifiers::NONE
            };
            if ctx.input_mut(|i| i.consume_key(wire_keys, Key::Escape)) {
                // Escape cancels the tool first, then clears the selection.
                if self.views.active.schematic.tool.take().is_some() {
                    self.message = None;
                } else {
                    self.select(Selection::default());
                }
            }
            if let Some(Tool::Wire { start, .. }) = &mut self.views.active.schematic.tool {
                if ctx.input_mut(|i| i.consume_key(wire_keys, Key::Space)) {
                    self.views.active.schematic.wire_draft.horizontal_first = !self.views.active.schematic.wire_draft.horizontal_first;
                }
                if ctx.input_mut(|i| i.consume_key(wire_keys, Key::Backspace)) && !self.views.active.schematic.wire_draft.back() {
                    *start = None;
                }
                self.views.active.schematic.wire_draft.finish_requested = ctx.input_mut(|i| i.consume_key(wire_keys, Key::Enter));
            }
            if !hierarchy_hovered
                && self.views.active.schematic.tool.is_none()
                && !self.views.active.schematic.selection.is_empty()
                && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace))
            {
                self.delete_selection();
            }
            if !hierarchy_hovered && self.views.active.schematic.tool.is_none() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::M)) {
                self.views.active.schematic.move_requested = true;
            }
            if !hierarchy_hovered && pressed(&ROTATE) {
                if let Some(Tool::PlaceSymbol { turns, .. }) = &mut self.views.active.schematic.tool {
                    *turns = (*turns + 1) % 4;
                } else if self.views.active.schematic.tool.is_none() {
                    self.views.active.schematic.rotate_requested = true;
                }
            }
            if !hierarchy_hovered
                && self.views.active.schematic.tool.is_none()
                && self.views.active.schematic.selection.single_symbol().is_some()
                && pressed(&EDIT_FIELDS)
            {
                self.edit_selected(SymbolAction::EditFields);
            }
            // Shift+N first: an unshifted pattern also matches a shifted key.
            if pressed(&COARSER_GRID) {
                self.cycle_grid(false);
            } else if pressed(&FINER_GRID) {
                self.cycle_grid(true);
            }
            if pressed(&ADD_SYMBOL) && self.session.available(PLACE_SYMBOL) {
                self.parts.focus_search();
            }
            if pressed(&ADD_WIRE) && self.session.available(CONNECT_PINS) {
                self.begin_wire();
                self.views.active.schematic.wire_draft.start_requested = true;
            }
        }
        if pressed(&COMMAND_Z) && self.session.available("edit.undo") {
            self.run("edit.undo", &Value::Null);
        }
        if pressed(&COMMAND_Y) && self.session.available("edit.redo") {
            self.run("edit.redo", &Value::Null);
        }
        if pressed(&COMMAND_COMMA) {
            self.open_preferences();
        }
        if pressed(&SWITCH_UNITS) {
            self.preferences.units = self.preferences.units.toggled();
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
                self.new_project(ctx);
            }
            if pressed(&COMMAND_Q) {
                self.request_project_close(ctx);
            }
        }
    }

    /// Draw the open dialog and retain it until the person finishes or dismisses it.
    fn dialogs(&mut self, ctx: &egui::Context) {
        match std::mem::take(&mut self.dialog) {
            Dialog::None => {}
            Dialog::Arrangement(edit) => self.arrangement_dialog(ctx, edit),
            Dialog::Name { command: id, mut text, mut error, mut focused } => {
                let title = command(id).map_or(id, |c| c.title);
                let (mut submit, mut cancel) = (false, false);
                let closing = shell::dialog(ctx, "name_dialog", title, |ui| {
                    let label = ui.label("Name");
                    let field = ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY)).labelled_by(label.id);
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
            Dialog::EditFields { source, symbol, mut reference, mut value, mut error, mut focused } => {
                let (mut submit, mut cancel) = (false, false);
                let closing = shell::dialog(ctx, "edit_fields", "Edit Symbol Fields", |ui| {
                    let label = ui.label("Reference");
                    let field = ui.add(egui::TextEdit::singleline(&mut reference).desired_width(f32::INFINITY)).labelled_by(label.id);
                    if !focused {
                        field.request_focus();
                        focused = true;
                    }
                    let label = ui.label("Value");
                    let value_field = ui.add(egui::TextEdit::singleline(&mut value).desired_width(f32::INFINITY)).labelled_by(label.id);
                    submit = (field.lost_focus() || value_field.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter));
                    if let Some(error) = &error {
                        ui.label(shell::error(error.as_str()));
                    }
                    ui.horizontal(|ui| {
                        submit |= ui.button("OK").clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
                if submit {
                    // Stale dialogs close with a status message; validation errors stay editable.
                    if !self.views.active.accepts(source, &self.session) {
                        self.message = Some("View or project changed; reopen the symbol fields to continue editing.".to_owned());
                        return;
                    }
                    let mut params = json!({"symbol": symbol, "reference": reference, "value": value, "expected_revision": source.revision});
                    if let Some(crate::editor_view::DocumentTarget::Sheet(sheet)) = self.views.active.target() {
                        params["sheet"] = json!(sheet.id());
                    }
                    match self.session.apply(SET_SYMBOL_FIELDS, &params) {
                        Ok(_) => self.message = None,
                        Err(e) => {
                            error = Some(e.to_string());
                            self.dialog = Dialog::EditFields { source, symbol, reference, value, error, focused };
                        }
                    }
                } else if !(closing || cancel) {
                    self.dialog = Dialog::EditFields { source, symbol, reference, value, error, focused };
                }
            }
            Dialog::Footprint(edit) => self.footprint_assignment_dialog(ctx, edit),
            Dialog::Sheet(edit) => self.sheet_dialog(ctx, edit),
            Dialog::Hierarchy(edit) => self.hierarchy_dialog(ctx, edit),
            Dialog::SaveChanges(then) => self.save_changes_dialog(ctx, then),
            Dialog::OpenLocation => {
                let mut selected = None;
                let mut cancel = false;
                let closing = shell::dialog(ctx, "open_location", "Open Project", |ui| {
                    ui.vertical(|ui| {
                        if ui.button("Project file or archive…").clicked() {
                            selected = Some(false);
                        }
                        if ui.button("Project folder…").clicked() {
                            selected = Some(true);
                        }
                        cancel = ui.button("Cancel").clicked();
                    });
                });
                if let Some(folder) = selected {
                    self.message = self.files.open(self.session.revision(), ctx, folder).err();
                } else if !(closing || cancel) {
                    self.dialog = Dialog::OpenLocation;
                }
            }
            Dialog::SaveEncoding => {
                let mut selected = None;
                let mut cancel = false;
                let title = if IN_BROWSER { "Download Project" } else { "Save Project As" };
                let closing = shell::dialog(ctx, "save_encoding", title, |ui| {
                    ui.vertical(|ui| {
                        if !IN_BROWSER && ui.button("Project folder…").clicked() {
                            selected = Some(SaveFormat::Folder);
                        }
                        if ui.button("Packed project (.pc.zip)…").clicked() {
                            selected = Some(SaveFormat::Archive);
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
            Dialog::Preferences(mut window) => {
                let shortcuts = shortcut_list(ctx);
                #[cfg(not(target_arch = "wasm32"))]
                let outcome = window.show(ctx, &mut self.preferences, &shortcuts, &mut self.assistant);
                #[cfg(target_arch = "wasm32")]
                let outcome = window.show(ctx, &mut self.preferences, &shortcuts);
                match outcome {
                    preferences::Outcome::Open => self.dialog = Dialog::Preferences(window),
                    preferences::Outcome::Closed => ctx.request_repaint(),
                    #[cfg(not(target_arch = "wasm32"))]
                    preferences::Outcome::AddAgent(form) => {
                        self.dialog = Dialog::Agent { form, return_to: Some(window) };
                        ctx.request_repaint();
                    }
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Dialog::Agent { mut form, return_to } => {
                if self.assistant.agent_dialog(ctx, &mut form) {
                    if let Some(window) = return_to {
                        self.dialog = Dialog::Preferences(window);
                    }
                    ctx.request_repaint();
                } else {
                    self.dialog = Dialog::Agent { form, return_to };
                }
            }
            Dialog::About => {
                let mut close = false;
                let closing = shell::dialog(ctx, "about_dialog", "About PCBCraft", |ui| {
                    ui.label(format!("PCBCraft {VERSION}"));
                    ui.label("Electronics design that feels like KiCad, built for people and agents to work on the same board.");
                    ui.label(dim("MIT OR Apache-2.0. Third-party notices are in ATTRIBUTION.md."));
                    workspace::ribbon_licenses(ui);
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
        let dismissed = shell::dialog(ctx, "local_mcp", "Connect AI assistants", |ui| {
            if let Some(control) = &self.live_control {
                ui.label("Clients with this configuration can inspect and edit this project and control its editor windows.");
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
                ui.label("Let a local MCP client inspect and edit this project and control its editor windows.");
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

    /// Run at most one live request after completed file jobs, before any window draws. Document
    /// commands advance the same revision used by the canvas and save status, and the status bar
    /// names the edit; window commands change only views, windows and settings. A stopped
    /// listener clears the enabled state and reports why.
    #[cfg(not(target_arch = "wasm32"))]
    fn poll_live_control(&mut self, ctx: &egui::Context) {
        let request = match self.live_control.as_ref().map(live_control::LiveControl::poll) {
            None | Some(Ok(None)) => return,
            Some(Err(error)) => {
                self.live_control = None;
                self.message = Some(error);
                return;
            }
            Some(Ok(Some(request))) => request,
        };
        let revision = self.session.revision();
        let result = pcbcraft_mcp::live::call_editor(&mut live_control::LiveEditor { app: self, ctx }, request.body());
        if self.session.revision() != revision {
            self.views.sync(&self.session);
            let title = request.body()["params"]["command"].as_str().and_then(command).map_or("Edit", |command| command.title);
            self.message = None;
            self.file_notice = Some(format!("{title} through MCP"));
        }
        request.answer(result);
        self.repaint_editors(ctx);
    }

    /// Poll the shared project exactly once through the framework root, even after its native
    /// window is hidden. Visible hosts borrow that state; none starts another service or file worker.
    pub fn frame(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let before = (self.session.revision(), self.preferences, std::mem::discriminant(&self.dialog), self.home);
        self.sync_style(&ctx);
        self.checkpoint_arrangements();
        self.handle_close_request(&ctx);
        if let Some(update) = self.files.poll(&mut self.session, &ctx) {
            match update {
                project_files::Update::Opened => {
                    self.views.bind_opened_project(&self.session);
                    for view in self.views.all_mut() {
                        view.camera.request_fit();
                    }
                    self.home = false;
                    self.dialog = Dialog::None;
                    self.message = None;
                    self.file_notice = Some(self.files.open_notice());
                }
                project_files::Update::Message(text) => self.file_notice = Some(text),
                project_files::Update::Error(error) => self.message = Some(error),
            }
        }
        self.finish_waiting(&ctx);
        #[cfg(target_arch = "wasm32")]
        self.warn_before_leaving();
        self.views.sync(&self.session);
        self.sync_arrangements(&ctx);
        #[cfg(not(target_arch = "wasm32"))]
        self.poll_live_control(&ctx);
        if self.views.hosts.contains_key(&editor_layout::HostId::ROOT) {
            self.editor_host_frame(ui, self.integrated_titlebar);
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.detached_editor(&ctx);
        self.finish_view_actions(&ctx);
        self.finish_arrangement_action(&ctx);
        self.checkpoint_arrangements();
        if let Some(notice) = self.arrangements.notice.take() {
            self.message = Some(notice);
        }
        if before != (self.session.revision(), self.preferences, std::mem::discriminant(&self.dialog), self.home) {
            self.repaint_editors(&ctx);
        }
    }

    /// Draw the same editor in any visible window. Project dialogs, Home and the assistant
    /// belong to the current main host, whose native identity can survive the original window.
    /// Arrangement forms stay in their initiating host and block edits in every window.
    /// Blocked project actions disable editor controls, while native window interactions remain
    /// enabled above project dialogs and still route Close through the shared lifecycle policy.
    fn editor_host_frame(&mut self, ui: &mut Ui, integrated_titlebar: bool) {
        let ctx = ui.ctx().clone();
        self.editor_window_geometry(&ctx);
        let main = self.views.host == self.views.main;
        let before = (self.session.revision(), self.preferences, std::mem::discriminant(&self.dialog), self.home);
        let had_dialog = !matches!(self.dialog, Dialog::None);
        let was_home = self.home;
        self.pointer = self.pointer_next.take();
        self.editor_window_focus(&ctx);
        self.views.sync(&self.session);
        self.editor_pointer_focus(&ctx);
        #[cfg(not(target_arch = "wasm32"))]
        let assistant_keys = main && self.assistant_shortcuts(&ctx);
        #[cfg(target_arch = "wasm32")]
        let assistant_keys = false;
        if main || (!self.home && !had_dialog) {
            self.shortcuts(&ctx, assistant_keys, self.views.active.id());
        }
        let integrated = std::mem::replace(&mut self.integrated_titlebar, integrated_titlebar);
        let enabled = !self.project_action_pending() && (main || (!self.home && !had_dialog)) && self.arrangement_dialog_host().is_none();
        self.app_bar(ui, enabled);
        ui.add_enabled_ui(enabled, |ui| {
            let workspace = if self.home && main {
                self.status(ui);
                self.home_page(ui);
                None
            } else {
                self.ribbon(ui);
                self.views.sync(&self.session);
                if !matches!(self.dialog, Dialog::None) {
                    self.views.cancel_gestures();
                }
                if egui::DragAndDrop::payload::<PartDrag>(&ctx).is_some_and(|drag| !self.views.active.accepts(drag.source, &self.session)) {
                    egui::DragAndDrop::clear_payload(&ctx);
                }
                self.status(ui);
                let workspace = ui.available_rect_before_wrap();
                self.editor_workspace(ui, workspace);
                Some(workspace)
            };
            #[cfg(not(target_arch = "wasm32"))]
            if main {
                let can_edit = workspace.is_some() && matches!(self.dialog, Dialog::None) && !self.files.busy() && !self.project_action_pending();
                self.assistant.show(&ctx, workspace, &self.session, can_edit);
                self.serve_chat(&ctx);
                if let Some(request) = self.assistant.take_settings_request() {
                    self.dialog = match request {
                        assistant::SettingsRequest::Settings => Dialog::Preferences(preferences::PreferencesWindow::at_ai(self.preferences)),
                        assistant::SettingsRequest::Add(form) => Dialog::Agent { form, return_to: None },
                    };
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    ctx.request_repaint();
                }
            }
            #[cfg(target_arch = "wasm32")]
            let _ = workspace;
        });
        self.integrated_titlebar = integrated;
        let dialog_host = self.arrangement_dialog_host().unwrap_or(self.views.main);
        if self.views.host == dialog_host {
            self.project_dialogs(&ctx);
        } else if (!had_dialog && !matches!(self.dialog, Dialog::None)) || (!was_home && self.home) {
            ctx.send_viewport_cmd_to(editor_windows::viewport(dialog_host), egui::ViewportCommand::Focus);
        }
        if integrated_titlebar {
            shell::resize_window_edges(ui);
        }
        self.views.sync(&self.session);
        if ctx.viewport_id() != egui::ViewportId::ROOT && self.pointer != self.pointer_next {
            ctx.request_repaint();
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
        if before != (self.session.revision(), self.preferences, std::mem::discriminant(&self.dialog), self.home) {
            self.repaint_editors(&ctx);
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
    }
}

/// The grid point under the pointer when it is over this canvas and not over a panel floating on
/// it, snapped by the pane's own camera as a click there would be. Call it before drawing the
/// canvas, while `ui` still has the canvas's whole rectangle available.
fn pointer_on_canvas(ui: &Ui, camera: &schematic_canvas::Camera) -> Option<pcbcraft_engine::schematic::Point> {
    let rect = ui.available_rect_before_wrap();
    let pointer = ui.input(|input| input.pointer.hover_pos())?;
    ui.rect_contains_pointer(rect).then(|| camera.snap(pointer, rect))
}

/// The keys the window answers to, as (section, action, keys), for the Shortcuts page.
/// Browser builds leave out the shortcuts the browser keeps for itself.
fn shortcut_list(ctx: &egui::Context) -> Vec<(&'static str, &'static str, String)> {
    let key = |shortcut: &KeyboardShortcut| ctx.format_shortcut(shortcut);
    let mut list = Vec::new();
    if !IN_BROWSER {
        list.push(("Project", "New project", key(&COMMAND_N)));
        list.push(("Project", "Open project", key(&COMMAND_O)));
        list.push(("Project", "Save project", key(&COMMAND_S)));
        list.push(("Project", "Save project as", key(&SAVE_AS)));
    }
    list.push(("Project", "Preferences", key(&COMMAND_COMMA)));
    if !IN_BROWSER {
        list.push(("Project", "Quit", key(&COMMAND_Q)));
    }
    list.push(("Edit", "Undo", key(&COMMAND_Z)));
    list.push(("Edit", "Redo", key(&COMMAND_Y)));
    list.push(("Schematic", "Add symbol", key(&ADD_SYMBOL)));
    list.push(("Schematic", "Place hierarchical sheet", key(&ADD_SHEET)));
    list.push(("Schematic", "Place hierarchical label", key(&ADD_HIERARCHICAL_LABEL)));
    list.push(("Schematic", "Draw wire", key(&ADD_WIRE)));
    list.push(("Schematic", "Disconnect and move symbol", "M".to_owned()));
    list.push(("Schematic", "Drag wire segment", "G".to_owned()));
    list.push(("Schematic", "Rotate the selected symbol", key(&ROTATE)));
    list.push(("Schematic", "Edit the selected symbol's fields", key(&EDIT_FIELDS)));
    list.push(("Schematic", "Delete selected items or segment", "Del".to_owned()));
    list.push(("Schematic", "Cancel the current tool", "Esc".to_owned()));
    list.push(("PCB layout", "Move footprint", "M or left drag".to_owned()));
    list.push(("PCB layout", "Rotate footprint counterclockwise", "R".to_owned()));
    list.push(("PCB layout", "Rotate footprint clockwise", "Shift+R".to_owned()));
    list.push(("PCB layout", "Cancel placement", "Esc or right click".to_owned()));
    list.push(("View", "Zoom", "Mouse wheel".to_owned()));
    list.push(("View", "Pan", "Middle-button drag".to_owned()));
    list.push(("View", "Finer grid", key(&FINER_GRID)));
    list.push(("View", "Coarser grid", key(&COARSER_GRID)));
    list.push(("View", "Switch units", key(&SWITCH_UNITS)));
    list.push(("View", "Back from Home", "Esc".to_owned()));
    list
}

impl eframe::App for PcbApp {
    /// Run shared project logic and visible hosts, then release a dismissed native root and
    /// flush changed presentation settings and desktop history without an electrical edit or save.
    fn ui(&mut self, ui: &mut Ui, frame: &mut Frame) {
        let before = self.preferences;
        self.frame(ui);
        #[cfg(not(target_arch = "wasm32"))]
        if !self.views.hosts.contains_key(&editor_layout::HostId::ROOT) {
            // Release the native surface after the frame; surviving host/window IDs stay put.
            frame.retire_root_window();
        }
        let changed = self.preferences != before;
        #[cfg(not(target_arch = "wasm32"))]
        let changed = self.assistant.take_settings_changed() || changed;
        #[cfg(not(target_arch = "wasm32"))]
        let changed = self.files.recent.take_changed() || changed;
        // eframe's own save runs every 30 seconds, too late if the page reloads right after a change.
        let store_layout =
            self.arrangements.dirty && ui.ctx().input(|input| input.time - self.arrangements.last_flush >= 1.0 || input.pointer.any_released());
        if (changed || store_layout)
            && let Some(storage) = frame.storage_mut()
        {
            self.save(storage);
            storage.flush();
            self.arrangements.dirty = false;
            self.arrangements.last_flush = ui.ctx().input(|input| input.time);
        }
    }

    /// Persist presentation state and desktop history; unsupported arrangement records stay intact.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.checkpoint_arrangements();
        if let Some(text) = self.arrangements.storage_text() {
            storage.set_string(arrangements::STORAGE_KEY, text);
        }
        storage.set_string(PREFERENCES_KEY, self.preferences.to_text());
        #[cfg(not(target_arch = "wasm32"))]
        {
            storage.set_string(RECENT_PROJECTS_KEY, self.files.recent.to_text());
            storage.set_string(ASSISTANT_SETTINGS_KEY, self.assistant.settings_text());
            storage.set_string(ASSISTANT_LAYOUT_KEY, self.assistant.layout_setting());
        }
    }
}

#[cfg(test)]
mod tests;
