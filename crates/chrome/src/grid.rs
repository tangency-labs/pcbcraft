//! The canvas grid: its spacing, dot or line display, zoom thinning and opacity.
//!
//! Style and opacity only affect drawing. They never change the grid's spacing or what the pointer
//! snaps to, so a hidden grid still places parts on the same points. Spacing changes where new
//! points snap; it never rescales the drawing or moves anything already placed.

use std::{fmt, ops::RangeInclusive};

use egui::{Color32, Painter, Pos2, Rect, RichText, Stroke, Ui, pos2};

use crate::{
    MenuItem, Tokens, menu, theme,
    units::{self, LengthError, Unit},
};

pub use crate::units::NM_PER_MIL;
/// The schematic grid when nothing has been chosen: 50 mil (1.27 mm), the pitch symbol pins use.
pub const SCHEMATIC_SPACING_DEFAULT_NM: i64 = 50 * NM_PER_MIL;
/// The schematic spacings offered in the grid menu, coarsest first.
pub const SCHEMATIC_SPACING_PRESETS_NM: [i64; 5] = [100 * NM_PER_MIL, 50 * NM_PER_MIL, 25 * NM_PER_MIL, 10 * NM_PER_MIL, 5 * NM_PER_MIL];
/// The PCB grid when nothing has been chosen: 50 mil (1.27 mm), retaining the spacing PCB views
/// inherited before they remembered their own. It can diverge from the schematic default later
/// without changing existing schematic or board behavior.
pub const BOARD_SPACING_DEFAULT_NM: i64 = 50 * NM_PER_MIL;
/// The PCB spacings offered in the grid menu, coarsest first. They currently match the schematic
/// presets but stay a separate list so board work can choose different defaults in one place.
pub const BOARD_SPACING_PRESETS_NM: [i64; 5] = [100 * NM_PER_MIL, 50 * NM_PER_MIL, 25 * NM_PER_MIL, 10 * NM_PER_MIL, 5 * NM_PER_MIL];
/// The spacings a person can choose: 0.01 mm up to 1000 mil (25.4 mm).
pub const SPACING_RANGE_NM: RangeInclusive<i64> = 10_000..=1000 * NM_PER_MIL;
/// Drawn grid points are at least this many times their spacing apart after thinning out.
const THIN_FACTOR: f32 = 5.0;

/// Grid drawing is independent of snapping. Unknown saved styles fall back to Off without
/// discarding unrelated preferences; callers choose a workspace's default explicitly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Style {
    Dots,
    Lines,
    #[default]
    #[serde(other)]
    Off,
}

/// Why typed grid spacing was not accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpacingError {
    Empty,
    NotANumber,
    /// The text after the number, which is not `mil`, `mm` or `in`.
    Unit(String),
    /// Zero, negative, or outside [`SPACING_RANGE_NM`].
    OutOfRange,
}

impl From<LengthError> for SpacingError {
    fn from(error: LengthError) -> Self {
        match error {
            LengthError::Empty => Self::Empty,
            LengthError::NotANumber => Self::NotANumber,
            LengthError::Unit(unit) => Self::Unit(unit),
            LengthError::OutOfRange => Self::OutOfRange,
        }
    }
}

impl fmt::Display for SpacingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "Enter a spacing, such as 20 mil or 0.5 mm"),
            Self::NotANumber => write!(f, "Enter a number, such as 20 mil or 0.5 mm"),
            Self::Unit(unit) => write!(f, "Unknown unit \"{unit}\". Use mil, mm or in"),
            Self::OutOfRange => {
                write!(f, "Use {} to {}", units::label(*SPACING_RANGE_NM.start(), Unit::Mm), units::label(*SPACING_RANGE_NM.end(), Unit::Mil))
            }
        }
    }
}

/// Read typed grid spacing in whole nanometres with [`units::parse_length`]: a bare number is in
/// `unit`, the display unit, and `mil`, `mm` or `in` after it override that. The spacing must be
/// inside [`SPACING_RANGE_NM`], so zero and negative values are refused.
pub fn parse_spacing(text: &str, unit: Unit) -> Result<i64, SpacingError> {
    let nm = units::parse_length(text, unit)?;
    if !SPACING_RANGE_NM.contains(&nm) {
        return Err(SpacingError::OutOfRange);
    }
    Ok(nm)
}

