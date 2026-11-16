//! Sheet templates: the border, zone markers and title block drawn on a sheet's paper.
//!
//! A template is a library item addressed by library, name and revision. One template fits every
//! paper size: its border follows the paper, and its lines and texts are placed relative to a
//! paper corner. Texts may hold parameter references such as `@sheet.Title`, resolved for each
//! sheet. On a sheet the frame is drawn behind everything and cannot be selected.
//!
//! Only the templates bundled with the app exist so far; sheets refer to them by revision and a
//! revision's content never changes. The Default template's title block is ported from Coline
//! (`crates/sheet/src/template.rs`, MIT OR Apache-2.0) with its Y axis flipped; see
//! `ATTRIBUTION.md`. Zone markers are new.

use std::fmt;
use std::ops::Range;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::page::Page;
use crate::parameters::Resolved;
use crate::schematic::Point;

/// The schema every template document declares.
pub const SCHEMA: &str = "pcbcraft-sheet-template/1";
/// The most lines, rectangles and texts one template holds.
const MAX_ITEMS: usize = 1000;
/// The longest text, title or description, in bytes.
const MAX_TEXT: usize = 1000;
/// Item offsets stay within the longest paper side.
const MAX_OFFSET_NM: i64 = 1_189_000_000;
/// The border and zone band together stay within this distance of the paper's edge.
const MAX_MARGIN_NM: i64 = 50_000_000;
/// Zones are at least this wide, so even A0 has a bounded number of markers.
const MIN_PITCH_NM: i64 = 10_000_000;
/// Text sizes are positive and at most this large.
const MAX_TEXT_SIZE_NM: i64 = 50_000_000;
/// Zone rows are lettered without I and O, which read like 1 and 0 (ISO 5457).
const ROW_LETTERS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";

/// The bundled template documents, parsed once by [`bundled`].
const BUNDLED: &[&str] = &[include_str!("../../../../libraries/basic-frames/templates/default.r1.json")];

/// The address a sheet stores for its template.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateRef {
    /// The library's identity, such as `basic-frames`.
    pub library: String,
    /// The template's name within the library, such as `default`.
    pub name: String,
    /// The pinned revision, from 1.
    pub revision: u32,
}

impl TemplateRef {
    /// The bundled template this names, or `None` when there is no such template revision.
    pub fn resolve(&self) -> Option<&'static Template> {
        bundled().iter().find(|template| template.library == self.library && template.name == self.name && template.revision == self.revision)
    }
}

impl fmt::Display for TemplateRef {
    /// `library/name` and the revision, as in messages: `basic-frames/default r1`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{} r{}", self.library, self.name, self.revision)
    }
}

/// The paper corner an item is placed from. Offsets use the sheet's axes, so Y points down and
/// items in a bottom corner have negative Y offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Which part of a text sits on its anchor point; the anchor is at mid height.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    Left,
    Center,
    Right,
}

/// How a frame text is drawn: a field label or the value it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextStyle {
    /// Small, quiet captions such as `Drawn by` and zone markers.
    Label,
    /// The values the title block shows.
    Value,
}

/// The border run inside the paper's edge, with an optional band of zone markers inside it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Border {
    /// Distance from the paper's edge to the border, in nanometres.
    inset_nm: i64,
    /// Zone markers, numbered across and lettered down.
    #[serde(default)]
    zones: Option<Zones>,
}

/// A band inside the border divided into equal zones about `pitch_nm` long on every side.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Zones {
    /// The band's width; a second border runs this far inside the first.
    band_nm: i64,
    /// The preferred zone length; each side gets the nearest whole number of equal zones.
    pitch_nm: i64,
    /// Text size of the zone markers.
    size_nm: i64,
}

/// One drawn item, placed from a paper corner.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Item {
    /// A straight line between two offsets.
    Line { anchor: Anchor, from_nm: [i64; 2], to_nm: [i64; 2] },
    /// An outline from its top-left offset, with positive width and height.
    Rect { anchor: Anchor, at_nm: [i64; 2], size_nm: [i64; 2] },
    /// One line of text whose references are resolved for each sheet.
    Text { anchor: Anchor, at_nm: [i64; 2], size_nm: i64, align: Align, style: TextStyle, text: String },
}

