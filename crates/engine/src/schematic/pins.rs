//! Original pin appearance; electrical endpoints remain distinct from drawn lead geometry.
use super::{Point, TextAlign};
use crate::library_graphics::{Fill, Geometry, Graphic, Transform};
use serde::{Deserialize, Serialize};

/// Direction from the wire attachment point toward the symbol body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinOrientation {
    #[default]
    Right,
    Down,
    Left,
    Up,
}
impl PinOrientation {
    /// Cardinal orientation in definition axes (positive Y points down).
    pub fn degrees(self) -> u16 {
        match self {
            Self::Right => 0,
            Self::Down => 90,
            Self::Left => 180,
            Self::Up => 270,
        }
    }
    /// Apply rotations/reflections without moving the electrical endpoint.
    pub fn transformed(self, t: Transform) -> Self {
        match t.angle(self.degrees() as i16) {
            0 => Self::Right,
            90 => Self::Down,
            180 => Self::Left,
            _ => Self::Up,
        }
    }
}
/// Graphical pin markers do not change ERC/electrical type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinStyle {
    #[default]
    Line,
    Inverted,
    Clock,
    InvertedClock,
}
/// Optional native appearance preserves old definitions with separately drawn leads unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinAppearance {
    pub orientation: PinOrientation,
    pub length_nm: i64,
    pub style: PinStyle,
    pub show_name: bool,
    pub show_number: bool,
    pub name_height_nm: i64,
    pub number_height_nm: i64,
}
impl Default for PinAppearance {
    fn default() -> Self {
        Self {
            orientation: PinOrientation::Right,
            length_nm: 2_540_000,
            style: PinStyle::Line,
            show_name: true,
            show_number: true,
            name_height_nm: 1_000_000,
            number_height_nm: 800_000,
        }
    }
}
impl PinAppearance {
    /// Resolve the body end of a lead. Validation separately bounds both endpoints.
    pub fn end(&self, at: Point) -> Point {
        match self.orientation {
            PinOrientation::Right => Point { x_nm: at.x_nm.saturating_add(self.length_nm), ..at },
            PinOrientation::Left => Point { x_nm: at.x_nm.saturating_sub(self.length_nm), ..at },
            PinOrientation::Down => Point { y_nm: at.y_nm.saturating_add(self.length_nm), ..at },
            PinOrientation::Up => Point { y_nm: at.y_nm.saturating_sub(self.length_nm), ..at },
        }
    }
    /// Bound physical pin/label dimensions and lead position without imposing a screen zoom.
    pub fn validate(&self, at: Point) -> Result<(), String> {
        let end = self.end(at);
        if !(100_000..=100_000_000).contains(&self.length_nm)
            || [self.name_height_nm, self.number_height_nm].iter().any(|n| !(100_000..=5_000_000).contains(n))
            || [end.x_nm, end.y_nm].iter().any(|n| !(-100_000_000..=100_000_000).contains(n))
        {
            return Err("Invalid pin length, label size or lead extent.".into());
        }
        Ok(())
    }
    /// Generate lead, inversion/clock markers and independently visible labels from stored parameters.
    pub fn graphics(&self, at: Point, number: &str, name: Option<&str>) -> Vec<Graphic> {
        let end = self.end(at);
        let a = f64::from(self.orientation.degrees()).to_radians();
        let local = |x: i64, y: i64| Point {
            x_nm: at.x_nm + (x as f64 * a.cos() - y as f64 * a.sin()).round() as i64,
            y_nm: at.y_nm + (x as f64 * a.sin() + y as f64 * a.cos()).round() as i64,
        };
        let make = |geometry| Graphic { geometry, stroke_nm: 120_000, fill: Fill::None };
        let inverted = matches!(self.style, PinStyle::Inverted | PinStyle::InvertedClock);
        let clock = matches!(self.style, PinStyle::Clock | PinStyle::InvertedClock);
        let radius = (self.length_nm / 6).min(300_000);
        let mut result = vec![make(Geometry::Line { from: at, to: if inverted { local(self.length_nm - 2 * radius, 0) } else { end } })];
        if inverted {
            result.push(make(Geometry::Circle { center: local(self.length_nm - radius, 0), radius_nm: radius }));
        }
        if clock {
            let d = 400_000;
            result.push(make(Geometry::Polyline {
                points: vec![local(self.length_nm, -d), local(self.length_nm + d, 0), local(self.length_nm, d)],
                closed: false,
            }));
        }
        if self.show_number {
            let mut middle = Point { x_nm: (at.x_nm + end.x_nm) / 2, y_nm: (at.y_nm + end.y_nm) / 2 };
            let align = if matches!(self.orientation, PinOrientation::Up | PinOrientation::Down) {
                middle.x_nm += 400_000;
                TextAlign::Start
            } else {
                middle.y_nm -= 600_000;
                TextAlign::Middle
            };
            result.push(make(Geometry::Text { at: middle, text: number.into(), height_nm: self.number_height_nm, rotation: 0, align }));
        }
        if self.show_name
            && let Some(name) = name.filter(|name| !name.is_empty())
        {
            let inset = if clock { 700_000 } else { 400_000 };
            let (point, align) = match self.orientation {
                PinOrientation::Right => (Point { x_nm: end.x_nm + inset, ..end }, TextAlign::Start),
                PinOrientation::Left => (Point { x_nm: end.x_nm - inset, ..end }, TextAlign::End),
                PinOrientation::Up => (Point { y_nm: end.y_nm - 700_000, ..end }, TextAlign::Middle),
                PinOrientation::Down => (Point { y_nm: end.y_nm + 700_000, ..end }, TextAlign::Middle),
            };
            result.push(make(Geometry::Text { at: point, text: name.into(), height_nm: self.name_height_nm, rotation: 0, align }));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Vertical numbers sit beside the lead; hidden labels do not leave generated text behind.
    #[test]
    fn vertical_pin_labels_clear_the_lead_and_visibility_survives() {
        let at = Point { x_nm: 0, y_nm: 0 };
        for orientation in [PinOrientation::Up, PinOrientation::Down] {
            let mut a = PinAppearance { orientation, ..Default::default() };
            let graphics = a.graphics(at, "1", Some("VCC"));
            assert!(matches!(graphics[1].geometry, Geometry::Text { at: Point { x_nm: 400_000, .. }, align: TextAlign::Start, .. }));
            a.show_name = false;
            a.show_number = false;
            assert_eq!(a.graphics(at, "1", Some("VCC")).len(), 1);
            assert_eq!(a.end(at).x_nm, 0);
            a.validate(at).unwrap();
        }
    }
}
