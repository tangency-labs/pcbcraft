use super::{CoordinateFrame, Layer, LayerRole, Mark, add, coordinate, decimal, point, xy};
use crate::schematic::Point;
use std::{collections::BTreeMap, sync::Arc};

enum Aperture {
    Circle(i64),
    Rectangle([i64; 2]),
    Obround([i64; 2]),
    Diamond(i64),
    Macro(Arc<[Mark]>),
}

#[derive(Default)]
struct Plot {
    role: Option<LayerRole>,
    polarity: Option<bool>,
    format: bool,
    metric: bool,
    dark: bool,
    linear: bool,
    frame: Option<CoordinateFrame>,
    apertures: BTreeMap<u32, Aperture>,
    macros: BTreeMap<String, Arc<[Mark]>>,
    selected: Option<u32>,
    at: Option<Point>,
    region: Option<Vec<Point>>,
    marks: Vec<Mark>,
}

/// Parse the PCBcraft writer's line-delimited X2 subset, including rounded-pad macros.
/// Fail a layer on unsupported geometry rather than showing a partial manufacturing plot.
pub(super) fn read(text: &str, budget: &mut usize) -> Result<(Layer, CoordinateFrame), String> {
    const MAX_DEFINITIONS: usize = 65_536;
    let mut plot = Plot::default();
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    loop {
        let line = lines.next().ok_or("Missing Gerber end record")?;
        if line == "M02*" {
            break;
        }
        if line.len() > 16_384 {
            return Err("Gerber record is too long".into());
        }
        if plot.apertures.len() + plot.macros.len() > MAX_DEFINITIONS {
            return Err("Gerber plot has too many aperture definitions".into());
        }
        if let Some(name) = line.strip_prefix("%AM").and_then(|name| name.strip_suffix('*')) {
            if !name.starts_with("RoundRect") || plot.macros.contains_key(name) {
                return Err("Unsupported or duplicate aperture macro".into());
            }
            let mut marks = Vec::new();
            loop {
                let record = lines.next().ok_or("Unterminated aperture macro")?;
                if record == "%" {
                    break;
                }
                if marks.len() >= 9 || record.len() > 512 {
                    return Err("Aperture macro has too many primitives".into());
                }
                marks.push(macro_primitive(record)?);
            }
            if marks.is_empty() {
                return Err("Gerber aperture macro has no primitives".into());
            }
            plot.macros.insert(name.into(), marks.into());
        } else {
            plot.record(line, budget).map_err(|error| format!("{error} ({})", line.chars().take(75).collect::<String>()))?;
        }
    }
    let role = plot.role.ok_or("Gerber file function is missing")?;
    let frame = plot.frame.ok_or("Gerber coordinate frame is missing")?;
    let is_mask = matches!(role, LayerRole::TopMask | LayerRole::BottomMask);
    if lines.next().is_some() || plot.region.is_some() || !plot.ready() || plot.polarity != Some(!is_mask) {
        return Err("Incomplete or unsupported PCBcraft Gerber plot".into());
    }
    Ok((Layer { name: String::new(), role, marks: plot.marks, tools: Vec::new() }, frame))
}

impl Plot {
    /// Geometry and overlays require the writer's common frame, metric units and dark primitives.
    fn ready(&self) -> bool {
        self.format && self.metric && self.dark && self.linear && self.frame.is_some()
    }

