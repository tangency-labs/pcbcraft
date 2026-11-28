//! KiCad library symbols as native symbol documents.
//!
//! A pin out of native range or carrying an unrepresentable identity refuses the
//! symbol with a reason. Drawing details the native format holds differently are converted with
//! a note in the report.

use super::Report;
use pcbcraft_kicad::schematic::{Electrical, Fill, Graphic, Justify, LibSymbol, Pin, Shape, SymbolUnit, Text};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

use crate::schematic::{MAX_EXACT_PIN_OFFSET_NM, legacy_pin_offset};
/// How far any native drawing point may sit from the origin on either axis.
const DRAWING_REACH_NM: i64 = 100_000_000;
/// The most units a native symbol holds.
const MAX_UNITS: usize = 32;
/// The most drawing entries one native unit holds.
const MAX_DRAWING: usize = 256;
/// The most points one native polyline holds.
const MAX_POINTS: usize = 64;
/// The stroke KiCad's "default width" (0) is drawn with.
const DEFAULT_STROKE_NM: i64 = 152_400;
/// The thickest native stroke.
const MAX_STROKE_NM: i64 = 2_000_000;
/// How many straight pieces a Bézier curve is drawn with.
const CURVE_STEPS: usize = 16;
/// The longest pin name, in bytes, an alternate body style's pins are compared with.
const MAX_COMPARED_NAME: usize = 512;
/// The most bytes one native library file holds.
const MAX_FILE_BYTES: usize = crate::library::sources::MAX_FILE_BYTES;
/// How many drawing points one import may convert, over every symbol and unit. A common part is
/// drawn again in each unit and symbols can share one drawing through `extends`, so the work
/// isn't bounded by the file's size alone.
///
/// Reading a symbol costs one point per unit and pin it has, and converting each pin of each
/// unit one more.
pub(super) const MAX_DRAWING_WORK: usize = 2_000_000;

/// A converted symbol: its native document and how to place it.
pub(super) struct Converted {
    pub(super) id: String,
    /// The native document as JSON text, at most a library file's size.
    pub(super) text: String,
    /// The native unit ID for each KiCad unit number that can be placed.
    pub(super) units: BTreeMap<u32, String>,
    /// For a power symbol, the terminal whose net the placed Value names.
    pub(super) power_terminal: Option<String>,
    /// For a power symbol, where that terminal's pin is in the library symbol: the index of its
    /// unit in `units` and of the pin in that unit, so its name is read from the file, not copied.
    pub(super) power_pin: Option<(usize, usize)>,
}

/// What an alternate body style must keep of a pin: unit, number, name, position,
/// electrical type and whether it is hidden.
type StylePin<'a> = (u32, &'a str, &'a str, i64, i64, String, bool);

/// A native unit being built: its pins in order and its drawing.
struct NativeUnit {
    id: String,
    pins: Vec<Value>,
    drawing: Vec<Value>,
}