/// Describe a spacing as it would be typed in the display unit: `1_270_000` reads "50 mil" or
/// "1.27 mm".
pub fn spacing_label(nm: i64, unit: Unit) -> String {
    units::label(nm, unit)
}

/// How many snap intervals apart the drawn dots or lines are when `step` points separate grid points: 1
/// normally, then 5, 25, … once zooming out would crowd them closer than
/// [`theme::SCHEMATIC_MIN_GRID_SPACING`]. `None` for a step that isn't a positive finite number.
pub fn shown_every(step: f32) -> Option<u32> {
    if !step.is_finite() || step <= 0.0 {
        return None;
    }
    let mut every = 1u32;
    let mut shown = step;
    // 5^13 still fits a u32; anything finer than that cannot be drawn usefully anyway.
    for _ in 0..13 {
        if shown >= theme::SCHEMATIC_MIN_GRID_SPACING {
            break;
        }
        shown *= THIN_FACTOR;
        every *= THIN_FACTOR as u32;
    }
    Some(every)
}

/// Grid opacity when nothing has been chosen, in percent: the grid at its full token colour.
pub const OPACITY_DEFAULT: u8 = 100;
/// The opacities a person can choose, in percent. Zero hides the grid.
pub const OPACITY_RANGE: RangeInclusive<u8> = 0..=100;
/// The slider moves in steps of this many percent.
pub const OPACITY_STEP: u8 = 5;

/// The colour to paint the grid at `opacity_percent` of `base`, or `None` when the grid is
/// hidden. Values above 100% paint at full strength.
pub fn color(base: Color32, opacity_percent: u8) -> Option<Color32> {
    match opacity_percent.min(*OPACITY_RANGE.end()) {
        0 => None,
        100 => Some(base),
        percent => Some(base.gamma_multiply(f32::from(percent) / 100.0)),
    }
}

/// Paint the grid dots that fall inside `rect`, with the grid's zero at `origin` and `step` points
/// between placement grid points. When zoomed out, only every [`shown_every`]th point is drawn.
/// Nothing is painted at zero opacity or for a step that isn't a positive finite number.
pub fn paint(painter: &Painter, rect: Rect, origin: Pos2, step: f32, opacity_percent: u8) {
    let Some(color) = color(Tokens::get().canvas_grid, opacity_percent) else {
        return;
    };
    paint_style(painter, rect, origin, step, color, Style::Dots);
}

/// Draw dots or lines using the supplied workspace color. Off paints nothing, and both
/// visible styles use the same origin and zoom thinning without changing the snap interval.
pub fn paint_style(painter: &Painter, rect: Rect, origin: Pos2, step: f32, color: Color32, style: Style) {
    painter.extend(shapes(rect, origin, step, color, style));
}

/// Build the same bounded grid for canvas and deferred ribbon painting. Fine marks fade
/// near the density threshold while every fifth mark retains its color, so changing the
/// displayed interval is continuous. Snapping is independent of these presentation levels.
/// Off and invalid spacing return no shapes.
pub fn shapes(rect: Rect, origin: Pos2, step: f32, color: Color32, style: Style) -> Vec<egui::Shape> {
    let mut shapes = Vec::new();
    if style == Style::Off {
        return shapes;
    }
    let Some(every) = shown_every(step) else {
        return shapes;
    };
    let step = step * every as f32;
    let fade = (step / theme::SCHEMATIC_MIN_GRID_SPACING - 1.0).clamp(0.0, 1.0);
    let fade = fade * fade * (3.0 - 2.0 * fade);
    let minor = (fade > 0.0).then(|| color.gamma_multiply(fade)).filter(|color| color.a() > 0);
    let first_x = ((rect.left() - origin.x) / step).ceil() as i32;
    let last_x = ((rect.right() - origin.x) / step).floor() as i32;
    let first_y = ((rect.top() - origin.y) / step).ceil() as i32;
    let last_y = ((rect.bottom() - origin.y) / step).floor() as i32;
    if style == Style::Lines {
        for x in first_x..=last_x {
            let Some(color) = (if x % THIN_FACTOR as i32 == 0 { Some(color) } else { minor }) else { continue };
            shapes.push(egui::Shape::line_segment(
                [pos2(origin.x + x as f32 * step, rect.top()), pos2(origin.x + x as f32 * step, rect.bottom())],
                Stroke::new(1.0, color),
            ));
        }
        for y in first_y..=last_y {
            let Some(color) = (if y % THIN_FACTOR as i32 == 0 { Some(color) } else { minor }) else { continue };
            shapes.push(egui::Shape::line_segment(
                [pos2(rect.left(), origin.y + y as f32 * step), pos2(rect.right(), origin.y + y as f32 * step)],
                Stroke::new(1.0, color),
            ));
        }
        return shapes;
    }
    for x in first_x..=last_x {
        for y in first_y..=last_y {
            let major = x % THIN_FACTOR as i32 == 0 && y % THIN_FACTOR as i32 == 0;
            let Some(color) = (if major { Some(color) } else { minor }) else { continue };
            let center = pos2(origin.x + x as f32 * step, origin.y + y as f32 * step);
            shapes.push(egui::Shape::circle_filled(center, theme::SCHEMATIC_GRID_DOT_RADIUS, color));
        }
    }
    shapes
}

