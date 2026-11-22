//! The person's settings and the Preferences window. They persist through eframe's storage: a
//! file on the desktop, local storage in the browser. Changes apply as they are made; Cancel
//! puts back what was there when the window opened or at the last Apply.

use egui::Ui;
use pcbcraft_chrome::{
    grid,
    i18n::{self, LanguageChoice},
    menu::MenuIcon,
    settings::{self, Footer, Page, WindowState},
    units::Unit,
    workspace::PanelStyle,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    /// Follow the desktop or browser.
    System,
    /// The family's default look.
    #[default]
    Dark,
    Light,
}

/// The interface scale presets, in percent, as in SolidCraft. The slider moves in 5% steps.
const SCALE_PRESETS: [u16; 6] = [75, 100, 110, 125, 150, 200];
const SCALE_RANGE: std::ops::RangeInclusive<u16> = 75..=200;
const ZOOM_SPEED_RANGE: std::ops::RangeInclusive<u16> = 50..=200;

/// What the scroll wheel does on the canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScrollAction {
    #[default]
    Zoom,
    /// Scrolling pans; hold Ctrl (⌘ on macOS) to zoom. Suits trackpads.
    Pan,
}

/// The point that stays still while zooming.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZoomAround {
    #[default]
    Pointer,
    Center,
}

/// Canvas navigation. The defaults keep the wheel zooming around the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Navigation {
    pub scroll: ScrollAction,
    pub reverse_zoom: bool,
    /// Wheel zoom speed in percent, in 10% steps.
    pub zoom_speed_percent: u16,
    pub zoom_around: ZoomAround,
    /// Dragging with the middle button zooms up and down instead of panning. Off keeps the
    /// KiCad-style middle-button pan; the wheel, pinch and Fit always stay available.
    pub middle_drag_zoom: bool,
    /// Right-button drag pans the canvas instead of starting a selection box. A right-click
    /// without a drag still opens the canvas menu and cancels an active tool.
    pub right_drag_pan: bool,
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            scroll: ScrollAction::Zoom,
            reverse_zoom: false,
            zoom_speed_percent: 100,
            zoom_around: ZoomAround::Pointer,
            middle_drag_zoom: false,
            right_drag_pan: false,
        }
    }
}

impl Navigation {
    /// The wheel zoom speed as a multiplier.
    pub fn zoom_speed(self) -> f32 {
        f32::from(self.zoom_speed_percent) / 100.0
    }
}

/// Which views may pan to reveal a selection. The serialized names retain compatibility with
/// the original two-view settings; secondary now means every view except the primary role.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionFollow {
    #[default]
    Off,
    Both,
    SecondaryFollowsPrimary,
    PrimaryFollowsSecondary,
}

/// How paper is distinguished from the surrounding editing canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperBackground {
    Shaded,
    Custom,
    #[default]
    #[serde(other)]
    Uniform,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub theme: ThemeChoice,
    /// Interface language preference (`system`, `en`, or `he`).
    pub language: LanguageChoice,
    pub scale_percent: u16,
    pub navigation: Navigation,
    /// How strongly visible canvas grids show, in percent; 0 hides them. Shared across views
    /// and windows, this changes drawing only, never snapping, cameras or project history.
    pub grid_opacity_percent: u8,
    /// Schematic grid visibility is independent of PCB grid visibility and snapping.
    pub schematic_grid_visible: bool,
    /// Whether the schematic grid continues on the canvas outside the sheet's paper. On by
    /// default, so parts can still be arranged on a grid off the paper; off leaves a clean
    /// surround. Drawing only: snapping is the same either way.
    pub schematic_grid_outside_paper: bool,
    /// Paper appearance is personal presentation, never project data or a placement boundary.
    pub schematic_paper_background: PaperBackground,
    /// Opaque custom paper colours are remembered separately for each theme; None follows
    /// the theme's shaded-paper colour. They remain saved while another background mode is used.
    pub schematic_paper_color_light: Option<[u8; 3]>,
    pub schematic_paper_color_dark: Option<[u8; 3]>,
    /// Canvas overrides also follow their active theme until a colour is explicitly chosen.
    pub schematic_canvas_color_light: Option<[u8; 3]>,
    pub schematic_canvas_color_dark: Option<[u8; 3]>,
    /// PCB grid visibility preserves the selected dot or line style when hidden.
    pub board_grid_visible: bool,
    /// Display style for every PCB view. Off is the default, including for older saved settings.
    /// Schematic views retain their dot grid; snap spacing stays independent in each view.
    pub board_grid_style: grid::Style,
    /// The schematic snap spacing last chosen in any schematic view, in nanometres. Every
    /// schematic view starts from it when the app starts; after that each view keeps its own
    /// spacing through focus changes, splitting and reopening files. It is never saved in the
    /// project and is independent of [`Self::board_grid_nm`].
    pub schematic_grid_nm: i64,
    /// The PCB snap spacing last chosen in any board view, in nanometres. Board views start from
    /// it when the app starts and keep their own spacing afterward, independently of the
    /// schematic preference. It is presentation only and never saved in the project.
    pub board_grid_nm: i64,
    /// Clean up touched wire routes after editing. Explicit Heal is available in either mode.
    /// This is a user preference, never part of a project or its undo history.
    pub auto_heal_wires: bool,
    /// Automatic counterpart navigation; highlighting remains enabled when this is Off.
    pub selection_follow: SelectionFollow,
    /// The unit lengths and coordinates are shown in, and that numbers typed without a unit are
    /// read in. It changes only what is shown, never stored geometry or the project.
    pub units: Unit,
    /// How the workspace shell is framed: Floating keeps the ribbon, workspace tile and
    /// edge-anchored cards as separate cards with gaps and shadows, while Compact joins them to
    /// the window edge and each other. Cards placed over the canvas always float, and each view
    /// retains its own dock side and floating positions.
    pub panel_style: PanelStyle,
    /// Minutes between recovery snapshots of unsaved work, one of
    /// [`INTERVAL_MINUTES`]. They go to the recovery recorder, not the ordinary project save.
    /// 0, the default, attaches no recorder and records nothing; a long synchronous GUI edit
    /// requests an early snapshot only when an interval is set (live/Assistant, direct session
    /// and asynchronous edits are not timed by that hook).
    pub autosave_minutes: u8,
    /// Minutes between automatic saves of a project that already has a folder or `.pc` file,
    /// through the ordinary save; one of [`INTERVAL_MINUTES`]. 0, the default, never saves on
    /// its own.
    pub save_to_project_minutes: u8,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::default(),
            language: LanguageChoice::default(),
            scale_percent: 100,
            navigation: Navigation::default(),
            grid_opacity_percent: grid::OPACITY_DEFAULT,
            schematic_grid_visible: true,
            schematic_grid_outside_paper: true,
            schematic_paper_background: PaperBackground::Uniform,
            schematic_paper_color_light: None,
            schematic_paper_color_dark: None,
            schematic_canvas_color_light: None,
            schematic_canvas_color_dark: None,
            board_grid_visible: true,
            board_grid_style: grid::Style::Off,
            schematic_grid_nm: grid::SCHEMATIC_SPACING_DEFAULT_NM,
            board_grid_nm: grid::BOARD_SPACING_DEFAULT_NM,
            auto_heal_wires: true,
            selection_follow: SelectionFollow::default(),
            units: Unit::default(),
            panel_style: PanelStyle::default(),
            autosave_minutes: DEFAULT_SNAPSHOT_MINUTES,
            save_to_project_minutes: 0,
        }
    }
}

