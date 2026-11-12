//! Physical silkscreen defaults, versioned stroke lettering and pad-clearance clipping.
use crate::{CommandError, Session, schematic::Point};
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod technical;
pub(crate) use technical::lettering;

/// Board-local print dimensions in nanometers. Defaults follow a 1 mm / 0.15 mm text style
/// and a 0.25 mm copper clearance; they are editable design choices, not a fabrication approval.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, try_from = "RawSettings")]
pub struct Settings {
    pub text_height_nm: i64,
    pub stroke_width_nm: i64,
    pub pad_clearance_nm: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSettings {
    text_height_nm: i64,
    stroke_width_nm: i64,
    pad_clearance_nm: i64,
}
impl Default for Settings {
    fn default() -> Self {
        Self { text_height_nm: 1_000_000, stroke_width_nm: 150_000, pad_clearance_nm: 250_000 }
    }
}
impl TryFrom<RawSettings> for Settings {
    type Error = String;
    /// Bound physical dimensions and prevent a stroke from swallowing the letter's interior.
    fn try_from(raw: RawSettings) -> Result<Self, String> {
        if !(1..=10_000_000).contains(&raw.text_height_nm)
            || !(1..=raw.text_height_nm / 2).contains(&raw.stroke_width_nm)
            || !(0..=10_000_000).contains(&raw.pad_clearance_nm)
        {
            return Err(
                "silkscreen requires positive text height up to 10 mm, stroke width up to half the text height, and clearance from 0 to 10 mm".into(),
            );
        }
        Ok(Self { text_height_nm: raw.text_height_nm, stroke_width_nm: raw.stroke_width_nm, pad_clearance_nm: raw.pad_clearance_nm })
    }
}
/// Original sample geometry uses the same physical lettering and pad-clipping path as boards.
#[derive(Debug)]
pub struct Preview {
    pub paths: Vec<Vec<Point>>,
    pub pads: Vec<super::definition::RectPad>,
}

/// Validate a staged style before generating a fixed two-pad sample; no project or library
/// state is changed. Invalid dimensions return the same diagnostic as authored settings.
pub fn preview(settings: Settings) -> Result<Preview, String> {
    use super::definition::{CopperBinding, PadShape, RectPad};
    let settings = Settings::try_from(RawSettings {
        text_height_nm: settings.text_height_nm,
        stroke_width_nm: settings.stroke_width_nm,
        pad_clearance_nm: settings.pad_clearance_nm,
    })?;
    let pads = [-1_500_000, 1_500_000]
        .into_iter()
        .enumerate()
        .map(|(i, x)| RectPad {
            number: (i + 1).to_string(),
            center: Point { x_nm: x, y_nm: 0 },
            width_nm: 1_000_000,
            height_nm: 1_600_000,
            rotation: 0,
            shape: PadShape::Rectangle,
            copper: CopperBinding::ComponentSide,
            corner_radius_nm: 0,
            drill_nm: None,
        })
        .collect::<Vec<_>>();
    let mut paths = lettering("R1", -2_000_000, settings);
    paths.push(vec![
        Point { x_nm: -1_400_000, y_nm: -800_000 },
        Point { x_nm: 1_400_000, y_nm: -800_000 },
        Point { x_nm: 1_400_000, y_nm: 800_000 },
        Point { x_nm: -1_400_000, y_nm: 800_000 },
        Point { x_nm: -1_400_000, y_nm: -800_000 },
    ]);
    Ok(Preview { paths: clipped(paths, &pads, settings), pads })
}

impl Session {
    /// Change one addressed board's print style atomically with shared undo; invalid or stale
    /// edits leave both geometry and history intact. Setting the current style is a no-op.
    pub(crate) fn set_silkscreen(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let settings: Settings = serde_json::from_value(params["settings"].clone()).map_err(|e| CommandError::InvalidParams(e.to_string()))?;
        let project = self.open_project()?;
        if project.board.silkscreen.unwrap_or_default() != settings {
            let mut project = project.clone();
            std::sync::Arc::make_mut(&mut project.board).silkscreen = Some(settings);
            self.change(project)?;
        }
        Ok(())
    }
}

/// Original monoline glyphs on a 4-by-6 grid. Lowercase uses small-cap letter forms;
/// unsupported Unicode gets an explicit crossed missing-glyph box. No external font is used.
fn glyph(c: char) -> &'static str {
    match c.to_ascii_uppercase() {
        'A' => "06 20 46|14 34",
        'B' => "06 00 30 41 42 33 03|33 44 45 36 06",
        'C' => "40 10 01 05 16 46",
        'D' => "06 00 20 41 45 26 06",
        'E' => "40 00 06 46|03 33",
        'F' => "06 00 40|03 33",
        'G' => "40 10 01 05 16 46 43 23",
        'H' => "00 06|40 46|03 43",
        'I' => "00 40|20 26|06 46",
        'J' => "00 40 45 36 16 05",
        'K' => "00 06|40 03 46",
        'L' => "00 06 46",
        'M' => "06 00 23 40 46",
        'N' => "06 00 46 40",
        'O' => "10 30 41 45 36 16 05 01 10",
        'P' => "06 00 30 41 42 33 03",
        'Q' => "10 30 41 44 26 16 05 01 10|24 46",
        'R' => "06 00 30 41 42 33 03|23 46",
        'S' => "40 10 01 02 13 33 44 45 36 06",
        'T' => "00 40|20 26",
        'U' => "00 05 16 36 45 40",
        'V' => "00 26 40",
        'W' => "00 06 23 46 40",
        'X' => "00 46|40 06",
        'Y' => "00 23 40|23 26",
        'Z' => "00 40 06 46",
        '0' => "10 30 41 45 36 16 05 01 10|04 42",
        '1' => "11 20 26|06 46",
        '2' => "01 10 30 41 42 06 46",
        '3' => "00 30 41 42 33 23|33 44 45 36 06",
        '4' => "30 03 43|30 36",
        '5' => "40 00 03 33 44 45 36 06",
        '6' => "40 10 01 05 16 36 45 44 33 03",
        '7' => "00 40 16",
        '8' => "10 30 41 42 33 13 02 01 10|13 04 05 16 36 45 44 33",
        '9' => "43 13 02 01 10 30 41 45 36 06",
        '-' => "03 43",
        '_' => "06 46",
        '+' => "03 43|21 25",
        '/' => "40 06",
        ' ' => "",
        _ => "00 40 46 06 00|00 46|40 06",
    }
}

