//! Native, bounded drawing primitives shared by symbol and footprint definitions.
use crate::schematic::{Point, TextAlign};
use serde::{Deserialize, Serialize};

/// A physical stroke or body fill; dimensions are nanometres and never screen pixels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Graphic {
    pub geometry: Geometry,
    pub stroke_nm: i64,
    pub fill: Fill,
}
/// Drawing geometry retains editable parameters rather than flattening curves and text into paths.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Geometry {
    Line { from: Point, to: Point },
    Rectangle { min: Point, max: Point },
    Circle { center: Point, radius_nm: i64 },
    Arc { center: Point, radius_nm: i64, start_degrees: i16, sweep_degrees: i16 },
    Polyline { points: Vec<Point>, closed: bool },
    Text { at: Point, text: String, height_nm: i64, rotation: u16, align: TextAlign },
}
/// Fill is independent of electrical connectivity. Background uses the active canvas palette.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fill {
    #[default]
    None,
    Solid,
    Background,
}
/// Shared, revision-checked transformations operate about an explicit local pivot.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    pub pivot: Point,
    pub offset: Point,
    pub quarter_turns: i16,
    pub mirror_x: bool,
    pub mirror_y: bool,
}
impl Transform {
    /// Translate/reflect/rotate with wide arithmetic; rejected overflow cannot partially edit a group.
    pub fn point(self, p: Point) -> Result<Point, String> {
        let mut x = i128::from(p.x_nm) - i128::from(self.pivot.x_nm);
        let mut y = i128::from(p.y_nm) - i128::from(self.pivot.y_nm);
        if self.mirror_x {
            x = -x;
        }
        if self.mirror_y {
            y = -y;
        }
        let (x, y) = match self.quarter_turns.rem_euclid(4) {
            0 => (x, y),
            1 => (-y, x),
            2 => (-x, -y),
            _ => (y, -x),
        };
        Ok(Point {
            x_nm: i64::try_from(x + i128::from(self.pivot.x_nm) + i128::from(self.offset.x_nm)).map_err(|_| "X coordinate overflow.")?,
            y_nm: i64::try_from(y + i128::from(self.pivot.y_nm) + i128::from(self.offset.y_nm)).map_err(|_| "Y coordinate overflow.")?,
        })
    }
    /// Transform an orientation in the same screen-independent axes as its coordinates.
    pub fn angle(self, degrees: i16) -> u16 {
        let mut angle = i32::from(degrees);
        if self.mirror_x {
            angle = 180 - angle;
        }
        if self.mirror_y {
            angle = -angle;
        }
        (angle + i32::from(self.quarter_turns) * 90).rem_euclid(360) as u16
    }
}
impl Graphic {
    /// A stable grab point for move/duplicate operations.
    pub fn anchor(&self) -> Point {
        match &self.geometry {
            Geometry::Line { from, .. } => *from,
            Geometry::Rectangle { min, max } => Point {
                x_nm: ((i128::from(min.x_nm) + i128::from(max.x_nm)) / 2) as i64,
                y_nm: ((i128::from(min.y_nm) + i128::from(max.y_nm)) / 2) as i64,
            },
            Geometry::Circle { center, .. } | Geometry::Arc { center, .. } => *center,
            Geometry::Polyline { points, .. } => points.first().copied().unwrap_or(Point { x_nm: 0, y_nm: 0 }),
            Geometry::Text { at, .. } => *at,
        }
    }
    /// Closed geometry can be filled; open strokes and lettering cannot.
    pub fn closed(&self) -> bool {
        matches!(self.geometry, Geometry::Rectangle { .. } | Geometry::Circle { .. } | Geometry::Polyline { closed: true, .. })
    }
    /// Project exact parameters into bounded polylines for painting, hit-testing and extents.
    /// Lettering reuses the already qualified PCBcraft Technical stroke data.
    pub fn paths(&self) -> Vec<Vec<Point>> {
        let point = |x, y| Point { x_nm: x, y_nm: y };
        match &self.geometry {
            Geometry::Line { from, to } => vec![vec![*from, *to]],
            Geometry::Rectangle { min, max } => vec![vec![*min, point(max.x_nm, min.y_nm), *max, point(min.x_nm, max.y_nm)]],
            Geometry::Circle { center, radius_nm } | Geometry::Arc { center, radius_nm, .. } => {
                let (start, sweep, steps) = match self.geometry {
                    Geometry::Arc { start_degrees, sweep_degrees, .. } => (f64::from(start_degrees), f64::from(sweep_degrees), 64),
                    _ => (0., 360., 96),
                };
                vec![
                    (0..=steps)
                        .map(|i| {
                            let a = (start + sweep * f64::from(i) / f64::from(steps)).to_radians();
                            point(
                                center.x_nm.saturating_add((*radius_nm as f64 * a.cos()).round() as i64),
                                center.y_nm.saturating_add((*radius_nm as f64 * a.sin()).round() as i64),
                            )
                        })
                        .collect(),
                ]
            }
            Geometry::Polyline { points, .. } => vec![points.clone()],
            Geometry::Text { at, text, height_nm, rotation, align } => {
                let style = crate::board::silkscreen::Settings {
                    text_height_nm: *height_nm,
                    stroke_width_nm: self.stroke_nm.min(*height_nm / 3).max(1),
                    pad_clearance_nm: 0,
                };
                let mut paths = crate::board::silkscreen::lettering(text, 0, style);
                let left = paths.iter().flatten().map(|p| p.x_nm).min().unwrap_or(0);
                let right = paths.iter().flatten().map(|p| p.x_nm).max().unwrap_or(0);
                let offset = match align {
                    TextAlign::Start => left,
                    TextAlign::Middle => (left + right) / 2,
                    TextAlign::End => right,
                };
                let a = f64::from(*rotation).to_radians();
                for p in paths.iter_mut().flatten() {
                    let (x, y) = ((p.x_nm - offset) as f64, (p.y_nm - *height_nm / 2) as f64);
                    *p = point(
                        at.x_nm.saturating_add((x * a.cos() - y * a.sin()).round() as i64),
                        at.y_nm.saturating_add((x * a.sin() + y * a.cos()).round() as i64),
                    );
                }
                paths
            }
        }
    }
    /// Validate parameters before tessellation, then bound every generated point, including text extent.
    pub fn validate(&self, limit: i64) -> Result<(), String> {
        if limit <= 0 {
            return Err("Drawing bounds must be positive.".into());
        }
        let size = |n: i64| (1..=limit).contains(&n);
        let valid = match &self.geometry {
            Geometry::Line { from, to } => from != to,
            Geometry::Rectangle { min, max } => min.x_nm < max.x_nm && min.y_nm < max.y_nm,
            Geometry::Circle { radius_nm, .. } => size(*radius_nm),
            Geometry::Arc { radius_nm, start_degrees, sweep_degrees, .. } => {
                size(*radius_nm) && (-360..=360).contains(start_degrees) && *sweep_degrees != 0 && (-360..=360).contains(sweep_degrees)
            }
            Geometry::Polyline { points, closed } => {
                points.len() >= if *closed { 3 } else { 2 } && points.len() <= 64 && points.windows(2).all(|p| p[0] != p[1])
            }
            Geometry::Text { text, height_nm, rotation, .. } => {
                !text.trim().is_empty()
                    && text.chars().count() <= 128
                    && !text.chars().any(char::is_control)
                    && (100_000..=10_000_000).contains(height_nm)
                    && *rotation < 360
            }
        };
        if !valid || !(1..=2_000_000).contains(&self.stroke_nm) || (self.fill != Fill::None && !self.closed()) {
            return Err("Invalid drawing geometry, stroke or fill.".into());
        }
        if self.paths().iter().flatten().any(|p| !(-limit..=limit).contains(&p.x_nm) || !(-limit..=limit).contains(&p.y_nm)) {
            return Err("Drawing extends outside the definition bounds.".into());
        }
        if self.fill != Fill::None
            && let Geometry::Polyline { points, .. } = &self.geometry
        {
            let mut sign = 0;
            for i in 0..points.len() {
                let a = points[i];
                let b = points[(i + 1) % points.len()];
                let c = points[(i + 2) % points.len()];
                let cross = (i128::from(b.x_nm) - i128::from(a.x_nm)) * (i128::from(c.y_nm) - i128::from(b.y_nm))
                    - (i128::from(b.y_nm) - i128::from(a.y_nm)) * (i128::from(c.x_nm) - i128::from(b.x_nm));
                let next = cross.signum();
                if next != 0 {
                    if sign != 0 && sign != next {
                        return Err("Filled polygons must be convex; use an unfilled outline for this shape.".into());
                    }
                    sign = next;
                }
            }
            if sign == 0 {
                return Err("A filled polygon needs a nonzero area.".into());
            }
            for i in 0..points.len() {
                let a = points[i];
                let b = points[(i + 1) % points.len()];
                if points.iter().any(|p| {
                    let cross = (i128::from(b.x_nm) - i128::from(a.x_nm)) * (i128::from(p.y_nm) - i128::from(a.y_nm))
                        - (i128::from(b.y_nm) - i128::from(a.y_nm)) * (i128::from(p.x_nm) - i128::from(a.x_nm));
                    cross.signum() != 0 && cross.signum() != sign
                }) {
                    return Err("Filled polygons must be convex and cannot intersect themselves.".into());
                }
            }
        }

        Ok(())
    }
    /// Transform parameters without flattening editable geometry. Single-axis reflections
    /// normalize upside-down text and swap its alignment; ordinary rotations retain authored angles.
    pub fn transformed(&self, t: Transform) -> Result<Self, String> {
        let geometry = match &self.geometry {
            Geometry::Line { from, to } => Geometry::Line { from: t.point(*from)?, to: t.point(*to)? },
            Geometry::Rectangle { min, max } => {
                let a = t.point(*min)?;
                let b = t.point(*max)?;
                Geometry::Rectangle {
                    min: Point { x_nm: a.x_nm.min(b.x_nm), y_nm: a.y_nm.min(b.y_nm) },
                    max: Point { x_nm: a.x_nm.max(b.x_nm), y_nm: a.y_nm.max(b.y_nm) },
                }
            }
            Geometry::Circle { center, radius_nm } => Geometry::Circle { center: t.point(*center)?, radius_nm: *radius_nm },
            Geometry::Arc { center, radius_nm, start_degrees, sweep_degrees } => Geometry::Arc {
                center: t.point(*center)?,
                radius_nm: *radius_nm,
                start_degrees: t.angle(*start_degrees) as i16,
                sweep_degrees: if t.mirror_x ^ t.mirror_y { -*sweep_degrees } else { *sweep_degrees },
            },
            Geometry::Polyline { points, closed } => {
                Geometry::Polyline { points: points.iter().map(|p| t.point(*p)).collect::<Result<_, _>>()?, closed: *closed }
            }
            Geometry::Text { at, text, height_nm, rotation, align } => {
                let mut rotation = t.angle(*rotation as i16);
                let mut align = *align;
                if t.mirror_x ^ t.mirror_y && rotation > 90 && rotation <= 270 {
                    rotation = (rotation + 180) % 360;
                    align = match align {
                        TextAlign::Start => TextAlign::End,
                        TextAlign::End => TextAlign::Start,
                        TextAlign::Middle => TextAlign::Middle,
                    };
                }
                Geometry::Text { at: t.point(*at)?, text: text.clone(), height_nm: *height_nm, rotation, align }
            }
        };
        Ok(Self { geometry, ..self.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(x_nm: i64, y_nm: i64) -> Point {
        Point { x_nm, y_nm }
    }
    /// Extreme/unfilled/concave geometry is rejected before renderer-specific tessellation.
    #[test]
    fn filled_polygons_are_bounded_convex_and_nonintersecting() {
        let graphic = |points| Graphic { geometry: Geometry::Polyline { points, closed: true }, stroke_nm: 100_000, fill: Fill::Solid };
        assert!(graphic(vec![p(0, 0), p(5, 0), p(5, 5), p(0, 5)]).validate(100).is_ok());
        for points in [
            vec![p(i64::MIN, i64::MAX), p(i64::MAX, i64::MIN), p(0, 0)],
            vec![p(0, 0), p(5, 5), p(0, 5), p(5, 0)],
            vec![p(0, 0), p(5, 0), p(1, 1), p(5, 5), p(0, 5)],
            vec![p(0, 0), p(1, 1), p(2, 2)],
        ] {
            assert!(graphic(points).validate(100).is_err());
        }
    }
    /// Four rotations retain exact editable parameters; reflection reverses arc traversal.
    #[test]
    fn transforms_keep_arc_parameters_and_reject_overflow() {
        let g = Graphic {
            geometry: Geometry::Arc { center: p(10, 20), radius_nm: 30, start_degrees: 45, sweep_degrees: 90 },
            stroke_nm: 100_000,
            fill: Fill::None,
        };
        let mut next = g.clone();
        let mut t = Transform { pivot: p(0, 0), offset: p(0, 0), quarter_turns: 1, mirror_x: false, mirror_y: false };
        for _ in 0..4 {
            next = next.transformed(t).unwrap();
        }
        assert_eq!(next, g);
        t.quarter_turns = 0;
        t.mirror_x = true;
        assert!(matches!(g.transformed(t).unwrap().geometry, Geometry::Arc { start_degrees: 135, sweep_degrees: -90, .. }));
        t.offset = p(i64::MAX, 0);
        assert!(t.point(p(-1, 0)).is_err());
    }
}

#[cfg(test)]
mod mirror_text_tests {
    use super::*;
    /// Reflection keeps labels readable and anchored; ordinary rotation retains authored angles.
    #[test]
    fn mirrored_text_normalizes_only_upside_down_reflections() {
        let g = Graphic {
            geometry: Geometry::Text {
                at: Point { x_nm: 2_000_000, y_nm: 1_000_000 },
                text: "TEXT".into(),
                height_nm: 1_000_000,
                rotation: 0,
                align: TextAlign::Start,
            },
            stroke_nm: 100_000,
            fill: Fill::None,
        };
        let t =
            Transform { pivot: Point { x_nm: 0, y_nm: 0 }, offset: Point { x_nm: 0, y_nm: 0 }, quarter_turns: 0, mirror_x: true, mirror_y: false };
        let reflected = g.transformed(t).unwrap();
        assert!(matches!(
            reflected.geometry,
            Geometry::Text { at: Point { x_nm: -2_000_000, y_nm: 1_000_000 }, rotation: 0, align: TextAlign::End, .. }
        ));
        assert_eq!(reflected.transformed(t).unwrap(), g);
        assert!(matches!(
            g.transformed(Transform { quarter_turns: 2, mirror_x: false, ..t }).unwrap().geometry,
            Geometry::Text { rotation: 180, align: TextAlign::Start, .. }
        ));
        assert!(matches!(
            g.transformed(Transform { mirror_x: false, mirror_y: true, ..t }).unwrap().geometry,
            Geometry::Text { rotation: 0, align: TextAlign::Start, .. }
        ));
        assert!(matches!(
            g.transformed(Transform { mirror_y: true, ..t }).unwrap().geometry,
            Geometry::Text { rotation: 180, align: TextAlign::Start, .. }
        ));
    }
}