/// The intervals, in minutes, a person may choose for timed snapshots or saves. 0 is Off.
pub(crate) const INTERVAL_MINUTES: [u8; 4] = [0, 1, 5, 10];
/// Recovery snapshots are Off by default until PCBCraft can offer the recorded work back after a
/// crash; until then an entry would only take up space.
const DEFAULT_SNAPSHOT_MINUTES: u8 = 0;

/// Whether `minutes` is one of the intervals Preferences offers for snapshots and saves.
pub(crate) fn valid_interval(minutes: u8) -> bool {
    INTERVAL_MINUTES.contains(&minutes)
}

impl Preferences {
    /// Effective drawing opacity for a workspace, preserving the chosen opacity while hidden.
    pub(crate) fn grid_opacity(self, board: bool) -> u8 {
        let visible = if board { self.board_grid_visible } else { self.schematic_grid_visible };
        if visible { self.grid_opacity_percent } else { 0 }
    }

    /// Resolve paper/canvas colours and effective grid visibility for a schematic view.
    /// These inputs affect painting only, never snapping or project geometry.
    pub(crate) fn schematic_display(self) -> crate::schematic_canvas::Display {
        crate::schematic_canvas::Display {
            opacity: self.grid_opacity(false),
            outside_paper: self.schematic_grid_outside_paper,
            appearance: self.paper(),
        }
    }

    /// Resolve canvas and paper colours from the current palette, including System theme changes.
    /// Missing overrides follow the theme. Uniform mode uses the canvas for the whole drawing area.
    pub(crate) fn paper(self) -> pcbcraft_chrome::paper::Appearance {
        let t = pcbcraft_chrome::Tokens::get();
        let dark = pcbcraft_chrome::theme::is_dark();
        let color = |rgb: Option<[u8; 3]>| rgb.map(|[r, g, b]| egui::Color32::from_rgb(r, g, b));
        let canvas = color(if dark { self.schematic_canvas_color_dark } else { self.schematic_canvas_color_light }).unwrap_or(t.canvas);
        let paper = match self.schematic_paper_background {
            PaperBackground::Uniform => None,
            PaperBackground::Shaded => Some(t.schematic_paper),
            PaperBackground::Custom => {
                Some(color(if dark { self.schematic_paper_color_dark } else { self.schematic_paper_color_light }).unwrap_or(t.schematic_paper))
            }
        };
        pcbcraft_chrome::paper::Appearance { canvas, paper }
    }

    /// Whether the workspace currently draws a grid, including legacy opacity-zero and PCB-Off settings.
    pub(crate) fn grid_visible(self, board: bool) -> bool {
        self.grid_opacity(board) > 0 && (!board || self.board_grid_style != grid::Style::Off)
    }

    /// Show or hide one workspace without altering snapping. Showing a legacy opacity-zero grid
    /// restores default opacity while keeping the other workspace hidden; PCB Off starts as dots.
    pub(crate) fn set_grid_visible(&mut self, board: bool, visible: bool) {
        if visible && self.grid_opacity_percent == 0 {
            self.schematic_grid_visible = false;
            self.board_grid_visible = false;
            self.grid_opacity_percent = grid::OPACITY_DEFAULT;
        }
        if board {
            self.board_grid_visible = visible;
            if visible && self.board_grid_style == grid::Style::Off {
                self.board_grid_style = grid::Style::Dots;
            }
        } else {
            self.schematic_grid_visible = visible;
        }
    }

    /// Read saved settings. Anything unreadable falls back to the defaults rather than stopping the
    /// app from starting, a scale or zoom speed off its range or step goes back to 100%, and a
    /// grid opacity above 100%, a grid spacing outside its range or a snapshot or save interval
    /// that Preferences does not offer goes back to the default.
    pub fn parse(text: &str) -> Self {
        let mut preferences: Self = serde_json::from_str(text).unwrap_or_else(|e| {
            log::warn!("preferences: ignoring what was saved: {e}");
            Self::default()
        });
        if !SCALE_RANGE.contains(&preferences.scale_percent) || !preferences.scale_percent.is_multiple_of(5) {
            preferences.scale_percent = 100;
        }
        let speed = preferences.navigation.zoom_speed_percent;
        if !ZOOM_SPEED_RANGE.contains(&speed) || !speed.is_multiple_of(10) {
            preferences.navigation.zoom_speed_percent = 100;
        }
        if !grid::OPACITY_RANGE.contains(&preferences.grid_opacity_percent) {
            preferences.grid_opacity_percent = grid::OPACITY_DEFAULT;
        }
        if !grid::SPACING_RANGE_NM.contains(&preferences.schematic_grid_nm) {
            preferences.schematic_grid_nm = grid::SCHEMATIC_SPACING_DEFAULT_NM;
        }
        if !grid::SPACING_RANGE_NM.contains(&preferences.board_grid_nm) {
            preferences.board_grid_nm = grid::BOARD_SPACING_DEFAULT_NM;
        }
        let defaults = Self::default();
        if !valid_interval(preferences.autosave_minutes) {
            preferences.autosave_minutes = defaults.autosave_minutes;
        }
        if !valid_interval(preferences.save_to_project_minutes) {
            preferences.save_to_project_minutes = defaults.save_to_project_minutes;
        }
        preferences
    }

    pub fn to_text(self) -> String {
        serde_json::to_string(&self).unwrap_or_default()
    }

    /// Whether to use the dark palette. With no answer from the system, dark, like SolidCraft.
    pub fn is_dark(self, system: Option<egui::Theme>) -> bool {
        match self.theme {
            ThemeChoice::Dark => true,
            ThemeChoice::Light => false,
            ThemeChoice::System => system != Some(egui::Theme::Light),
        }
    }

    pub fn zoom(self) -> f32 {
        f32::from(self.scale_percent) / 100.0
    }
}

