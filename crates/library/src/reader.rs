use crate::{
    sexpr::{self, Node},
    *,
};
use pcbcraft_geom::Point;
use std::collections::{BTreeMap, BTreeSet};

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
fn num(s: &str) -> Result<f64, Error> {
    let n = s
        .parse::<f64>()
        .map_err(|_| invalid(format!("invalid number {s:?}")))?;
    if !n.is_finite() || n.abs() > 100_000.0 {
        return Err(invalid("number outside finite geometry bounds"));
    }
    Ok(n)
}
fn point(node: &Node, field: &str, flip: bool) -> Result<Point, Error> {
    let p = node
        .child(field)
        .ok_or_else(|| invalid(format!("missing {field}")))?;
    Ok(Point::new(
        num(p.value(1))?,
        num(p.value(2))? * if flip { -1.0 } else { 1.0 },
    ))
}
fn angle(node: &Node) -> Result<f64, Error> {
    node.child("at")
        .map(|a| {
            if a.value(3).is_empty() {
                Ok(0.0)
            } else {
                num(a.value(3))
            }
        })
        .unwrap_or(Ok(0.0))
}
fn prop(node: &Node, name: &str) -> Option<String> {
    node.children
        .iter()
        .find(|x| x.name() == "property" && x.value(1) == name)
        .map(|x| x.value(2).to_owned())
}
fn graphic(node: &Node, flip: bool) -> Result<Option<Graphic>, Error> {
    let width = match node
        .child("stroke")
        .and_then(|x| x.child("width"))
        .or_else(|| node.child("width"))
    {
        Some(w) => num(w.value(1))?,
        None => 0.0,
    };
    let layer = node.field("layer").to_owned();
    let fill = node
        .child("fill")
        .map(|x| {
            if x.field("type").is_empty() {
                x.value(1)
            } else {
                x.field("type")
            }
        })
        .unwrap_or("none")
        .to_owned();
    let fill = match fill.as_str() {
        "no" => "none".to_owned(),
        "yes" => "solid".to_owned(),
        _ => fill,
    };
    Ok(Some(match node.name() {
        "polyline" | "fp_poly" => {
            let pts = node
                .child("pts")
                .ok_or_else(|| invalid("missing polygon points"))?;
            let points = pts
                .children
                .iter()
                .filter(|x| x.name() == "xy")
                .map(|x| {
                    Ok(Point::new(
                        num(x.value(1))?,
                        num(x.value(2))? * if flip { -1.0 } else { 1.0 },
                    ))
                })
                .collect::<Result<Vec<_>, Error>>()?;
            Graphic::Polyline {
                points,
                width,
                fill,
                layer,
            }
        }
        "rectangle" | "fp_rect" => Graphic::Rectangle {
            start: point(node, "start", flip)?,
            end: point(node, "end", flip)?,
            width,
            fill,
            layer,
        },
        "fp_line" => Graphic::Line {
            start: point(node, "start", flip)?,
            end: point(node, "end", flip)?,
            width,
            layer,
        },
        "circle" => Graphic::Circle {
            center: point(node, "center", flip)?,
            radius: num(node.field("radius"))?,
            width,
            fill,
            layer,
        },
        "fp_circle" => {
            let center = point(node, "center", flip)?;
            Graphic::Circle {
                center,
                radius: center.distance(point(node, "end", flip)?),
                width,
                fill,
                layer,
            }
        }
        "arc" | "fp_arc" => Graphic::Arc {
            start: point(node, "start", flip)?,
            mid: point(node, "mid", flip)?,
            end: point(node, "end", flip)?,
            width,
            layer,
        },
        "text" | "fp_text" => {
            let size = node
                .child("effects")
                .and_then(|x| x.child("font"))
                .map(|x| point(x, "size", false))
                .transpose()?
                .unwrap_or(Point::new(1.27, 1.27));
            Graphic::Text {
                position: point(node, "at", flip)?,
                angle: angle(node)?,
                text: node
                    .value(if node.name() == "fp_text" { 2 } else { 1 })
                    .to_owned(),
                size,
                layer,
            }
        }
        _ => return Ok(None),
    }))
}
fn unit(name: &str) -> Result<(u32, u32), Error> {
    let mut parts = name.rsplit('_');
    let style = parts
        .next()
        .and_then(|x| x.parse().ok())
        .ok_or_else(|| invalid("invalid symbol body style"))?;
    let unit = parts
        .next()
        .and_then(|x| x.parse().ok())
        .ok_or_else(|| invalid("invalid symbol unit"))?;
    Ok((unit, style))
}
fn resolve(
    name: &str,
    nodes: &BTreeMap<String, &Node>,
    source: &str,
    resolved: &mut BTreeMap<String, SymbolDefinition>,
    stack: &mut BTreeSet<String>,
    output_bytes: &mut usize,
) -> Result<SymbolDefinition, Error> {
    if let Some(s) = resolved.get(name) {
        return Ok(s.clone());
    }
    if stack.len() >= 64 {
        return Err(Error::Limit("inheritance depth"));
    }
    if !stack.insert(name.to_owned()) {
        return Err(invalid(format!("cyclic symbol inheritance at {name}")));
    }
    let node = nodes
        .get(name)
        .ok_or_else(|| invalid(format!("missing inherited symbol {name}")))?;
    let parent = node.child("extends").map(|x| x.value(1).to_owned());
    let mut s = if let Some(parent) = &parent {
        resolve(parent, nodes, source, resolved, stack, output_bytes)?
    } else {
        SymbolDefinition {
            name: name.to_owned(),
            reference: String::new(),
            value: name.to_owned(),
            description: String::new(),
            keywords: String::new(),
            footprint: String::new(),
            pins: Vec::new(),
            graphics: Vec::new(),
            extends: None,
            raw_source: String::new(),
            unsupported: Vec::new(),
        }
    };
    s.name = name.to_owned();
    s.extends = parent;
    s.raw_source = source[node.span.clone()].to_owned();
    for (key, target) in [
        ("Reference", &mut s.reference),
        ("Value", &mut s.value),
        ("Description", &mut s.description),
        ("ki_keywords", &mut s.keywords),
        ("Footprint", &mut s.footprint),
    ] {
        if let Some(value) = prop(node, key) {
            *target = value;
        }
    }
    if let Some(value) = prop(node, "ki_description") {
        s.description = value;
    }
    let mut local_units = BTreeSet::new();
    for child in &node.children {
        if child.name() == "symbol" {
            let (u, b) = unit(child.value(1))?;
            if !local_units.insert((u, b)) {
                return Err(invalid("duplicate symbol unit/body definition"));
            }
            // A derived local unit replaces that unit's inherited drawing/pins.
            s.pins.retain(|p| p.unit != u || p.body_style != b);
            s.graphics.retain(|g| g.unit != u || g.body_style != b);
            for part in &child.children {
                if part.name() == "pin" {
                    if part.value(2) != "line" {
                        s.unsupported.push(format!(
                            "pin {} graphical style {} is preserved in raw source",
                            part.field("number"),
                            part.value(2)
                        ));
                    }
                    s.pins.push(SymbolPin {
                        number: part.field("number").to_owned(),
                        name: part.field("name").to_owned(),
                        position: point(part, "at", true)?,
                        angle: angle(part)?,
                        length: num(part.field("length"))?,
                        electrical_type: part.value(1).to_owned(),
                        unit: u,
                        body_style: b,
                    });
                    if part.child("alternate").is_some() {
                        s.unsupported.push(format!(
                            "pin {} alternate functions preserved in raw source",
                            part.field("number")
                        ));
                    }
                } else if let Some(primitive) = graphic(part, true)? {
                    s.graphics.push(UnitGraphic {
                        unit: u,
                        body_style: b,
                        primitive,
                    });
                } else if part.atom.is_none() {
                    s.unsupported
                        .push(format!("symbol unit expression {}", part.name()));
                }
            }
        } else if child.atom.is_none()
            && !matches!(
                child.name(),
                "property"
                    | "extends"
                    | "pin_names"
                    | "pin_numbers"
                    | "in_bom"
                    | "on_board"
                    | "exclude_from_sim"
                    | "in_pos_files"
                    | "duplicate_pin_numbers_are_jumpers"
                    | "embedded_fonts"
            )
        {
            s.unsupported
                .push(format!("symbol expression {}", child.name()));
        }
    }
    if node.field("embedded_fonts") == "yes" {
        s.unsupported
            .push("embedded fonts require unsupported font decoding".into());
    }
    s.validate()?;
    // Inheritance can expand a tiny source into many large copies. Bound the
    // resulting definitions as well as the input AST, before adding each copy.
    let bytes = s.raw_source.len()
        + s.name.len()
        + s.reference.len()
        + s.value.len()
        + s.description.len()
        + s.keywords.len()
        + s.footprint.len()
        + s.pins
            .iter()
            .map(|p| 256 + p.number.len() + p.name.len() + p.electrical_type.len())
            .sum::<usize>()
        + s.graphics
            .iter()
            .map(|g| {
                512 + match &g.primitive {
                    Graphic::Polyline { points, .. } => points.len() * 32,
                    Graphic::Text { text, .. } => text.len(),
                    _ => 0,
                }
            })
            .sum::<usize>()
        + s.unsupported.iter().map(|x| 64 + x.len()).sum::<usize>();
    *output_bytes = output_bytes
        .checked_add(bytes)
        .ok_or(Error::Limit("resolved library bytes"))?;
    if *output_bytes > 128 * 1024 * 1024 {
        return Err(Error::Limit("resolved library bytes"));
    }
    stack.remove(name);
    resolved.insert(name.to_owned(), s.clone());
    Ok(s)
}
pub fn read_symbols(source: &str) -> Result<Vec<SymbolDefinition>, Error> {
    let root = sexpr::parse(source)?;
    if root.name() != "kicad_symbol_lib" {
        return Err(invalid("expected kicad_symbol_lib"));
    }
    let mut nodes = BTreeMap::new();
    for node in root.children.iter().filter(|x| x.name() == "symbol") {
        if nodes.insert(node.value(1).to_owned(), node).is_some() {
            return Err(invalid("duplicate symbol name"));
        }
    }
    if nodes.len() > 20_000 {
        return Err(Error::Limit("symbol count"));
    }
    let mut result = BTreeMap::new();
    let mut output_bytes = 0;
    for name in nodes.keys() {
        resolve(
            name,
            &nodes,
            source,
            &mut result,
            &mut BTreeSet::new(),
            &mut output_bytes,
        )?;
    }
    Ok(result.into_values().collect())
}
pub fn read_footprint(source: &str) -> Result<FootprintDefinition, Error> {
    let node = sexpr::parse(source)?;
    if !matches!(node.name(), "footprint" | "module") {
        return Err(invalid("expected footprint"));
    }
    let mut s = FootprintDefinition {
        name: node.value(1).to_owned(),
        description: node.field("descr").to_owned(),
        keywords: node.field("tags").to_owned(),
        pads: Vec::new(),
        graphics: Vec::new(),
        raw_source: source[node.span.clone()].to_owned(),
        unsupported: Vec::new(),
    };
    for child in &node.children {
        if child.name() == "pad" {
            let drill = child
                .child("drill")
                .map(|d| {
                    let oval = d.value(1) == "oval";
                    let x = num(d.value(if oval { 2 } else { 1 }))?;
                    let y = if oval { num(d.value(3))? } else { x };
                    Ok(Drill {
                        size: Point::new(x, y),
                        offset: if d.child("offset").is_some() {
                            point(d, "offset", false)?
                        } else {
                            Point::default()
                        },
                    })
                })
                .transpose()?;
            let layers = child
                .child("layers")
                .ok_or_else(|| invalid("missing pad layers"))?
                .children
                .iter()
                .skip(1)
                .filter_map(|x| x.atom.clone())
                .collect();
            let shape = child.value(3).to_owned();
            if !matches!(shape.as_str(), "circle" | "rect" | "oval" | "roundrect") {
                s.unsupported.push(format!(
                    "pad {} shape {shape} requires unsupported geometry",
                    child.value(1)
                ));
            }
            if child.child("primitives").is_some() || child.child("chamfer").is_some() {
                s.unsupported
                    .push(format!("pad {} custom/chamfer geometry", child.value(1)));
            }
            s.pads.push(FootprintPad {
                number: child.value(1).to_owned(),
                pad_type: child.value(2).to_owned(),
                shape,
                position: point(child, "at", false)?,
                rotation: angle(child)?,
                size: point(child, "size", false)?,
                layers,
                drill,
                roundrect_ratio: child
                    .child("roundrect_rratio")
                    .map(|r| num(r.value(1)))
                    .transpose()?,
                raw_source: source[child.span.clone()].to_owned(),
            });
        } else if let Some(g) = graphic(child, false)? {
            s.graphics.push(g);
        } else if child.atom.is_none()
            && !matches!(
                child.name(),
                "version"
                    | "generator"
                    | "generator_version"
                    | "layer"
                    | "descr"
                    | "tags"
                    | "property"
                    | "attr"
                    | "duplicate_pad_numbers_are_jumpers"
                    | "embedded_fonts"
                    | "model"
            )
        {
            s.unsupported
                .push(format!("footprint expression {}", child.name()));
        }
    }
    s.validate()?;
    Ok(s)
}
