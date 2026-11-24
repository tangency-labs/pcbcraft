//! The board solid and its placed component models as one STEP assembly, which SolidCraft
//! inserts with Insert STEP: one product for the board, one per model, and one occurrence per
//! placed footprint at its component pose.
//!
//! The caller supplies the models and binds them to footprints; nothing here reads or stores a
//! model record. The assembly is derived from one immutable board snapshot and the same input
//! gives the same bytes, so writing it again after an edit changes only what the edit moved.

use std::collections::BTreeMap;
use std::fmt;

use pcbcraft_engine::board::view::{BoardTarget, BoardView};
use pcbcraft_engine::schematic::Point;
use pcbcraft_geometry::{AssemblyPart, Body, GeometryError, Occurrence, Transform, write_step_assembly};

use crate::BoardSolid;

/// The board's product and occurrence name in the assembly.
pub const BOARD_NAME: &str = "board";

/// A model bound to one placed footprint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Binding {
    /// The model placed (index into the models given to [`BoardAssembly::build`]).
    pub model: usize,
    /// Where the model sits in the footprint's model frame: the binding's alignment after any
    /// correction of the model file's own frame. A rotation and a translation only.
    pub alignment: Transform,
}

/// One placed footprint with a model.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedComponent {
    /// The footprint's ID.
    pub footprint: String,
    /// The model placed (index into the models given to [`BoardAssembly::build`]).
    pub model: usize,
    /// The model's pose in the board/1 frame.
    pub transform: Transform,
}

/// Why an assembly was not made.
#[derive(Clone, Debug, PartialEq)]
pub enum AssemblyError {
    /// Board Setup declares no finished thickness. Bottom parts sit on the bottom surface, so no
    /// nominal value stands in for it.
    NoFinishedThickness,
    /// A binding names a footprint that is not on the board.
    UnknownFootprint(String),
    /// A binding names a model that was not supplied.
    UnknownModel { footprint: String, model: usize },
    /// The board solid or the STEP writer refused.
    Geometry(GeometryError),
}

impl fmt::Display for AssemblyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFinishedThickness => f.write_str("declare the finished board thickness in Board Setup before exporting the assembly"),
            Self::UnknownFootprint(id) => write!(f, "a model is bound to {id}, which is not on the board"),
            Self::UnknownModel { footprint, model } => write!(f, "{footprint} is bound to model {model}, which was not supplied"),
            Self::Geometry(e) => fmt::Display::fmt(e, f),
        }
    }
}

impl std::error::Error for AssemblyError {}

impl From<GeometryError> for AssemblyError {
    fn from(e: GeometryError) -> Self {
        Self::Geometry(e)
    }
}

/// A board and its bound component models as one STEP assembly. Derived data: never store it in
/// the project or history.
#[derive(Debug)]
pub struct BoardAssembly {
    pub target: BoardTarget,
    /// The declared finished thickness, in nanometres.
    pub thickness_nm: i64,
    /// Each placed footprint with a model, in board order.
    pub components: Vec<PlacedComponent>,
    /// The placed footprints without a binding, in board order. They get no body.
    pub unbound: Vec<String>,
    /// The AP242 STEP assembly: the board at the identity, then each component at its pose.
    pub step: Vec<u8>,
}