    /// Handle format and drawing records; attribute dictionaries carry no additional geometry.
    fn record(&mut self, record: &str, budget: &mut usize) -> Result<(), String> {
        match record {
            "%FSLAX46Y46*%" => self.format = true,
            "%MOMM*%" => self.metric = true,
            "%LPD*%" => self.dark = true,
            "G01*" => self.linear = true,
            r if r.starts_with("%TF.SameCoordinates,") && self.frame.is_none() => {
                let value = r.strip_prefix("%TF.SameCoordinates,").and_then(|s| s.strip_suffix("*%")).ok_or("Invalid Gerber coordinate frame")?;
                self.frame = Some(CoordinateFrame::parse(value)?);
            }
            "%TF.FilePolarity,Positive*%" => self.polarity = Some(true),
            "%TF.FilePolarity,Negative*%" => self.polarity = Some(false),
            "G36*" if self.region.is_none() && self.ready() => self.region = Some(Vec::new()),
            "G37*" => {
                let mut vertices = self.region.take().ok_or("No open region")?;
                if vertices.len() < 3 || vertices.first() != vertices.last() {
                    return Err("Gerber region is open".into());
                }
                vertices.pop();
                for triangle in triangulate(vertices)? {
                    add(&mut self.marks, Mark::Polygon(Box::new(triangle)), budget)?;
                }
            }
            r if r.starts_with("G04") && r.strip_suffix('*').is_some_and(|comment| !comment.contains('*')) => {}
            r if r.starts_with("%TF.FileFunction,") => self.role = Some(role(r)?),
            r if ["%TF.GenerationSoftware,", "%TA.", "%TO.", "%TD"].iter().any(|prefix| r.starts_with(prefix))
                && r[1..].strip_suffix("*%").is_some_and(|attribute| !attribute.contains(['*', '%'])) => {}
            r if r.starts_with("%ADD") => {
                let (id, aperture) = aperture(r, &self.macros)?;
                if self.apertures.insert(id, aperture).is_some() {
                    return Err("Duplicate Gerber aperture".into());
                }
            }
            r if r.starts_with('D') && r.ends_with('*') => {
                let id = r[1..r.len() - 1].parse::<u32>().map_err(|_| "Invalid aperture selection")?;
                if !self.apertures.contains_key(&id) {
                    return Err("Undefined aperture".into());
                }
                self.selected = Some(id);
            }
            r if r.starts_with('X') && r.ends_with('*') && self.ready() => self.draw(r, budget)?,
            _ => return Err("Unsupported Gerber record".into()),
        }
        Ok(())
    }

    /// Interpret a move, linear stroke, flash or single-contour region vertex.
    fn draw(&mut self, record: &str, budget: &mut usize) -> Result<(), String> {
        let (position, operation) = record.split_once('D').ok_or("Expected explicit Gerber operation")?;
        let p = xy(position, coordinate)?;
        let previous = self.at.replace(p);
        if let Some(region) = self.region.as_mut() {
            if region.len() >= 64 || (if region.is_empty() { operation != "02*" } else { operation != "01*" }) {
                return Err("Unsupported Gerber region contour".into());
            }
            region.push(p);
            return Ok(());
        }
        if operation == "02*" {
            return Ok(());
        }
        let aperture = self.selected.and_then(|id| self.apertures.get(&id)).ok_or("No active Gerber aperture")?;
        match (operation, aperture) {
            ("01*", Aperture::Circle(diameter_nm)) => {
                add(&mut self.marks, Mark::Stroke { ends: [previous.ok_or("No starting point for stroke")?, p], diameter_nm: *diameter_nm }, budget)
            }
            ("03*", aperture) => aperture.flash(p, &mut self.marks, budget),
            _ => Err("Unsupported Gerber draw operation".into()),
        }
    }
}

