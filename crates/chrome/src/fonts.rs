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

/// Return whether `bytes` parses as a font whose primary face covers Hebrew letters.
#[cfg(all(not(has_hebrew_craft_font), not(target_arch = "wasm32")))]
fn covers_hebrew(bytes: &[u8]) -> bool {
    ttf_parser::Face::parse(bytes, 0).is_ok_and(|face| face.glyph_index('א').is_some() && face.glyph_index('ע').is_some())
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
        const CANDIDATES: [(&str, &str); 13] = [
            // Windows
            ("segoe-ui", "C:\\Windows\\Fonts\\segoeui.ttf"),
            ("arial", "C:\\Windows\\Fonts\\arial.ttf"),
            // macOS
            ("arial-hebrew", "/System/Library/Fonts/ArialHB.ttc"),
            ("sf-hebrew", "/System/Library/Fonts/SFHebrew.ttf"),
            ("arial-macos", "/System/Library/Fonts/Supplemental/Arial.ttf"),
            // Linux
            ("noto-sans-hebrew-arch", "/usr/share/fonts/noto/NotoSansHebrew-Regular.ttf"),
            ("noto-sans-hebrew", "/usr/share/fonts/truetype/noto/NotoSansHebrew-Regular.ttf"),
            ("noto-sans-hebrew-fedora", "/usr/share/fonts/google-noto/NotoSansHebrew-Regular.ttf"),
            ("noto-sans-hebrew-fedora-vf", "/usr/share/fonts/google-noto-vf/NotoSansHebrew[wght].ttf"),
            ("noto-sans-hebrew-otf", "/usr/share/fonts/opentype/noto/NotoSansHebrew-Regular.ttf"),
            ("dejavu-sans", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
            ("dejavu-sans-arch", "/usr/share/fonts/TTF/DejaVuSans.ttf"),
            ("dejavu-sans-fedora", "/usr/share/fonts/dejavu/DejaVuSans.ttf"),
        ];
        for (name, path) in CANDIDATES {
            if let Ok(bytes) = std::fs::read(path)
                && covers_hebrew(&bytes)
            {
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
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let has_hebrew =
                ctx.fonts_mut(|fonts| fonts.has_glyph(&FontId::proportional(14.0), 'ע') && fonts.has_glyph(&FontId::proportional(14.0), 'א'));
            assert!(has_hebrew, "expected system Hebrew fallback font to cover Hebrew glyphs on {}", std::env::consts::OS);
        }
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