/// Convert `symbol` under the native ID `id`. Fails with the reason when its electrical content
/// can't be represented exactly, or when its document would exceed a library file's size. Each
/// unit's drawing stops at [`MAX_DRAWING`] entries, with a note, before anything past the limit
/// is converted. Reading its units and pins, and every pin, drawing and text converted, is paid
/// for from `budget`, in points; when it runs short, it is set to zero, conversion stops there,
/// and the caller refuses the import. Messages show at most 16 characters of a pin number.
pub(super) fn convert(
    symbol: &LibSymbol,
    id: String,
    budget: &mut usize,
    report: &mut Report,
    preserve_local_power: bool,
) -> Result<Converted, String> {
    // Reading the symbol below visits each of its units and pins, and symbols can share one
    // drawing through `extends`, so that is paid for first: units before they are counted
    // through, then pins.
    if !pay(budget, symbol.units.len().max(1)) || !pay(budget, symbol.units.iter().map(|unit| unit.pins.len()).sum::<usize>().max(1)) {
        return Err("it has more units and pins than an import converts".into());
    }
    // Every pin number of the styles read is checked before any is compared or kept.
    for pin in symbol.units.iter().filter(|unit| unit.style <= 2).flat_map(|unit| &unit.pins) {
        if !valid_number(&pin.number) {
            return Err(format!("pin number `{}` isn't 1 to 8 letters, digits, +, - or _", shown(&pin.number)));
        }
    }
    // Style 0 is drawn in every body style, 1 is the normal style and 2 the alternate one.
    // A placed symbol that says it shows another style than the normal one is refused, but a
    // file may omit the selector, so an alternate style definition is accepted
    // only when it puts exactly the same pins in the same places.
    if symbol.units.iter().any(|unit| unit.style == 2) {
        // Names compare in full, as the file writes them, so names that differ only in spaces
        // or control characters (which the import trims) differ too; a name too long to
        // compare cheaply refuses instead.
        let compared = symbol.units.iter().filter(|unit| unit.style <= 2).flat_map(|unit| &unit.pins);
        if compared.clone().any(|pin| pin.name.len() > MAX_COMPARED_NAME) {
            return Err(format!("its alternate (De Morgan) body style can't be compared: a pin name is longer than {MAX_COMPARED_NAME} bytes"));
        }
        let pins = |style: u32| -> BTreeSet<StylePin<'_>> {
            symbol
                .units
                .iter()
                .filter(|unit| unit.style == 0 || unit.style == style)
                .flat_map(|unit| {
                    unit.pins.iter().map(move |pin| {
                        (unit.unit, pin.number.as_str(), pin.name.as_str(), pin.at.x_nm, pin.at.y_nm, format!("{:?}", pin.electrical), pin.hidden)
                    })
                })
                .collect()
        };
        if pins(1) != pins(2) {
            return Err("its alternate (De Morgan) body style places, names, types or hides pins differently".into());
        }
        report.note("alternate (De Morgan) body styles of symbols left out");
    }
    if symbol.units.iter().any(|unit| unit.style > 2) {
        report.note("symbol body styles beyond the alternate one left out");
    }
    let normal: Vec<&SymbolUnit> = symbol.units.iter().filter(|unit| unit.style <= 1).collect();
    let common: Vec<&SymbolUnit> = normal.iter().copied().filter(|unit| unit.unit == 0).collect();
    let mut numbers: Vec<u32> = normal.iter().map(|unit| unit.unit).filter(|n| *n > 0).collect::<BTreeSet<_>>().into_iter().collect();
    if numbers.is_empty() {
        // A symbol drawn only in its common part is one unit.
        numbers.push(1);
    }
    if numbers.len() > MAX_UNITS {
        return Err(format!("it has {} units; a native symbol holds at most {MAX_UNITS}", numbers.len()));
    }
    let single = numbers.len() == 1;
    let power_pin = symbol
        .power
        .then(|| {
            symbol
                .units
                .iter()
                .enumerate()
                .filter(|(_, unit)| unit.style <= 1)
                .flat_map(|(u, unit)| unit.pins.iter().enumerate().map(move |(p, pin)| (u, p, pin)))
                .find(|(_, _, pin)| pin.electrical == Electrical::PowerIn)
        })
        .flatten();
    let power_terminal = power_pin.map(|(_, _, pin)| pin.number.clone());
    let power_pin = power_pin.map(|(u, p, _)| (u, p));

    let mut terminals: BTreeMap<String, (Option<String>, &'static str)> = BTreeMap::new();
    let mut terminal_order: Vec<String> = Vec::new();
    let mut units = BTreeMap::new();
    let mut native_units = Vec::new();
    let mut exact = false;
    let mut shared_contacts = false;
    for (index, number) in numbers.iter().enumerate() {
        let unit_id = if single { "main".to_owned() } else { unit_letters(index) };
        let parts: Vec<&SymbolUnit> = common.iter().copied().chain(normal.iter().copied().filter(|u| u.unit == *number)).collect();
        let mut native = NativeUnit { id: unit_id.clone(), pins: Vec::new(), drawing: Vec::new() };
        let mut positions = BTreeSet::new();
        let mut numbers_here: BTreeSet<&str> = BTreeSet::new();
        for pin in parts.iter().flat_map(|unit| &unit.pins) {
            // Electrical pins cannot be skipped if conversion exhausts its budget.
            if !pay(budget, 1) {
                return Err("it has more pins than the remaining import budget".into());
            }
            let terminal = terminal(pin, symbol, power_terminal.as_deref(), report)?;
            match terminals.get(&pin.number) {
                Some(existing) if *existing != terminal => {
                    return Err(format!("pin {} has a different name or type in different units", pin.number));
                }
                Some(_) => {}
                None => {
                    terminal_order.push(pin.number.clone());
                    terminals.insert(pin.number.clone(), terminal);
                }
            }
            if !numbers_here.insert(&pin.number) {
                return Err(format!("pin number {} appears twice in one unit", pin.number));
            }
            if [pin.at.x_nm, pin.at.y_nm].into_iter().any(|nm| nm.unsigned_abs() > MAX_EXACT_PIN_OFFSET_NM as u64) {
                return Err(format!("pin {} is more than 100 mm from the symbol's origin", pin.number));
            }
            let at = (pin.at.x_nm, -pin.at.y_nm);
            exact |= !legacy_pin_offset(at.0, at.1);
            shared_contacts |= !positions.insert(at);
            let mut saved = json!({"terminal": pin.number, "at_nm": [at.0, at.1]});
            if let Some(appearance) = appearance(pin, at, symbol, report) {
                saved["appearance"] = appearance;
            }
            native.pins.push(saved);
        }
        exact |= native.pins.is_empty();
        // Stop at the native limit rather than converting everything and cutting it back: a
        // common part is drawn again in every unit, and a file can share one large drawing
        // among many symbols.
        let mut full = false;
        'parts: for unit in &parts {
            for graphic in &unit.graphics {
                if native.drawing.len() >= MAX_DRAWING {
                    full = true;
                    break 'parts;
                }
                if !pay(budget, cost(graphic)) {
                    break 'parts;
                }
                full |= drawing(graphic, &mut native.drawing, report);
            }
            for text in &unit.texts {
                if native.drawing.len() >= MAX_DRAWING {
                    full = true;
                    break 'parts;
                }
                if !pay(budget, 1) {
                    break 'parts;
                }
                if let Some(entry) = text_entry(text, report) {
                    native.drawing.push(entry);
                }
            }
        }
        if full {
            report.note("symbol drawing beyond 256 strokes per unit left out");
        }
        units.insert(*number, unit_id);
        native_units.push(native);
    }

    let terminals: Vec<Value> = terminal_order
        .iter()
        .filter_map(|number| {
            let (name, electrical_type) = terminals.get(number)?;
            let mut terminal = json!({"id": number, "electrical_type": electrical_type});
            if let Some(name) = name {
                terminal["name"] = json!(name);
            }
            Some(terminal)
        })
        .collect();
    let mut document = json!({
        "format": match (shared_contacts, exact) {
            (true, _) => crate::library::SYMBOL_FORMAT_V7,
            (_, true) => crate::library::SYMBOL_FORMAT_V6,
            _ => crate::library::SYMBOL_FORMAT_V4,
        },
        "id": id,
        "revision": 1,
        "name": clip(&symbol.name, 64).unwrap_or_else(|| id.clone()),
        "reference_prefix": reference_prefix(symbol.property("Reference").unwrap_or_default()),
        "default_value": symbol.property("Value").and_then(|v| clip(v, 64)).or_else(|| clip(&symbol.name, 64)).unwrap_or_else(|| id.clone()),
        "terminals": terminals,
        "units": native_units.into_iter().map(|u| json!({"id": u.id, "drawing": u.drawing, "pins": u.pins})).collect::<Vec<_>>(),
    });
    if let Some(terminal) = &power_terminal
        && !(preserve_local_power && symbol.power_local)
    {
        document["behavior"] = json!({"type": "power", "terminal": terminal});
    }
    let text = document.to_string();
    if text.len() > MAX_FILE_BYTES {
        return Err(format!("its native document would take {} KiB; a library file holds at most {} KiB", text.len() >> 10, MAX_FILE_BYTES >> 10));
    }
    Ok(Converted { id, text, units, power_terminal, power_pin })
}

