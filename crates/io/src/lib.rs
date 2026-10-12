//! Native project persistence and original, limited KiCad PCB / preview writers.
use pcbcraft_doc::Project;
use pcbcraft_geom::Point;
use std::io::Read;
#[cfg(not(target_arch = "wasm32"))]
use std::io::Write;
use std::path::Path;
mod library_render;
use library_render::*;

pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

pub fn load(path: &Path) -> Result<Project, String> {
    let text = read_text(path)?;
    let project: Project = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    project.validate()?;
    Ok(project)
}
pub fn read_text(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err("project exceeds 8 MiB".into());
    }
    Ok(text)
}
#[cfg(not(target_arch = "wasm32"))]
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temporary.write_all(bytes).map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
#[cfg(target_arch = "wasm32")]
pub fn atomic_write(_path: &Path, _bytes: &[u8]) -> Result<(), String> {
    Err("host filesystem writes are unavailable in the browser; download support is planned".into())
}
pub fn save(project: &Project, path: &Path) -> Result<(), String> {
    project.validate()?;
    let bytes = serde_json::to_vec_pretty(project).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("project exceeds 8 MiB".into());
    }
    atomic_write(path, &bytes)
}
fn quote(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
}
fn uuid(id: u64, kind: usize) -> String {
    format!("{kind:08x}-0000-4000-8000-{id:012x}")
}
fn footprint_bounds(p: &Project, f: &pcbcraft_doc::Footprint) -> (Point, Point) {
    let points = f.drawing_points(p);
    let min = points
        .iter()
        .fold(Point::new(f64::INFINITY, f64::INFINITY), |bounds, p| {
            Point::new(bounds.x.min(p.x), bounds.y.min(p.y))
        });
    let max = points.iter().fold(
        Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
        |bounds, p| Point::new(bounds.x.max(p.x), bounds.y.max(p.y)),
    );
    if min.valid() && max.valid() {
        (min, max)
    } else {
        (Point::new(-3.4, -1.4), Point::new(3.4, 1.4))
    }
}