/// A percentage slider for grid opacity and a Reset button that returns it to
/// [`OPACITY_DEFAULT`]. Reset is disabled while the value is already the default. Returns true
/// when the value changed this frame.
pub fn opacity_control(ui: &mut Ui, opacity_percent: &mut u8) -> bool {
    let before = *opacity_percent;
    let slider = egui::Slider::new(opacity_percent, OPACITY_RANGE).step_by(f64::from(OPACITY_STEP)).suffix("%");
    ui.add(slider).accessible_name("Grid opacity");
    if crate::controls::enabled_button(ui, *opacity_percent != OPACITY_DEFAULT, "Reset")
        .on_hover_text(format!("Back to {OPACITY_DEFAULT}%"))
        .clicked()
    {
        *opacity_percent = OPACITY_DEFAULT;
    }
    *opacity_percent != before
}

/// The grid button for the status bar and its menu, which opens upward: the presets, a field for any other
/// spacing, and the default. `spacing_nm` changes only when a row is picked or valid text is
/// applied; invalid text shows why under the field and keeps the current spacing. `every` is
/// [`shown_every`] for the view; `style` labels sparser visible dots/lines and suppresses that
/// annotation for a hidden grid. Style is read-only here and configured through Preferences.
/// The field's text is kept in egui's temporary memory under the button. Menu and accessible
/// labels include `unit`; the status-bar caption shares the adjacent unit switch instead of
/// repeating its suffix. A number typed without a unit is read in it. Returns true when the spacing changed.
pub fn spacing_menu(ui: &mut Ui, spacing_nm: &mut i64, presets: &[i64], default_nm: i64, every: u32, unit: Unit, style: Style) -> bool {
    let t = Tokens::get();
    let before = *spacing_nm;
    let current = spacing_label(*spacing_nm, unit);
    let text = if every > 1 && style != Style::Off {
        let marks = if style == Style::Lines { "lines" } else { "dots" };
        format!("Grid {current} · {marks} {}", spacing_label(spacing_nm.saturating_mul(i64::from(every)), unit))
    } else {
        format!("Grid {current}")
    };
    let caption = text.replace(&format!(" {}", unit.symbol()), "");
    let response = solidcraft_ui_kit::text_button(ui, &caption, true).on_hover_text("Grid spacing of the focused view (N, Shift+N)");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &text));
    let field_id = response.id.with("custom_spacing");
    egui::Popup::menu(&response)
        .frame(menu::frame())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        // The button sits on the bottom edge, so the menu rises from it, right-aligned.
        .align(egui::RectAlign::TOP_END)
        .align_alternatives(&[egui::RectAlign::TOP_START, egui::RectAlign::TOP])
        .show(|ui| {
            // A fixed width, as Help has: rows fill what they are given, and messages wrap in it.
            ui.set_width(t.control_height * 8.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            for &preset in presets {
                let label = spacing_label(preset, unit);
                let label = if preset == default_nm { format!("{label} (default)") } else { label };
                if MenuItem::new(&label).choice(*spacing_nm == preset).show(ui) {
                    *spacing_nm = preset;
                }
            }
            if !presets.contains(spacing_nm) {
                MenuItem::new(&format!("{current} (custom)")).choice(true).show(ui);
            }
            menu::separator(ui);
            let (mut draft, mut error) = ui.data(|d| d.get_temp::<(String, Option<String>)>(field_id)).unwrap_or_default();
            let inset = egui::Margin { left: t.space_8 as i8, right: t.space_8 as i8, top: 0, bottom: t.space_4 as i8 };
            egui::Frame::new().inner_margin(inset).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = t.space_4;
                ui.horizontal(|ui| {
                    let label = ui.label(RichText::new("Custom").size(t.font_label));
                    let field = egui::TextEdit::singleline(&mut draft).hint_text("20 mil or 0.5 mm").desired_width(t.control_height * 4.0);
                    let field = ui.add(field).labelled_by(label.id);
                    let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if field.changed() {
                        error = None;
                    }
                    if crate::controls::button(ui, RichText::new("Apply").size(t.font_label)).clicked() || entered {
                        match parse_spacing(&draft, unit) {
                            Ok(nm) => {
                                *spacing_nm = nm;
                                draft.clear();
                                error = None;
                                ui.close();
                            }
                            Err(e) => error = Some(e.to_string()),
                        }
                    }
                });
                if let Some(error) = &error {
                    ui.add(egui::Label::new(crate::shell::error(error.as_str())).wrap());
                }
                // Some pins fall between the snap points; they still take wires when clicked.
                if *spacing_nm > 0 && default_nm % *spacing_nm != 0 {
                    let hint = format!("Pins are on {}; wire to them by clicking the pin.", spacing_label(default_nm, unit));
                    ui.add(egui::Label::new(crate::shell::dim(hint)).wrap());
                }
            });
            ui.data_mut(|d| d.insert_temp(field_id, (draft, error)));
        });
    *spacing_nm != before
}

