//! Display-only length labels, readouts and the status bar's unit control.
//!
//! Parsing, exact conversion and editable formatting are re-exported from the UI-independent
//! `pcbcraft-units` crate so existing callers keep the same types and conversion/error rules.
//! Display rounding here never changes the canonical integer-nanometre geometry.

use egui::Ui;

pub use pcbcraft_units::{LengthError, MAX_LENGTH_NM, NM_PER_MIL, NM_PER_MM, Unit, edit_label, parse_length};

/// Describe a length as it would be typed, in `unit` with at most four decimals and no trailing
/// zeros: `1_270_000` reads "50 mil" or "1.27 mm". Values that don't convert exactly are rounded
/// for display only.
pub fn label(nm: i64, unit: Unit) -> String {
    let text = format!("{:.4}", nm as f64 / unit.nm_per_unit() as f64);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    let text = if text == "-0" { "0" } else { text };
    format!("{text} {}", unit.symbol())
}

/// A coordinate for a readout, with a fixed number of decimals so the text doesn't jump as the
/// pointer moves: "1250.00" in mils, "31.7500" in millimetres. The unit is shown separately.
pub fn readout(nm: i64, unit: Unit) -> String {
    let decimals = match unit {
        Unit::Mil => 2,
        Unit::Mm => 4,
    };
    let text = format!("{:.*}", decimals, nm as f64 / unit.nm_per_unit() as f64);
    // A tiny negative value rounds to zero; don't show it as "-0.00".
    if text.trim_start_matches('-').bytes().all(|b| b == b'0' || b == b'.') { text.trim_start_matches('-').to_owned() } else { text }
}

/// The status bar's unit button. It shows the display unit, and a click switches to the other one.
/// `shortcut` is shown in its tooltip. Returns true when the unit changed.
pub fn toggle(ui: &mut Ui, unit: &mut Unit, shortcut: &str) -> bool {
    let tip = if shortcut.is_empty() { "Switch units".to_owned() } else { format!("Switch units ({shortcut})") };
    let response = solidcraft_ui_kit::text_button(ui, unit.symbol(), true).on_hover_text(tip);
    let shown = *unit;
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Units: {}", shown.symbol())));
    if response.clicked() {
        *unit = unit.toggled();
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Display labels retain the existing unit conversion and trimmed precision.
    #[test]
    fn labels_follow_the_display_unit() {
        assert_eq!(label(2_540_000, Unit::Mil), "100 mil");
        assert_eq!(label(2_540_000, Unit::Mm), "2.54 mm");
        assert_eq!(label(NM_PER_MIL, Unit::Mm), "0.0254 mm");
    }

    /// Presentation keeps signed coordinates while suppressing negative rounded zero.
    #[test]
    fn negative_lengths_display_without_negative_zero() {
        assert_eq!(label(-635_000, Unit::Mil), "-25 mil");
        assert_eq!(readout(-635_000, Unit::Mil), "-25.00");
        assert_eq!(readout(-635_000, Unit::Mm), "-0.6350");
        assert_eq!(readout(-1, Unit::Mil), "0.00");
        assert_eq!(label(-1, Unit::Mm), "0 mm");
    }

    /// Readouts retain fixed display precision far from zero, separately from editable values.
    #[test]
    fn readout_precision_holds_far_from_zero() {
        assert_eq!(readout(123_456_789_012, Unit::Mm), "123456.7890");
    }
}
