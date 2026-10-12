//! Native project persistence and original, limited KiCad PCB / preview writers.
use pcbcraft_doc::Project;
use pcbcraft_geom::Point;
use std::io::Read;
#[cfg(not(target_arch = "wasm32"))]
use std::io::Write;
use std::path::Path;

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

pub fn kicad_board(p: &Project) -> Result<String, String> {
    p.validate()?;
    let b = &p.board;
    let mut s = format!(
        "(kicad_pcb (version 20240108) (generator \"pcbcraft\")\n (general (thickness {})) (paper \"A4\")\n (layers (0 \"F.Cu\" signal) (31 \"B.Cu\" signal) (36 \"B.SilkS\" user \"b.silkscreen\") (37 \"F.SilkS\" user \"f.silkscreen\") (38 \"B.Mask\" user) (39 \"F.Mask\" user) (44 \"Edge.Cuts\" user))\n (setup (pad_to_mask_clearance 0))\n (net 0 \"\")\n",
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
        s.push_str(&format!(" (footprint {} (layer \"F.Cu\") (uuid {}) (at {} {} {})\n (property \"Reference\" {} (at 0 -2.5 {}) (layer \"F.SilkS\") (effects (font (size 1 1) (thickness 0.15))))\n (property \"Value\" {} (at 0 2.5 {}) (layer \"F.SilkS\") (effects (font (size 1 1) (thickness 0.15))))\n (attr smd)\n (fp_rect (start -3.4 -1.4) (end 3.4 1.4) (stroke (width 0.12) (type solid)) (fill none) (layer \"F.SilkS\"))\n",quote(&format!("pcbcraft_{}",c.kind)),uuid(f.id,1),f.position.x,f.position.y,f.rotation,quote(&c.reference),f.rotation,quote(&c.value),f.rotation));
        for (i, pad) in f.pads.iter().enumerate() {
            let net = pad.net.map(|n| p.net_name(n)).unwrap_or("");
            s.push_str(&format!(" (pad {} smd circle (at {} {} {}) (size {} {}) (layers \"F.Cu\" \"F.Mask\") (net {} {}) (uuid {}))\n",quote(&pad.number),pad.offset.x,pad.offset.y,f.rotation,pad.diameter,pad.diameter,ordinal(pad.net),quote(net),uuid(f.id,i+2)));
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
pub fn svg(p: &Project, view: &str) -> Result<String, String> {
    p.validate()?;
    if view != "pcb" && view != "schematic" {
        return Err("view must be pcb or schematic".into());
    }
    let mut s = String::from(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1000\" height=\"680\" viewBox=\"-8 -8 100 68\"><rect x=\"-8\" y=\"-8\" width=\"100\" height=\"68\" fill=\"#131c29\"/><g font-family=\"sans-serif\" font-size=\"1.5\" fill=\"#dbe7f2\">",
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
            for pad in &f.pads {
                if let Some(pos) = f.pad_position(&pad.number) {
                    s.push_str(&format!(
                        "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"#e6c27d\"/>",
                        pos.x,
                        pos.y,
                        pad.diameter / 2.0
                    ));
                }
            }
            if let Some(c) = p.component(f.component) {
                s.push_str(&format!(
                    "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\">{}</text>",
                    f.position.x,
                    f.position.y - 2.0,
                    escape(&c.reference)
                ));
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
        for c in &p.components {
            s.push_str(&format!("<g transform=\"translate({} {}) rotate({})\"><rect x=\"-3\" y=\"-1.5\" width=\"6\" height=\"3\" fill=\"#1e2c40\" stroke=\"#dbe7f2\" stroke-width=\"0.15\"/><path d=\"M -5 0 H -3 M 3 0 H 5\" stroke=\"#dbe7f2\" stroke-width=\"0.2\"/></g><text x=\"{}\" y=\"{}\" text-anchor=\"middle\">{} · {}</text>",c.position.x,c.position.y,c.rotation,c.position.x,c.position.y-3.0,escape(&c.reference),escape(&c.value)));
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
    let scale = if view == "pcb" {
        (880.0 / p.board.width).min(560.0 / p.board.height)
    } else {
        8.0
    };
    let to_pixel = |p: Point| Point::new(60.0 + p.x * scale, 60.0 + p.y * scale);
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
            for pad in &f.pads {
                if let Some(pos) = f.pad_position(&pad.number) {
                    disk(
                        &mut image,
                        to_pixel(pos),
                        pad.diameter * scale / 2.0,
                        [230, 194, 125],
                    );
                }
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
        for c in &p.components {
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
