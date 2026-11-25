//! The board surfaces shared by raster and ray-traced rendering.

use pcbcraft_engine::board::view::BoardView;
use pcbcraft_geometry::substrate::BoardMesh;
use solidcore_record::Vec3;
use solidcraft_viewport::vec::{Bounds, Vec3Ops};

#[derive(Clone, Copy)]
pub(super) enum Material {
    Substrate,
    Copper,
}

impl Material {
    /// Preview colors in sRGB, shared with the material's base color in the ray tracer.
    pub(super) fn rgba(self) -> [u8; 4] {
        match self {
            Self::Substrate => [36, 134, 88, 255],
            Self::Copper => [198, 140, 61, 255],
        }
    }
}

/// Visit exterior copper and substrate in millimeters relative to the board center.
/// Downstream renderers see the same clipping, outward normals and board camera frame.
pub(super) fn visit(
    mesh: BoardMesh,
    board: BoardView<'_>,
    thickness_nm: i64,
    copper_offset_mm: f64,
    mut emit: impl FnMut(Material, [Vec3; 3], Vec3),
) -> Result<Bounds, String> {
    let positions: Vec<_> = mesh.positions_nm.into_iter().map(|[x, y, z]| Vec3::new(x as f64, y as f64, z as f64) * 1e-6).collect();
    let mut bounds = Bounds::EMPTY;
    for &p in &positions {
        bounds.add(p);
    }
    let center = bounds.center();
    let mut has_substrate = false;
    let mut append = |material, points: [Vec3; 3]| {
        let [a, b, c] = points.map(|p| p - center);
        if let Some(normal) = (b - a).cross(c - a).normalized() {
            if matches!(material, Material::Substrate) {
                has_substrate = true;
            }
            emit(material, [a, b, c], normal);
        }
    };
    for triangle in mesh.triangles {
        append(Material::Substrate, triangle.map(|i| positions[i as usize]));
    }
    super::copper::visit(board, thickness_nm, copper_offset_mm, |points| append(Material::Copper, points)).map_err(|err| err.to_string())?;
    if !has_substrate {
        return Err("The Board Outline produced no renderable faces.".into());
    }
    Ok(Bounds { min: bounds.min - center, max: bounds.max - center })
}