/// The open Preferences window: its page and search, and the settings Cancel goes back to.
#[derive(Clone, Debug)]
pub(crate) struct PreferencesWindow {
    state: WindowState,
    /// What Cancel restores: the settings when the window opened, or at the last Apply.
    kept: Preferences,
    /// Saving values are inert until Apply or OK; a close can replace this dialog.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) saving: Option<(u8, u8)>,
    /// The slider's displayed value while a pointer gesture still owns it.
    pending_scale: Option<u16>,
    libraries: Option<crate::placement_filters::Editor>,
    pub(crate) return_to_catalog: bool,
}

/// What the window asks the app to do after a frame.
#[derive(Debug)]
pub(crate) enum Outcome {
    Open,
    Closed,
    /// The AI page asked to add an agent. The window waits while that dialog is open.
    #[cfg(not(target_arch = "wasm32"))]
    AddAgent(Box<crate::assistant::AgentForm>),
}

const APPEARANCE: usize = 0;
const NAVIGATION: usize = 1;
const VIEWS: usize = 2;
const SCHEMATIC: usize = 3;
const LIBRARIES: usize = 4;
const SHORTCUTS: usize = 5;
/// Recovery snapshots and timed saves to the project folder. Desktop only: the browser keeps
/// no recovery store and saves by download.
#[cfg(not(target_arch = "wasm32"))]
const SAVING: usize = 6;
/// The AI page follows Shortcuts and Saving, as SolidCraft's Assistant page does. Desktop only.
#[cfg(not(target_arch = "wasm32"))]
const AI: usize = 7;

/// The settings a page draws, and the shortcut list the read-only Shortcuts page shows.
pub(crate) struct PageData<'a> {
    pub preferences: &'a mut Preferences,
    libraries: &'a mut crate::placement_filters::Editor,
    pub shortcuts: &'a [(&'static str, &'static str, String)],
    pending_scale: &'a mut Option<u16>,
    #[cfg(not(target_arch = "wasm32"))]
    saving: &'a mut Option<(u8, u8)>,
    /// The Assistant draws the AI page and may ask to add an agent from it.
    #[cfg(not(target_arch = "wasm32"))]
    pub assistant: &'a mut crate::assistant::Assistant,
    #[cfg(not(target_arch = "wasm32"))]
    pub request: Option<crate::assistant::SettingsRequest>,
}

impl PreferencesWindow {
    /// Open on Appearance, remembering the current settings for Cancel.
    pub(crate) fn new(preferences: Preferences) -> Self {
        Self {
            state: WindowState::default(),
            kept: preferences,
            #[cfg(not(target_arch = "wasm32"))]
            saving: None,
            pending_scale: None,
            libraries: None,
            return_to_catalog: false,
        }
    }

    /// Open placement discovery settings while retaining the originating catalog's browsing state.
    pub(crate) fn at_libraries(preferences: Preferences) -> Self {
        Self { state: WindowState { page: LIBRARIES, query: String::new() }, return_to_catalog: true, ..Self::new(preferences) }
    }
    /// Open on Views from the View ribbon, with the same Apply and Cancel behavior.
    pub(crate) fn at_views(preferences: Preferences) -> Self {
        Self { state: WindowState { page: VIEWS, query: String::new() }, ..Self::new(preferences) }
    }

    /// Open on the Shortcuts page, for Help › Keyboard Shortcuts.
    pub(crate) fn at_shortcuts(preferences: Preferences) -> Self {
        Self { state: WindowState { page: SHORTCUTS, query: String::new() }, ..Self::new(preferences) }
    }

    /// Open on the AI page, for the Assistant's own Settings button.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn at_ai(preferences: Preferences) -> Self {
        Self { state: WindowState { page: AI, query: String::new() }, ..Self::new(preferences) }
    }

    /// Draw the window. OK keeps the changes; Cancel, Esc and × restore the kept settings;
    /// Apply keeps them and stays open; Restore page defaults resets the open page only.
    /// `shortcuts` lists (section, action, keys) for the Shortcuts page. The AI page saves its
    /// own settings as they change, so Cancel doesn't undo them, and asking to add an agent
    /// returns [`Outcome::AddAgent`] with this window left as it was.
    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        preferences: &mut Preferences,
        shortcuts: &[(&'static str, &'static str, String)],
        library_preferences: &mut crate::placement_filters::Settings,
        catalog: &pcbcraft_engine::library::sources::Catalog,
        #[cfg(not(target_arch = "wasm32"))] assistant: &mut crate::assistant::Assistant,
    ) -> Outcome {
        let libraries = self.libraries.get_or_insert_with(|| crate::placement_filters::Editor::new(library_preferences.clone(), catalog));
        #[cfg_attr(target_arch = "wasm32", expect(unused_mut))]
        let mut pages: Vec<Page<PageData<'_>>> = vec![
            Page { icon: MenuIcon::Appearance, label: "Appearance", draw: appearance },
            Page { icon: MenuIcon::Navigation, label: "Navigation", draw: navigation },
            Page { icon: MenuIcon::Settings, label: "Views", draw: views },
            Page { icon: MenuIcon::Settings, label: "Schematic", draw: schematic },
            Page { icon: MenuIcon::Settings, label: "Libraries", draw: library_page },
            Page { icon: MenuIcon::Keyboard, label: "Shortcuts", draw: shortcuts_page },
        ];
        #[cfg(not(target_arch = "wasm32"))]
        pages.push(Page { icon: MenuIcon::Settings, label: "Saving", draw: saving });
        #[cfg(not(target_arch = "wasm32"))]
        pages.push(Page { icon: MenuIcon::Assistant, label: "AI", draw: ai_page });
        #[cfg(not(target_arch = "wasm32"))]
        let saving_page = self.state.page == SAVING;
        #[cfg(target_arch = "wasm32")]
        let saving_page = false;
        // Search results span pages, so there is no single page whose defaults to restore.
        let defaults_enabled = (matches!(self.state.page, APPEARANCE | NAVIGATION | VIEWS | SCHEMATIC | LIBRARIES) || saving_page)
            && !settings::searching(&self.state.query);
        #[cfg(not(target_arch = "wasm32"))]
        let saving_dirty = self.saving.is_some_and(|draft| draft != (preferences.autosave_minutes, preferences.save_to_project_minutes));
        #[cfg(target_arch = "wasm32")]
        let saving_dirty = false;
        let footer = Footer { apply_enabled: *preferences != self.kept || libraries.dirty() || saving_dirty, defaults_enabled };
        let mut data = PageData {
            preferences,
            libraries,
            shortcuts,
            pending_scale: &mut self.pending_scale,
            #[cfg(not(target_arch = "wasm32"))]
            saving: &mut self.saving,
            #[cfg(not(target_arch = "wasm32"))]
            assistant,
            #[cfg(not(target_arch = "wasm32"))]
            request: None,
        };
        let action = settings::paged_window(ctx, "preferences_window", "Preferences", &mut self.state, &pages, &mut data, footer);
        // Discard an interrupted drag even when search or a page switch hid the slider.
        if action != settings::Action::None || !ctx.input(|input| input.focused && input.pointer.primary_down()) {
            *data.pending_scale = None;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(crate::assistant::SettingsRequest::Add(form)) = data.request.take()
            && action == settings::Action::None
        {
            return Outcome::AddAgent(form);
        }
        if matches!(action, settings::Action::Apply | settings::Action::Ok) {
            match data.libraries.value() {
                Ok(value) => {
                    *library_preferences = value.clone();
                    data.libraries.applied(value);
                    #[cfg(not(target_arch = "wasm32"))]
                    if let Some((snapshots, project)) = data.saving.take() {
                        data.preferences.autosave_minutes = snapshots;
                        data.preferences.save_to_project_minutes = project;
                    }
                }
                Err(error) => {
                    data.libraries.error = Some(error);
                    return Outcome::Open;
                }
            }
        }
        match action {
            settings::Action::None => Outcome::Open,
            settings::Action::Ok => Outcome::Closed,
            settings::Action::Apply => {
                self.kept = *data.preferences;
                Outcome::Open
            }
            settings::Action::Cancel => {
                // Explicit live preference changes must survive Cancel, too.
                *data.preferences = Preferences {
                    autosave_minutes: data.preferences.autosave_minutes,
                    save_to_project_minutes: data.preferences.save_to_project_minutes,
                    ..self.kept
                };
                Outcome::Closed
            }
            settings::Action::Defaults => {
                if self.state.page == LIBRARIES {
                    data.libraries.defaults();
                }
                #[cfg(not(target_arch = "wasm32"))]
                if saving_page {
                    let mut defaults = *data.preferences;
                    restore_page_defaults(&mut defaults, SAVING);
                    *data.saving = Some((defaults.autosave_minutes, defaults.save_to_project_minutes));
                    return Outcome::Open;
                }
                restore_page_defaults(data.preferences, self.state.page);
                Outcome::Open
            }
        }
    }
}