/// A template document, as bundled in `libraries/basic-frames/templates`. Construct one with
/// [`parse`], which checks every field.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    schema: String,
    library: String,
    name: String,
    revision: u32,
    title: String,
    description: String,
    border: Border,
    items: Vec<Item>,
}

/// A template's border, zones, lines and resolved texts on one sheet's paper, in sheet
/// coordinates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// Straight lines; rectangles arrive as their four edges.
    pub lines: Vec<[Point; 2]>,
    /// Texts with references already resolved.
    pub texts: Vec<FrameText>,
}

/// One text of a [`Frame`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameText {
    /// The anchor point; the text's mid height sits on it.
    pub at: Point,
    /// Text size in nanometres, measured like component field text.
    pub size_nm: i64,
    /// Which part of the text sits on `at`.
    pub align: Align,
    /// Label or value.
    pub style: TextStyle,
    /// What to draw.
    pub text: String,
    /// Byte ranges of `text` that are unresolved references, drawn as typed and marked.
    pub unknown: Vec<Range<usize>>,
}

/// Read and check a template document. Errors name the first problem found.
pub fn parse(text: &str) -> Result<Template, String> {
    let template: Template = serde_json::from_str(text).map_err(|error| error.to_string())?;
    template.validate()?;
    Ok(template)
}

/// Every bundled template, in library and name order. A bundled document that fails [`parse`]
/// is left out rather than stopping the app; tests check that every one parses.
pub fn bundled() -> &'static [Template] {
    static ALL: OnceLock<Vec<Template>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut all: Vec<_> = BUNDLED.iter().filter_map(|text| parse(text).ok()).collect();
        all.sort_by(|a, b| (&a.library, &a.name, a.revision).cmp(&(&b.library, &b.name, b.revision)));
        all
    })
}

/// The template new sheets start with: `basic-frames/default` revision 1. `None` only if that
/// bundled document failed to parse, in which case new sheets get a plain border.
pub fn default_template() -> Option<&'static Template> {
    TemplateRef { library: "basic-frames".into(), name: "default".into(), revision: 1 }.resolve()
}