impl BoardAssembly {
    /// Make the assembly `name` of the explicitly addressed immutable board: its board solid
    /// ([`BoardSolid::build`]) and, for each placed footprint with a binding, its model at the
    /// component pose ([`component_pose`]). Each model is written once, however often it is
    /// placed; models no binding uses are not written. Building changes no board state or undo.
    ///
    /// # Errors
    ///
    /// [`AssemblyError::NoFinishedThickness`] without a declared finished thickness; a binding to
    /// a footprint the board does not have or to a model not supplied; and the refusals of the
    /// board solid and [`write_step_assembly`].
    pub fn build(board: BoardView<'_>, name: &str, models: &[&Body], bindings: &BTreeMap<String, Binding>) -> Result<Self, AssemblyError> {
        let Some(thickness_nm) = board.setup().and_then(|setup| setup.stackup().finished_thickness_nm) else {
            return Err(AssemblyError::NoFinishedThickness);
        };
        let mut components = Vec::new();
        let mut unbound = Vec::new();
        for footprint in board.footprints() {
            let Some(binding) = bindings.get(&footprint.id) else {
                unbound.push(footprint.id);
                continue;
            };
            if binding.model >= models.len() {
                return Err(AssemblyError::UnknownModel { footprint: footprint.id, model: binding.model });
            }
            let transform = component_pose(footprint.position, footprint.rotation, footprint.bottom, thickness_nm, &binding.alignment);
            components.push(PlacedComponent { footprint: footprint.id, model: binding.model, transform });
        }
        if let Some(id) = bindings.keys().find(|id| !components.iter().any(|c| &c.footprint == *id)) {
            return Err(AssemblyError::UnknownFootprint(id.clone()));
        }

        let solid = BoardSolid::build(board)?;
        let board_bodies = [&solid.body];
        let model_bodies: Vec<[&Body; 1]> = models.iter().map(|body| [*body]).collect();
        let mut parts = vec![AssemblyPart { name: BOARD_NAME, bodies: &board_bodies }];
        parts.extend(models.iter().zip(&model_bodies).map(|(body, bodies)| AssemblyPart { name: body.name(), bodies }));
        let names: Vec<String> = components.iter().map(|c| format!("component:{}", c.footprint)).collect();
        let mut occurrences = vec![Occurrence { part: 0, name: BOARD_NAME, transform: Transform::IDENTITY }];
        occurrences.extend(components.iter().zip(&names).map(|(c, name)| Occurrence { part: c.model + 1, name, transform: c.transform }));
        let step = write_step_assembly(name, &parts, &occurrences)?;
        Ok(Self { target: solid.target, thickness_nm, components, unbound, step })
    }
}

/// The pose of a model in the board/1 frame for a footprint at board `position`, turned
/// `rotation` whole degrees counterclockwise as seen from the top, on the bottom side when
/// `bottom`, on a board `thickness_nm` thick:
///
/// ```text
/// P = T(x, −y, 0) · Rz(r) · S · A
/// S = I                         on the top side
/// S = T(0, 0, −t) · Ry(180°)    on the bottom side
/// ```
///
/// `A` is `alignment`. A bottom part is turned over about Y, never mirrored, because the 2D
/// bottom placement mirrors local X before the rotation. Quarter turns are exact.
pub fn component_pose(position: Point, rotation: u16, bottom: bool, thickness_nm: i64, alignment: &Transform) -> Transform {
    let (sin, cos) = match rotation % 360 {
        0 => (0.0, 1.0),
        90 => (1.0, 0.0),
        180 => (0.0, -1.0),
        270 => (-1.0, 0.0),
        r => f64::from(r).to_radians().sin_cos(),
    };
    let turn =
        Transform { axes: [[cos, sin, 0.0], [-sin, cos, 0.0], [0.0, 0.0, 1.0]], translation_nm: [position.x_nm, position.y_nm.saturating_neg(), 0] };
    let side = if bottom {
        Transform { axes: [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]], translation_nm: [0, 0, thickness_nm.saturating_neg()] }
    } else {
        Transform::IDENTITY
    };
    compose(&compose(&turn, &side), alignment)
}

/// `outer · inner`: `inner` first, then `outer`. The translation is rounded once to the nearest
/// nanometre.
fn compose(outer: &Transform, inner: &Transform) -> Transform {
    let apply = |v: [f64; 3]| -> [f64; 3] {
        let [x, y, z] = outer.axes;
        [0, 1, 2].map(|i| v[0] * x[i] + v[1] * y[i] + v[2] * z[i])
    };
    let axes = inner.axes.map(apply);
    let moved = apply(inner.translation_nm.map(|v| v as f64));
    let mut translation_nm = [0; 3];
    for ((t, m), o) in translation_nm.iter_mut().zip(moved).zip(outer.translation_nm) {
        *t = (m.round() as i64).saturating_add(o);
    }
    Transform { axes, translation_nm }
}
