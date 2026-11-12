//! Layer-addressed non-copper board graphics, with physical strokes on technical layers.
use super::{
    setup::{Layer, LayerPurpose, Stratum},
    view::BoardView,
};
use crate::{CommandError, Project, Session, schematic::Point};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Add an open or closed graphic on a non-copper layer. Copper uses the conductive track commands.
pub const ADD_DRAWING: &str = "pcb.add_drawing";
/// Replace one graphic's vertices while retaining its identity and layer.
pub const SET_DRAWING: &str = "pcb.set_drawing";
/// Remove one graphic without changing its layer or other board objects.
pub const DELETE_DRAWING: &str = "pcb.delete_drawing";
const MAX_ITEMS: usize = 512;
const MAX_POINTS: usize = 256;
const MAX_TOTAL_POINTS: usize = 4096;

/// An independently selectable non-copper layer graphic. Missing stroke width preserves
/// legacy Board Outline hairlines; new technical graphics carry an explicit physical width.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Drawing {
    pub id: Uuid,
    pub layer: Uuid,
    pub points: Vec<Point>,
    pub closed: bool,
    #[serde(default, deserialize_with = "explicit_stroke", skip_serializing_if = "Option::is_none")]
    pub stroke_nm: Option<i64>,
}

/// An omitted width retains a legacy hairline; an explicitly present width cannot be null.
fn explicit_stroke<'de, D: serde::Deserializer<'de>>(value: D) -> Result<Option<i64>, D::Error> {
    i64::deserialize(value).map(Some)
}

/// Additional technical layers do not invent a copper stackup or rebind legacy footprints.
/// Configured-board drawings can instead reference their existing setup layer UUIDs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Drawings {
    pub layers: Vec<Layer>,
    pub items: Vec<Drawing>,
}

/// Derive only compatibility/presentation identities, never identities for newly authored items.
fn compatibility_id(board: Uuid, domain: &[u8]) -> Uuid {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(domain);
    hash.update(board.as_bytes());
    let digest = hash.finalize();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

impl BoardView<'_> {
    /// One resolved layer catalog, including technical layers on legacy boards without a stackup.
    /// The virtual Board Outline entry becomes authored metadata only when its first drawing is made.
    pub fn layers(self) -> Vec<Layer> {
        let mut layers = self.setup().map_or_else(Vec::new, |setup| setup.layers().to_vec());
        if let Some(data) = &self.project().board.drawings {
            layers.extend(data.layers.iter().cloned());
        }
        let edges = layers.iter().filter(|layer| layer.purpose == LayerPurpose::Outline).count();
        if edges == 0 || (self.outline().is_some() && self.project().board.drawings.is_none() && edges > 1) {
            let mut name = "Board Outline".to_owned();
            let mut suffix = 2;
            while layers.iter().any(|layer| layer.name.eq_ignore_ascii_case(&name)) {
                name = format!("Board Outline {suffix}");
                suffix += 1;
            }
            layers.push(Layer {
                id: compatibility_id(self.target.board, b"pcbcraft:board-edge-layer:1\0"),
                name,
                purpose: LayerPurpose::Outline,
                side: None,
            });
        }
        layers
    }

    /// The Board Outline's bounding box as (min, max) corners, from every outline drawing, or
    /// None when the board has no outline geometry.
    pub fn outline_bounds(self) -> Option<(Point, Point)> {
        let layers = self.layers();
        let points: Vec<Point> = self
            .drawings()
            .into_iter()
            .filter(|drawing| layers.iter().any(|layer| layer.id == drawing.layer && layer.purpose == LayerPurpose::Outline))
            .flat_map(|drawing| drawing.points)
            .collect();
        let first = *points.first()?;
        Some(points.iter().fold((first, first), |(min, max), p| {
            (Point { x_nm: min.x_nm.min(p.x_nm), y_nm: min.y_nm.min(p.y_nm) }, Point { x_nm: max.x_nm.max(p.x_nm), y_nm: max.y_nm.max(p.y_nm) })
        }))
    }

    /// Resolve authored drawings and the earlier local rectangle preview without mutating a file.
    /// Editing that preview materializes ordinary layer geometry; untouched saves keep its bytes.
    pub fn drawings(self) -> Vec<Drawing> {
        if let Some(data) = &self.project().board.drawings {
            return data.items.clone();
        }
        self.outline()
            .and_then(|outline| {
                let layers = self.layers();
                let mut edge = layers.iter().filter(|layer| layer.purpose == LayerPurpose::Outline);
                let layer = edge.next()?;
                // Older rectangles were not layer-addressed. A fresh compatibility layer avoids
                // choosing an arbitrary layer when the old setup has several outline layers.
                let id = if edge.next().is_some() { compatibility_id(self.target.board, b"pcbcraft:board-edge-layer:1\0") } else { layer.id };
                Some(vec![Drawing {
                    id: compatibility_id(self.target.board, b"pcbcraft:legacy-outline:1\0"),
                    layer: id,
                    points: outline.corners().to_vec(),
                    closed: true,
                    stroke_nm: None,
                }])
            })
            .unwrap_or_default()
    }
}