/// A pin's logical terminal: its name and native electrical type. Fails for a number the native
/// format can't hold, and for a hidden power input on an ordinary symbol, which KiCad joins to a
/// net named after the pin without any wire.
fn terminal(pin: &Pin, symbol: &LibSymbol, power_terminal: Option<&str>, report: &mut Report) -> Result<(Option<String>, &'static str), String> {
    if !valid_number(&pin.number) {
        return Err(format!("pin number `{}` isn't 1 to 8 letters, digits, +, - or _", shown(&pin.number)));
    }
    if pin.hidden && pin.electrical == Electrical::PowerIn && power_terminal != Some(pin.number.as_str()) && !symbol.power {
        return Err(format!("hidden power pin {} joins a net by its name, which PCBCraft doesn't do", pin.number));
    }
    let electrical_type = match pin.electrical {
        Electrical::Input => "input",
        Electrical::Output => "output",
        Electrical::Bidirectional => "bidirectional",
        Electrical::TriState => "tri_state",
        Electrical::Passive => "passive",
        Electrical::Unspecified => "unspecified",
        Electrical::PowerIn => "power_in",
        Electrical::PowerOut => "power_out",
        Electrical::OpenCollector => "open_collector",
        Electrical::OpenEmitter => "open_emitter",
        Electrical::NoConnect => "no_connect",
        Electrical::Free => {
            report.note("free pins imported as unspecified");
            "unspecified"
        }
    };
    // A name of more than 128 bytes is longer than 32 characters; checked first so a huge name
    // isn't counted through.
    let name = if pin.name.len() > 128 || pin.name.chars().count() > 32 {
        report.note("pin names longer than 32 characters shortened");
        clip(&pin.name, 32)
    } else {
        clip(&pin.name, 32)
    };
    Ok((name, electrical_type))
}

