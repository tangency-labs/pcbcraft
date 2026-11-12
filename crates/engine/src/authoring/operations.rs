//! Atomic authoring transforms, duplication and arrays; rejected candidates consume no identities.
use super::*;
use crate::library_graphics::{Fill, Geometry, Graphic, Transform};

/// Increment a trailing numeric designator while preserving its prefix and avoiding collisions.
/// Number spelling is validated for both terminal and pad definitions before a candidate is returned.
pub fn next_number(item: &Item, seed: &str) -> Result<String, String> {
    let split = seed.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let prefix = &seed[..split];
    let first = seed[split..].parse::<u32>().unwrap_or(0);
    for step in 1..=10_000 {
        let n = first.checked_add(step).ok_or("Electrical number limit reached.")?;
        let candidate = format!("{prefix}{n}");
        if crate::schematic::PinNumber::new(&candidate).is_some()
            && !item.objects.values().chain(item.other_units.values().flat_map(|objects| objects.values())).any(|p| match p {
                Primitive::Pin { number, .. } => number == &candidate,
                Primitive::Pad { pad } => pad.number == candidate,
                _ => false,
            })
        {
            return Ok(candidate);
        }
    }
    Err("No unused electrical number is available for this prefix.".into())
}
impl Primitive {
    /// Transform all stored parameters while retaining electrical and primitive identities.
    /// Pin rotation changes its lead direction; the wire attachment point remains its anchor.
    pub fn transformed(&self, t: Transform) -> Result<Self, String> {
        Ok(match self {
            Self::Pin { number, name, electrical_type, at, appearance } => Self::Pin {
                number: number.clone(),
                name: name.clone(),
                electrical_type: *electrical_type,
                at: t.point(*at)?,
                appearance: appearance.as_ref().map(|a| crate::schematic::PinAppearance { orientation: a.orientation.transformed(t), ..a.clone() }),
            },
            Self::Graphic { graphic } => Self::Graphic { graphic: graphic.transformed(t)? },
            Self::Artwork { artwork } => {
                Self::Artwork { artwork: footprint::Artwork { purpose: artwork.purpose, graphic: artwork.graphic.transformed(t)? } }
            }
            Self::Line { from, to } => Self::Line { from: t.point(*from)?, to: t.point(*to)? },
            Self::Rectangle { min, max } => {
                let a = t.point(*min)?;
                let b = t.point(*max)?;
                Self::Rectangle {
                    min: Point { x_nm: a.x_nm.min(b.x_nm), y_nm: a.y_nm.min(b.y_nm) },
                    max: Point { x_nm: a.x_nm.max(b.x_nm), y_nm: a.y_nm.max(b.y_nm) },
                }
            }
            Self::Circle { center, radius_nm } => Self::Circle { center: t.point(*center)?, radius_nm: *radius_nm },
            Self::Polyline { points } => Self::Polyline { points: points.iter().map(|p| t.point(*p)).collect::<Result<_, _>>()? },
            Self::Text { at, text, align } => Self::Graphic {
                graphic: Graphic {
                    geometry: Geometry::Text { at: *at, text: text.clone(), height_nm: 1_000_000, rotation: 0, align: *align },
                    stroke_nm: 100_000,
                    fill: Fill::None,
                }
                .transformed(t)?,
            },
            Self::Pad { pad } => {
                Self::Pad { pad: footprint::RectPad { center: t.point(pad.center)?, rotation: t.angle(pad.rotation as i16), ..pad.clone() } }
            }
            Self::Outline { graphic } => Self::Outline {
                graphic: footprint::GraphicRect { center: t.point(graphic.center)?, rotation: t.angle(graphic.rotation as i16), ..graphic.clone() },
            },
            Self::Silkscreen { path } => Self::Silkscreen {
                path: footprint::SilkPath { points: path.points.iter().map(|p| t.point(*p)).collect::<Result<_, _>>()?, closed: path.closed },
            },
        })
    }
}
/// Resolve a nonempty, distinct selection before mutating the candidate document.
pub(super) fn selected(item: &Item, objects: &[u64]) -> Result<Vec<Primitive>, String> {
    if objects.is_empty() || objects.len() > 512 {
        return Err("Choose between one and 512 primitives.".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    objects
        .iter()
        .map(|id| {
            if !seen.insert(id) {
                return Err("Duplicate primitive ID in selection.".into());
            }
            item.objects.get(id).cloned().ok_or("Unknown primitive ID.".into())
        })
        .collect()
}
/// Allocate a bounded batch into a temporary candidate, giving copied electrical objects unused numbers.
pub(super) fn paste(item: &mut Item, primitives: &[Primitive], offset: Point, cursor: &mut u64) -> Result<(), String> {
    if primitives.is_empty() || item.objects.len().saturating_add(primitives.len()) > 512 {
        return Err("A definition can contain at most 512 primitives.".into());
    }
    for original in primitives {
        let mut primitive = original.translated(offset)?;
        match &mut primitive {
            Primitive::Pin { number, .. } => *number = next_number(item, number)?,
            Primitive::Pad { pad } => pad.number = next_number(item, &pad.number)?,
            _ => {}
        }
        let id = *cursor;
        *cursor = cursor.checked_add(1).ok_or("Primitive ID limit reached.")?;
        item.objects.insert(id, primitive);
    }
    Ok(())
}

/// An explicit fork can integrate an unambiguous legacy lead/label into its pin. The source
/// definition and old files stay untouched; ambiguous drawing strokes remain independent.
pub(super) fn legacy_pin(
    definition: &crate::schematic::SymbolUnit,
    pin: &crate::schematic::PinDefinition,
    consumed: &mut std::collections::BTreeSet<usize>,
) -> Option<crate::schematic::PinAppearance> {
    use crate::schematic::{PinAppearance, PinOrientation};
    let candidates: Vec<_> = definition
        .drawing
        .iter()
        .enumerate()
        .filter_map(|(index, shape)| {
            let Shape::Line(a, b) = shape else { return None };
            let end = if *a == pin.offset {
                *b
            } else if *b == pin.offset {
                *a
            } else {
                return None;
            };
            if consumed.contains(&index) || end == pin.offset || definition.pins().iter().any(|p| p.offset == end) {
                return None;
            }
            let dx = end.x_nm - pin.offset.x_nm;
            let dy = end.y_nm - pin.offset.y_nm;
            let (orientation, length_nm) = if dy == 0 {
                (if dx > 0 { PinOrientation::Right } else { PinOrientation::Left }, dx.abs())
            } else if dx == 0 {
                (if dy > 0 { PinOrientation::Down } else { PinOrientation::Up }, dy.abs())
            } else {
                return None;
            };
            Some((index, orientation, length_nm, end))
        })
        .collect();
    let [(index, orientation, length_nm, end)] = candidates.as_slice() else { return None };
    let mut appearance =
        PinAppearance { orientation: *orientation, length_nm: *length_nm, show_name: false, show_number: false, ..Default::default() };
    if appearance.validate(pin.offset).is_err() {
        return None;
    }
    consumed.insert(*index);
    for (index, shape) in definition.drawing.iter().enumerate() {
        let Shape::Text(at, text, _) = shape else { continue };
        let near = at.x_nm >= pin.offset.x_nm.min(end.x_nm) - 1_500_000
            && at.x_nm <= pin.offset.x_nm.max(end.x_nm) + 1_500_000
            && at.y_nm >= pin.offset.y_nm.min(end.y_nm) - 1_500_000
            && at.y_nm <= pin.offset.y_nm.max(end.y_nm) + 1_500_000;
        if !near || consumed.contains(&index) {
            continue;
        }
        if *text == pin.number {
            appearance.show_number = true;
            consumed.insert(index);
        } else if pin.name.as_deref() == Some(text.as_str()) {
            appearance.show_name = true;
            consumed.insert(index);
        }
    }
    Some(appearance)
}
