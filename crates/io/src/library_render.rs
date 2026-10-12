use super::{Point, escape, line, quote};
use pcbcraft_library::Graphic;

pub(super) fn kicad_graphic(g: &Graphic, rotation: f64) -> Result<String, String> {
    g.validate().map_err(|e| e.to_string())?;
    let supported_layer = |layer: &str| -> Result<String, String> {
        if !matches!(
            layer,
            "F.Cu"
                | "B.Cu"
                | "F.Paste"
                | "B.Paste"
                | "F.Mask"
                | "B.Mask"
                | "F.SilkS"
                | "B.SilkS"
                | "F.Fab"
                | "B.Fab"
                | "F.CrtYd"
                | "B.CrtYd"
                | "Edge.Cuts"
        ) {
            return Err(format!("unsupported footprint graphic layer {layer}"));
        }
        Ok(quote(layer))
    };
    let fill_token = |fill: &str| {
        if matches!(fill, "solid" | "background" | "outline") {
            "solid"
        } else {
            "none"
        }
    };
    Ok(match g {
        Graphic::Line {
            start,
            end,
            width,
            layer,
        } => format!(
            " (fp_line (start {} {}) (end {} {}) (stroke (width {width}) (type solid)) (layer {}))\n",
            start.x,
            start.y,
            end.x,
            end.y,
            supported_layer(layer)?
        ),
        Graphic::Rectangle {
            start,
            end,
            width,
            fill,
            layer,
        } => format!(
            " (fp_rect (start {} {}) (end {} {}) (stroke (width {width}) (type solid)) (fill {}) (layer {}))\n",
            start.x,
            start.y,
            end.x,
            end.y,
            fill_token(fill),
            supported_layer(layer)?
        ),
        Graphic::Circle {
            center,
            radius,
            width,
            fill,
            layer,
        } => format!(
            " (fp_circle (center {} {}) (end {} {}) (stroke (width {width}) (type solid)) (fill {}) (layer {}))\n",
            center.x,
            center.y,
            center.x + radius,
            center.y,
            fill_token(fill),
            supported_layer(layer)?
        ),
        Graphic::Arc {
            start,
            mid,
            end,
            width,
            layer,
        } => format!(
            " (fp_arc (start {} {}) (mid {} {}) (end {} {}) (stroke (width {width}) (type solid)) (layer {}))\n",
            start.x,
            start.y,
            mid.x,
            mid.y,
            end.x,
            end.y,
            supported_layer(layer)?
        ),
        Graphic::Polyline {
            points,
            width,
            fill,
            layer,
        } => format!(
            " (fp_poly (pts {}) (stroke (width {width}) (type solid)) (fill {}) (layer {}))\n",
            points
                .iter()
                .map(|p| format!("(xy {} {})", p.x, p.y))
                .collect::<Vec<_>>()
                .join(" "),
            fill_token(fill),
            supported_layer(layer)?
        ),
        Graphic::Text {
            position,
            angle,
            text,
            size,
            layer,
        } => format!(
            " (fp_text user {} (at {} {} {}) (layer {}) (effects (font (size {} {}) (thickness 0.15))))\n",
            quote(text),
            position.x,
            position.y,
            angle + rotation,
            supported_layer(layer)?,
            size.x,
            size.y
        ),
    })
}

