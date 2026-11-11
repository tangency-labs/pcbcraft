//! The person's settings and the Preferences window. They persist through eframe's storage: a
//! file on the desktop, local storage in the browser. Changes apply as they are made; Cancel
//! puts back what was there when the window opened or at the last Apply.

use egui::Ui;
use pcbcraft_chrome::{
    grid,
    menu::MenuIcon,
    settings::{self, Footer, Page, WindowState},
    units::Unit,
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
}

impl Default for Navigation {
    fn default() -> Self {
        Self { scroll: ScrollAction::Zoom, reverse_zoom: false, zoom_speed_percent: 100, zoom_around: ZoomAround::Pointer }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub theme: ThemeChoice,
    pub scale_percent: u16,
    pub navigation: Navigation,
    /// How strongly visible canvas grids show, in percent; 0 hides them. Shared across views
    /// and windows, this changes drawing only, never snapping, cameras or project history.
    pub grid_opacity_percent: u8,
    /// Schematic grid visibility is independent of PCB grid visibility and snapping.
    pub schematic_grid_visible: bool,
    /// PCB grid visibility preserves the selected dot or line style when hidden.
    pub board_grid_visible: bool,
    /// Display style for every PCB view. Off is the default, including for older saved settings.
    /// Schematic views retain their dot grid; snap spacing stays independent in each view.
    pub board_grid_style: grid::Style,
    /// The schematic snap spacing last chosen in any view, in nanometres. Both views start from
    /// it when the app starts; after that each view keeps its own spacing through focus changes,
    /// splitting and reopening files. It is never saved in the project.
    pub schematic_grid_nm: i64,
    /// Clean up touched wire routes after editing. Explicit Heal is available in either mode.
    /// This is a user preference, never part of a project or its undo history.
    pub auto_heal_wires: bool,
    /// Automatic counterpart navigation; highlighting remains enabled when this is Off.
    pub selection_follow: SelectionFollow,
    /// The unit lengths and coordinates are shown in, and that numbers typed without a unit are
    /// read in. It changes only what is shown, never stored geometry or the project.
    pub units: Unit,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::default(),
            scale_percent: 100,
            navigation: Navigation::default(),
            grid_opacity_percent: grid::OPACITY_DEFAULT,
            schematic_grid_visible: true,
            board_grid_visible: true,
            board_grid_style: grid::Style::Off,
            schematic_grid_nm: grid::SCHEMATIC_SPACING_DEFAULT_NM,
            auto_heal_wires: true,
            selection_follow: SelectionFollow::default(),
            units: Unit::default(),
        }
    }
}