/// Reset the settings on one page and leave the others as they are.
fn restore_page_defaults(preferences: &mut Preferences, page: usize) {
    let defaults = Preferences::default();
    match page {
        APPEARANCE => {
            preferences.theme = defaults.theme;
            preferences.language = defaults.language;
            preferences.scale_percent = defaults.scale_percent;
            preferences.grid_opacity_percent = defaults.grid_opacity_percent;
            preferences.schematic_grid_visible = defaults.schematic_grid_visible;
            preferences.schematic_grid_outside_paper = defaults.schematic_grid_outside_paper;
            preferences.schematic_paper_background = defaults.schematic_paper_background;
            preferences.schematic_paper_color_light = defaults.schematic_paper_color_light;
            preferences.schematic_paper_color_dark = defaults.schematic_paper_color_dark;
            preferences.schematic_canvas_color_light = defaults.schematic_canvas_color_light;
            preferences.schematic_canvas_color_dark = defaults.schematic_canvas_color_dark;
            preferences.board_grid_visible = defaults.board_grid_visible;
            preferences.board_grid_style = defaults.board_grid_style;
            preferences.units = defaults.units;
            preferences.panel_style = defaults.panel_style;
        }
        NAVIGATION => preferences.navigation = defaults.navigation,
        VIEWS => preferences.selection_follow = defaults.selection_follow,
        SCHEMATIC => preferences.auto_heal_wires = defaults.auto_heal_wires,
        #[cfg(not(target_arch = "wasm32"))]
        SAVING => {
            preferences.autosave_minutes = defaults.autosave_minutes;
            preferences.save_to_project_minutes = defaults.save_to_project_minutes;
        }
        _ => {}
    }
}

/// The interval choices for a snapshot or save setting, Off first.
#[cfg(not(target_arch = "wasm32"))]
const INTERVAL_CHOICES: [(u8, &str); 4] = [(0, "Off"), (1, "1 min"), (5, "5 min"), (10, "10 min")];

/// Recovery snapshots of unsaved work, and the opt-in timed save to the project folder.
#[cfg(not(target_arch = "wasm32"))]
fn saving(ui: &mut Ui, data: &mut PageData<'_>) {
    settings::heading(ui, "Saving", "How PCBCraft keeps unsaved work safe.");
    let effective = (data.preferences.autosave_minutes, data.preferences.save_to_project_minutes);
    let mut draft = data.saving.unwrap_or(effective);
    settings::section(ui, "Recovery", |ui| {
        settings::row_with_keywords(ui, "Recovery snapshots", &["autosave", "crash", "recover", "backup", "interval"], |ui| {
            settings::segmented(ui, &mut draft.0, &INTERVAL_CHOICES);
        });
        settings::hint(
            ui,
            "Keeps a copy of unsaved work in a recovery folder outside the project, renewed at this interval and right after a slow change. Off keeps nothing. Copies left by earlier sessions are removed after 14 days. Restoring a copy after a crash is not available yet.",
        );
    });
    settings::section(ui, "Project", |ui| {
        settings::row_with_keywords(ui, "Save to project", &["autosave", "automatic", "save", "interval"], |ui| {
            settings::segmented(ui, &mut draft.1, &INTERVAL_CHOICES);
        });
        settings::hint(
            ui,
            "Saves a project that already has a folder or .pc file after it has been changed, the same way Save does. A project changed by another program is not overwritten.",
        );
    });
    *data.saving = (draft != effective).then_some(draft);
}

/// Automatic wire cleanup shares the Modify menu's saved preference. Disabling it preserves
/// authored detours and loops; explicit Heal remains available without changing this setting.
fn schematic(ui: &mut Ui, data: &mut PageData<'_>) {
    settings::section(ui, "Wiring", |ui| {
        settings::row_with_keywords(ui, "Auto-heal wires", &["smart routing", "cleanup", "detours", "loops", "overlaps"], |ui| {
            settings::checkbox(ui, "Enabled", "Auto-heal wires", &mut data.preferences.auto_heal_wires);
        });
        settings::hint(
            ui,
            "Simplifies overlapping routes, detours and loops while drawing or moving. Turn off to preserve their shape; use Heal for manual cleanup.",
        );
    });
}

/// Edit one opaque colour override. An untouched picker or reset keeps the value absent so
/// subsequent theme changes still use the theme palette.
fn paper_color(ui: &mut Ui, name: &str, saved: &mut Option<[u8; 3]>, theme_color: egui::Color32) {
    let [r, g, b, _] = theme_color.to_array();
    let mut rgb = saved.unwrap_or([r, g, b]);
    if settings::color(ui, name, &mut rgb).changed() {
        *saved = Some(rgb);
    }
    if ui.add_enabled(saved.is_some(), settings::action_button("Use theme", false)).clicked() {
        *saved = None;
    }
}