/// The preset after `spacing_nm` in `presets` (finer when `finer`, else coarser), wrapping at
/// either end. A custom spacing moves to the nearest preset in that direction. `None` when there
/// are no presets.
pub fn cycle(spacing_nm: i64, presets: &[i64], finer: bool) -> Option<i64> {
    let mut sorted = presets.to_vec();
    sorted.sort_unstable();
    let (first, last) = (*sorted.first()?, *sorted.last()?);
    Some(if finer {
        sorted.iter().rev().copied().find(|&p| p < spacing_nm).unwrap_or(last)
    } else {
        sorted.iter().copied().find(|&p| p > spacing_nm).unwrap_or(first)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read a spacing with mils as the display unit, as the grid menu does by default.
    fn parse_spacing_mil(text: &str) -> Result<i64, SpacingError> {
        parse_spacing(text, Unit::Mil)
    }

    #[test]
    fn opacity_scales_the_grid_colour_and_zero_hides_it() {
        let base = Color32::from_rgb(194, 201, 211);
        assert_eq!(color(base, 0), None);
        assert_eq!(color(base, 100), Some(base));
        assert_eq!(color(base, 250), Some(base));
        let half = color(base, 50).unwrap_or(base);
        assert_eq!(half, base.gamma_multiply(0.5));
        assert!(half.a() < base.a());
    }

    /// Count the dots painted over a canvas, by drawing into a frame and reading its shapes.
    fn dots(rect: Rect, step: f32, opacity_percent: u8) -> usize {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(rect), ..Default::default() }, |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::background());
            paint(&painter, rect, rect.center(), step, opacity_percent);
        });
        output.textures_delta.clear();
        output.shapes.len()
    }

    #[test]
    fn painting_follows_opacity_without_changing_the_points() {
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(100.0, 100.0));
        // A 10 point step over 100 points gives 11 dots on each axis.
        assert_eq!(dots(rect, 10.0, 100), 121);
        assert_eq!(dots(rect, 10.0, 35), 121);
        assert_eq!(dots(rect, 10.0, 0), 0);
        // Zoomed out, every fifth point is drawn, whatever the opacity.
        assert_eq!(dots(rect, 2.0, 100), 121);
        assert_eq!(dots(rect, 2.0, 5), 121);
        assert_eq!(dots(rect, 0.0, 100), 0);
        assert_eq!(dots(rect, f32::NAN, 100), 0);
    }

    /// Fine marks fade before the fivefold density change; the surviving coarse marks
    /// retain their positions and ink on both sides of the boundary, for dots and lines.
    #[test]
    fn grid_density_changes_preserve_the_surviving_marks() {
        let area = Rect::from_min_max(pos2(-99., -99.), pos2(99., 99.));
        for style in [Style::Dots, Style::Lines] {
            let exact = shapes(area, Pos2::ZERO, 5., Color32::WHITE, style);
            let below = shapes(area, Pos2::ZERO, 4.99999, Color32::WHITE, style);
            assert_eq!(exact.len(), below.len());
            for (a, b) in exact.iter().zip(&below) {
                match (a, b) {
                    (egui::Shape::Circle(a), egui::Shape::Circle(b)) => {
                        assert!(a.center.distance(b.center) < 0.001);
                        assert_eq!(a.fill, b.fill);
                    }
                    (egui::Shape::LineSegment { points: a, stroke: sa }, egui::Shape::LineSegment { points: b, stroke: sb }) => {
                        assert!(a[0].distance(b[0]) < 0.001 && a[1].distance(b[1]) < 0.001);
                        assert_eq!(sa, sb);
                    }
                    _ => panic!("unexpected grid shape"),
                }
            }
        }
        let minor_alpha = |step| {
            shapes(area, Pos2::ZERO, step, Color32::WHITE, Style::Dots)
                .iter()
                .find_map(|shape| match shape {
                    egui::Shape::Circle(dot) if dot.center == pos2(step, 0.) => Some(dot.fill.a()),
                    _ => None,
                })
                .unwrap_or(0)
        };
        assert_eq!(minor_alpha(5.), 0);
        assert!(minor_alpha(5.5) > 0 && minor_alpha(5.5) < minor_alpha(7.5));
        assert!(minor_alpha(7.5) < minor_alpha(10.));
        assert_eq!(minor_alpha(10.), 255);
    }

    #[test]
    fn typed_spacing_reads_mils_millimetres_and_inches() {
        assert_eq!(parse_spacing_mil("25"), Ok(25 * NM_PER_MIL));
        assert_eq!(parse_spacing_mil(" 25 mil "), Ok(25 * NM_PER_MIL));
        assert_eq!(parse_spacing_mil("2.5MILS"), Ok(63_500));
        assert_eq!(parse_spacing_mil("10th"), Ok(10 * NM_PER_MIL));
        assert_eq!(parse_spacing_mil("1.27 mm"), Ok(SCHEMATIC_SPACING_DEFAULT_NM));
        assert_eq!(parse_spacing_mil("0.5mm"), Ok(500_000));
        assert_eq!(parse_spacing_mil("0.1in"), Ok(100 * NM_PER_MIL));
        assert_eq!(parse_spacing_mil("0.05\""), Ok(SCHEMATIC_SPACING_DEFAULT_NM));
        // 100 mil is exactly 2.54 mm; rounding happens once, at the nanometre.
        assert_eq!(parse_spacing_mil("2.54 mm"), parse_spacing_mil("100 mil"));
        assert_eq!(parse_spacing_mil("0.0000004 mm"), Err(SpacingError::OutOfRange));
        assert_eq!(parse_spacing_mil("0.01 mm"), Ok(*SPACING_RANGE_NM.start()));
        assert_eq!(parse_spacing_mil("1000"), Ok(*SPACING_RANGE_NM.end()));
    }

    #[test]
    fn typed_spacing_refuses_what_it_cannot_read() {
        assert_eq!(parse_spacing_mil("  "), Err(SpacingError::Empty));
        assert_eq!(parse_spacing_mil("mil"), Err(SpacingError::NotANumber));
        assert_eq!(parse_spacing_mil("1.2.3"), Err(SpacingError::NotANumber));
        assert_eq!(parse_spacing_mil("."), Err(SpacingError::NotANumber));
        assert_eq!(parse_spacing_mil("-5"), Err(SpacingError::OutOfRange));
        assert_eq!(parse_spacing_mil("1e3"), Err(SpacingError::Unit("e3".into())));
        assert_eq!(parse_spacing_mil("5 cm"), Err(SpacingError::Unit("cm".into())));
        assert_eq!(parse_spacing_mil("0"), Err(SpacingError::OutOfRange));
        assert_eq!(parse_spacing_mil("1001 mil"), Err(SpacingError::OutOfRange));
        assert_eq!(parse_spacing_mil("99999999999999999999999 in"), Err(SpacingError::OutOfRange));
        assert_eq!(SpacingError::OutOfRange.to_string(), "Use 0.01 mm to 1000 mil");
    }

    /// Labels follow the display unit, and a label typed back in that unit gives the same spacing.
    #[test]
    fn labels_follow_the_display_unit() {
        assert_eq!(spacing_label(SCHEMATIC_SPACING_DEFAULT_NM, Unit::Mil), "50 mil");
        assert_eq!(spacing_label(63_500, Unit::Mil), "2.5 mil");
        assert_eq!(spacing_label(500_000, Unit::Mm), "0.5 mm");
        assert_eq!(spacing_label(10_000, Unit::Mm), "0.01 mm");
        assert_eq!(spacing_label(1_234_567, Unit::Mm), "1.2346 mm");
        assert_eq!(spacing_label(SCHEMATIC_SPACING_DEFAULT_NM, Unit::Mm), "1.27 mm");
        assert_eq!(spacing_label(500_000, Unit::Mil), "19.685 mil", "shown rounded; the spacing itself stays exact");
        for (text, unit) in [("37 mil", Unit::Mil), ("0.25 mm", Unit::Mm), ("1.5 mil", Unit::Mil), ("0.01 mm", Unit::Mm)] {
            assert_eq!(parse_spacing(text, unit).map(|nm| spacing_label(nm, unit)).as_deref(), Ok(text));
        }
        // A bare number is read in the display unit.
        assert_eq!(parse_spacing("0.5", Unit::Mm), Ok(500_000));
        assert_eq!(parse_spacing("0.5", Unit::Mil), Ok(12_700));
    }

    #[test]
    fn thinning_reports_how_sparse_the_dots_are() {
        assert_eq!(shown_every(10.0), Some(1));
        assert_eq!(shown_every(theme::SCHEMATIC_MIN_GRID_SPACING), Some(1));
        assert_eq!(shown_every(2.0), Some(5));
        assert_eq!(shown_every(0.1), Some(125));
        assert_eq!(shown_every(f32::MIN_POSITIVE), Some(5u32.pow(13)));
        assert_eq!(shown_every(0.0), None);
        assert_eq!(shown_every(f32::INFINITY), None);
    }

    #[test]
    fn cycling_walks_the_presets_and_wraps() {
        let presets = SCHEMATIC_SPACING_PRESETS_NM;
        assert_eq!(cycle(50 * NM_PER_MIL, &presets, true), Some(25 * NM_PER_MIL));
        assert_eq!(cycle(50 * NM_PER_MIL, &presets, false), Some(100 * NM_PER_MIL));
        assert_eq!(cycle(5 * NM_PER_MIL, &presets, true), Some(100 * NM_PER_MIL));
        assert_eq!(cycle(100 * NM_PER_MIL, &presets, false), Some(5 * NM_PER_MIL));
        // A custom 30 mil goes to its neighbours.
        assert_eq!(cycle(30 * NM_PER_MIL, &presets, true), Some(25 * NM_PER_MIL));
        assert_eq!(cycle(30 * NM_PER_MIL, &presets, false), Some(50 * NM_PER_MIL));
        assert_eq!(cycle(30 * NM_PER_MIL, &[], false), None);
    }

    /// The PCB list is independent but must stay usable by the same typed-entry and cycle rules.
    #[test]
    fn board_presets_include_the_board_default_and_stay_in_range() {
        assert!(BOARD_SPACING_PRESETS_NM.contains(&BOARD_SPACING_DEFAULT_NM));
        assert!(BOARD_SPACING_PRESETS_NM.iter().chain([&BOARD_SPACING_DEFAULT_NM]).all(|nm| SPACING_RANGE_NM.contains(nm)));
        assert_eq!(cycle(BOARD_SPACING_DEFAULT_NM, &BOARD_SPACING_PRESETS_NM, true), Some(25 * NM_PER_MIL));
    }
}