impl Preferences {
    /// Effective drawing opacity for a workspace, preserving the chosen opacity while hidden.
    pub(crate) fn grid_opacity(self, board: bool) -> u8 {
        let visible = if board { self.board_grid_visible } else { self.schematic_grid_visible };
        if visible { self.grid_opacity_percent } else { 0 }
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
    /// grid opacity above 100% or a grid spacing outside its range goes back to the default.
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
    /// The slider's displayed value while a pointer gesture still owns it.
    pending_scale: Option<u16>,
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
const SHORTCUTS: usize = 4;
/// The AI page follows Shortcuts, as SolidCraft's Assistant page does. Desktop only.
#[cfg(not(target_arch = "wasm32"))]
const AI: usize = 5;

/// The settings a page draws, and the shortcut list the read-only Shortcuts page shows.
pub(crate) struct PageData<'a> {
    pub preferences: &'a mut Preferences,
    pub shortcuts: &'a [(&'static str, &'static str, String)],
    pending_scale: &'a mut Option<u16>,
    /// The Assistant draws the AI page and may ask to add an agent from it.
    #[cfg(not(target_arch = "wasm32"))]
    pub assistant: &'a mut crate::assistant::Assistant,
    #[cfg(not(target_arch = "wasm32"))]
    pub request: Option<crate::assistant::SettingsRequest>,
}

impl PreferencesWindow {
    /// Open on Appearance, remembering the current settings for Cancel.
    pub(crate) fn new(preferences: Preferences) -> Self {
        Self { state: WindowState::default(), kept: preferences, pending_scale: None }
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
        #[cfg(not(target_arch = "wasm32"))] assistant: &mut crate::assistant::Assistant,
    ) -> Outcome {
        #[cfg_attr(target_arch = "wasm32", expect(unused_mut))]
        let mut pages: Vec<Page<PageData<'_>>> = vec![
            Page { icon: MenuIcon::Appearance, label: "Appearance", draw: appearance },
            Page { icon: MenuIcon::Navigation, label: "Navigation", draw: navigation },
            Page { icon: MenuIcon::Settings, label: "Views", draw: views },
            Page { icon: MenuIcon::Settings, label: "Schematic", draw: schematic },
            Page { icon: MenuIcon::Keyboard, label: "Shortcuts", draw: shortcuts_page },
        ];
        #[cfg(not(target_arch = "wasm32"))]
        pages.push(Page { icon: MenuIcon::Assistant, label: "AI", draw: ai_page });
        // Search results span pages, so there is no single page whose defaults to restore.
        let defaults_enabled = matches!(self.state.page, APPEARANCE | NAVIGATION | VIEWS | SCHEMATIC) && !settings::searching(&self.state.query);
        let footer = Footer { apply_enabled: *preferences != self.kept, defaults_enabled };
        let mut data = PageData {
            preferences,
            shortcuts,
            pending_scale: &mut self.pending_scale,
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
        match action {
            settings::Action::None => Outcome::Open,
            settings::Action::Ok => Outcome::Closed,
            settings::Action::Apply => {
                self.kept = *data.preferences;
                Outcome::Open
            }
            settings::Action::Cancel => {
                *data.preferences = self.kept;
                Outcome::Closed
            }
            settings::Action::Defaults => {
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
            preferences.scale_percent = defaults.scale_percent;
            preferences.grid_opacity_percent = defaults.grid_opacity_percent;
            preferences.schematic_grid_visible = defaults.schematic_grid_visible;
            preferences.board_grid_visible = defaults.board_grid_visible;
            preferences.board_grid_style = defaults.board_grid_style;
            preferences.units = defaults.units;
        }
        NAVIGATION => preferences.navigation = defaults.navigation,
        VIEWS => preferences.selection_follow = defaults.selection_follow,
        SCHEMATIC => preferences.auto_heal_wires = defaults.auto_heal_wires,
        _ => {}
    }
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

/// Theme, interface scale, PCB grid display, shared grid opacity and display units.
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
    settings::section(ui, "Canvas", |ui| {
        settings::row_with_keywords(ui, "PCB grid", &["board", "layout", "grid", "dots", "lines", "hide"], |ui| {
            let mut style = if preferences.grid_visible(true) { preferences.board_grid_style } else { grid::Style::Off };
            if settings::segmented(ui, &mut style, &[(grid::Style::Off, "Off"), (grid::Style::Dots, "Dots"), (grid::Style::Lines, "Lines")]) {
                preferences.board_grid_style = style;
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
    settings::section(ui, "Panning", |ui| {
        settings::row_with_keywords(ui, "Pan", &["middle", "drag"], |ui| {
            settings::key_chip(ui, "Middle-button drag");
        });
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

    #[test]
    fn saved_settings_come_back() {
        let preferences = Preferences {
            theme: ThemeChoice::Light,
            scale_percent: 110,
            navigation: Navigation { scroll: ScrollAction::Pan, reverse_zoom: true, zoom_speed_percent: 150, zoom_around: ZoomAround::Center },
            grid_opacity_percent: 35,
            schematic_grid_visible: false,
            board_grid_visible: true,
            board_grid_style: grid::Style::Dots,
            schematic_grid_nm: 25 * grid::NM_PER_MIL,
            auto_heal_wires: false,
            selection_follow: SelectionFollow::SecondaryFollowsPrimary,
            units: Unit::Mm,
        };
        for style in [grid::Style::Off, grid::Style::Dots, grid::Style::Lines] {
            let preferences = Preferences { board_grid_style: style, ..preferences };
            assert_eq!(Preferences::parse(&preferences.to_text()), preferences);
        }
        // Settings saved before units existed show mils, as the grid did then.
        assert_eq!(Preferences::parse("{\"grid_opacity_percent\":40}").units, Unit::Mil);
        let hidden = Preferences { grid_opacity_percent: 0, ..Preferences::default() };
        assert_eq!(Preferences::parse(&hidden.to_text()), hidden);
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
        assert_eq!(Preferences::parse("{\"grid_opacity_percent\":101}").grid_opacity_percent, grid::OPACITY_DEFAULT);
        assert_eq!(Preferences::parse("{\"grid_opacity_percent\":-1}"), Preferences::default());
        assert_eq!(older.schematic_grid_nm, grid::SCHEMATIC_SPACING_DEFAULT_NM);
        assert_eq!(Preferences::parse("{\"schematic_grid_nm\":635000}").schematic_grid_nm, 635_000);
        // An out-of-range spacing resets only itself; a value of the wrong type is unreadable,
        // so everything falls back.
        for bad in ["0", "-1270000", "999999999999"] {
            let parsed = Preferences::parse(&format!("{{\"schematic_grid_nm\":{bad},\"grid_opacity_percent\":40}}"));
            assert_eq!(parsed.schematic_grid_nm, grid::SCHEMATIC_SPACING_DEFAULT_NM, "{bad}");
            assert_eq!(parsed.grid_opacity_percent, 40, "{bad}");
        }
        for bad in ["1.5", "\"50 mil\""] {
            let parsed = Preferences::parse(&format!("{{\"schematic_grid_nm\":{bad},\"grid_opacity_percent\":40}}"));
            assert_eq!(parsed, Preferences::default(), "{bad}");
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

    #[test]
    fn page_defaults_reset_only_that_page() {
        let mut preferences = Preferences {
            theme: ThemeChoice::Light,
            scale_percent: 150,
            navigation: Navigation { reverse_zoom: true, ..Navigation::default() },
            grid_opacity_percent: 20,
            schematic_grid_visible: false,
            board_grid_visible: false,
            board_grid_style: grid::Style::Lines,
            schematic_grid_nm: 10 * grid::NM_PER_MIL,
            auto_heal_wires: false,
            selection_follow: SelectionFollow::Both,
            units: Unit::Mm,
        };
        restore_page_defaults(&mut preferences, NAVIGATION);
        // Grid spacing is chosen in the views, not on a page, so no page reset touches it.
        assert_eq!(preferences.schematic_grid_nm, 10 * grid::NM_PER_MIL);
        assert_eq!(preferences.navigation, Navigation::default());
        assert_eq!((preferences.theme, preferences.scale_percent, preferences.grid_opacity_percent), (ThemeChoice::Light, 150, 20));
        assert_eq!((preferences.selection_follow, preferences.units), (SelectionFollow::Both, Unit::Mm));
        restore_page_defaults(&mut preferences, VIEWS);
        assert_eq!(preferences.selection_follow, SelectionFollow::Off);
        assert_eq!((preferences.theme, preferences.grid_opacity_percent), (ThemeChoice::Light, 20));
        restore_page_defaults(&mut preferences, APPEARANCE);
        assert_eq!(preferences, Preferences { schematic_grid_nm: 10 * grid::NM_PER_MIL, auto_heal_wires: false, ..Preferences::default() });
        restore_page_defaults(&mut preferences, SCHEMATIC);
        assert_eq!(preferences, Preferences { schematic_grid_nm: 10 * grid::NM_PER_MIL, ..Preferences::default() });
    }
}