/// Theme, scale, panel style, grid display, schematic paper/canvas colours and units.
/// The PCB style reflects effective visibility; choosing dots or lines restores a hidden grid.
fn appearance(ui: &mut Ui, data: &mut PageData<'_>) {
    settings::heading(ui, "Appearance", "How PCBCraft looks. Cancel restores your previous settings.");
    let preferences = &mut *data.preferences;
    settings::section(ui, "Theme", |ui| {
        settings::row(ui, "Theme", |ui| {
            settings::segmented(
                ui,
                &mut preferences.theme,
                &[(ThemeChoice::System, "System"), (ThemeChoice::Dark, "Dark"), (ThemeChoice::Light, "Light")],
            );
        });
        settings::hint(ui, "System follows your desktop or browser.");
    });
    settings::section(ui, "Schematic canvas", |ui| {
        settings::row_with_keywords(ui, "Paper background", &["sheet", "shade", "uniform", "color", "colour"], |ui| {
            settings::segmented(
                ui,
                &mut preferences.schematic_paper_background,
                &[(PaperBackground::Uniform, "Uniform"), (PaperBackground::Shaded, "Shaded"), (PaperBackground::Custom, "Custom")],
            );
        });
        let t = pcbcraft_chrome::Tokens::get();
        let dark = pcbcraft_chrome::theme::is_dark();
        settings::row_with_keywords(ui, "Canvas color", &["background", "schematic", "theme", "colour"], |ui| {
            let saved = if dark { &mut preferences.schematic_canvas_color_dark } else { &mut preferences.schematic_canvas_color_light };
            paper_color(ui, "Canvas color", saved, t.canvas);
        });
        settings::row_with_keywords(ui, "Sheet color", &["paper", "background", "custom", "colour"], |ui| {
            ui.add_enabled_ui(preferences.schematic_paper_background == PaperBackground::Custom, |ui| {
                let saved = if dark { &mut preferences.schematic_paper_color_dark } else { &mut preferences.schematic_paper_color_light };
                paper_color(ui, "Sheet color", saved, t.schematic_paper);
            });
        });
        settings::row_with_keywords(ui, "Show grid outside paper", &["schematic", "sheet", "page", "surround", "off-paper"], |ui| {
            settings::checkbox(ui, "Enabled", "Show grid outside paper", &mut preferences.schematic_grid_outside_paper);
        });
    });
    settings::section(ui, "Scale", |ui| {
        settings::row(ui, "Interface scale", |ui| {
            interface_scale(ui, &mut preferences.scale_percent, data.pending_scale);
        });
        settings::row(ui, "Scale presets", |ui| {
            for preset in SCALE_PRESETS {
                if ui.selectable_label(preferences.scale_percent == preset, format!("{preset}%")).clicked() {
                    preferences.scale_percent = preset;
                    *data.pending_scale = None;
                }
            }
        });
    });
    settings::section(ui, "Panels", |ui| {
        settings::row_with_keywords(ui, "Panel style", &["floating", "compact", "docked", "cards", "panels"], |ui| {
            settings::segmented(ui, &mut preferences.panel_style, &[(PanelStyle::Floating, "Floating"), (PanelStyle::Compact, "Compact")]);
        });
        settings::hint(
            ui,
            "Floating separates the ribbon and panels into cards. Compact joins them to the window edge and each other. Cards over the canvas stay floating.",
        );
    });
    settings::section(ui, "Canvas", |ui| {
        settings::row_with_keywords(ui, "PCB grid", &["board", "layout", "grid", "dots", "lines", "hide"], |ui| {
            let mut style = if preferences.grid_visible(true) { preferences.board_grid_style } else { grid::Style::Off };
            if settings::segmented(ui, &mut style, &[(grid::Style::Off, "Off"), (grid::Style::Dots, "Dots"), (grid::Style::Lines, "Lines")]) {
                if style != grid::Style::Off {
                    preferences.board_grid_style = style;
                }
                preferences.set_grid_visible(true, style != grid::Style::Off);
            }
        });
        settings::row_with_keywords(ui, "Grid opacity", &["transparency", "fade", "dots"], |ui| {
            grid::opacity_control(ui, &mut preferences.grid_opacity_percent);
        });
        settings::hint(ui, "Only the grid fades. Snapping still follows the grid at 0%.");
        settings::row_with_keywords(ui, "Units", &["mil", "mm", "millimetres", "millimeters", "metric", "imperial", "inch"], |ui| {
            settings::segmented(ui, &mut preferences.units, &[(Unit::Mil, "mil"), (Unit::Mm, "mm")]);
        });
        settings::hint(ui, "Coordinates and the grid show in this unit, and numbers typed without one use it. Stored geometry never changes.");
    });
    settings::section(ui, "Language", |ui| {
        let options = i18n::language_options(preferences.language);
        let refs: Vec<(LanguageChoice, &str)> = options.iter().map(|(choice, label)| (*choice, label.as_str())).collect();
        settings::row_with_keywords(ui, "Language", &["language", "locale", "hebrew", "rtl", "עברית"], |ui| {
            settings::choice(ui, "Language", &mut preferences.language, &refs);
        });
        settings::hint(ui, "System follows your desktop or browser language.");
    });
}

/// Preview the percentage in the slider during a pointer drag and apply it on release.
/// Keyboard steps apply immediately, while the number field applies a typed value on confirmation.
fn interface_scale(ui: &mut Ui, scale: &mut u16, pending: &mut Option<u16>) {
    let mut value = pending.take().unwrap_or(*scale);
    let slider = egui::Slider::new(&mut value, SCALE_RANGE).step_by(5.0).suffix("%").update_while_editing(false);
    let response = ui.add(slider).accessible_name("Interface scale");
    if !ui.input(|input| input.focused) {
        return;
    }
    if response.is_pointer_button_down_on() {
        *pending = Some(value);
    } else if response.drag_stopped_by(egui::PointerButton::Primary) || response.changed() {
        *scale = value;
    }
}

