//! Colours and sizes. Every colour the chrome uses comes from [`Tokens`]. The values are SolidCraft's
//! dark and light palettes, so PCBCraft looks like family from the first frame.
//!
//! The canvas has its own tokens because it will be themed apart from the chrome: `VISION.md` wants
//! it close to what KiCad users know, and always themeable.

use std::cell::Cell;

use egui::{Color32, CornerRadius, Stroke};

thread_local! {
    /// Theme state belongs to the UI thread drawing the frame, so tests running side by side can't
    /// change one another's palette.
    static DARK: Cell<bool> = const { Cell::new(true) };
}

/// Default schematic zoom and drawing weights, in logical points.
pub const SCHEMATIC_GRID_SPACING: f32 = 10.16;
pub const SCHEMATIC_GRID_DOT_RADIUS: f32 = 0.75;
/// Minimum spacing between painted grid dots when zoomed out.
pub const SCHEMATIC_MIN_GRID_SPACING: f32 = 5.0;
/// Space around the contents when fitting the schematic to its viewport.
pub const SCHEMATIC_FIT_MARGIN: f32 = 28.0;
pub const SCHEMATIC_SYMBOL_STROKE: f32 = 1.6;
/// Radius of a terminal marker while the Wire tool is active.
pub const SCHEMATIC_PIN_RADIUS: f32 = 3.0;
/// Distance at which the Wire tool snaps to a terminal.
pub const SCHEMATIC_PIN_HIT_RADIUS: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    /// Background of the title bar that holds the brand and project actions.
    pub app_bar: Color32,
    pub app_bar_tab: Color32,
    pub ribbon_tab_text: Color32,
    pub panel: Color32,
    pub panel_header: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub hover: Color32,
    pub field: Color32,
    pub error: Color32,
    pub canvas: Color32,
    pub canvas_ink: Color32,
    pub canvas_grid: Color32,
    pub schematic_wire: Color32,
    /// Home page: page fill, card surface and border, and secondary text. These and the rail and
    /// primary tile colours below are derived from the rest of the palette by [`Tokens::with_home`].
    pub file_background: Color32,
    pub file_surface: Color32,
    pub file_border: Color32,
    pub file_dim: Color32,
    /// Home page navigation rail and the ink drawn on it.
    pub file_rail: Color32,
    pub file_rail_ink: Color32,
    pub file_rail_hover: Color32,
    /// The Home page's primary action tile.
    pub file_primary: Color32,
    pub file_primary_ink: Color32,
    pub file_primary_border: Color32,
    /// Paged windows such as Preferences: the page body, the sidebar and footer bands, and the
    /// rules between them. Derived from the palette by [`Tokens::with_dialogs`].
    pub dialog_body: Color32,
    pub dialog_footer: Color32,
    pub dialog_rule: Color32,
    /// The selected sidebar page and its ink.
    pub selected_bg: Color32,
    pub form_primary: Color32,
    /// Hints and section titles in forms.
    pub form_dim: Color32,

    pub space_2: f32,
    pub space_3: f32,
    pub space_4: f32,
    pub space_6: f32,
    pub space_8: f32,
    pub space_12: f32,
    pub space_16: f32,
    pub space_24: f32,
    pub radius_sm: u8,
    pub radius_md: u8,
    pub radius_lg: u8,
    pub control_height: f32,
    pub control_radius: u8,
    /// Home page layout, in logical points.
    pub file_nav_width: f32,
    pub file_nav_height: f32,
    pub file_back_height: f32,
    pub file_tile_height: f32,
    pub file_tile_icon: f32,
    pub file_tile_min_width: f32,
    pub file_grid_gap: f32,
    pub file_content_inset_x: f32,
    pub file_content_inset_y: f32,
    pub file_section_gap: f32,
    /// Form layout: the label column unit, the gap between rows, the title bar and footer
    /// heights of a paged window, and the card width its size is measured in.
    pub form_label_width: f32,
    pub form_gap_y: f32,
    pub form_header_height: f32,
    pub form_footer_height: f32,
    pub form_card_width: f32,
    pub font_small: f32,
    pub font_body: f32,
    pub font_label: f32,
    pub font_button: f32,
    pub font_title: f32,
    /// Home page navigation rows and tile titles.
    pub font_heading: f32,
    /// Home page section titles.
    pub font_intro: f32,
    /// The Home page title.
    pub font_start_title: f32,
    /// A settings page heading.
    pub font_page_heading: f32,
}