impl Aperture {
    /// Expand a flash to artwork in the common output frame.
    fn flash(&self, center: Point, marks: &mut Vec<Mark>, budget: &mut usize) -> Result<(), String> {
        let shift = |p: Point| point(p.x_nm + center.x_nm, p.y_nm + center.y_nm);
        let mark = match self {
            Self::Circle(diameter_nm) => Mark::Stroke { ends: [center; 2], diameter_nm: *diameter_nm },
            Self::Rectangle(size_nm) => Mark::Rectangle { center, size_nm: *size_nm },
            Self::Obround([w, h]) => {
                let dx = (w - h).max(0) / 2;
                let dy = (h - w).max(0) / 2;
                Mark::Stroke { ends: [shift(point(-dx, -dy)), shift(point(dx, dy))], diameter_nm: *w.min(h) }
            }
            Self::Diamond(diameter) => {
                let r = diameter / 2;
                Mark::Polygon(Box::new([point(-r, 0), point(0, r), point(r, 0), point(0, -r)].map(shift)))
            }
            Self::Macro(shapes) => {
                for shape in shapes.iter() {
                    let shape = match shape {
                        Mark::Stroke { ends, diameter_nm } => Mark::Stroke { ends: ends.map(shift), diameter_nm: *diameter_nm },
                        Mark::Strip { ends, width_nm } => Mark::Strip { ends: ends.map(shift), width_nm: *width_nm },
                        Mark::Polygon(vertices) => Mark::Polygon(vertices.iter().copied().map(shift).collect()),
                        Mark::Rectangle { .. } => return Err("Unsupported Gerber macro flash".into()),
                    };
                    add(marks, shape, budget)?;
                }
                return Ok(());
            }
        };
        add(marks, mark, budget)
    }
}

/// Map X2 file-function tokens to fabrication roles without inferring them from filenames.
fn role(record: &str) -> Result<LayerRole, String> {
    let value = record.strip_prefix("%TF.FileFunction,").and_then(|r| r.strip_suffix("*%")).ok_or("Invalid X2 file function")?;
    let fields: Vec<_> = value.split(',').collect();
    Ok(match fields.as_slice() {
        ["Copper", layer, side] if layer.strip_prefix('L').is_some_and(|n| n.parse::<u8>().is_ok_and(|n| (1..=64).contains(&n))) => match *side {
            "Top" => LayerRole::TopCopper,
            "Inr" => LayerRole::InnerCopper,
            "Bot" => LayerRole::BottomCopper,
            _ => return Err("Unsupported X2 copper layer".into()),
        },
        ["Soldermask", "Top"] => LayerRole::TopMask,
        ["Soldermask", "Bot"] => LayerRole::BottomMask,
        ["Paste", "Top"] => LayerRole::TopPaste,
        ["Paste", "Bot"] => LayerRole::BottomPaste,
        ["Legend", "Top"] => LayerRole::TopSilkscreen,
        ["Legend", "Bot"] => LayerRole::BottomSilkscreen,
        ["Profile", "NP"] => LayerRole::Profile,
        _ => return Err("Unsupported X2 file function".into()),
    })
}

/// Parse the aperture definitions the standalone board writer emits.
fn aperture(record: &str, macros: &BTreeMap<String, Arc<[Mark]>>) -> Result<(u32, Aperture), String> {
    let definition = record.strip_prefix("%ADD").and_then(|s| s.strip_suffix("*%")).ok_or("Invalid aperture definition")?;
    let shape = definition.trim_start_matches(|c: char| c.is_ascii_digit());
    let id = definition[..definition.len() - shape.len()].parse::<u32>().ok().filter(|id| *id >= 10).ok_or("Invalid aperture ID")?;
    let aperture = if let Some((kind, sizes)) = shape.split_once(',') {
        let dimensions: Vec<_> = sizes.split('X').collect();
        match (kind, dimensions.as_slice()) {
            ("C", [d]) => Aperture::Circle(size(d)?),
            ("R", [w, h]) => Aperture::Rectangle([size(w)?, size(h)?]),
            ("O", [w, h]) if (size(w)? - size(h)?) % 2 == 0 => Aperture::Obround([size(w)?, size(h)?]),
            ("P", [d, "4", "0.0"]) => Aperture::Diamond(size(d)?),
            _ => return Err("Unsupported aperture definition".into()),
        }
    } else {
        Aperture::Macro(macros.get(shape).ok_or("Undefined Gerber macro")?.clone())
    };
    Ok((id, aperture))
}