/// How the scroll wheel and pointer move around the schematic.
fn navigation(ui: &mut Ui, data: &mut PageData<'_>) {
    settings::heading(ui, "Navigation", "How the mouse and trackpad move around the canvas.");
    let navigation = &mut data.preferences.navigation;
    settings::section(ui, "Mouse wheel", |ui| {
        settings::row(ui, "Scrolling", |ui| {
            settings::segmented(ui, &mut navigation.scroll, &[(ScrollAction::Zoom, "Zooms"), (ScrollAction::Pan, "Pans")]);
        });
        if navigation.scroll == ScrollAction::Pan {
            settings::hint(ui, "Hold Ctrl and scroll to zoom. Pinch zooms on a trackpad.");
        }
        settings::row(ui, "Zoom direction", |ui| {
            settings::segmented(ui, &mut navigation.reverse_zoom, &[(false, "Normal"), (true, "Reversed")]);
        });
        settings::row(ui, "Zoom speed", |ui| {
            ui.add(egui::Slider::new(&mut navigation.zoom_speed_percent, ZOOM_SPEED_RANGE).step_by(10.0).suffix("%")).accessible_name("Zoom speed");
        });
    });
    settings::section(ui, "Zoom", |ui| {
        settings::row(ui, "Zoom around", |ui| {
            settings::segmented(ui, &mut navigation.zoom_around, &[(ZoomAround::Pointer, "Pointer"), (ZoomAround::Center, "Center of view")]);
        });
    });
    settings::section(ui, "Mouse buttons", |ui| {
        settings::row_with_keywords(ui, "Middle-button drag", &["middle", "drag", "pan", "zoom"], |ui| {
            settings::segmented(ui, &mut navigation.middle_drag_zoom, &[(false, "Pan"), (true, "Zoom")]);
        });
        settings::row_with_keywords(ui, "Right-button drag", &["right", "drag", "pan", "select", "box"], |ui| {
            settings::segmented(ui, &mut navigation.right_drag_pan, &[(false, "Select"), (true, "Pan view")]);
        });
        if navigation.middle_drag_zoom {
            // The drag follows Zoom direction, so the hint does too.
            let hint = if navigation.reverse_zoom {
                "Drag down to zoom in and up to zoom out. The wheel still follows Mouse wheel."
            } else {
                "Drag up to zoom in and down to zoom out. The wheel still follows Mouse wheel."
            };
            settings::hint(ui, hint);
        }
        if navigation.right_drag_pan {
            settings::hint(ui, "A right drag pans. Right-click menus and tool cancellation are unchanged.");
        }
    });
}

/// Selection following is navigation only, with stable roles even when focus changes.
fn views(ui: &mut Ui, data: &mut PageData<'_>) {
    settings::heading(
        ui,
        "Views",
        "Following reveals selections without changing zoom. Primary is the original view, even when detached; closing it transfers that role to the main window’s focused view.",
    );
    settings::section(ui, "Selection", |ui| {
        settings::row_with_keywords(ui, "Follow selection", &["split", "pan", "highlight", "primary", "secondary"], |ui| {
            settings::choice(
                ui,
                "Follow selection mode",
                &mut data.preferences.selection_follow,
                &[
                    (SelectionFollow::Off, "Highlight only"),
                    (SelectionFollow::Both, "All views"),
                    (SelectionFollow::SecondaryFollowsPrimary, "Other views follow primary"),
                    (SelectionFollow::PrimaryFollowsSecondary, "Primary follows other views"),
                ],
            );
        });
    });
}

/// The Assistant's settings, drawn by the Assistant. While searching, one row stands in for them
/// so "agent" or "assistant" still finds this page.
#[cfg(not(target_arch = "wasm32"))]
fn ai_page(ui: &mut Ui, data: &mut PageData<'_>) {
    if settings::is_searching(ui) {
        settings::row_with_keywords(ui, "AI settings", &["assistant", "agent", "model", "AI features"], |ui| {
            ui.label(egui::RichText::new("Press Enter to open").size(pcbcraft_chrome::Tokens::get().font_small));
        });
        return;
    }
    data.request = data.assistant.settings_page(ui);
}

/// The keys PCBCraft answers to, grouped as in the menus. Read-only for now.
fn shortcuts_page(ui: &mut Ui, data: &mut PageData<'_>) {
    settings::heading(ui, "Shortcuts", "The keys PCBCraft answers to. They can't be changed yet.");
    let mut sections: Vec<&str> = Vec::new();
    for (section, _, _) in data.shortcuts {
        if !sections.contains(section) {
            sections.push(section);
        }
    }
    for section in sections {
        settings::section(ui, section, |ui| {
            for (_, action, keys) in data.shortcuts.iter().filter(|(s, _, _)| *s == section) {
                settings::row_with_keywords(ui, action, &[keys.as_str()], |ui| {
                    settings::key_chip(ui, keys);
                });
            }
        });
    }
}