pub(super) fn graphic_style(g: &Graphic) -> (f64, bool, &str) {
    match g {
        Graphic::Line { width, layer, .. } | Graphic::Arc { width, layer, .. } => {
            (*width, false, layer)
        }
        Graphic::Rectangle {
            width, fill, layer, ..
        }
        | Graphic::Circle {
            width, fill, layer, ..
        }
        | Graphic::Polyline {
            width, fill, layer, ..
        } => (
            *width,
            matches!(fill.as_str(), "solid" | "background" | "outline"),
            layer,
        ),
        Graphic::Text { layer, .. } => (0.0, false, layer),
    }
}
pub(super) fn layer_color(layer: &str) -> (&'static str, [u8; 3]) {
    match layer {
        "F.CrtYd" | "B.CrtYd" => ("#75928b", [117, 146, 139]),
        "F.Fab" | "B.Fab" => ("#83aaa1", [131, 170, 161]),
        _ => ("#dbe7f2", [219, 231, 242]),
    }
}
pub(super) fn svg_path(points: &[Point], stroke: &str, width: f64, fill: &str) -> String {
    if points.is_empty() {
        return String::new();
    }
    let coords = points
        .iter()
        .map(|p| format!("{},{}", p.x, p.y))
        .collect::<Vec<_>>()
        .join(" ");
    let element = if fill == "none" {
        "polyline"
    } else {
        "polygon"
    };
    format!(
        "<{element} points=\"{coords}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"{}\" stroke-linejoin=\"round\"/>",
        width.max(0.05)
    )
}
pub(super) fn svg_graphic(g: &Graphic) -> String {
    let (width, fill, layer) = graphic_style(g);
    let color = layer_color(layer).0;
    if let Graphic::Text {
        position,
        angle,
        text,
        size,
        ..
    } = g
    {
        return format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"{}\" text-anchor=\"middle\" transform=\"rotate({} {} {})\">{}</text>",
            position.x,
            position.y,
            size.y,
            -angle,
            position.x,
            position.y,
            escape(text)
        );
    }
    svg_path(
        &g.path_points(),
        color,
        width,
        if fill {
            if background_fill(g) { "#1e2c40" } else { color }
        } else {
            "none"
        },
    )
}
pub(super) fn png_graphic(
    image: &mut image::RgbImage,
    g: &Graphic,
    transform: impl Fn(Point) -> Point,
    scale: f64,
) {
    let (width, fill, layer) = graphic_style(g);
    let points = g
        .path_points()
        .into_iter()
        .map(transform)
        .collect::<Vec<_>>();
    if fill {
        polygon(
            image,
            &points,
            if background_fill(g) {
                [30, 44, 64]
            } else {
                layer_color(layer).1
            },
        );
    }
    for pair in points.windows(2) {
        line(
            image,
            pair[0],
            pair[1],
            width.max(0.1) * scale / 2.0,
            layer_color(layer).1,
        );
    }
    if fill && points.len() > 2 {
        line(
            image,
            points[points.len() - 1],
            points[0],
            width.max(0.1) * scale / 2.0,
            layer_color(layer).1,
        );
    }
}
fn background_fill(g: &Graphic) -> bool {
    matches!(g,Graphic::Rectangle{fill,..}|Graphic::Circle{fill,..}|Graphic::Polyline{fill,..} if fill=="background")
}
pub(super) fn polygon(image: &mut image::RgbImage, points: &[Point], color: [u8; 3]) {
    if points.len() < 3 {
        return;
    }
    let left = points
        .iter()
        .map(|p| p.x)
        .fold(f64::INFINITY, f64::min)
        .max(0.0) as u32;
    let right = points
        .iter()
        .map(|p| p.x)
        .fold(f64::NEG_INFINITY, f64::max)
        .min(999.0) as u32;
    let top = points
        .iter()
        .map(|p| p.y)
        .fold(f64::INFINITY, f64::min)
        .max(0.0) as u32;
    let bottom = points
        .iter()
        .map(|p| p.y)
        .fold(f64::NEG_INFINITY, f64::max)
        .min(679.0) as u32;
    for y in top..=bottom {
        for x in left..=right {
            let test = Point::new(x as f64 + 0.5, y as f64 + 0.5);
            let mut inside = false;
            let mut previous = points[points.len() - 1];
            for current in points {
                if (current.y > test.y) != (previous.y > test.y)
                    && test.x
                        < (previous.x - current.x) * (test.y - current.y) / (previous.y - current.y)
                            + current.x
                {
                    inside = !inside;
                }
                previous = *current;
            }
            if inside {
                image.put_pixel(x, y, image::Rgb(color));
            }
        }
    }
}