pub fn kicad_board(p: &Project) -> Result<String, String> {
    p.validate()?;
    let b = &p.board;
    let mut s = format!(
        "(kicad_pcb (version 20240108) (generator \"pcbcraft\")\n (general (thickness {})) (paper \"A4\")\n (layers (0 \"F.Cu\" signal) (31 \"B.Cu\" signal) (34 \"B.Paste\" user) (35 \"F.Paste\" user) (36 \"B.SilkS\" user \"b.silkscreen\") (37 \"F.SilkS\" user \"f.silkscreen\") (38 \"B.Mask\" user) (39 \"F.Mask\" user) (44 \"Edge.Cuts\" user) (46 \"B.CrtYd\" user) (47 \"F.CrtYd\" user) (48 \"B.Fab\" user) (49 \"F.Fab\" user))\n (setup (pad_to_mask_clearance 0))\n (net 0 \"\")\n",
        b.thickness
    );
    for (i, n) in p.nets.iter().enumerate() {
        s.push_str(&format!(" (net {} {})\n", i + 1, quote(&n.name)));
    }
    let ordinal = |net: Option<u64>| {
        p.nets
            .iter()
            .position(|n| Some(n.id) == net)
            .map(|i| i + 1)
            .unwrap_or(0)
    };
    for f in &b.footprints {
        let Some(c) = p.component(f.component) else {
            return Err("missing component".into());
        };
        let name = c
            .library_footprint
            .as_ref()
            .map(|l| {
                if l.library == "project" {
                    format!("pcbcraft_{}", l.definition.name)
                } else {
                    format!("{}:{}", l.library, l.definition.name)
                }
            })
            .unwrap_or_else(|| format!("pcbcraft_{}", c.kind));
        let attr = if f.pads.iter().any(|p| {
            p.geometry
                .as_ref()
                .is_some_and(|g| g.pad_type == "thru_hole")
        }) {
            "through_hole"
        } else {
            "smd"
        };
        let (min, max) = footprint_bounds(p, f);
        let label_x = (min.x + max.x) / 2.0;
        let reference_y = min.y - 1.2;
        let value_y = max.y + 1.2;
        s.push_str(&format!(" (footprint {} (layer \"F.Cu\") (uuid {}) (at {} {} {})\n (property \"Reference\" {} (at {label_x} {reference_y} {}) (layer \"F.SilkS\") (effects (font (size 1 1) (thickness 0.15))))\n (property \"Value\" {} (at {label_x} {value_y} {}) (layer \"F.SilkS\") (effects (font (size 1 1) (thickness 0.15))))\n (attr {attr})\n",quote(&name),uuid(f.id,1),f.position.x,f.position.y,f.rotation,quote(&c.reference),f.rotation,quote(&c.value),f.rotation));
        if let Some(library) = &c.library_footprint {
            for graphic in &library.definition.graphics {
                s.push_str(&kicad_graphic(graphic, f.rotation)?);
            }
        } else {
            s.push_str(" (fp_rect (start -3.4 -1.4) (end 3.4 1.4) (stroke (width 0.12) (type solid)) (fill none) (layer \"F.SilkS\"))\n");
        }
        for (i, pad) in f.pads.iter().enumerate() {
            let net = pad.net.map(|n| p.net_name(n)).unwrap_or("");
            let (pad_type, shape, size, rotation, layers, ratio, drill) =
                if let Some(g) = &pad.geometry {
                    if !matches!(g.pad_type.as_str(), "smd" | "thru_hole")
                        || !matches!(g.shape.as_str(), "circle" | "rect" | "oval" | "roundrect")
                        || (g.pad_type == "smd" && g.drill.is_some())
                        || (g.pad_type == "thru_hole" && g.drill.is_none())
                    {
                        return Err(
                        "KiCad export supports standard SMD or plated through-hole pad geometry"
                            .into(),
                    );
                    }
                    (
                        g.pad_type.as_str(),
                        g.shape.as_str(),
                        g.size,
                        f.rotation + g.rotation,
                        g.layers
                            .iter()
                            .map(|l| quote(l))
                            .collect::<Vec<_>>()
                            .join(" "),
                        if g.shape == "roundrect" {
                            format!(" (roundrect_rratio {})", g.roundrect_ratio.unwrap_or(0.0))
                        } else {
                            String::new()
                        },
                        g.drill
                            .as_ref()
                            .map(|d| {
                                let shape = if (d.size.x - d.size.y).abs() < 1e-9 {
                                    d.size.x.to_string()
                                } else {
                                    format!("oval {} {}", d.size.x, d.size.y)
                                };
                                format!(" (drill {shape} (offset {} {}))", d.offset.x, d.offset.y)
                            })
                            .unwrap_or_default(),
                    )
                } else {
                    (
                        "smd",
                        "circle",
                        Point::new(pad.diameter, pad.diameter),
                        f.rotation,
                        "\"F.Cu\" \"F.Mask\"".into(),
                        String::new(),
                        String::new(),
                    )
                };
            s.push_str(&format!(" (pad {} {pad_type} {shape} (at {} {} {rotation}) (size {} {}) (layers {layers}){ratio}{drill} (net {} {}) (uuid {}))\n",quote(&pad.number),pad.offset.x,pad.offset.y,size.x,size.y,ordinal(pad.net),quote(net),uuid(f.id,i+2)));
        }
        s.push_str(" )\n");
    }
    for t in &b.tracks {
        s.push_str(&format!(
            " (segment (start {} {}) (end {} {}) (width {}) (layer {}) (net {}) (uuid {}))\n",
            t.start.x,
            t.start.y,
            t.end.x,
            t.end.y,
            t.width,
            quote(t.layer.name()),
            ordinal(Some(t.net)),
            uuid(t.id, 1)
        ));
    }
    s.push_str(&format!(" (gr_rect (start 0 0) (end {} {}) (stroke (width 0.05) (type solid)) (fill none) (layer \"Edge.Cuts\"))\n",b.width,b.height));
    for c in &b.cutouts {
        s.push_str(&format!(" (gr_circle (center {} {}) (end {} {}) (stroke (width 0.05) (type solid)) (fill none) (layer \"Edge.Cuts\") (uuid {}))\n",c.center.x,c.center.y,c.center.x+c.radius,c.center.y,uuid(c.id,1)));
    }
    s.push_str(")\n");
    Ok(s)
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn preview_bounds(p: &Project, view: &str) -> (Point, Point) {
    if view == "pcb" || p.components.is_empty() {
        return (Point::default(), Point::new(p.board.width, p.board.height));
    }
    let mut min = Point::new(f64::INFINITY, f64::INFINITY);
    let mut max = Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
    for c in p.components.iter().flat_map(|c| c.schematic_views()) {
        for point in c.drawing_points() {
            let point = c.position.offset(point.rotate(-c.rotation));
            min.x = min.x.min(point.x);
            min.y = min.y.min(point.y);
            max.x = max.x.max(point.x);
            max.y = max.y.max(point.y);
        }
    }
    if !min.x.is_finite() || !min.y.is_finite() || !max.x.is_finite() || !max.y.is_finite() {
        return (Point::default(), Point::new(60.0, 40.0));
    }
    (min, max)
}
pub fn svg(p: &Project, view: &str) -> Result<String, String> {
    p.validate()?;
    if view != "pcb" && view != "schematic" {
        return Err("view must be pcb or schematic".into());
    }
    let (min, max) = preview_bounds(p, view);
    let width = (max.x - min.x).max(1.0) + 16.0;
    let height = (max.y - min.y).max(1.0) + 16.0;
    let mut s = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1000\" height=\"680\" viewBox=\"{} {} {width} {height}\"><rect x=\"{}\" y=\"{}\" width=\"{width}\" height=\"{height}\" fill=\"#131c29\"/><g font-family=\"sans-serif\" font-size=\"1.5\" fill=\"#dbe7f2\">",
        min.x - 8.0,
        min.y - 8.0,
        min.x - 8.0,
        min.y - 8.0
    );
    if view == "pcb" {
        s.push_str(&format!("<rect width=\"{}\" height=\"{}\" rx=\"0.4\" fill=\"#193a36\" stroke=\"#5ae0bc\" stroke-width=\"0.15\"/>",p.board.width,p.board.height));
        for c in &p.board.cutouts {
            s.push_str(&format!("<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"#131c29\" stroke=\"#5ae0bc\" stroke-width=\"0.1\"/>",c.center.x,c.center.y,c.radius));
        }
        for t in &p.board.tracks {
            s.push_str(&format!("<path d=\"M {} {} L {} {}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\" stroke-linecap=\"round\"/>",t.start.x,t.start.y,t.end.x,t.end.y,if t.layer==pcbcraft_doc::Layer::Front {"#f58d79"} else {"#74a7ec"},t.width));
        }
        for f in &p.board.footprints {
            if let Some(library) = p
                .component(f.component)
                .and_then(|c| c.library_footprint.as_ref())
            {
                s.push_str(&format!(
                    "<g transform=\"translate({} {}) rotate({})\">",
                    f.position.x, f.position.y, -f.rotation
                ));
                for g in &library.definition.graphics {
                    let mut g = g.clone();
                    if let pcbcraft_library::Graphic::Text { text, .. } = &mut g
                        && let Some(c) = p.component(f.component)
                    {
                        *text = text
                            .replace("${REFERENCE}", &c.reference)
                            .replace("${VALUE}", &c.value)
                            .replace("%R", &c.reference)
                            .replace("%V", &c.value);
                    }
                    s.push_str(&svg_graphic(&g));
                }
                s.push_str("</g>");
            }
            for pad in &f.pads {
                let points = pad
                    .outline_points()
                    .into_iter()
                    .map(|pos| f.position.offset(pos.rotate(-f.rotation)))
                    .collect::<Vec<_>>();
                s.push_str(&svg_path(&points, "#e6c27d", 0.0, "#e6c27d"));
                let drill = pad
                    .drill_outline_points()
                    .into_iter()
                    .map(|pos| f.position.offset(pos.rotate(-f.rotation)))
                    .collect::<Vec<_>>();
                if !drill.is_empty() {
                    s.push_str(&format!(
                        "<g data-drill=\"{}\">{}</g>",
                        escape(&pad.number),
                        svg_path(&drill, "#131c29", 0.0, "#131c29")
                    ));
                }
            }
            if let Some(c) = p.component(f.component) {
                let (min, max) = footprint_bounds(p, f);
                let label_x = (min.x + max.x) / 2.0;
                for (label, y) in [(&c.reference, min.y - 1.2), (&c.value, max.y + 1.2)] {
                    let point = f
                        .position
                        .offset(Point::new(label_x, y).rotate(-f.rotation));
                    s.push_str(&format!("<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" dominant-baseline=\"middle\" font-size=\"1\" transform=\"rotate({} {} {})\">{}</text>",point.x,point.y,-f.rotation,point.x,point.y,escape(label)));
                }
            }
        }
    } else {
        for w in &p.wires {
            if let Some(path) = p.wire_path(w) {
                let points = path
                    .iter()
                    .map(|p| format!("{},{}", p.x, p.y))
                    .collect::<Vec<_>>()
                    .join(" ");
                s.push_str(&format!("<polyline points=\"{points}\" fill=\"none\" stroke=\"#5ae0bc\" stroke-width=\"0.2\"/>"));
            }
        }
        for c in p.components.iter().flat_map(|c| c.schematic_views()) {
            s.push_str(&format!(
                "<g transform=\"translate({} {}) rotate({})\">",
                c.position.x, c.position.y, -c.rotation
            ));
            if let Some(library) = &c.library_symbol {
                for g in library
                    .definition
                    .graphics
                    .iter()
                    .filter(|g| (g.unit == 0 || g.unit == library.unit) && g.body_style <= 1)
                {
                    s.push_str(&svg_graphic(&g.primitive));
                }
                for pin in library.definition.pins.iter().filter(|pin| {
                    (pin.unit == 0 && library.unit == 1 || pin.unit == library.unit)
                        && pin.body_style <= 1
                }) {
                    let angle = pin.angle.to_radians();
                    let end = pin.position.offset(Point::new(
                        pin.length * angle.cos(),
                        -pin.length * angle.sin(),
                    ));
                    s.push_str(&svg_path(&[pin.position, end], "#dbe7f2", 0.15, "none"));
                    s.push_str(&format!(
                        "<text x=\"{}\" y=\"{}\" font-size=\"0.9\">{}</text>",
                        pin.position.x + 0.3,
                        pin.position.y - 0.3,
                        escape(&pin.number)
                    ));
                }
            } else {
                s.push_str("<rect x=\"-3\" y=\"-1.5\" width=\"6\" height=\"3\" fill=\"#1e2c40\" stroke=\"#dbe7f2\" stroke-width=\"0.15\"/><path d=\"M -5 0 H -3 M 3 0 H 5\" stroke=\"#dbe7f2\" stroke-width=\"0.2\"/>");
            }
            s.push_str(&format!(
                "</g><text x=\"{}\" y=\"{}\" text-anchor=\"middle\">{} · {}</text>",
                c.position.x,
                c.position.y - 3.0,
                escape(&c.reference),
                escape(&c.value)
            ));
        }
    }
    s.push_str("</g></svg>");
    Ok(s)
}

/// A bounded CPU preview for headless MCP. The desktop has its own actual UI capture.
pub fn preview_png(p: &Project, view: &str) -> Result<Vec<u8>, String> {
    p.validate()?;
    if view != "pcb" && view != "schematic" {
        return Err("view must be pcb or schematic".into());
    }
    let mut image = image::RgbImage::from_pixel(1000, 680, image::Rgb([19, 28, 41]));
    let (mut min, mut max) = preview_bounds(p, view);
    let scale = if view == "pcb" {
        (880.0 / p.board.width).min(560.0 / p.board.height)
    } else {
        min = min.offset(Point::new(-4.0, -4.0));
        max = max.offset(Point::new(4.0, 4.0));
        (880.0 / (max.x - min.x).max(1.0)).min(560.0 / (max.y - min.y).max(1.0))
    };
    let to_pixel =
        |p: Point| Point::new(60.0 + (p.x - min.x) * scale, 60.0 + (p.y - min.y) * scale);
    if view == "pcb" {
        let w = (p.board.width * scale).min(880.0) as u32;
        let h = (p.board.height * scale).min(560.0) as u32;
        for x in 60..60 + w {
            for y in 60..60 + h {
                image.put_pixel(x, y, image::Rgb([25, 58, 54]));
            }
        }
        for c in &p.board.cutouts {
            disk(
                &mut image,
                to_pixel(c.center),
                c.radius * scale,
                [19, 28, 41],
            );
        }
        for t in &p.board.tracks {
            line(
                &mut image,
                to_pixel(t.start),
                to_pixel(t.end),
                t.width * scale / 2.0,
                if t.layer == pcbcraft_doc::Layer::Front {
                    [245, 141, 121]
                } else {
                    [116, 167, 236]
                },
            );
        }
        for f in &p.board.footprints {
            if let Some(library) = p
                .component(f.component)
                .and_then(|c| c.library_footprint.as_ref())
            {
                for g in &library.definition.graphics {
                    png_graphic(
                        &mut image,
                        g,
                        |point| to_pixel(f.position.offset(point.rotate(-f.rotation))),
                        scale,
                    );
                }
            }
            for pad in &f.pads {
                let points = pad
                    .outline_points()
                    .into_iter()
                    .map(|pos| to_pixel(f.position.offset(pos.rotate(-f.rotation))))
                    .collect::<Vec<_>>();
                polygon(&mut image, &points, [230, 194, 125]);
                let drill = pad
                    .drill_outline_points()
                    .into_iter()
                    .map(|pos| to_pixel(f.position.offset(pos.rotate(-f.rotation))))
                    .collect::<Vec<_>>();
                polygon(&mut image, &drill, [19, 28, 41]);
            }
        }
    } else {
        for w in &p.wires {
            if let Some(points) = p.wire_path(w) {
                for pair in points.windows(2) {
                    if let (Some(a), Some(b)) = (pair.first(), pair.get(1)) {
                        line(&mut image, to_pixel(*a), to_pixel(*b), 1.0, [90, 224, 188]);
                    }
                }
            }
        }
        for c in p.components.iter().flat_map(|c| c.schematic_views()) {
            if let Some(library) = &c.library_symbol {
                for g in library
                    .definition
                    .graphics
                    .iter()
                    .filter(|g| (g.unit == 0 || g.unit == library.unit) && g.body_style <= 1)
                {
                    png_graphic(
                        &mut image,
                        &g.primitive,
                        |point| to_pixel(c.position.offset(point.rotate(-c.rotation))),
                        scale,
                    );
                }
                for pin in library.definition.pins.iter().filter(|pin| {
                    (pin.unit == 0 && library.unit == 1 || pin.unit == library.unit)
                        && pin.body_style <= 1
                }) {
                    let angle = pin.angle.to_radians();
                    let end = pin.position.offset(Point::new(
                        pin.length * angle.cos(),
                        -pin.length * angle.sin(),
                    ));
                    line(
                        &mut image,
                        to_pixel(c.position.offset(pin.position.rotate(-c.rotation))),
                        to_pixel(c.position.offset(end.rotate(-c.rotation))),
                        0.7,
                        [219, 231, 242],
                    );
                }
                continue;
            }
            let outline = [
                Point::new(-3.0, -1.5),
                Point::new(3.0, -1.5),
                Point::new(3.0, 1.5),
                Point::new(-3.0, 1.5),
                Point::new(-3.0, -1.5),
            ];
            for pair in outline.windows(2) {
                if let (Some(a), Some(b)) = (pair.first(), pair.get(1)) {
                    line(
                        &mut image,
                        to_pixel(c.position.offset(a.rotate(-c.rotation))),
                        to_pixel(c.position.offset(b.rotate(-c.rotation))),
                        1.0,
                        [219, 231, 242],
                    );
                }
            }
            for pin in &c.pins {
                line(
                    &mut image,
                    to_pixel(c.position.offset(pin.offset.rotate(-c.rotation))),
                    to_pixel(
                        c.position.offset(
                            Point::new(pin.offset.x.signum() * 3.0, 0.0).rotate(-c.rotation),
                        ),
                    ),
                    1.0,
                    [219, 231, 242],
                );
            }
        }
    }
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(bytes.into_inner())
}
fn disk(image: &mut image::RgbImage, p: Point, r: f64, color: [u8; 3]) {
    let radius = r.clamp(0.7, 1200.0);
    let left = (p.x - radius).max(0.0) as u32;
    let right = (p.x + radius).min(999.0) as u32;
    let top = (p.y - radius).max(0.0) as u32;
    let bottom = (p.y + radius).min(679.0) as u32;
    for x in left..=right {
        for y in top..=bottom {
            if Point::new(x as f64, y as f64).distance(p) <= radius {
                image.put_pixel(x, y, image::Rgb(color));
            }
        }
    }
}
fn line(image: &mut image::RgbImage, a: Point, b: Point, r: f64, color: [u8; 3]) {
    // Pixel-space bounding scan clips hostile/out-of-view geometry without long stepping loops.
    let radius = r.clamp(0.7, 1200.0);
    let left = (a.x.min(b.x) - radius).max(0.0) as u32;
    let right = (a.x.max(b.x) + radius).min(999.0) as u32;
    let top = (a.y.min(b.y) - radius).max(0.0) as u32;
    let bottom = (a.y.max(b.y) + radius).min(679.0) as u32;
    for x in left..=right {
        for y in top..=bottom {
            if pcbcraft_geom::point_segment(Point::new(x as f64, y as f64), a, b) <= radius {
                image.put_pixel(x, y, image::Rgb(color));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn real_library_project() -> Result<Project, String> {
        let definition = pcbcraft_library::read_symbols(include_str!(
            "../../library/tests/fixtures/device.kicad_sym"
        ))
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| s.name == "R")
        .ok_or("fixture resistor missing")?;
        let footprint = pcbcraft_library::read_footprint(include_str!(
            "../../library/tests/fixtures/R_0805_2012Metric.kicad_mod"
        ))
        .map_err(|e| e.to_string())?;
        let pins = definition
            .pins
            .iter()
            .filter(|p| p.unit <= 1 && p.body_style <= 1)
            .map(|p| pcbcraft_doc::Pin {
                number: p.number.clone(),
                name: p.name.clone(),
                offset: p.position,
                net: None,
                electrical_type: p.electrical_type.clone(),
                direction: Some(p.angle),
                unit: p.unit,
            })
            .collect();
        let pads = footprint
            .pads
            .iter()
            .map(|p| pcbcraft_doc::Pad {
                number: p.number.clone(),
                offset: p.position,
                diameter: p.size.x.min(p.size.y),
                net: None,
                geometry: Some(p.clone()),
            })
            .collect();
        let mut project = Project::default();
        project.components.push(pcbcraft_doc::Component {
            id: 1,
            reference: "R1".into(),
            value: "1k".into(),
            kind: "library".into(),
            position: Point::new(20.0, 20.0),
            rotation: 0.0,
            pins,
            units: vec![],
            library_symbol: Some(pcbcraft_doc::LibrarySymbol {
                library: "Device".into(),
                unit: 1,
                definition,
            }),
            library_footprint: Some(pcbcraft_doc::LibraryFootprint {
                library: "Resistor_SMD".into(),
                definition: footprint,
            }),
        });
        project.board.footprints.push(pcbcraft_doc::Footprint {
            id: 2,
            component: 1,
            position: Point::new(10.0, 10.0),
            rotation: 0.0,
            pads,
        });
        project.validate()?;
        Ok(project)
    }
    fn extract_footprint(board: &str) -> Result<&str, String> {
        let start = board.find("(footprint ").ok_or("no exported footprint")?;
        let mut depth = 0;
        let mut quote = false;
        let mut escape = false;
        for (offset, c) in board[start..].char_indices() {
            if escape {
                escape = false;
                continue;
            }
            if quote && c == '\\' {
                escape = true;
                continue;
            }
            if c == '"' {
                quote = !quote;
            }
            if quote {
                continue;
            }
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    return Ok(&board[start..start + offset + 1]);
                }
            }
        }
        Err("unclosed exported footprint".into())
    }
    fn real_dip_project() -> Result<Project, String> {
        let mut project = real_library_project()?;
        let definition = pcbcraft_library::read_symbols(include_str!(
            "../../library/tests/fixtures/lm358.kicad_sym"
        ))
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| s.name == "LM358")
        .ok_or("fixture LM358 missing")?;
        let footprint = pcbcraft_library::read_footprint(include_str!(
            "../../library/tests/fixtures/DIP-8_W7.62mm.kicad_mod"
        ))
        .map_err(|e| e.to_string())?;
        let c = &mut project.components[0];
        c.reference = "U1".into();
        c.value = "LM358".into();
        c.pins = definition
            .pins
            .iter()
            .filter(|p| p.body_style <= 1)
            .map(|p| pcbcraft_doc::Pin {
                number: p.number.clone(),
                name: p.name.clone(),
                offset: p.position,
                net: None,
                electrical_type: p.electrical_type.clone(),
                direction: Some(p.angle),
                unit: p.unit,
            })
            .collect();
        c.library_symbol = Some(pcbcraft_doc::LibrarySymbol {
            library: "Amplifier_Operational".into(),
            unit: 1,
            definition,
        });
        c.units = vec![
            pcbcraft_doc::SymbolUnitPlacement {
                unit: 2,
                position: Point::new(40.0, 20.0),
                rotation: 90.0,
            },
            pcbcraft_doc::SymbolUnitPlacement {
                unit: 3,
                position: Point::new(60.0, 20.0),
                rotation: 0.0,
            },
        ];
        project.board.footprints[0].pads = footprint
            .pads
            .iter()
            .map(|p| pcbcraft_doc::Pad {
                number: p.number.clone(),
                offset: p.position,
                diameter: p.size.x.min(p.size.y),
                net: None,
                geometry: Some(p.clone()),
            })
            .collect();
        c.library_footprint = Some(pcbcraft_doc::LibraryFootprint {
            library: "Package_DIP".into(),
            definition: footprint,
        });
        project.validate()?;
        Ok(project)
    }
    #[test]
    fn through_hole_export_preserves_drills_layers_and_offset_slots() -> Result<(), String> {
        let mut project = real_dip_project()?;
        project.board.footprints[0].rotation = 90.0;
        // A slot with a local offset tests metadata beyond the circular DIP fixture.
        let slot = pcbcraft_library::Drill {
            size: Point::new(0.8, 0.5),
            offset: Point::new(0.1, 0.0),
        };
        let c = &mut project.components[0];
        let source = c.library_footprint.as_mut().ok_or("no footprint")?;
        source.definition.pads[0].drill = Some(slot.clone());
        project.board.footprints[0].pads[0].geometry = Some(source.definition.pads[0].clone());
        let board = kicad_board(&project)?;
        assert!(board.contains("(attr through_hole)"));
        assert!(board.contains("(drill oval 0.8 0.5 (offset 0.1 0))"));
        let exported = pcbcraft_library::read_footprint(extract_footprint(&board)?)
            .map_err(|e| e.to_string())?;
        assert_eq!(exported.pads.len(), 8);
        assert!(
            exported
                .pads
                .iter()
                .all(|p| p.pad_type == "thru_hole" && p.layers == vec!["*.Cu", "*.Mask"])
        );
        assert_eq!(exported.pads[0].drill, Some(slot));
        assert_eq!(exported.pads[0].rotation, 90.0);
        Ok(())
    }
    #[test]
    fn dip_reference_and_value_are_outside_actual_footprint_bounds() -> Result<(), String> {
        let mut project = real_dip_project()?;
        project.board.footprints[0].rotation = 90.0;
        let f = &project.board.footprints[0];
        let (min, max) = footprint_bounds(&project, f);
        let x = (min.x + max.x) / 2.0;
        let board = kicad_board(&project)?;
        assert!(board.contains(&format!(
            "(property \"Reference\" \"U1\" (at {x} {} 90)",
            min.y - 1.2
        )));
        assert!(board.contains(&format!(
            "(property \"Value\" \"LM358\" (at {x} {} 90)",
            max.y + 1.2
        )));
        assert!(
            max.y > 7.62,
            "DIP8 geometry extends beyond generic 2.5mm value location"
        );
        let exported = pcbcraft_library::read_footprint(extract_footprint(&board)?)
            .map_err(|e| e.to_string())?;
        let original = project.components[0]
            .library_footprint
            .as_ref()
            .ok_or("missing footprint")?;
        let mut graphics = original.definition.graphics.clone();
        for g in &mut graphics {
            if let pcbcraft_library::Graphic::Text { angle, .. } = g {
                *angle += 90.0;
            }
        }
        assert_eq!(
            exported.graphics, graphics,
            "property placement must not alter library artwork"
        );
        Ok(())
    }
    #[test]
    fn all_symbol_units_and_drilled_holes_appear_in_previews() -> Result<(), String> {
        let project = real_dip_project()?;
        let views = project.components[0].schematic_views();
        assert_eq!(views.len(), 3);
        let schematic = svg(&project, "schematic")?;
        assert!(schematic.contains("translate(40 20) rotate(-90)"));
        assert!(schematic.contains("translate(60 20)"));
        for view in views {
            assert!(schematic.contains(&format!("{} ·", escape(&view.reference))));
        }
        assert!(!preview_png(&project, "schematic")?.is_empty());
        let pcb = svg(&project, "pcb")?;
        assert_eq!(pcb.matches("data-drill=").count(), 8);
        let image = image::load_from_memory(&preview_png(&project, "pcb")?)
            .map_err(|e| e.to_string())?
            .to_rgb8();
        let scale = (880.0 / project.board.width).min(560.0 / project.board.height);
        let x = (60.0 + 10.0 * scale).round() as u32;
        let y = (60.0 + 10.0 * scale).round() as u32;
        assert_eq!(image.get_pixel(x, y).0, [19, 28, 41]);
        assert_eq!(
            image.get_pixel(x + (0.65 * scale) as u32, y).0,
            [230, 194, 125]
        );
        Ok(())
    }
    #[test]
    fn shared_physical_pin_is_rendered_only_on_primary_unit() -> Result<(), String> {
        let mut project = real_library_project()?;
        let mut definition = project.components[0]
            .library_symbol
            .as_ref()
            .ok_or("no symbol")?
            .definition
            .clone();
        definition.pins[1].unit = 2;
        let mut common = definition.pins[0].clone();
        common.number = "C".into();
        common.name = "Common".into();
        common.unit = 0;
        common.position = Point::new(-20.0, 0.0);
        common.angle = 0.0;
        definition.pins.push(common);
        project.components[0] = pcbcraft_doc::library_component(
            definition,
            "test".into(),
            1,
            "X1".into(),
            Point::new(20.0, 20.0),
            None,
        )?;
        project.board.footprints.clear();
        project.validate()?;
        let schematic = svg(&project, "schematic")?;
        assert_eq!(
            schematic.matches(">C</text>").count(),
            1,
            "unit0 pin is one physical pin in primary unit, not a ghost pin in every view"
        );
        Ok(())
    }
    #[test]
    fn actual_footprint_export_preserves_geometry_layers_and_rotation() -> Result<(), String> {
        let mut project = real_library_project()?;
        project.board.footprints[0].rotation = 90.0;
        let board = kicad_board(&project)?;
        assert!(!board.contains("(start -3.4 -1.4)"));
        assert!(board.contains("(35 \"F.Paste\" user)"));
        let exported = pcbcraft_library::read_footprint(extract_footprint(&board)?)
            .map_err(|e| e.to_string())?;
        assert_eq!(exported.name, "Resistor_SMD:R_0805_2012Metric");
        let original = project.components[0]
            .library_footprint
            .as_ref()
            .ok_or("no embedded footprint")?;
        assert_eq!(exported.pads.len(), original.definition.pads.len());
        for (actual, expected) in exported.pads.iter().zip(&original.definition.pads) {
            assert_eq!(actual.number, expected.number);
            assert_eq!(actual.shape, expected.shape);
            assert_eq!(actual.size, expected.size);
            assert_eq!(actual.position, expected.position);
            assert_eq!(actual.layers, expected.layers);
            assert_eq!(actual.roundrect_ratio, expected.roundrect_ratio);
            assert_eq!(actual.rotation, expected.rotation + 90.0);
        }
        let mut expected_graphics = original.definition.graphics.clone();
        for graphic in &mut expected_graphics {
            if let pcbcraft_library::Graphic::Text { angle, .. } = graphic {
                *angle += 90.0;
            }
        }
        assert_eq!(exported.graphics, expected_graphics);
        let native = serde_json::to_string(&project).map_err(|e| e.to_string())?;
        let restored: Project = serde_json::from_str(&native).map_err(|e| e.to_string())?;
        restored.validate()?;
        assert_eq!(restored, project);
        Ok(())
    }
    #[test]
    fn project_footprint_export_uses_embedded_name_without_external_nickname() -> Result<(), String>
    {
        let mut project = real_library_project()?;
        let footprint = project.components[0]
            .library_footprint
            .as_mut()
            .ok_or("missing footprint")?;
        footprint.library = "project".into();
        footprint.definition.name = "Edited0805".into();
        let board = kicad_board(&project)?;
        let exported = pcbcraft_library::read_footprint(extract_footprint(&board)?)
            .map_err(|e| e.to_string())?;
        assert_eq!(exported.name, "pcbcraft_Edited0805");
        assert!(!board.contains("project:Edited0805"));
        assert_eq!(exported.pads.len(), 2);
        Ok(())
    }
    #[test]
    fn previews_use_actual_symbol_and_non_circular_copper() -> Result<(), String> {
        let project = real_library_project()?;
        let schematic = svg(&project, "schematic")?;
        assert!(!schematic.contains("width=\"6\" height=\"3\""));
        assert!(schematic.contains("<polyline"));
        let pcb = svg(&project, "pcb")?;
        assert!(pcb.contains("#75928b"));
        assert!(pcb.contains("#e6c27d"));
        assert!(!pcb.contains("${REFERENCE}"));
        assert!(pcb.contains(">R1</text>"));
        let image = image::load_from_memory(&preview_png(&project, "pcb")?)
            .map_err(|e| e.to_string())?
            .to_rgb8();
        let scale = (880.0 / project.board.width).min(560.0 / project.board.height);
        let pad = &project.board.footprints[0].pads[0];
        let x = (60.0 + (10.0 + pad.offset.x) * scale).round() as u32;
        let y = (60.0 + (10.0 + 0.6) * scale).floor() as u32;
        // At y=0.6 mm the real 1.4-mm-tall pad still has copper; a generic
        // 1.025-mm-diameter circular replacement would miss this location.
        assert_eq!(image.get_pixel(x, y).0, [230, 194, 125]);
        assert!(!preview_png(&project, "schematic")?.is_empty());
        Ok(())
    }
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    #[ignore = "requires installed kicad-cli"]
    fn kicad_cli_reads_rotated_real_library_export() -> Result<(), String> {
        for mut project in [real_library_project()?, real_dip_project()?] {
            project.board.footprints[0].rotation = 90.0;
            let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
            let board = directory.path().join("library-board.kicad_pcb");
            std::fs::write(&board, kicad_board(&project)?).map_err(|e| e.to_string())?;
            let output = std::process::Command::new("kicad-cli")
                .args([
                    "pcb",
                    "export",
                    "svg",
                    "--mode-single",
                    "--layers",
                    "F.Cu,F.SilkS,F.Fab,F.CrtYd",
                    "--output",
                ])
                .arg(directory.path().join("library-board.svg"))
                .arg(&board)
                .output()
                .map_err(|e| e.to_string())?;
            if !output.status.success() {
                return Err(format!(
                    "KiCad rejected export: {} {}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            assert!(
                std::fs::read_dir(directory.path())
                    .map_err(|e| e.to_string())?
                    .filter_map(Result::ok)
                    .any(|entry| entry.path().extension().and_then(|s| s.to_str()) == Some("svg"))
            );
            let output = std::process::Command::new("kicad-cli")
                .args(["pcb", "export", "drill", "--format", "excellon", "--output"])
                .arg(directory.path())
                .arg(&board)
                .output()
                .map_err(|e| e.to_string())?;
            if !output.status.success() {
                return Err(format!(
                    "KiCad rejected drill export: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            if project.board.footprints[0].pads.iter().any(|p| {
                p.geometry
                    .as_ref()
                    .is_some_and(|g| g.pad_type == "thru_hole")
            }) {
                let mut positions = 0;
                for entry in std::fs::read_dir(directory.path()).map_err(|e| e.to_string())? {
                    let path = entry.map_err(|e| e.to_string())?.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("drl") {
                        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
                        positions += text
                            .lines()
                            .filter(|line| line.starts_with('X') && line.contains('Y'))
                            .count();
                    }
                }
                assert_eq!(
                    positions, 8,
                    "DIP8 must export all eight plated drill holes"
                );
            }
        }
        Ok(())
    }
    #[test]
    fn atomic_save_replaces_existing_project_and_load_validates() -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
        let path = directory.path().join("design.pcbcraft");
        let mut p = Project::default();
        save(&p, &path)?;
        p.name = "Changed".into();
        save(&p, &path)?;
        assert_eq!(load(&path)?, p);
        atomic_write(&path, b"{\"format\":\"bad\"}")?;
        assert!(load(&path).is_err());
        Ok(())
    }
    #[test]
    fn document_roundtrip_and_unsupported_format() {
        let p = Project::default();
        let text = serde_json::to_string(&p);
        assert!(text.is_ok());
        if let Ok(text) = text {
            let loaded = serde_json::from_str::<Project>(&text);
            assert!(loaded.is_ok_and(|v| v == p));
        }
        let mut bad = p;
        bad.version = 999;
        assert!(bad.validate().is_err());
    }
}
