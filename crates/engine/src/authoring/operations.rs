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
            Self::NonPlatedHole { hole } => Self::NonPlatedHole { hole: footprint::CircularNpth { center: t.point(hole.center)?, ..*hole } },
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
    let limit = item.max_objects();
    if objects.is_empty() || objects.len() > limit {
        return Err(format!("Choose between one and {limit} primitives."));
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
    let limit = item.max_objects();
    if primitives.is_empty() || item.objects.len().saturating_add(primitives.len()) > limit {
        return Err(format!("A definition can contain at most {limit} primitives."));
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

/// Parameters for generating a ball-grid pad array from one seed pad.
#[derive(Clone, Copy, Debug)]
pub(super) struct BallGridSpec {
    pub(super) columns: u16,
    pub(super) rows: u16,
    pub(super) column_step: Point,
    pub(super) row_step: Point,
    pub(super) skip_jedec: bool,
}

/// Generate a ball-grid pad array from one seed pad, naming rows with letters and columns with 1-based numbers.
///
/// The seed pad becomes cell `(0, 0)` (`A1`) and the remaining `rows * columns - 1` pads are allocated
/// into `item`. Rejections leave the caller's committed session untouched because `item` is a candidate clone.
pub(super) fn ball_grid_array(item: &mut Item, objects: &[u64], spec: BallGridSpec, cursor: &mut u64) -> Result<(), String> {
    let BallGridSpec { columns, rows, column_step, row_step, skip_jedec } = spec;
    if item.kind != Kind::Footprint {
        return Err("Ball-grid numbering is only supported for footprint definitions.".into());
    }
    let [seed_id] = objects else {
        return Err("Ball-grid numbering requires selecting a single pad.".into());
    };
    let Some(Primitive::Pad { pad: seed_pad }) = item.objects.get(seed_id).cloned() else {
        return Err("Ball-grid numbering requires selecting a single pad.".into());
    };
    let zero = Point { x_nm: 0, y_nm: 0 };
    if (columns > 1 && column_step == zero) || (rows > 1 && row_step == zero) {
        return Err("Ball-grid array step must be nonzero along any axis with more than one cell.".into());
    }
    let names = ball_grid::generate_identifiers(rows, columns, skip_jedec)?;
    let count = names.len();
    let limit = item.max_objects();
    if item.objects.len().saturating_add(count.saturating_sub(1)) > limit {
        return Err(format!("Array exceeds the definition's {limit} primitive limit."));
    }
    let existing_pads = item.objects.values().filter(|p| matches!(p, Primitive::Pad { .. })).count();
    if existing_pads.saturating_add(count.saturating_sub(1)) > footprint::MAX_PADS {
        return Err(format!("Array exceeds the footprint's {} pad limit.", footprint::MAX_PADS));
    }
    let existing_other: std::collections::BTreeSet<&str> = item
        .objects
        .iter()
        .filter(|(id, _)| *id != seed_id)
        .filter_map(|(_, primitive)| match primitive {
            Primitive::Pad { pad } => Some(pad.number.as_str()),
            _ => None,
        })
        .collect();
    if let Some(collision) = names.iter().find(|name| existing_other.contains(name.as_str())) {
        return Err(format!("Ball-grid pad identifier `{collision}` is already used by another pad."));
    }
    let mut name_iter = names.into_iter();
    for row in 0..rows {
        for column in 0..columns {
            let number = name_iter.next().ok_or("Ball-grid identifier count mismatch.")?;
            if row == 0 && column == 0 {
                if let Some(Primitive::Pad { pad }) = item.objects.get_mut(seed_id) {
                    pad.number = number;
                }
                continue;
            }
            let axis = |a: i64, b: i64| {
                i64::try_from(i128::from(a) * i128::from(column) + i128::from(b) * i128::from(row)).map_err(|_| "Array offset overflow.")
            };
            let offset = Point { x_nm: axis(column_step.x_nm, row_step.x_nm)?, y_nm: axis(column_step.y_nm, row_step.y_nm)? };
            let mut primitive = Primitive::Pad { pad: seed_pad.clone() }.translated(offset)?;
            if let Primitive::Pad { pad } = &mut primitive {
                pad.number = number;
            }
            let id = *cursor;
            *cursor = cursor.checked_add(1).ok_or("Primitive ID limit reached.")?;
            item.objects.insert(id, primitive);
        }
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