impl Drawing {
    /// Bound path size and coordinates, rejecting collapsed segments and duplicated closing
    /// vertices and out-of-range physical widths. Open paths are valid editing state;
    /// closure never depends on a screen tolerance.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.id.is_nil()
            || self.layer.is_nil()
            || !((if self.closed { 3 } else { 2 })..=MAX_POINTS).contains(&self.points.len())
            || self.points.iter().any(|point| !crate::wire::valid_point(point))
            || self.points.windows(2).any(|pair| pair[0] == pair[1])
            || (self.closed && self.points.first() == self.points.last())
            || self.stroke_nm.is_some_and(|width| !(super::track::MIN_WIDTH_NM..=super::track::MAX_WIDTH_NM).contains(&width))
        {
            return Err("a drawing needs a non-nil identity/layer and 2–256 bounded distinct consecutive points (at least 3 when closed)".into());
        }
        Ok(())
    }
}

impl Drawings {
    /// Validate the complete layer/graphic candidate before adoption, including every reference.
    /// Technical additions cannot shadow a configured layer, introduce copper, or exceed bounds.
    pub(crate) fn validate(&mut self, project: &Project) -> Result<(), String> {
        let mut layers: BTreeMap<_, _> =
            project.board.setup.as_ref().into_iter().flat_map(|setup| setup.layers()).map(|layer| (layer.id, layer)).collect();
        let mut reserved: BTreeSet<_> = layers.keys().copied().collect();
        if let Some(setup) = &project.board.setup {
            reserved.insert(setup.stackup().id);
            for stratum in &setup.stackup().strata {
                if let Stratum::Dielectric { id, construction, .. } = stratum {
                    reserved.insert(*id);
                    if let Some(construction) = construction {
                        reserved.extend(construction.ply_ids());
                    }
                }
            }
        }
        let mut names: BTreeSet<_> = layers.values().map(|layer| layer.name.to_lowercase()).collect();
        for layer in &self.layers {
            if layer.id.is_nil()
                || !reserved.insert(layer.id)
                || layer.purpose != LayerPurpose::Outline
                || layer.side.is_some()
                || layer.name.trim() != layer.name
                || layer.name.is_empty()
                || layer.name.chars().count() > 80
                || layer.name.chars().any(char::is_control)
                || !names.insert(layer.name.to_lowercase())
                || layers.insert(layer.id, layer).is_some()
            {
                return Err("invalid or conflicting Board Outline layer".into());
            }
        }
        if layers.len() > 128 || self.items.len() > MAX_ITEMS || self.items.iter().map(|item| item.points.len()).sum::<usize>() > MAX_TOTAL_POINTS {
            return Err("board drawing resource limit exceeded".into());
        }
        let mut ids = BTreeSet::new();
        for item in &self.items {
            item.validate()?;
            if !ids.insert(item.id)
                || !layers.get(&item.layer).is_some_and(|layer| {
                    layer.purpose != LayerPurpose::Copper && (layer.purpose == LayerPurpose::Outline || item.stroke_nm.is_some())
                })
            {
                return Err("duplicate graphic, unknown/copper layer, or missing technical stroke width".into());
            }
        }
        self.layers.sort_by_key(|layer| layer.id);
        self.items.sort_by_key(|item| item.id);
        Ok(())
    }
}