/// How a visible pin is drawn: its lead from the connection point toward the body, its marks
/// and labels. `None` for hidden or zero-length pins, which draw nothing, and for leads the
/// native format can't draw (noted).
fn appearance(pin: &Pin, at: (i64, i64), symbol: &LibSymbol, report: &mut Report) -> Option<Value> {
    if pin.hidden || pin.length_nm == 0 {
        return None;
    }
    // KiCad's angle points from the connection point toward the body, counterclockwise with y
    // upward; on the y-down native canvas the same directions read as these.
    let (orientation, step) = match pin.rotation_mdeg {
        0 => ("right", (1, 0)),
        90_000 => ("up", (0, -1)),
        180_000 => ("left", (-1, 0)),
        270_000 => ("down", (0, 1)),
        _ => {
            report.note("pin leads at other than a quarter turn drawn without their lead");
            return None;
        }
    };
    let end = (at.0 + step.0 * pin.length_nm, at.1 + step.1 * pin.length_nm);
    let reach = -DRAWING_REACH_NM..=DRAWING_REACH_NM;
    if !(100_000..=100_000_000).contains(&pin.length_nm) || !reach.contains(&end.0) || !reach.contains(&end.1) {
        report.note("pin leads shorter than 0.1 mm or reaching beyond 100 mm drawn without their lead");
        return None;
    }
    let style = match pin.style.as_str() {
        "line" => "line",
        "inverted" => "inverted",
        "clock" => "clock",
        "inverted_clock" => "inverted_clock",
        _ => {
            report.note("pin marks other than inverted and clock drawn as plain leads");
            "line"
        }
    };
    let offset = symbol.pin_name_offset_nm.clamp(0, 100_000_000);
    let name_height = pin.name_height_nm.clamp(100_000, 5_000_000);
    let number_height = pin.number_height_nm.clamp(100_000, 5_000_000);
    if (offset, name_height, number_height) != (symbol.pin_name_offset_nm, pin.name_height_nm, pin.number_height_nm) {
        report.note("pin text dimensions outside native bounds clamped");
    }
    Some(json!({
        "orientation": orientation,
        "length_nm": pin.length_nm,
        "style": style,
        "show_name": !symbol.pin_names_hidden && !pin.name.is_empty(),
        "show_number": !symbol.pin_numbers_hidden,
        "name_height_nm": name_height,
        "number_height_nm": number_height,
        "name_offset_nm": offset,
    }))
}

/// How many points converting `graphic` visits.
fn cost(graphic: &Graphic) -> usize {
    match &graphic.shape {
        Shape::Rectangle { .. } | Shape::Circle { .. } => 2,
        Shape::Arc { .. } => 3,
        Shape::Polyline(points) => points.len().max(1),
        Shape::Bezier(_) => CURVE_STEPS + 1,
    }
}

