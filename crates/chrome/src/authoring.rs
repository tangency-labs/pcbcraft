//! Library editor presentation uses the same ribbon, cards and canvas palette as the project.
use crate::{Tokens, fonts, workspace};
use egui::{Frame, Rect, RichText, Ui};

/// Sequential regions let a document owner share one draft across its tools and inspectors.
#[derive(Clone, Copy, Debug)]
pub enum Region {
    Tools,
    Canvas { area: Rect, covered: [Rect; 2] },
    Browser,
    Properties,
}

/// Lay out a dedicated authoring workspace without owning a library document or project session.
/// Compact hosts retain a full canvas and collapsible, scrollable cards.
pub fn workspace(ui: &mut Ui, collapsed: [bool; 2], mut draw: impl FnMut(&mut Ui, Region)) -> [bool; 2] {
    let t = Tokens::get();
    egui::Panel::top(ui.id().with("library_tools")).frame(Frame::new().fill(t.panel).inner_margin(t.space_8)).show(ui, |ui| draw(ui, Region::Tools));
    let area = ui.available_rect_before_wrap();
    let (browser, b) = workspace::floating_card_aligned(
        ui,
        ui.id().with("library_browser"),
        "Browser",
        crate::properties::browser_area(area),
        collapsed[0],
        false,
        |ui| draw(ui, Region::Browser),
    );
    let (properties, p) =
        crate::properties::inspector(ui, ui.id().with("library_properties"), crate::properties::inspector_area(area, browser), collapsed[1], |ui| {
            draw(ui, Region::Properties)
        });
    draw(ui, Region::Canvas { area, covered: [browser, properties] });
    [collapsed[0] ^ b, collapsed[1] ^ p]
}

/// Identify the active definition workspace beside its compact drawing actions.
pub fn heading(ui: &mut Ui, title: &str) {
    let t = Tokens::get();
    ui.label(RichText::new(title).font(fonts::interface_heading(ui, t.font_intro)));
}

/// Screen-space affordances stay independent of native copper and symbol dimensions.
pub const PICK_PADDING: f32 = 6.0;
/// Connection-point ink in the definition editor.
pub const PIN_RADIUS: f32 = 4.0;
/// Hotspot tolerance for choosing a terminal endpoint.
pub const PIN_PICK_RADIUS: f32 = 8.0;
/// Small extra reach around a physical land's projected bounds.
pub const PAD_PICK_PADDING: f32 = 3.0;
/// Text remains selectable at its anchor even while zoomed out.
pub const TEXT_PICK_PADDING: f32 = 12.0;
/// A quiet authoring grid leaves technical ink and copper dominant.
pub const GRID_OPACITY: u8 = 15;
/// Definition graphics share the schematic ink palette and ordinary stroke weight.
pub fn ink_stroke() -> egui::Stroke {
    egui::Stroke::new(1.5, Tokens::get().canvas_ink)
}
/// Selection and origin markers use the shared secondary ink.
pub fn selection_stroke() -> egui::Stroke {
    egui::Stroke::new(1.0, Tokens::get().text_dim)
}
/// Place terminal labels clear of their electrical endpoint.
pub fn pin_label_offset() -> egui::Vec2 {
    egui::vec2(6.0, -6.0)
}
/// The fixed local origin is a screen-space cross, independent of zoom.
pub fn origin(painter: &egui::Painter, at: egui::Pos2) {
    painter.line_segment([at - egui::vec2(6.0, 0.0), at + egui::vec2(6.0, 0.0)], selection_stroke());
    painter.line_segment([at - egui::vec2(0.0, 6.0), at + egui::vec2(0.0, 6.0)], selection_stroke());
}
