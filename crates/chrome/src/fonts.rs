//! Shared Craft interface faces behind PCBcraft's existing chrome API.

use std::sync::Arc;

use egui::{FontData, FontId};

/// The OFL fallback bundled by egui 0.36.2, retained across egui 0.37's move to platform glyphs.
const EMOJI: &[u8] = include_bytes!("../assets/fonts/noto-emoji/NotoEmoji-Regular.ttf");

/// Install the pinned shared look's Plex faces in this egui context, preserving symbol fallbacks.
pub fn install(ctx: &egui::Context) {
    let mut fonts = solidcraft_look::fonts::definitions();
    let fallback = "pcbcraft-emoji";
    fonts.font_data.insert(fallback.into(), Arc::new(FontData::from_static(EMOJI)));
    for family in fonts.families.values_mut() {
        family.push(fallback.into());
    }
    solidcraft_look::fonts::apply(ctx, fonts);
}

/// Semibold interface text at this size.
pub fn semibold(size: f32) -> FontId {
    solidcraft_ui_kit::type_face::semibold(size)
}

/// Show the shared interface and bundled font notices in the host's About dialog.
pub fn licenses(ui: &mut egui::Ui) {
    ui.collapsing("Shared UI licenses", |ui| {
        egui::ScrollArea::vertical().id_salt("shared_ui_licenses").max_height(240.0).show(ui, |ui| {
            for text in [
                include_str!("../assets/shared-ui/SOLIDCRAFT-UI-LICENSE"),
                include_str!("../assets/fonts/ibm-plex-sans/OFL.txt"),
                include_str!("../assets/fonts/ibm-plex-mono/OFL.txt"),
                include_str!("../assets/fonts/noto-emoji/OFL.txt"),
            ] {
                ui.label(egui::RichText::new(text).small());
            }
        });
    });
}

/// Use the bundled semibold face once egui has loaded it, with a first-frame fallback.
pub(crate) fn interface_heading(ui: &egui::Ui, size: f32) -> FontId {
    let font = semibold(size);
    if ui.fonts(|fonts| fonts.families().contains(&font.family)) { font } else { FontId::proportional(size) }
}