/// Take `cost` from `budget`, which must stay above zero. When it is short, empties it and
/// returns false, so an empty budget always means the drawing stopped.
fn pay(budget: &mut usize, cost: usize) -> bool {
    match budget.checked_sub(cost).filter(|left| *left > 0) {
        Some(left) => {
            *budget = left;
            true
        }
        None => {
            *budget = 0;
            false
        }
    }
}

/// Append the native entries for one KiCad drawing, in the y-down frame. Drawings out of reach
/// are left out; curves, long polylines and fills the native format can't hold are converted
/// with a note. Returns whether pieces of a long polyline were left out because `out` reached
/// [`MAX_DRAWING`] entries.
fn drawing(graphic: &Graphic, out: &mut Vec<Value>, report: &mut Report) -> bool {
    let flip = |p: pcbcraft_kicad::Point| (p.x_nm, -p.y_nm);
    let stroke = match graphic.stroke_nm {
        0 => DEFAULT_STROKE_NM,
        width if width > MAX_STROKE_NM => {
            report.note("symbol strokes thicker than 2 mm drawn at 2 mm");
            MAX_STROKE_NM
        }
        width => width,
    };
    let fill = match graphic.fill {
        Fill::None => "none",
        Fill::Outline => "solid",
        Fill::Background => "background",
        Fill::Color => {
            report.note("custom symbol fill colors drawn solid");
            "solid"
        }
    };
    let entry = |geometry: Value, fill: &str| json!({"type": "graphic", "graphic": {"geometry": geometry, "stroke_nm": stroke, "fill": fill}});
    let within = |points: &[(i64, i64)]| points.iter().all(|(x, y)| x.abs() <= DRAWING_REACH_NM && y.abs() <= DRAWING_REACH_NM);
    let point = |(x, y): (i64, i64)| json!({"x_nm": x, "y_nm": y});
    match &graphic.shape {
        Shape::Rectangle { start, end } => {
            let (a, b) = (flip(*start), flip(*end));
            if !within(&[a, b]) || a.0 == b.0 || a.1 == b.1 {
                report.note("symbol drawings that are empty or beyond 100 mm left out");
                return false;
            }
            let (min, max) = ((a.0.min(b.0), a.1.min(b.1)), (a.0.max(b.0), a.1.max(b.1)));
            out.push(entry(json!({"type": "rectangle", "min": point(min), "max": point(max)}), fill));
        }
        Shape::Circle { center, radius_nm } => {
            let c = flip(*center);
            if !within(&[(c.0 - radius_nm, c.1 - radius_nm), (c.0 + radius_nm, c.1 + radius_nm)]) {
                report.note("symbol drawings that are empty or beyond 100 mm left out");
                return false;
            }
            out.push(entry(json!({"type": "circle", "center": point(c), "radius_nm": radius_nm}), fill));
        }
        Shape::Arc { start, mid, end } => {
            let (a, m, b) = (flip(*start), flip(*mid), flip(*end));
            match arc(a, m, b) {
                Some((center, radius, start_degrees, sweep_degrees))
                    if within(&[(center.0 - radius, center.1 - radius), (center.0 + radius, center.1 + radius)]) =>
                {
                    let geometry = json!({"type": "arc", "center": point(center), "radius_nm": radius, "start_degrees": start_degrees, "sweep_degrees": sweep_degrees});
                    out.push(entry(geometry, "none"));
                }
                _ if within(&[a, b]) && a != b => {
                    report.note("symbol arcs too flat to draw as arcs drawn as lines");
                    out.push(entry(json!({"type": "line", "from": point(a), "to": point(b)}), "none"));
                }
                _ => report.note("symbol drawings that are empty or beyond 100 mm left out"),
            }
        }
        Shape::Polyline(points) => return polyline(points.iter().copied().map(flip).collect(), fill, &entry, out, report),
        Shape::Bezier(control) => {
            report.note("symbol curves drawn as polylines");
            let [p0, p1, p2, p3] = control.map(flip);
            let points = (0..=CURVE_STEPS)
                .map(|i| {
                    let t = i as f64 / CURVE_STEPS as f64;
                    let u = 1.0 - t;
                    let mix = |a: i64, b: i64, c: i64, d: i64| {
                        (u * u * u * a as f64 + 3.0 * u * u * t * b as f64 + 3.0 * u * t * t * c as f64 + t * t * t * d as f64).round() as i64
                    };
                    (mix(p0.0, p1.0, p2.0, p3.0), mix(p0.1, p1.1, p2.1, p3.1))
                })
                .collect();
            return polyline(points, "none", &entry, out, report);
        }
    }
    false
}

