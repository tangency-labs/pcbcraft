//! PCBcraft Technical: an OFL-1.1 stroke-data derivative of Relief SingleLine.
//! Font data and notices live in assets/fonts/pcbcraft-technical; layout code is original.
use super::{Point, Settings};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Deserialize)]
struct Font {
    cap_height: f64,
    glyphs: BTreeMap<String, Glyph>,
}
#[derive(Deserialize)]
struct Glyph {
    advance: f64,
    paths: Vec<Vec<[f64; 2]>>,
}

/// Resolve the immutable bundled font once; no installed font or network source affects a board.
/// A malformed compiled asset yields missing-glyph boxes rather than panicking in a paint pass.
fn font() -> Option<&'static Font> {
    static FONT: OnceLock<Option<Font>> = OnceLock::new();
    FONT.get_or_init(|| {
        serde_json::from_str::<Font>(include_str!("../../../assets/fonts/pcbcraft-technical/strokes.json")).ok().filter(|font| {
            font.cap_height.is_finite()
                && font.cap_height > 0.
                && font.glyphs.contains_key("?")
                && font
                    .glyphs
                    .values()
                    .all(|glyph| glyph.advance.is_finite() && glyph.advance >= 0. && glyph.paths.iter().flatten().flatten().all(|v| v.is_finite()))
        })
    })
    .as_ref()
}

/// Borrowed outline and advance in the regular face's Y-up font units.
pub(crate) struct StrokeGlyph {
    pub cap_height: f64,
    pub advance: f64,
    pub paths: &'static [Vec<[f64; 2]>],
}

/// Single-line capital omega (`Ω` / `Ω`) in the regular face's Y-up font units (`cap_height = 680`).
/// Keeps `strokes.json` untouched as the direct OFL conversion while giving PDF schematic values
/// a real ohm glyph instead of `?`.
fn omega_glyph() -> &'static Glyph {
    static OMEGA: OnceLock<Glyph> = OnceLock::new();
    OMEGA.get_or_init(|| {
        const STEPS: usize = 32;
        const CX: f64 = 358.;
        const CY: f64 = 390.;
        const RX: f64 = 248.;
        const RY: f64 = 290.;
        let start = 238_f64.to_radians();
        let end = (-58_f64).to_radians();
        let mut path = Vec::with_capacity(STEPS + 5);
        path.push([95., 0.]);
        path.push([260., 0.]);
        for i in 0..=STEPS {
            let angle = start + (end - start) * i as f64 / STEPS as f64;
            path.push([CX + RX * angle.cos(), CY + RY * angle.sin()]);
        }
        path.push([456., 0.]);
        path.push([621., 0.]);
        Glyph { advance: 716., paths: vec![path] }
    })
}

/// Borrow the regular face's glyph. Common schematic ohm (`Ω`, `Ω`) and Greek-mu (`μ`) characters
/// resolve to real stroke outlines; other unknown characters draw a question mark while callers
/// still map the encoded glyph back to the original Unicode scalar.
pub(crate) fn stroke_glyph(c: char) -> Option<StrokeGlyph> {
    let font = font()?;
    let glyph = match c {
        'Ω' | '\u{2126}' => omega_glyph(),
        'μ' => font.glyphs.get("µ").or_else(|| font.glyphs.get("?"))?,
        _ => {
            let mut utf8 = [0; 4];
            let key: &str = c.encode_utf8(&mut utf8);
            font.glyphs.get(key).or_else(|| font.glyphs.get("?"))?
        }
    };
    Some(StrokeGlyph { cap_height: font.cap_height, advance: glyph.advance, paths: &glyph.paths })
}

/// Lay out smooth proportional strokes at nominal capital height. Round glyphs retain their
/// optical overshoot and descenders; ink spacing is at least one sixth of text height. Missing
/// characters use the font's question mark, preserving a visible indication rather than omission.
pub(crate) fn lettering(text: &str, top: i64, style: Settings) -> Vec<Vec<Point>> {
    let Some(font) = font() else {
        return super::legacy_lettering(&"\u{fffd}".repeat(text.chars().count()), top, style);
    };
    let scale = (style.text_height_nm - style.stroke_width_nm) as f64 / font.cap_height;
    let gap = (style.stroke_width_nm + style.text_height_nm / 6) as f64;
    let mut pen = 0.;
    let mut right: Option<f64> = None;
    let mut paths = Vec::new();
    for c in text.chars() {
        let glyph = font.glyphs.get(&c.to_string()).unwrap_or_else(|| &font.glyphs["?"]);
        let bounds = glyph.paths.iter().flatten().fold((f64::INFINITY, f64::NEG_INFINITY), |(left, right), p| (left.min(p[0]), right.max(p[0])));
        if bounds.0.is_finite()
            && let Some(previous) = right
        {
            pen = f64::max(pen, previous + gap - bounds.0 * scale);
        }
        paths.extend(glyph.paths.iter().map(|path| {
            path.iter()
                .map(|p| [pen + p[0] * scale, top as f64 + style.stroke_width_nm as f64 / 2. + (font.cap_height - p[1]) * scale])
                .collect::<Vec<_>>()
        }));
        if bounds.1.is_finite() {
            right = Some(pen + bounds.1 * scale);
        }
        pen += glyph.advance * scale;
    }
    let (left, right) = paths.iter().flatten().fold((f64::INFINITY, f64::NEG_INFINITY), |(left, right), p| (left.min(p[0]), right.max(p[0])));
    if !left.is_finite() {
        return Vec::new();
    }
    let center = (left + right) / 2.;
    paths
        .into_iter()
        .map(|path| path.into_iter().map(|p| Point { x_nm: (p[0] - center).round() as i64, y_nm: p[1].round() as i64 }).collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Qualified data supplies curved capitals, lowercase and punctuation at finite dimensions.
    #[test]
    fn font_has_proportional_curves_and_real_lowercase() {
        let font = font().unwrap();
        assert!(font.glyphs.len() > 400);
        assert!(font.glyphs["C"].paths.iter().map(Vec::len).sum::<usize>() > 30);
        assert_ne!(font.glyphs["R"].advance, font.glyphs["1"].advance);
        assert_ne!(font.glyphs["a"].paths, font.glyphs["A"].paths);
        for glyph in font.glyphs.values() {
            assert!(glyph.advance.is_finite() && glyph.advance >= 0.);
            assert!(glyph.paths.iter().all(|path| path.len() >= 2 && path.len() < 2048 && path.iter().flatten().all(|v| v.is_finite())));
        }
    }
}