impl Tokens {
    /// The light palette and shared spacing for the workspace and its chrome.
    pub const fn light() -> Self {
        Self {
            app_bar: Color32::from_rgb(236, 238, 242),
            app_bar_tab: Color32::from_rgb(220, 226, 235),
            ribbon_tab_text: Color32::from_rgb(20, 69, 123),
            panel: Color32::from_rgb(250, 251, 252),
            panel_header: Color32::from_rgb(236, 238, 242),
            border: Color32::from_rgb(208, 212, 220),
            text: Color32::from_rgb(38, 42, 50),
            text_dim: Color32::from_rgb(110, 116, 128),
            accent: Color32::from_rgb(38, 120, 218),
            accent_soft: Color32::from_rgb(214, 230, 250),
            hover: Color32::from_rgb(226, 234, 246),
            field: Color32::WHITE,
            error: Color32::from_rgb(190, 40, 40),
            canvas: Color32::from_rgb(240, 243, 247),
            canvas_ink: Color32::from_rgb(110, 116, 128),
            canvas_grid: Color32::from_rgb(194, 201, 211),
            schematic_wire: Color32::from_rgb(20, 112, 75),
            // Placeholders until `with_home` derives them from the colours above.
            file_background: Color32::PLACEHOLDER,
            file_surface: Color32::PLACEHOLDER,
            file_border: Color32::PLACEHOLDER,
            file_dim: Color32::PLACEHOLDER,
            file_rail: Color32::PLACEHOLDER,
            file_rail_ink: Color32::PLACEHOLDER,
            file_rail_hover: Color32::PLACEHOLDER,
            file_primary: Color32::PLACEHOLDER,
            file_primary_ink: Color32::PLACEHOLDER,
            file_primary_border: Color32::PLACEHOLDER,
            dialog_body: Color32::PLACEHOLDER,
            dialog_footer: Color32::PLACEHOLDER,
            dialog_rule: Color32::PLACEHOLDER,
            selected_bg: Color32::PLACEHOLDER,
            form_primary: Color32::PLACEHOLDER,
            form_dim: Color32::PLACEHOLDER,

            space_2: 2.0,
            space_3: 3.0,
            space_4: 4.0,
            space_6: 6.0,
            space_8: 8.0,
            space_12: 12.0,
            space_16: 16.0,
            space_24: 24.0,
            radius_sm: 4,
            radius_md: 6,
            radius_lg: 12,
            control_height: 28.0,
            control_radius: 6,
            file_nav_width: 248.0,
            file_nav_height: 38.0,
            file_back_height: 40.0,
            file_tile_height: 84.0,
            file_tile_icon: 44.0,
            file_tile_min_width: 240.0,
            file_grid_gap: 14.0,
            file_content_inset_x: 40.0,
            file_content_inset_y: 32.0,
            file_section_gap: 28.0,
            form_label_width: 70.0,
            form_gap_y: 5.0,
            form_header_height: 40.0,
            form_footer_height: 48.0,
            form_card_width: 264.0,
            font_small: 11.0,
            font_body: 12.0,
            font_label: 12.5,
            font_button: 13.0,
            font_title: 22.0,
            font_heading: 14.0,
            font_intro: 16.0,
            font_start_title: 28.0,
            font_page_heading: 20.0,
        }
        .with_home(false)
        .with_dialogs(false)
    }