/// Require positive dimensions for dark artwork.
fn size(s: &str) -> Result<i64, String> {
    let value = decimal(s)?;
    if value > 0 { Ok(value) } else { Err("Invalid aperture dimensions".into()) }
}

/// Parse the exact exposed circle, strip and convex quadrilateral macro primitives.
fn macro_primitive(record: &str) -> Result<Mark, String> {
    let fields: Vec<_> = record.strip_suffix('*').ok_or("Invalid aperture macro primitive")?.split(',').collect();
    let at = |x, y| -> Result<Point, String> { Ok(point(decimal(x)?, decimal(y)?)) };
    match fields.as_slice() {
        ["1", "1", d, x, y, "0"] => Ok(Mark::Stroke { ends: [at(x, y)?; 2], diameter_nm: size(d)? }),
        ["20", "1", d, ax, ay, bx, by, "0"] => {
            let ends = [at(ax, ay)?, at(bx, by)?];
            if ends[0] == ends[1] {
                return Err("Aperture macro strip has identical endpoints".into());
            }
            Ok(Mark::Strip { ends, width_nm: size(d)? })
        }
        ["4", "1", "4", ax, ay, bx, by, cx, cy, dx, dy, ex, ey, "0"] if at(ax, ay)? == at(ex, ey)? => {
            let vertices = [at(ax, ay)?, at(bx, by)?, at(cx, cy)?, at(dx, dy)?];
            if triangulate(vertices.to_vec())?.len() != 2
                || !(0..4).all(|i| {
                    turn(vertices[i], vertices[(i + 1) % 4], vertices[(i + 2) % 4]).signum() == turn(vertices[0], vertices[1], vertices[2]).signum()
                })
            {
                return Err("Invalid aperture macro quadrilateral".into());
            }
            Ok(Mark::Polygon(Box::new(vertices)))
        }
        _ => Err("Unsupported aperture macro primitive".into()),
    }
}

fn turn(a: Point, b: Point, c: Point) -> i128 {
    i128::from(b.x_nm - a.x_nm) * i128::from(c.y_nm - a.y_nm) - i128::from(b.y_nm - a.y_nm) * i128::from(c.x_nm - a.x_nm)
}

/// Split short regions into convex triangles after removing duplicate and collinear vertices.
/// Accept collinear collapsed pads, and reject non-collinear contours whose signed areas cancel.
fn triangulate(mut points: Vec<Point>) -> Result<Vec<[Point; 3]>, String> {
    let area: i128 = points.iter().zip(points.iter().cycle().skip(1)).map(|(&a, &b)| turn(point(0, 0), a, b)).sum();
    if area == 0 {
        if let Some(edge) = points.windows(2).find(|edge| edge[0] != edge[1])
            && points.iter().any(|&p| turn(edge[0], edge[1], p) != 0)
        {
            return Err("Gerber region has zero signed area but is not collinear".into());
        }
        return Ok(Vec::new());
    }
    if area < 0 {
        points.reverse();
    }
    points.dedup();
    let mut triangles = Vec::new();
    while points.len() >= 3 {
        let len = points.len();
        let (index, triangle) = (0..len)
            .find_map(|i| {
                let [prev, next] = [(i + len - 1) % len, (i + 1) % len];
                let [a, b, c] = [points[prev], points[i], points[next]];
                match turn(a, b, c) {
                    0 => Some((i, None)),
                    t if t > 0
                        && !points
                            .iter()
                            .enumerate()
                            .any(|(j, &p)| j != prev && j != i && j != next && turn(a, b, p) >= 0 && turn(b, c, p) >= 0 && turn(c, a, p) >= 0) =>
                    {
                        Some((i, Some([a, b, c])))
                    }
                    _ => None,
                }
            })
            .ok_or("Unsupported Gerber region geometry")?;
        triangles.extend(triangle);
        points.remove(index);
    }
    Ok(triangles)
}