impl Session {
    /// Create, replace or delete one layer graphic atomically. No view state, footprint placement
    /// or connectivity changes; stale/unknown addresses and invalid paths preserve undo and bytes.
    pub(crate) fn edit_board_drawing(&mut self, command: &str, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let target = project.resolve_board(None)?;
        let view = project.board_view(target).ok_or_else(|| CommandError::Unavailable("board unavailable".into()))?;
        let mut data = project.board.drawings.clone().unwrap_or_else(|| {
            let items = view.drawings();
            let layers = view
                .layers()
                .into_iter()
                .filter(|layer| {
                    items.iter().any(|item| item.layer == layer.id)
                        && !view.setup().is_some_and(|setup| setup.layers().iter().any(|existing| existing.id == layer.id))
                })
                .collect();
            Drawings { layers, items }
        });
        let before = data.clone();
        if command == ADD_DRAWING {
            let layer: Uuid = serde_json::from_value(params["layer"].clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))?;
            let record = view
                .layers()
                .into_iter()
                .find(|record| record.id == layer && record.purpose != LayerPurpose::Copper)
                .ok_or_else(|| CommandError::InvalidParams("select a non-copper layer; use pcb.add_tracks for conductive copper".into()))?;
            if !view.setup().is_some_and(|setup| setup.layers().iter().any(|existing| existing.id == layer))
                && !data.layers.iter().any(|existing| existing.id == layer)
            {
                data.layers.push(record);
            }
            data.items.push(Drawing {
                id: crate::fresh_id()?,
                layer,
                points: serde_json::from_value(params["points"].clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))?,
                closed: params["closed"].as_bool().unwrap_or(false),
                stroke_nm: params
                    .get("stroke_nm")
                    .map(|value| serde_json::from_value(value.clone()))
                    .transpose()
                    .map_err(|error| CommandError::InvalidParams(error.to_string()))?,
            });
        } else {
            let id: Uuid = serde_json::from_value(params["drawing"].clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))?;
            let index =
                data.items.iter().position(|item| item.id == id).ok_or_else(|| CommandError::InvalidParams("unknown board drawing".into()))?;
            if command == DELETE_DRAWING {
                data.items.remove(index);
            } else {
                data.items[index].points =
                    serde_json::from_value(params["points"].clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))?;
                data.items[index].closed = params["closed"].as_bool().unwrap_or(false);
                if let Some(value) = params.get("stroke_nm") {
                    data.items[index].stroke_nm =
                        Some(serde_json::from_value(value.clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))?);
                }
            }
        }
        data.validate(project).map_err(CommandError::InvalidParams)?;
        if data != before {
            let mut next = project.clone();
            let board = std::sync::Arc::make_mut(&mut next.board);
            board.outline = None;
            board.drawings = Some(data);
            self.change(next)?;
        }
        Ok(())
    }
}

/// Describe editable geometry without assuming closure or claiming manufacturing validity.
pub(crate) fn geometry_schema() -> Value {
    json!({"type":"array","minItems":2,"maxItems":MAX_POINTS,"items":{"type":"object","properties":{
        "x_nm":{"type":"integer","minimum":-crate::schematic::MAX_CENTER_NM,"maximum":crate::schematic::MAX_CENTER_NM},
        "y_nm":{"type":"integer","minimum":-crate::schematic::MAX_CENTER_NM,"maximum":crate::schematic::MAX_CENTER_NM}
    },"required":["x_nm","y_nm"],"additionalProperties":false}})
}

#[cfg(test)]
mod tests;