    /// The dark palette using the same layout metrics as the light theme.
    pub const fn dark() -> Self {
        let mut t = Self::light();
        t.app_bar = Color32::from_rgb(28, 31, 38);
        t.app_bar_tab = Color32::from_rgb(53, 59, 71);
        t.ribbon_tab_text = Color32::from_rgb(170, 206, 247);
        t.panel = Color32::from_rgb(39, 43, 52);
        t.panel_header = Color32::from_rgb(47, 52, 62);
        t.border = Color32::from_rgb(63, 69, 82);
        t.text = Color32::from_rgb(222, 226, 232);
        t.text_dim = Color32::from_rgb(148, 155, 168);
        t.accent = Color32::from_rgb(58, 136, 238);
        t.accent_soft = Color32::from_rgb(42, 70, 110);
        t.hover = Color32::from_rgb(54, 61, 74);
        t.field = Color32::from_rgb(30, 34, 41);
        t.error = Color32::from_rgb(240, 110, 110);
        t.canvas = Color32::from_rgb(46, 52, 64);
        t.canvas_ink = Color32::from_rgb(148, 155, 168);
        t.canvas_grid = Color32::from_rgb(85, 93, 108);
        t.schematic_wire = Color32::from_rgb(111, 210, 160);
        t.with_home(true).with_dialogs(true)
    }

    /// Derive the paged window colours from the palette. In dark these land on SolidCraft's own
    /// dialog values; in light the bands sit between the panel and its header, on a white body.
    /// Selection uses the same accent pair as menus and the ribbon tab.
    pub const fn with_dialogs(mut self, dark: bool) -> Self {
        self.dialog_body = if dark { self.panel } else { self.field };
        self.dialog_footer = if dark { self.panel_header } else { mix(self.panel, self.panel_header, 0.43) };
        self.dialog_rule = if dark { self.app_bar_tab } else { mix(self.panel_header, self.border, 0.2) };
        self.selected_bg = self.accent_soft;
        self.form_primary = self.ribbon_tab_text;
        self.form_dim = self.file_dim;
        self
    }

    /// Derive the Home page colours from the palette, as SolidCraft does for theme files that
    /// leave them out: the page uses the panel and border colours, the cards lift off the page,
    /// and the rail is the accent deepened toward the palette's darkest colour. The hover moves
    /// back toward the accent in dark and deeper in light. The primary tile uses the ribbon tab's
    /// accent pair.
    pub const fn with_home(mut self, dark: bool) -> Self {
        self.file_background = self.panel;
        self.file_surface = if dark { self.panel_header } else { self.field };
        self.file_border = self.border;
        // Captions sit on cards and tinted tiles, so they lean further toward the text than `text_dim`.
        self.file_dim = mix(self.text_dim, self.text, 0.35);
        self.file_rail = if dark { mix(self.accent, self.app_bar, 0.45) } else { mix(self.accent, self.text, 0.15) };
        // The light rail is already near the accent, so its hover deepens instead of brightening.
        self.file_rail_hover = if dark { mix(self.file_rail, self.accent, 0.25) } else { mix(self.file_rail, self.text, 0.2) };
        self.file_rail_ink = if dark { self.text } else { self.panel };
        self.file_primary = self.accent_soft;
        self.file_primary_ink = self.ribbon_tab_text;
        self.file_primary_border = self.accent;
        self
    }

    /// The current palette on this thread.
    pub fn get() -> Self {
        if is_dark() { Self::dark() } else { Self::light() }
    }
}

/// Move `from` the fraction `k` of the way to `to`, channel by channel in gamma space, like
/// `Color32::lerp_to_gamma`, but usable while building the constant palettes.
const fn mix(from: Color32, to: Color32, k: f32) -> Color32 {
    const fn channel(a: u8, b: u8, k: f32) -> u8 {
        let value = a as f32 + (b as f32 - a as f32) * k;
        (value + 0.5) as u8
    }
    Color32::from_rgb(channel(from.r(), to.r(), k), channel(from.g(), to.g(), k), channel(from.b(), to.b(), k))
}

