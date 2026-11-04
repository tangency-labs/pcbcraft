//! The bundled IBM Plex interface faces, the same ones SolidCraft uses, ahead of egui's own fonts,
//! which stay as fallbacks for symbols.

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily, FontId};

const PLEX_SANS: &[u8] = include_bytes!("../assets/fonts/ibm-plex-sans/IBMPlexSans-Variable.ttf");
const PLEX_MONO: &[u8] = include_bytes!("../assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf");
const SEMIBOLD: &str = "ui-semibold";

fn definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    for (name, weight, family) in [("plex-sans", 400.0, FontFamily::Proportional), (SEMIBOLD, 600.0, FontFamily::Name(SEMIBOLD.into()))] {
        let mut data = FontData::from_static(PLEX_SANS);
        data.tweak.coords = egui::epaint::text::VariationCoords::new([(b"wght", weight)]);
        fonts.font_data.insert(name.to_owned(), Arc::new(data));
        fonts.families.insert(family, std::iter::once(name.to_owned()).chain(fallbacks.iter().cloned()).collect());
    }
    fonts.font_data.insert("plex-mono".to_owned(), Arc::new(FontData::from_static(PLEX_MONO)));
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "plex-mono".to_owned());
    fonts
}

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions());
}

/// Semibold interface text at this size.
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}