/// Whether `id` is a usable library or template name: a lowercase letter followed by up to 63
/// lowercase letters, digits and `-`.
fn valid_id(id: &str) -> bool {
    id.len() <= 64
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl Template {
    /// The address sheets store for this template.
    pub fn reference(&self) -> TemplateRef {
        TemplateRef { library: self.library.clone(), name: self.name.clone(), revision: self.revision }
    }

    /// The name shown in the interface, such as `Default`.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// What the template is for.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The listing shape used by `template.list`.
    pub(crate) fn summary(&self) -> Value {
        json!({"library":self.library,"name":self.name,"revision":self.revision,"title":self.title,"description":self.description})
    }

    /// Check every field: the schema, identities, the border and zone sizes and each item's
    /// offsets, sizes and text length.
    fn validate(&self) -> Result<(), String> {
        if self.schema != SCHEMA {
            return Err(format!("schema must be {SCHEMA}"));
        }
        if !valid_id(&self.library) || !valid_id(&self.name) {
            return Err("library and name use 1 to 64 lowercase letters, digits and -, starting with a letter".into());
        }
        if self.revision == 0 {
            return Err("revision starts at 1".into());
        }
        if self.title.trim().is_empty() || self.title.len() > 200 || self.description.len() > MAX_TEXT {
            return Err("the title needs 1 to 200 bytes and the description at most 1000".into());
        }
        let band = self.border.zones.as_ref().map_or(0, |zones| zones.band_nm);
        if self.border.inset_nm < 0 || band < 0 || self.border.inset_nm + band > MAX_MARGIN_NM {
            return Err("the border and its zone band stay within 50 mm of the paper's edge".into());
        }
        if let Some(zones) = &self.border.zones
            && (zones.band_nm == 0 || zones.pitch_nm < MIN_PITCH_NM || !(1..=zones.band_nm).contains(&zones.size_nm))
        {
            return Err("zones need a band, a pitch of at least 10 mm and a text size within the band".into());
        }
        if self.items.len() > MAX_ITEMS {
            return Err(format!("a template holds at most {MAX_ITEMS} items"));
        }
        let offset = |p: &[i64; 2]| p.iter().all(|v| v.abs() <= MAX_OFFSET_NM);
        for item in &self.items {
            let ok = match item {
                Item::Line { from_nm, to_nm, .. } => offset(from_nm) && offset(to_nm),
                Item::Rect { at_nm, size_nm, .. } => offset(at_nm) && size_nm.iter().all(|v| (1..=MAX_OFFSET_NM).contains(v)),
                Item::Text { at_nm, size_nm, text, .. } => {
                    offset(at_nm) && (1..=MAX_TEXT_SIZE_NM).contains(size_nm) && !text.is_empty() && text.len() <= MAX_TEXT
                }
            };
            if !ok {
                return Err("items need offsets within 1189 mm, rectangles a positive size, and texts 1 to 1000 bytes with a size up to 50 mm".into());
            }
        }
        Ok(())
    }

    /// The frame on `page`'s paper. `resolve` replaces the references of each item text.
    pub fn frame(&self, page: Page, resolve: &dyn Fn(&str) -> Resolved) -> Frame {
        let mut frame = Frame::default();
        let [min, max] = page.corners();
        let inset = self.border.inset_nm;
        let outer = [Point { x_nm: min.x_nm + inset, y_nm: min.y_nm + inset }, Point { x_nm: max.x_nm - inset, y_nm: max.y_nm - inset }];
        frame.lines.extend(edges(outer));
        if let Some(zones) = &self.border.zones {
            add_zones(&mut frame, outer, zones);
        }
        for item in &self.items {
            match item {
                Item::Line { anchor, from_nm, to_nm } => frame.lines.push([place(page, *anchor, *from_nm), place(page, *anchor, *to_nm)]),
                Item::Rect { anchor, at_nm, size_nm } => {
                    let a = place(page, *anchor, *at_nm);
                    frame.lines.extend(edges([a, Point { x_nm: a.x_nm + size_nm[0], y_nm: a.y_nm + size_nm[1] }]));
                }
                Item::Text { anchor, at_nm, size_nm, align, style, text } => {
                    let resolved = resolve(text);
                    frame.texts.push(FrameText {
                        at: place(page, *anchor, *at_nm),
                        size_nm: *size_nm,
                        align: *align,
                        style: *style,
                        text: resolved.text,
                        unknown: resolved.unknown_spans,
                    });
                }
            }
        }
        frame
    }

    /// The area the title block and other items cover on `page`, as top-left and bottom-right
    /// corners, so an adopted drawing can avoid it. Text extents are estimated generously.
    /// `None` for a template with only a border.
    pub(crate) fn items_bounds(&self, page: Page) -> Option<[Point; 2]> {
        let mut bounds: Option<[Point; 2]> = None;
        let mut add = |p: Point| {
            let [min, max] = bounds.get_or_insert([p, p]);
            *min = Point { x_nm: min.x_nm.min(p.x_nm), y_nm: min.y_nm.min(p.y_nm) };
            *max = Point { x_nm: max.x_nm.max(p.x_nm), y_nm: max.y_nm.max(p.y_nm) };
        };
        for item in &self.items {
            match item {
                Item::Line { anchor, from_nm, to_nm } => [from_nm, to_nm].into_iter().for_each(|p| add(place(page, *anchor, *p))),
                Item::Rect { anchor, at_nm, size_nm } => {
                    let a = place(page, *anchor, *at_nm);
                    add(a);
                    add(Point { x_nm: a.x_nm + size_nm[0], y_nm: a.y_nm + size_nm[1] });
                }
                Item::Text { anchor, at_nm, size_nm, .. } => {
                    let a = place(page, *anchor, *at_nm);
                    add(Point { x_nm: a.x_nm, y_nm: a.y_nm - size_nm });
                    add(Point { x_nm: a.x_nm, y_nm: a.y_nm + size_nm });
                }
            }
        }
        bounds
    }
}

/// An item offset from `anchor`'s paper corner, in sheet coordinates.
fn place(page: Page, anchor: Anchor, offset: [i64; 2]) -> Point {
    let [min, max] = page.corners();
    let x = match anchor {
        Anchor::TopLeft | Anchor::BottomLeft => min.x_nm,
        Anchor::TopRight | Anchor::BottomRight => max.x_nm,
    };
    let y = match anchor {
        Anchor::TopLeft | Anchor::TopRight => min.y_nm,
        Anchor::BottomLeft | Anchor::BottomRight => max.y_nm,
    };
    Point { x_nm: x + offset[0], y_nm: y + offset[1] }
}

/// The four edges of the rectangle between two opposite corners.
pub(crate) fn edges([a, b]: [Point; 2]) -> [[Point; 2]; 4] {
    let (tr, bl) = (Point { x_nm: b.x_nm, y_nm: a.y_nm }, Point { x_nm: a.x_nm, y_nm: b.y_nm });
    [[a, tr], [tr, b], [b, bl], [bl, a]]
}

/// The zone band inside `outer`: a second border, a tick between neighbouring zones and a
/// marker centred in each zone, numbered from 1 at the left and lettered from A at the top, on
/// both opposite sides.
fn add_zones(frame: &mut Frame, outer: [Point; 2], zones: &Zones) {
    let [min, max] = outer;
    let band = zones.band_nm;
    frame.lines.extend(edges([Point { x_nm: min.x_nm + band, y_nm: min.y_nm + band }, Point { x_nm: max.x_nm - band, y_nm: max.y_nm - band }]));
    let label =
        |at: Point, text: String| FrameText { at, size_nm: zones.size_nm, align: Align::Center, style: TextStyle::Label, text, unknown: Vec::new() };
    let width = max.x_nm - min.x_nm;
    let columns = zone_count(width, zones.pitch_nm);
    for i in 0..columns {
        let (left, right) = (min.x_nm + width * i / columns, min.x_nm + width * (i + 1) / columns);
        if i > 0 {
            frame.lines.push([Point { x_nm: left, y_nm: min.y_nm }, Point { x_nm: left, y_nm: min.y_nm + band }]);
            frame.lines.push([Point { x_nm: left, y_nm: max.y_nm - band }, Point { x_nm: left, y_nm: max.y_nm }]);
        }
        let x = left + (right - left) / 2;
        for y in [min.y_nm + band / 2, max.y_nm - band / 2] {
            frame.texts.push(label(Point { x_nm: x, y_nm: y }, (i + 1).to_string()));
        }
    }
    let height = max.y_nm - min.y_nm;
    let rows = zone_count(height, zones.pitch_nm);
    for i in 0..rows {
        let (top, bottom) = (min.y_nm + height * i / rows, min.y_nm + height * (i + 1) / rows);
        if i > 0 {
            frame.lines.push([Point { x_nm: min.x_nm, y_nm: top }, Point { x_nm: min.x_nm + band, y_nm: top }]);
            frame.lines.push([Point { x_nm: max.x_nm - band, y_nm: top }, Point { x_nm: max.x_nm, y_nm: top }]);
        }
        let y = top + (bottom - top) / 2;
        for x in [min.x_nm + band / 2, max.x_nm - band / 2] {
            frame.texts.push(label(Point { x_nm: x, y_nm: y }, row_letter(i as usize)));
        }
    }
}

/// How many equal zones of about `pitch` fit along `length`: the nearest whole number, at
/// least one.
fn zone_count(length: i64, pitch: i64) -> i64 {
    ((length + pitch / 2) / pitch).max(1)
}

/// The marker for zone row `index`: A to Z without I and O, then AA, AB and so on.
fn row_letter(index: usize) -> String {
    let n = ROW_LETTERS.len();
    let letter = |i: usize| char::from(ROW_LETTERS[i]);
    if index < n { letter(index).to_string() } else { format!("{}{}", letter((index / n - 1) % n), letter(index % n)) }
}

#[cfg(test)]
mod tests;