/// Append a polyline, closed when its ends meet. A fill is kept only on a closed convex outline
/// of at most [`MAX_POINTS`] corners; longer runs are split into joined pieces. Pieces stop once
/// `out` holds [`MAX_DRAWING`] entries; returns whether any were left out for that reason.
fn polyline(mut points: Vec<(i64, i64)>, fill: &str, entry: &dyn Fn(Value, &str) -> Value, out: &mut Vec<Value>, report: &mut Report) -> bool {
    points.dedup();
    if points.len() < 2 || points.iter().any(|(x, y)| x.abs() > DRAWING_REACH_NM || y.abs() > DRAWING_REACH_NM) {
        report.note("symbol drawings that are empty or beyond 100 mm left out");
        return false;
    }
    let point = |&(x, y): &(i64, i64)| json!({"x_nm": x, "y_nm": y});
    let closed = points.len() >= 4 && points.first() == points.last();
    if closed && points.len() - 1 <= MAX_POINTS {
        points.pop();
        let fill = if fill != "none" && !convex(&points) {
            report.note("fills of symbol outlines that aren't convex left out");
            "none"
        } else {
            fill
        };
        out.push(entry(json!({"type": "polyline", "points": points.iter().map(point).collect::<Vec<_>>(), "closed": true}), fill));
        return false;
    }
    if fill != "none" {
        report.note("fills of open or very long symbol outlines left out");
    }
    let mut start = 0;
    while start + 1 < points.len() {
        if out.len() >= MAX_DRAWING {
            return true;
        }
        let end = (start + MAX_POINTS).min(points.len());
        let piece = points.get(start..end).unwrap_or_default();
        out.push(entry(json!({"type": "polyline", "points": piece.iter().map(point).collect::<Vec<_>>(), "closed": false}), "none"));
        start = end - 1;
    }
    false
}

/// Whether a closed outline (without its repeated first point) turns the same way at every
/// corner, so the native renderer can fill it.
fn convex(points: &[(i64, i64)]) -> bool {
    let n = points.len();
    if n < 3 {
        return false;
    }
    let mut sign = 0i128;
    for i in 0..n {
        let (a, b, c) = (points[i], points[(i + 1) % n], points[(i + 2) % n]);
        let cross = i128::from(b.0 - a.0) * i128::from(c.1 - b.1) - i128::from(b.1 - a.1) * i128::from(c.0 - b.0);
        if cross != 0 {
            if sign != 0 && cross.signum() != sign {
                return false;
            }
            sign = cross.signum();
        }
    }
    sign != 0
}

/// The circle through three points in the y-down frame, as center, radius and whole-degree start
/// and sweep from `a` through `m` to `b`. `None` when the points are (nearly) in a line or the
/// sweep rounds to nothing.
fn arc(a: (i64, i64), m: (i64, i64), b: (i64, i64)) -> Option<((i64, i64), i64, i64, i64)> {
    let (ax, ay, mx, my, bx, by) = (a.0 as f64, a.1 as f64, m.0 as f64, m.1 as f64, b.0 as f64, b.1 as f64);
    let d = 2.0 * (ax * (my - by) + mx * (by - ay) + bx * (ay - my));
    if d.abs() < 1.0 {
        return None;
    }
    let sq = |x: f64, y: f64| x * x + y * y;
    let cx = (sq(ax, ay) * (my - by) + sq(mx, my) * (by - ay) + sq(bx, by) * (ay - my)) / d;
    let cy = (sq(ax, ay) * (bx - mx) + sq(mx, my) * (ax - bx) + sq(bx, by) * (mx - ax)) / d;
    let radius = (ax - cx).hypot(ay - cy);
    let angle = |x: f64, y: f64| (y - cy).atan2(x - cx).to_degrees();
    let (start, through, end) = (angle(ax, ay), angle(mx, my), angle(bx, by));
    // Sweep from start to end in the direction that passes the middle point.
    let forward = (end - start).rem_euclid(360.0);
    let sweep = if (through - start).rem_euclid(360.0) <= forward { forward } else { forward - 360.0 };
    let (start, sweep) = (start.round() as i64, sweep.round() as i64);
    let radius = radius.round() as i64;
    (sweep != 0 && radius > 0 && radius <= DRAWING_REACH_NM).then_some(((cx.round() as i64, cy.round() as i64), radius, start, sweep))
}