/// Personal discovery controls apply only after the shared Preferences footer confirms them.
fn library_page(ui: &mut Ui, data: &mut PageData<'_>) {
    data.libraries.show(ui);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Older settings keep their grid defaults. Enabling one legacy opacity-zero grid leaves
    /// the other hidden and supplies a visible PCB style without changing the snap interval.
    #[test]
    fn grid_visibility_restores_legacy_hidden_settings_per_workspace() {
        let mut preferences = Preferences::parse("{\"grid_opacity_percent\":0}");
        assert!(!preferences.grid_visible(false) && !preferences.grid_visible(true));
        let spacing = preferences.schematic_grid_nm;
        preferences.set_grid_visible(true, true);
        assert!(preferences.grid_visible(true));
        assert!(!preferences.grid_visible(false));
        assert_eq!(preferences.board_grid_style, grid::Style::Dots);
        preferences.set_grid_visible(false, true);
        assert!(preferences.grid_visible(false));
        assert_eq!(preferences.schematic_grid_nm, spacing);
        assert_eq!(Preferences::parse(&preferences.to_text()), preferences);
    }

    /// Personal preferences, including the two save cadences, survive a settings round trip.
    #[test]
    fn saved_settings_come_back() {
        let preferences = Preferences {
            theme: ThemeChoice::Light,
            language: LanguageChoice::Hebrew,
            scale_percent: 110,
            navigation: Navigation {
                scroll: ScrollAction::Pan,
                reverse_zoom: true,
                zoom_speed_percent: 150,
                zoom_around: ZoomAround::Center,
                middle_drag_zoom: true,
                right_drag_pan: true,
            },
            grid_opacity_percent: 35,
            schematic_grid_visible: false,
            schematic_grid_outside_paper: false,
            schematic_paper_background: PaperBackground::Custom,
            schematic_paper_color_light: Some([245, 240, 230]),
            schematic_paper_color_dark: Some([30, 40, 50]),
            schematic_canvas_color_light: Some([240, 242, 245]),
            schematic_canvas_color_dark: Some([35, 40, 50]),
            board_grid_visible: true,
            board_grid_style: grid::Style::Dots,
            schematic_grid_nm: 25 * grid::NM_PER_MIL,
            board_grid_nm: 10 * grid::NM_PER_MIL,
            auto_heal_wires: false,
            selection_follow: SelectionFollow::SecondaryFollowsPrimary,
            units: Unit::Mm,
            panel_style: PanelStyle::Compact,
            autosave_minutes: 10,
            save_to_project_minutes: 1,
        };
        for style in [grid::Style::Off, grid::Style::Dots, grid::Style::Lines] {
            let preferences = Preferences { board_grid_style: style, ..preferences };
            assert_eq!(Preferences::parse(&preferences.to_text()), preferences);
        }
        // Settings saved before units existed show mils, as the grid did then.
        assert_eq!(Preferences::parse("{\"grid_opacity_percent\":40}").units, Unit::Mil);
        // Settings saved before paper keep the grid outside it, as the canvas did then.
        assert!(Preferences::parse("{\"grid_opacity_percent\":40}").schematic_grid_outside_paper);
        assert_eq!(Preferences::parse("{\"grid_opacity_percent\":40}").schematic_paper_background, PaperBackground::Uniform);
        let hidden = Preferences { grid_opacity_percent: 0, ..Preferences::default() };
        assert_eq!(Preferences::parse(&hidden.to_text()), hidden);
    }

    /// Theme defaults stay dynamic until explicitly overridden, and custom values remain
    /// separate across theme changes and a round trip through personal storage.
    #[test]
    fn paper_and_canvas_defaults_follow_the_active_theme() {
        let ctx = egui::Context::default();
        let defaults = Preferences::parse("{}");
        let custom = Preferences {
            schematic_paper_background: PaperBackground::Custom,
            schematic_canvas_color_light: Some([240, 242, 245]),
            schematic_canvas_color_dark: Some([25, 30, 40]),
            schematic_paper_color_light: Some([250, 245, 230]),
            schematic_paper_color_dark: Some([35, 45, 55]),
            ..defaults
        };
        let custom = Preferences::parse(&custom.to_text());
        for dark in [true, false] {
            pcbcraft_chrome::theme::apply(&ctx, dark);
            let t = pcbcraft_chrome::Tokens::get();
            assert_eq!(defaults.paper(), pcbcraft_chrome::paper::Appearance { canvas: t.canvas, paper: None });
            let rgb = |rgb: [u8; 3]| egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
            assert_eq!(custom.paper().canvas, rgb(if dark { [25, 30, 40] } else { [240, 242, 245] }));
            assert_eq!(custom.paper().fill(), rgb(if dark { [35, 45, 55] } else { [250, 245, 230] }));
            let follow = Preferences {
                schematic_canvas_color_light: None,
                schematic_canvas_color_dark: None,
                schematic_paper_color_light: None,
                schematic_paper_color_dark: None,
                ..custom
            };
            assert_eq!(follow.paper().canvas, t.canvas);
            assert_eq!(follow.paper().fill(), t.schematic_paper);
            assert_eq!(Preferences { schematic_paper_background: PaperBackground::Uniform, ..custom }.paper().fill(), custom.paper().canvas);
        }
    }

    #[test]
    fn unreadable_settings_fall_back_to_the_defaults() {
        assert_eq!(Preferences::parse("not json"), Preferences::default());
        assert_eq!(Preferences::parse("{\"theme\":\"Plaid\"}"), Preferences::default());
        assert_eq!(Preferences::parse("{\"scale_percent\":9000}"), Preferences::default());
        assert_eq!(Preferences::parse("{\"scale_percent\":112}").scale_percent, 100);
        assert_eq!(Preferences::parse("{\"navigation\":{\"zoom_speed_percent\":55}}").navigation.zoom_speed_percent, 100);
        assert_eq!(Preferences::parse("{\"theme\":\"Dark\"}").theme, ThemeChoice::Dark);
        assert_eq!(Preferences::parse(r#"{"theme":"Light","navigation":{"reverse_zoom":true}}"#).selection_follow, SelectionFollow::Off);
        // Settings saved before Navigation existed keep their theme and scale.
        let older = Preferences::parse("{\"theme\":\"Light\",\"scale_percent\":125}");
        assert_eq!((older.theme, older.scale_percent, older.navigation), (ThemeChoice::Light, 125, Navigation::default()));
        assert_eq!(older.grid_opacity_percent, grid::OPACITY_DEFAULT);
        assert_eq!(older.board_grid_style, grid::Style::Off);
        let future = Preferences::parse(r#"{"theme":"Light","board_grid_style":"FutureStyle"}"#);
        assert_eq!((future.theme, future.board_grid_style), (ThemeChoice::Light, grid::Style::Off));
        assert_eq!(Preferences::parse(r#"{"panel_style":"Compact"}"#), Preferences::default());
        assert_eq!(Preferences::parse("{\"grid_opacity_percent\":101}").grid_opacity_percent, grid::OPACITY_DEFAULT);
        assert_eq!(Preferences::parse("{\"grid_opacity_percent\":-1}"), Preferences::default());
        assert_eq!(older.schematic_grid_nm, grid::SCHEMATIC_SPACING_DEFAULT_NM);
        assert_eq!(older.board_grid_nm, grid::BOARD_SPACING_DEFAULT_NM);
        assert_eq!(Preferences::default().board_grid_nm, 254_000, "the PCB grid defaults to 10 mil");
        assert_eq!(Preferences::parse("{\"schematic_grid_nm\":635000}").schematic_grid_nm, 635_000);
        assert_eq!(Preferences::parse("{\"board_grid_nm\":254000}").board_grid_nm, 254_000);
        // An out-of-range spacing resets only itself; a value of the wrong type is unreadable,
        // so everything falls back.
        for bad in ["0", "-1270000", "999999999999"] {
            let parsed = Preferences::parse(&format!("{{\"schematic_grid_nm\":{bad},\"grid_opacity_percent\":40}}"));
            assert_eq!(parsed.schematic_grid_nm, grid::SCHEMATIC_SPACING_DEFAULT_NM, "{bad}");
            assert_eq!(parsed.grid_opacity_percent, 40, "{bad}");
            let parsed = Preferences::parse(&format!("{{\"board_grid_nm\":{bad},\"grid_opacity_percent\":40}}"));
            assert_eq!(parsed.board_grid_nm, grid::BOARD_SPACING_DEFAULT_NM, "{bad}");
            assert_eq!(parsed.grid_opacity_percent, 40, "{bad}");
        }
        for bad in ["1.5", "\"50 mil\""] {
            let parsed = Preferences::parse(&format!("{{\"schematic_grid_nm\":{bad},\"grid_opacity_percent\":40}}"));
            assert_eq!(parsed, Preferences::default(), "{bad}");
            let parsed = Preferences::parse(&format!("{{\"board_grid_nm\":{bad},\"grid_opacity_percent\":40}}"));
            assert_eq!(parsed, Preferences::default(), "{bad}");
        }
    }

    /// Unoffered u8 intervals reset only that setting; unreadable types reset the whole file.
    /// Settings saved before these fields existed get neither snapshots nor a timed save.
    #[test]
    fn saving_intervals_keep_only_the_offered_values() {
        let older = Preferences::parse("{\"grid_opacity_percent\":40}");
        assert_eq!((older.autosave_minutes, older.save_to_project_minutes), (0, 0));
        for minutes in [0u8, 1, 5, 10] {
            let parsed = Preferences::parse(&format!("{{\"autosave_minutes\":{minutes},\"save_to_project_minutes\":{minutes}}}"));
            assert_eq!((parsed.autosave_minutes, parsed.save_to_project_minutes), (minutes, minutes));
        }
        for bad in [2u8, 4, 6, 11, 255] {
            let parsed = Preferences::parse(&format!("{{\"autosave_minutes\":{bad},\"grid_opacity_percent\":40}}"));
            assert_eq!((parsed.autosave_minutes, parsed.grid_opacity_percent), (0, 40), "{bad}");
            let parsed = Preferences::parse(&format!("{{\"save_to_project_minutes\":{bad},\"grid_opacity_percent\":40}}"));
            assert_eq!((parsed.save_to_project_minutes, parsed.grid_opacity_percent), (0, 40), "{bad}");
        }
        // A value the stored type cannot hold makes the whole file unreadable, as for grid spacing.
        for bad in ["-1", "256", "1.5", "\"5\"", "null"] {
            assert_eq!(Preferences::parse(&format!("{{\"autosave_minutes\":{bad},\"grid_opacity_percent\":40}}")), Preferences::default(), "{bad}");
        }
    }

    #[test]
    fn the_system_theme_is_followed_only_when_asked() {
        let of = |theme| Preferences { theme, ..Preferences::default() };
        assert!(of(ThemeChoice::System).is_dark(None));
        assert!(!of(ThemeChoice::System).is_dark(Some(egui::Theme::Light)));
        assert!(of(ThemeChoice::Dark).is_dark(Some(egui::Theme::Light)));
        assert!(!of(ThemeChoice::Light).is_dark(Some(egui::Theme::Dark)));
    }

    /// Restoring a page resets its settings and preserves settings owned by other pages.
    #[test]
    fn page_defaults_reset_only_that_page() {
        let mut preferences = Preferences {
            theme: ThemeChoice::Light,
            language: LanguageChoice::Hebrew,
            scale_percent: 150,
            navigation: Navigation { reverse_zoom: true, ..Navigation::default() },
            grid_opacity_percent: 20,
            schematic_grid_visible: false,
            schematic_grid_outside_paper: false,
            schematic_paper_background: PaperBackground::Custom,
            schematic_paper_color_light: Some([245, 240, 230]),
            schematic_paper_color_dark: Some([30, 40, 50]),
            schematic_canvas_color_light: Some([240, 242, 245]),
            schematic_canvas_color_dark: Some([35, 40, 50]),
            board_grid_visible: false,
            board_grid_style: grid::Style::Lines,
            schematic_grid_nm: 10 * grid::NM_PER_MIL,
            board_grid_nm: 25 * grid::NM_PER_MIL,
            auto_heal_wires: false,
            selection_follow: SelectionFollow::Both,
            units: Unit::Mm,
            panel_style: PanelStyle::Compact,
            autosave_minutes: 10,
            save_to_project_minutes: 10,
        };
        #[cfg(not(target_arch = "wasm32"))]
        let (expected_autosave, expected_save_to_project) = {
            restore_page_defaults(&mut preferences, SAVING);
            assert_eq!((preferences.autosave_minutes, preferences.save_to_project_minutes), (0, 0));
            assert_eq!(preferences.theme, ThemeChoice::Light, "the Saving reset leaves other pages alone");
            (0, 0)
        };
        #[cfg(target_arch = "wasm32")]
        let (expected_autosave, expected_save_to_project) = (10, 10);
        restore_page_defaults(&mut preferences, NAVIGATION);
        // Grid spacing is chosen in the views, not on a page, so no page reset touches it.
        assert_eq!(preferences.schematic_grid_nm, 10 * grid::NM_PER_MIL);
        assert_eq!(preferences.navigation, Navigation::default());
        assert_eq!((preferences.theme, preferences.scale_percent, preferences.grid_opacity_percent), (ThemeChoice::Light, 150, 20));
        assert_eq!((preferences.selection_follow, preferences.units), (SelectionFollow::Both, Unit::Mm));
        assert_eq!(preferences.panel_style, PanelStyle::Compact, "another page's reset leaves the panel style alone");
        restore_page_defaults(&mut preferences, VIEWS);
        assert_eq!(preferences.selection_follow, SelectionFollow::Off);
        assert_eq!((preferences.theme, preferences.grid_opacity_percent), (ThemeChoice::Light, 20));
        restore_page_defaults(&mut preferences, APPEARANCE);
        assert_eq!(
            preferences,
            Preferences {
                schematic_grid_nm: 10 * grid::NM_PER_MIL,
                board_grid_nm: 25 * grid::NM_PER_MIL,
                auto_heal_wires: false,
                autosave_minutes: expected_autosave,
                save_to_project_minutes: expected_save_to_project,
                ..Preferences::default()
            }
        );
        restore_page_defaults(&mut preferences, SCHEMATIC);
        assert_eq!(
            preferences,
            Preferences {
                schematic_grid_nm: 10 * grid::NM_PER_MIL,
                board_grid_nm: 25 * grid::NM_PER_MIL,
                autosave_minutes: expected_autosave,
                save_to_project_minutes: expected_save_to_project,
                ..Preferences::default()
            }
        );
    }

    #[test]
    fn language_preferences_normalize_legacy_iw_and_reject_non_strings() {
        for choice in [LanguageChoice::System, LanguageChoice::English, LanguageChoice::Hebrew] {
            let prefs = Preferences { language: choice, ..Preferences::default() };
            assert_eq!(Preferences::parse(&prefs.to_text()).language, choice);
        }
        assert_eq!(Preferences::parse(r#"{"language":"iw"}"#).language, LanguageChoice::Hebrew);
        assert_eq!(Preferences::parse(r#"{"language":"iw-IL"}"#).language, LanguageChoice::Hebrew);
        assert_eq!(Preferences::parse(r#"{"language":"iw_IL.UTF-8"}"#).language, LanguageChoice::Hebrew);
        assert_eq!(Preferences::parse(r#"{"language":"en-US"}"#).language, LanguageChoice::English);
        assert_eq!(Preferences::parse(r#"{"language":"x-he"}"#).language, LanguageChoice::System);
        assert_eq!(Preferences::parse(r#"{"language":"unknown-lang"}"#).language, LanguageChoice::System);
        assert_eq!(Preferences::parse(r#"{"theme":"Light","language":123}"#), Preferences::default());
    }
}
