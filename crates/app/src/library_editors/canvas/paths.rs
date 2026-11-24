//! Adapt vertex-defined library objects to the shared canvas path editor.
use super::{Point, Primitive, foot};
use crate::path_edit::Path;
use pcbcraft_engine::library_graphics::{Geometry, Graphic};

/// A borrowed definition path retains its original type and style when its vertices change.
#[derive(Clone, Copy)]
pub(super) enum DefinitionPath<'a> {
    Symbol(Path<'a>, &'a Graphic),
    Artwork(Path<'a>, &'a foot::Artwork),
    Legacy(Path<'a>),
    Silkscreen(Path<'a>),
}

impl<'a> DefinitionPath<'a> {
    /// Curves, parameterized rectangles, pads and text keep their own model and editing rules.
    pub(super) fn of(primitive: &'a Primitive) -> Option<Self> {
        let graphic_path = |graphic: &'a Graphic| match &graphic.geometry {
            Geometry::Polyline { points, closed } => Some(Path { points, closed: *closed, stroke_nm: Some(graphic.stroke_nm) }),
            _ => None,
        };
        match primitive {
            Primitive::Graphic { graphic } => Some(Self::Symbol(graphic_path(graphic)?, graphic)),
            Primitive::Artwork { artwork } if artwork.purpose != foot::GraphicPurpose::Courtyard => {
                Some(Self::Artwork(graphic_path(&artwork.graphic)?, artwork))
            }
            Primitive::Polyline { points } => Some(Self::Legacy(Path { points, closed: false, stroke_nm: None })),
            Primitive::Silkscreen { path } => Some(Self::Silkscreen(Path { points: &path.points, closed: path.closed, stroke_nm: None })),
            _ => None,
        }
    }

    /// Project only the geometry and physical stroke that the canvas needs.
    pub(super) fn outline(self) -> Path<'a> {
        match self {
            Self::Symbol(path, _) | Self::Artwork(path, _) | Self::Legacy(path) | Self::Silkscreen(path) => path,
        }
    }

    /// Rebuild an object in the same schema, retaining fill, stroke and footprint purpose.
    pub(super) fn replace(self, points: Vec<Point>) -> Primitive {
        let closed = self.outline().closed;
        match self {
            Self::Symbol(_, original) => {
                let mut graphic = original.clone();
                graphic.geometry = Geometry::Polyline { points, closed };
                Primitive::Graphic { graphic }
            }
            Self::Artwork(_, original) => {
                let mut artwork = original.clone();
                artwork.graphic.geometry = Geometry::Polyline { points, closed };
                Primitive::Artwork { artwork }
            }
            Self::Legacy(_) => Primitive::Polyline { points },
            Self::Silkscreen(_) => Primitive::Silkscreen { path: foot::SilkPath { points, closed } },
        }
    }
}