/// Lay out centered lettering with exact overall ink height and stroke width. Letter spacing
/// is 0.2 times height; all paths remain physical geometry before any viewport projection.
pub(super) fn legacy_lettering(text: &str, top: i64, style: Settings) -> Vec<Vec<Point>> {
    let h = style.text_height_nm;
    let s = style.stroke_width_nm;
    let advance = h;
    let width = h * 4 / 5;
    let total = advance * text.chars().count().saturating_sub(1) as i64 + width;
    text.chars()
        .enumerate()
        .flat_map(|(index, c)| {
            glyph(c).split('|').filter(|part| !part.is_empty()).map(move |part| {
                part.split_whitespace()
                    .map(|pair| {
                        let xy = pair.as_bytes();
                        Point {
                            x_nm: -total / 2 + index as i64 * advance + s / 2 + i64::from(xy[0] - b'0') * (width - s) / 4,
                            y_nm: top + s / 2 + i64::from(xy[1] - b'0') * (h - s) / 6,
                        }
                    })
                    .collect()
            })
        })
        .collect()
}

/// Clip centerline segments outside conservative copper bounds expanded by clearance plus half
/// the ink width. Rounded/circular and locally rotated lands use enclosing bounds, never less
/// clearance than requested. Paths are derived; pinned library geometry is never rewritten.
pub(super) fn clipped(paths: Vec<Vec<Point>>, pads: &[super::definition::RectPad], style: Settings) -> Vec<Vec<Point>> {
    let margin = style.pad_clearance_nm as f64 + style.stroke_width_nm as f64 / 2.;
    let boxes: Vec<_> = pads
        .iter()
        .map(|pad| {
            let half_w = pad.width_nm as f64 / 2.;
            let half_h = pad.height_nm as f64 / 2.;
            let angle = f64::from(pad.rotation).to_radians();
            let dx = half_w * angle.cos().abs() + half_h * angle.sin().abs() + margin;
            let dy = half_h * angle.cos().abs() + half_w * angle.sin().abs() + margin;
            [pad.center.x_nm as f64 - dx, pad.center.y_nm as f64 - dy, pad.center.x_nm as f64 + dx, pad.center.y_nm as f64 + dy]
        })
        .collect();
    paths
        .into_iter()
        .flat_map(|path| {
            path.windows(2)
                .flat_map(|pair| {
                    let a = pair[0];
                    let b = pair[1];
                    let mut visible = vec![(0., 1.)];
                    for bounds in &boxes {
                        if let Some((enter, leave)) = intersection(a, b, *bounds) {
                            visible = visible
                                .into_iter()
                                .flat_map(|(start, end)| {
                                    let mut ranges = Vec::new();
                                    if start < enter.min(end) {
                                        ranges.push((start, enter.min(end)));
                                    }
                                    if leave.max(start) < end {
                                        ranges.push((leave.max(start), end));
                                    }
                                    ranges
                                })
                                .collect();
                        }
                    }
                    visible
                        .into_iter()
                        .filter_map(|(start, end)| {
                            let at = |t: f64| Point {
                                x_nm: (a.x_nm as f64 + (b.x_nm - a.x_nm) as f64 * t).round() as i64,
                                y_nm: (a.y_nm as f64 + (b.y_nm - a.y_nm) as f64 * t).round() as i64,
                            };
                            let (a, b) = (at(start), at(end));
                            (a != b).then_some(vec![a, b])
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Return the parametric portion of one segment inside a closed axis-aligned exclusion box.
fn intersection(a: Point, b: Point, bounds: [f64; 4]) -> Option<(f64, f64)> {
    let mut enter: f64 = 0.;
    let mut leave: f64 = 1.;
    for (origin, delta, low, high) in
        [(a.x_nm as f64, (b.x_nm - a.x_nm) as f64, bounds[0], bounds[2]), (a.y_nm as f64, (b.y_nm - a.y_nm) as f64, bounds[1], bounds[3])]
    {
        if delta == 0. {
            if origin < low || origin > high {
                return None;
            }
        } else {
            let (first, last) = ((low - origin) / delta, (high - origin) / delta);
            enter = enter.max(first.min(last));
            leave = leave.min(first.max(last));
            if enter > leave {
                return None;
            }
        }
    }
    Some((enter, leave))
}

#[cfg(test)]
mod tests;
