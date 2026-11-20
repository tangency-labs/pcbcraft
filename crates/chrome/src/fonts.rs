//! Shared Craft interface faces behind PCBcraft's existing chrome API.

use std::sync::Arc;

use egui::{FontData, FontId};

/// The OFL fallback bundled by egui 0.36.2, retained across egui 0.37's move to platform glyphs.
const EMOJI: &[u8] = include_bytes!("../assets/fonts/noto-emoji/NotoEmoji-Regular.ttf");

/// Install the pinned shared look's Plex faces in this egui context, preserving symbol fallbacks.
pub fn install(ctx: &egui::Context) {
    let mut fonts = solidcraft_look::fonts::definitions();
    install_hebrew_fallback_fonts(&mut fonts);
    let fallback = "pcbcraft-emoji";
    fonts.font_data.insert(fallback.into(), Arc::new(FontData::from_static(EMOJI)));
    for family in fonts.families.values_mut() {
        family.push(fallback.into());
    }
    solidcraft_look::fonts::apply(ctx, fonts);
}

/// Insert `name` as a font fallback before the symbol fallback across interface font families.
#[cfg(any(has_hebrew_craft_font, not(target_arch = "wasm32")))]
fn append_hebrew_fallback(fonts: &mut egui::FontDefinitions, name: &str) {
    for family in fonts.families.values_mut() {
        if !family.iter().any(|existing| existing == name) {
            family.push(name.to_owned());
        }
    }
}

/// Register a bundled (`CRAFT_FONTS_DIR`) or desktop system Hebrew fallback font without
/// committing binary font assets.
fn install_hebrew_fallback_fonts(fonts: &mut egui::FontDefinitions) {
    #[cfg(has_hebrew_craft_font)]
    {
        const BUNDLED_HEBREW: &str = "noto-sans-hebrew";
        let bytes = include_bytes!(env!("HEBREW_CRAFT_FONT_PATH"));
        fonts.font_data.insert(BUNDLED_HEBREW.to_owned(), Arc::new(FontData::from_static(bytes)));
        append_hebrew_fallback(fonts, BUNDLED_HEBREW);
    }

    #[cfg(all(not(has_hebrew_craft_font), not(target_arch = "wasm32")))]
    {
        const CANDIDATES: [(&str, &str); 8] = [
            ("arial-hebrew", "/System/Library/Fonts/Supplemental/Arial Hebrew.ttf"),
            ("sf-hebrew", "/System/Library/Fonts/SFHebrew.ttf"),
            ("arial-hebrew-local", "/Library/Fonts/Arial Hebrew.ttf"),
            ("noto-sans-hebrew", "/usr/share/fonts/truetype/noto/NotoSansHebrew-Regular.ttf"),
            ("noto-sans-hebrew-arch", "/usr/share/fonts/noto/NotoSansHebrew-Regular.ttf"),
            ("dejavu-sans", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
            ("arial", "C:\\Windows\\Fonts\\arial.ttf"),
            ("segoe-ui", "C:\\Windows\\Fonts\\segoeui.ttf"),
        ];
        for (name, path) in CANDIDATES {
            if let Ok(bytes) = std::fs::read(path) {
                fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_owned(bytes)));
                append_hebrew_fallback(fonts, name);
                break;
            }
        }
    }
    #[cfg(all(target_arch = "wasm32", not(has_hebrew_craft_font)))]
    let _ = fonts;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Applying the chrome font setup registers a Hebrew fallback font when bundled or installed.
    #[test]
    fn hebrew_fallback_font_renders_hebrew_text() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let galley = ui.painter().layout_no_wrap("עברית".into(), FontId::proportional(14.0), egui::Color32::WHITE);
            assert!(galley.size().x > 0.0);
        });
        out.textures_delta.clear();
        #[cfg(target_os = "macos")]
        {
            let has_hebrew = ctx.fonts_mut(|fonts| fonts.has_glyph(&FontId::proportional(14.0), 'ע'));
            assert!(has_hebrew, "expected system Hebrew fallback font on macOS");
        }
    }
}
