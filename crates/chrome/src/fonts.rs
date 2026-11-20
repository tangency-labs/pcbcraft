//! Shared Craft interface faces behind PCBcraft's existing chrome API.

use std::sync::{Arc, OnceLock};

use egui::{FontData, FontId};

/// The OFL fallback bundled by egui 0.36.2, retained across egui 0.37's move to platform glyphs.
const EMOJI: &[u8] = include_bytes!("../assets/fonts/noto-emoji/NotoEmoji-Regular.ttf");

/// Install the pinned shared look's Plex faces in this egui context, preserving symbol fallbacks.
pub fn install(ctx: &egui::Context) {
    let mut fonts = solidcraft_look::fonts::definitions();
    let hebrew = "pcbcraft-hebrew";
    let emoji = "pcbcraft-emoji";
    let fallback = hebrew_fallback();
    if let Some(font) = fallback {
        fonts.font_data.insert(hebrew.into(), Arc::clone(font));
    }
    fonts.font_data.insert(emoji.into(), Arc::new(FontData::from_static(EMOJI)));
    for family in fonts.families.values_mut() {
        if fallback.is_some() {
            family.push(hebrew.into());
        }
        family.push(emoji.into());
    }
    solidcraft_look::fonts::apply(ctx, fonts);
}

/// Share the selected font with locale selection and reuse it for every egui context.
/// Probe once so resolving the interface language never rereads system fonts per frame.
pub(crate) fn hebrew_fallback() -> Option<&'static Arc<FontData>> {
    static FALLBACK: OnceLock<Option<Arc<FontData>>> = OnceLock::new();
    FALLBACK.get_or_init(load_hebrew_fallback).as_ref()
}

/// Return whether `bytes` parses as a font whose primary face covers Hebrew letters.
#[cfg(all(not(has_hebrew_craft_font), not(target_arch = "wasm32")))]
fn covers_hebrew(bytes: &[u8]) -> bool {
    ttf_parser::Face::parse(bytes, 0).is_ok_and(|face| face.glyph_index('א').is_some() && face.glyph_index('ע').is_some())
}

/// Load the bundled fallback when configured, or use the first desktop font that covers Hebrew.
fn load_hebrew_fallback() -> Option<Arc<FontData>> {
    #[cfg(has_hebrew_craft_font)]
    {
        Some(Arc::new(FontData::from_static(include_bytes!(env!("HEBREW_CRAFT_FONT_PATH")))))
    }

    #[cfg(all(not(has_hebrew_craft_font), not(target_arch = "wasm32")))]
    {
        const CANDIDATES: [&str; 13] = [
            // Windows
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\arial.ttf",
            // macOS
            "/System/Library/Fonts/ArialHB.ttc",
            "/System/Library/Fonts/SFHebrew.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            // Linux
            "/usr/share/fonts/noto/NotoSansHebrew-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansHebrew-Regular.ttf",
            "/usr/share/fonts/google-noto/NotoSansHebrew-Regular.ttf",
            "/usr/share/fonts/google-noto-vf/NotoSansHebrew[wght].ttf",
            "/usr/share/fonts/opentype/noto/NotoSansHebrew-Regular.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        ];
        CANDIDATES.into_iter().filter_map(|path| std::fs::read(path).ok()).find(|bytes| covers_hebrew(bytes)).map(FontData::from_owned).map(Arc::new)
    }

    #[cfg(all(not(has_hebrew_craft_font), target_arch = "wasm32"))]
    {
        None
    }
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

    /// Preferences and locale activation agree with the glyphs registered by the real font setup.
    #[test]
    fn hebrew_locale_availability_matches_installed_glyphs() {
        use crate::i18n::{ENGLISH, HEBREW, LanguageChoice, hebrew_font_available, language_options, resolve_preference};

        let ctx = egui::Context::default();
        install(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        let has_glyphs =
            ctx.fonts_mut(|fonts| fonts.has_glyph(&FontId::proportional(14.0), 'ע') && fonts.has_glyph(&FontId::proportional(14.0), 'א'));
        assert_eq!(hebrew_font_available(), has_glyphs);
        assert_eq!(resolve_preference(LanguageChoice::Hebrew), if has_glyphs { HEBREW } else { ENGLISH });
        assert_eq!(language_options(LanguageChoice::System).iter().any(|(choice, _)| *choice == LanguageChoice::Hebrew), has_glyphs);
    }

    /// Reject invalid bytes and valid fonts that lack Hebrew glyphs before registering a fallback.
    #[cfg(all(not(has_hebrew_craft_font), not(target_arch = "wasm32")))]
    #[test]
    fn covers_hebrew_rejects_non_hebrew_and_invalid_fonts() {
        assert!(!covers_hebrew(b""));
        assert!(!covers_hebrew(b"not a font"));
        let bundled = solidcraft_look::fonts::definitions();
        for data in bundled.font_data.values() {
            assert!(!covers_hebrew((*data.font).as_ref()), "bundled Latin font should not pass Hebrew coverage check");
        }
    }
}