pub fn is_dark() -> bool {
    DARK.get()
}

fn visuals(t: Tokens, dark: bool) -> egui::Visuals {
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = t.panel;
    v.window_fill = t.panel;
    v.window_stroke = Stroke::new(1.0, t.border);
    v.extreme_bg_color = t.field;
    v.faint_bg_color = t.panel_header;
    v.override_text_color = Some(t.text);
    v.error_fg_color = t.error;
    v.widgets.inactive.weak_bg_fill = t.panel_header;
    v.widgets.inactive.bg_fill = t.panel_header;
    v.widgets.active.weak_bg_fill = t.accent_soft;
    v.selection.bg_fill = t.accent_soft;
    v.selection.stroke = Stroke::new(1.0, t.accent);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.open.weak_bg_fill = t.panel_header;
    v.window_corner_radius = CornerRadius::same(t.radius_md);
    v.menu_corner_radius = CornerRadius::same(t.radius_md);
    for widget in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.active, &mut v.widgets.open] {
        widget.corner_radius = CornerRadius::same(t.radius_sm / 2);
    }
    v.widgets.hovered.corner_radius = CornerRadius::same(t.radius_sm.saturating_sub(1));
    v
}

/// Switch to the dark or light palette and style the widgets to match. Call it when the choice
/// changes, not every frame: it asks for a repaint.
pub fn apply(ctx: &egui::Context, dark: bool) {
    DARK.set(dark);
    let t = Tokens::get();
    ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
    ctx.set_visuals(visuals(t, dark));
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(t.space_6, t.space_4);
        s.spacing.button_padding = egui::vec2(t.space_6, t.space_3);
    });
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance of an opaque colour.
    fn luminance(c: Color32) -> f32 {
        let channel = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn text_stays_readable_in_both_palettes() {
        for t in [Tokens::dark(), Tokens::light()] {
            for surface in [t.panel, t.panel_header, t.field, t.app_bar, t.app_bar_tab] {
                assert!(contrast(t.text, surface) >= 4.5);
            }
            assert!(contrast(t.text, t.accent_soft) >= 4.5);
            assert!(contrast(t.ribbon_tab_text, t.accent_soft) >= 4.5);
            assert!(contrast(t.text_dim, t.panel) >= 4.5);
            assert!(contrast(t.canvas_ink, t.canvas) >= 3.0);
            assert!(contrast(t.error, t.panel) >= 4.5);
        }
    }

    #[test]
    fn home_page_text_stays_readable_in_both_palettes() {
        for t in [Tokens::dark(), Tokens::light()] {
            for (ink, surface) in [
                (t.file_rail_ink, t.file_rail),
                (t.file_rail_ink, t.file_rail_hover),
                (t.file_primary_ink, t.file_primary),
                (t.text, t.file_surface),
                (t.text, t.file_background),
                (t.file_dim, t.file_surface),
                (t.file_dim, t.file_background),
                (t.file_dim, t.file_primary),
                (t.text, t.hover),
                (t.text, t.dialog_body),
                (t.text, t.dialog_footer),
                (t.form_dim, t.dialog_body),
                (t.form_dim, t.dialog_footer),
                (t.form_primary, t.selected_bg),
                (t.text, t.field),
            ] {
                assert!(contrast(ink, surface) >= 4.5, "{ink:?} on {surface:?}");
            }
        }
    }

    #[test]
    fn apply_sets_the_palette_and_the_widget_style() {
        let ctx = egui::Context::default();
        apply(&ctx, false);
        assert!(!is_dark());
        assert_eq!(ctx.global_style().visuals.panel_fill, Tokens::light().panel);
        apply(&ctx, true);
        assert!(is_dark());
        assert_eq!(ctx.global_style().visuals.panel_fill, Tokens::dark().panel);
    }
}
