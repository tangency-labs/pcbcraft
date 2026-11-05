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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
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
    pub control_height: f32,
    pub control_radius: u8,
    pub font_small: f32,
    pub font_body: f32,
    pub font_label: f32,
    pub font_button: f32,
    pub font_title: f32,
}

impl Tokens {
    pub const fn light() -> Self {
        Self {
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
            control_height: 28.0,
            control_radius: 6,
            font_small: 11.0,
            font_body: 12.0,
            font_label: 12.5,
            font_button: 13.0,
            font_title: 22.0,
        }
    }

    pub const fn dark() -> Self {
        let mut t = Self::light();
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
        t
    }

    /// The current palette on this thread.
    pub fn get() -> Self {
        if is_dark() { Self::dark() } else { Self::light() }
    }
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
            for surface in [t.panel, t.panel_header, t.field] {
                assert!(contrast(t.text, surface) >= 4.5);
            }
            assert!(contrast(t.text, t.accent_soft) >= 4.5);
            assert!(contrast(t.text_dim, t.panel) >= 4.5);
            assert!(contrast(t.canvas_ink, t.canvas) >= 3.0);
            assert!(contrast(t.error, t.panel) >= 4.5);
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