/// A symbol text as a native drawing entry; `None`, noted, for text the native format can't hold.
fn text_entry(text: &Text, report: &mut Report) -> Option<Value> {
    if text.hidden {
        return None;
    }
    let at = (text.at.x_nm, -text.at.y_nm);
    // More than 512 bytes is more than 128 characters; checked first so a huge text isn't scanned.
    let printable = text.text.len() <= 512 && !text.text.chars().any(char::is_control) && (1..=128).contains(&text.text.chars().count());
    if !printable || at.0.abs() > DRAWING_REACH_NM || at.1.abs() > DRAWING_REACH_NM {
        report.note("symbol texts that are multi-line, longer than 128 characters or beyond 100 mm left out");
        return None;
    }
    let height = text.size_nm.clamp(100_000, 10_000_000);
    let align = match text.justify {
        Justify::Left => "start",
        Justify::Center => "middle",
        Justify::Right => "end",
    };
    let rotation = ((text.rotation_mdeg + 500) / 1000).rem_euclid(360);
    let geometry =
        json!({"type": "text", "at": {"x_nm": at.0, "y_nm": at.1}, "text": text.text, "height_nm": height, "rotation": rotation, "align": align});
    Some(json!({"type": "graphic", "graphic": {"geometry": geometry, "stroke_nm": DEFAULT_STROKE_NM, "fill": "none"}}))
}

/// The native unit ID for the `index`-th unit: `a`, `b`, … `z`, `aa`, `ab`, …
fn unit_letters(index: usize) -> String {
    let letter = |i: usize| char::from(b'a' + (i % 26) as u8);
    if index < 26 { letter(index).to_string() } else { format!("{}{}", letter(index / 26 - 1), letter(index)) }
}

/// The native reference prefix: the capital letters among the first 64 characters of KiCad's
/// reference prefix, at most four; `U` when there are none.
fn reference_prefix(kicad: &str) -> String {
    let letters: String = kicad.chars().take(64).filter(char::is_ascii_alphabetic).map(|c| c.to_ascii_uppercase()).take(4).collect();
    if letters.is_empty() { "U".to_owned() } else { letters }
}

/// Whether `number` is a pin number the native format holds: 1 to 8 ASCII letters, digits, `+`,
/// `-` or `_`. The length is checked first, so a long number isn't read through.
pub(super) fn valid_number(number: &str) -> bool {
    (1..=8).contains(&number.len()) && number.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'_'))
}

/// `text` as it can be shown in a message: at most 16 characters, control characters as `?`,
/// with `…` when it was longer.
pub(super) fn shown(text: &str) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(16).map(|c| if c.is_control() { '?' } else { c }).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

/// `text` without control characters, at most `limit` characters taken from its first
/// 4 × `limit`; `None` when that leaves it blank, as it does for a text that starts with that
/// many control characters.
pub(super) fn clip(text: &str, limit: usize) -> Option<String> {
    // Look at a bounded prefix only, so a huge text costs no more than a short one.
    let clipped: String = text.chars().take(limit.saturating_mul(4)).filter(|c| !c.is_control()).take(limit).collect();
    let trimmed = clipped.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// A native symbol ID for a KiCad symbol name: lowercase letters and digits joined by single
/// hyphens, at most 56 characters, so a caller can add a short suffix to keep IDs distinct.
pub(super) fn native_id(name: &str) -> String {
    let mut id = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            id.push(c.to_ascii_lowercase());
        } else if !id.is_empty() && !id.ends_with('-') {
            id.push('-');
        }
        if id.len() >= 56 {
            break;
        }
    }
    let id = id.trim_end_matches('-').to_owned();
    if id.is_empty() { "symbol".to_owned() } else { id }
}
