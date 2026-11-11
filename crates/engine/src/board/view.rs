//! Immutable, board-addressed presentation and correspondence over the native model.

use crate::{Project, schematic::Point, selection::Selection};
use std::collections::BTreeSet;
use uuid::Uuid;

/// A board address distinguishes its project and physical board, independently of names,
/// stackup identity and runtime opening. Legacy files have one deterministically addressed board.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardTarget {
    pub project: Uuid,
    pub board: Uuid,
}

/// Derive the legacy board identity from a fixed domain and saved project UUID. This compatibility
/// address survives no-op saves without silently migrating files; a future explicit board list
/// must preserve it when migrating. It is not a rule for generating identities of additional boards.
fn legacy_board_id(project: Uuid) -> Uuid {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"pcbcraft:legacy-board:1\0");
    hash.update(project.as_bytes());
    let digest = hash.finalize();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    // UUID v8: application-defined identity with the standard variant.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

/// Resolved board data scoped to an explicitly validated target, independent of the GUI.
#[derive(Clone, Copy)]
pub struct BoardView<'a> {
    project: &'a Project,
    pub target: BoardTarget,
}

/// Physical copper membership resolved by the engine. Unconfigured boards retain their
/// explicit compatibility binding instead of inventing a stackup or layer identities.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CopperLayers {
    Unconfigured { binding: super::definition::CopperBinding },
    Resolved { binding: super::definition::CopperBinding, layers: Vec<Uuid> },
}

impl CopperLayers {
    /// Resolve library copper binding in physical stack order. Component-side lands use the
    /// front copper of a top-side placement and the back copper of a bottom-side one;
    /// all-copper lands span the stack.
    pub(crate) fn resolve(setup: Option<&super::setup::BoardSetup>, binding: super::definition::CopperBinding, bottom: bool) -> Self {
        use super::{
            definition::CopperBinding,
            setup::{Side, Stratum},
        };
        let Some(setup) = setup else { return Self::Unconfigured { binding } };
        let layers = setup
            .stackup()
            .strata
            .iter()
            .filter_map(|stratum| {
                let Stratum::Copper { layer, .. } = stratum else { return None };
                let side = if bottom { Side::Back } else { Side::Front };
                (binding == CopperBinding::AllCopper || setup.layers().iter().any(|record| record.id == *layer && record.side == Some(side)))
                    .then_some(*layer)
            })
            .collect();
        Self::Resolved { binding, layers }
    }
}

/// A resolved pad in board nanometers. Physical lands carry rotated bounds and contours; legacy markers
/// explicitly retain no shape and cannot be treated as manufactured pad geometry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PadView {
    pub number: String,
    pub center: Point,
    /// Land bounding corners in board coordinates; None denotes a legacy center-only marker.
    pub copper: Option<[Point; 4]>,
    /// None denotes a marker with no physical copper. Configured memberships contain layer UUIDs.
    pub copper_layers: Option<CopperLayers>,
    pub shape: super::definition::PadShape,
    pub corner_radius_nm: i64,
    pub drill_nm: Option<i64>,
    pub through_hole: bool,
    /// Snapshot-local connectivity label, matching Net Inspector; not a persisted net identity.
    pub net: Option<String>,
}

/// A technical footprint rectangle resolved independently of copper visibility and layer ownership.
#[derive(Clone, Debug)]
pub struct GraphicView {
    pub purpose: super::definition::GraphicPurpose,
    /// The board side whose technical layer holds this graphic: the placement's side.
    pub side: super::setup::Side,
    pub points: Vec<Point>,
    pub stroke_nm: Option<i64>,
    pub closed: bool,
    /// This path belongs to the generated reference text rather than fixed footprint artwork.
    pub reference: bool,
}

/// A placement resolved in one addressed board, including its logical component association.
#[derive(Clone, Debug)]
pub struct FootprintView<'a> {
    pub board: BoardTarget,
    pub id: String,
    pub component: String,
    pub reference: &'a str,
    pub value: &'a str,
    pub definition: &'a str,
    pub position: Point,
    /// Counterclockwise whole degrees around the footprint origin, from 0 through 359.
    pub rotation: u16,
    pub pads: Vec<PadView>,
    pub graphics: Vec<GraphicView>,
    pub physical_reference: bool,
    /// Additional local displacement from the library reference default, before rotation or side mirroring.
    pub reference_offset: Point,
    /// Sparse local print overrides, retained independently of the component's reference.
    pub reference_text: Option<super::reference::Text>,
    /// Effective font, words and print dimensions after resolving defaults and local overrides.
    pub reference_appearance: super::reference::Resolved,
    /// The accepted source was removed from the schematic; physical removal is pending.
    pub pending_removal: bool,
    /// Placed on the bottom side, mirrored and on the back layers.
    pub bottom: bool,
}

impl Project {
    /// Enumerate available boards in document order. The legacy representation has one board;
    /// callers should retain its target rather than infer identity from list position or name.
    pub fn boards(&self) -> impl Iterator<Item = BoardView<'_>> {
        self.board_target().map(|target| BoardView { project: self, target }).into_iter()
    }

    /// Resolve the sole registered board, retaining legacy compatibility identity on old files.
    /// Schematic-only projects return no target and never synthesize a physical owner.
    pub fn board_target(&self) -> Option<BoardTarget> {
        let board = match &self.board.ownership {
            super::identity::Ownership::Legacy => legacy_board_id(self.id),
            super::identity::Ownership::Explicit(Some(registration)) => registration.id,
            super::identity::Ownership::Explicit(None) => return None,
        };
        Some(BoardTarget { project: self.id, board })
    }

    /// Resolve exactly this board in this project; unknown or foreign targets never fall back.
    pub fn board_view(&self, target: BoardTarget) -> Option<BoardView<'_>> {
        self.boards().find(|board| board.target == target)
    }

    /// Validate an explicit board ID, retaining the legacy single-board default on omission.
    pub fn resolve_board(&self, id: Option<&str>) -> Result<BoardTarget, crate::CommandError> {
        let default = self.board_target().ok_or_else(|| crate::CommandError::Unavailable("no physical board exists; create a board first".into()))?;
        match id {
            None => Ok(default),
            Some(id) if id == default.board.to_string() => Ok(default),
            Some(_) => Err(crate::CommandError::InvalidParams("unknown board ID".to_owned())),
        }
    }
}

impl<'a> BoardView<'a> {
    /// Existing placements without accepted history need explicit adoption before
    /// reviewed updates. Fresh empty boards and already initialized boards do not.
    pub fn needs_update_initialization(self) -> bool {
        self.project.board.baseline.is_none() && !self.project.board.footprints().is_empty()
    }

    /// Access the resolved owner for board-local projections within the engine.
    pub(crate) fn project(self) -> &'a Project {
        self.project
    }
    /// Display name of the addressed board; identity never depends on this label.
    pub fn name(self) -> &'a str {
        match &self.project.board.ownership {
            super::identity::Ownership::Explicit(Some(registration)) => &registration.name,
            _ => "Board",
        }
    }

    /// The addressed physical board's setup, including local layers and material definitions.
    pub fn setup(self) -> Option<&'a super::setup::BoardSetup> {
        self.project.board.setup.as_ref()
    }

    /// Clone an existing setup or build a disposable initial draft with the requested copper
    /// count and retained drawing-layer IDs. New IDs may fail to allocate; no document changes.
    pub fn setup_draft(self, copper_layers: u64) -> Result<super::setup::BoardSetup, crate::CommandError> {
        match self.setup() {
            Some(setup) => Ok(setup.clone()),
            None => super::setup::BoardSetup::initial(&self.project.board, copper_layers),
        }
    }

    /// The addressed board's authored physical boundary; absent boards have no inferred edge.
    pub fn outline(self) -> Option<super::outline::Outline> {
        self.project.board.outline
    }

    /// Effective physical silkscreen dimensions; an untouched legacy board uses explicit defaults.
    pub fn silkscreen(self) -> super::silkscreen::Settings {
        self.project.board.silkscreen.unwrap_or_default()
    }

    /// Resolve placements lazily without assuming starter-definition geometry.
    /// Accepted terminal membership is indexed once per traversal, rather than
    /// scanning every net for every pad. The index borrows only this snapshot.
    pub fn footprints(self) -> impl Iterator<Item = FootprintView<'a>> {
        let accepted = self.project.board.baseline.as_ref().map(|baseline| (baseline, baseline.terminal_net_index()));
        self.project.board.footprints().iter().filter_map(move |footprint| {
            let component = self.project.components.get(&footprint.component);
            let historical = self.project.board.baseline.as_ref().and_then(|baseline| baseline.components.get(&footprint.component));
            let reference = component.map(|component| component.reference()).or_else(|| historical.map(|component| component.reference.as_str()))?;
            let value = component.map(|component| component.value()).or_else(|| historical.map(|component| component.value.as_str()))?;
            Some(FootprintView {
                board: self.target,
                id: format!("footprint-{}", footprint.id),
                component: format!("component-{}", footprint.component),
                reference,
                value,
                definition: &footprint.definition,
                position: Point { x_nm: footprint.x_nm, y_nm: footprint.y_nm },
                rotation: footprint.rotation,
                pads: self
                    .project
                    .board
                    .pad_numbers(footprint)
                    .into_iter()
                    .filter_map(|pad| {
                        let center = footprint.pad_position(pad, &self.project.board)?;
                        let definition = self.project.board.definition_for(footprint).and_then(|d| d.pads.iter().find(|p| p.number == pad.as_str()));
                        let copper = definition.map(|p| rectangle_points(footprint, p.center, p.width_nm, p.height_nm, p.rotation));
                        let terminal = footprint.component_pin(pad).and_then(|(component, pin)| self.project.component_terminal(component, pin));
                        let net = if let Some((baseline, index)) = &accepted {
                            footprint
                                .component_pin(pad)
                                .and_then(|(component, pin)| index.get(&(component, pin)).and_then(|index| baseline.net_name(*index)))
                        } else {
                            terminal.and_then(|terminal| net_label(self.project, terminal))
                        };
                        Some(PadView {
                            number: pad.as_str().to_owned(),
                            center,
                            copper,
                            copper_layers: definition.map(|pad| CopperLayers::resolve(self.setup(), pad.copper, footprint.bottom)),
                            shape: definition.map_or(super::definition::PadShape::Rectangle, |p| p.shape),
                            corner_radius_nm: definition.map_or(0, |p| p.corner_radius_nm),
                            drill_nm: definition.and_then(|p| p.drill_nm),
                            through_hole: definition.is_some_and(|p| p.copper == super::definition::CopperBinding::AllCopper),
                            net,
                        })
                    })
                    .collect(),
                graphics: graphics(self.project, footprint, reference),
                physical_reference: self
                    .project
                    .board
                    .definition_for(footprint)
                    .is_some_and(|d| d.reference.is_some() || footprint.reference_text.is_some()),
                reference_offset: footprint.reference_offset.unwrap_or(super::DEFAULT_REFERENCE_OFFSET),
                reference_text: footprint.reference_text.clone(),
                reference_appearance: super::reference::Resolved::new(
                    self.project.board.definition_for(footprint),
                    reference,
                    self.silkscreen(),
                    footprint.reference_text.as_ref(),
                ),
                pending_removal: !self.project.component_is_board_eligible(footprint.component),
                bottom: footprint.bottom,
            })
        })
    }

    /// Project a disposable reference move through the same silk clipping and transform as export.
    /// The view must belong to this board. Editor-only legacy labels have no physical paths.
    pub fn reference_preview(self, view: &FootprintView<'_>, offset: Point) -> Vec<GraphicView> {
        let Some(id) = crate::schematic::wiring::parse_id(&view.id, "footprint-").filter(|_| view.board == self.target) else { return Vec::new() };
        let placements = self.project.board.footprints();
        let Ok(index) = placements.binary_search_by_key(&id, |footprint| footprint.id) else { return Vec::new() };
        let mut placement = placements[index].clone();
        placement.reference_text = view.reference_text.clone();
        reference_graphics(self.project, &placement, view.reference, offset)
    }

    /// Project schematic component selection into this board's placement IDs. Wires and missing
    /// assignments produce no match; reference text and matching object ordinals are not links.
    pub fn footprints_for_selection(self, selection: &Selection) -> BTreeSet<String> {
        let components: BTreeSet<_> =
            self.project.all_symbols().filter(|symbol| selection.symbols.contains(&symbol.id())).map(|symbol| symbol.component).collect();
        self.project
            .board
            .footprints()
            .iter()
            .filter(|footprint| components.contains(&footprint.component))
            .map(|footprint| format!("footprint-{}", footprint.id))
            .collect()
    }

    /// Resolve this board's selected placements to schematic units through their shared component.
    /// The result is a highlight suggestion, not authorization to edit those schematic units.
    pub fn selection_for_footprints(self, footprints: &BTreeSet<String>) -> Selection {
        let components: BTreeSet<_> = self
            .project
            .board
            .footprints()
            .iter()
            .filter(|footprint| footprints.contains(&format!("footprint-{}", footprint.id)))
            .map(|footprint| footprint.component)
            .collect();
        Selection {
            symbols: self.project.all_symbols().filter(|symbol| components.contains(&symbol.component)).map(|symbol| symbol.id()).collect(),
            ..Selection::default()
        }
    }
}

/// Derive a sheet-qualified annotation from the terminal's owning sheet. Project-wide board
/// views must not inherit whichever schematic projection happened to request them.
fn net_label(project: &Project, terminal: crate::schematic::Terminal) -> Option<String> {
    let sheet = project.sheets.iter().find(|sheet| sheet.symbols.binary_search_by_key(&terminal.symbol, |symbol| symbol.ordinal).is_ok())?;
    let projection = project.for_sheet(crate::SheetTarget { project: project.id, sheet: sheet.sheet_id })?;
    let connectivity = projection.connectivity();
    let index = connectivity.net_index_for(terminal)?;
    let name = connectivity.net_name(index);
    Some(if project.sheets.len() > 1 { format!("{}: {name}", sheet.sheet_name) } else { name })
}

/// Resolve saved technical rectangles and physical silk/reference paths without changing the
/// pinned definition. Board settings control actual ink dimensions before viewport projection.
fn graphics(project: &Project, footprint: &super::Footprint, reference: &str) -> Vec<GraphicView> {
    let Some(definition) = project.board.definition_for(footprint) else { return Vec::new() };
    let style = project.board.silkscreen.unwrap_or_default();
    let side = if footprint.bottom { super::setup::Side::Back } else { super::setup::Side::Front };
    let mut result: Vec<_> = definition
        .graphics
        .iter()
        .map(|g| GraphicView {
            purpose: g.purpose,
            side,
            points: rectangle_points(footprint, g.center, g.width_nm, g.height_nm, g.rotation).to_vec(),
            stroke_nm: None,
            closed: true,
            reference: false,
        })
        .collect();
    for artwork in &definition.artwork {
        for mut points in artwork.graphic.paths() {
            if artwork.graphic.closed()
                && points.first() != points.last()
                && let Some(first) = points.first().copied()
            {
                points.push(first);
            }
            if artwork.purpose == super::definition::GraphicPurpose::Silkscreen {
                let style = super::silkscreen::Settings { stroke_width_nm: artwork.graphic.stroke_nm, ..style };
                result.extend(super::silkscreen::clipped(vec![points], &definition.pads, style).into_iter().map(|path| GraphicView {
                    purpose: artwork.purpose,
                    side,
                    points: path.into_iter().map(|p| footprint.point(p)).collect(),
                    stroke_nm: Some(artwork.graphic.stroke_nm),
                    closed: false,
                    reference: false,
                }));
            } else {
                result.push(GraphicView {
                    purpose: artwork.purpose,
                    side,
                    points: points.into_iter().map(|p| footprint.point(p)).collect(),
                    stroke_nm: Some(artwork.graphic.stroke_nm),
                    closed: artwork.graphic.closed(),
                    reference: false,
                });
            }
        }
    }
    let paths: Vec<_> = definition
        .silkscreen
        .iter()
        .map(|path| {
            let mut points = path.points.clone();
            if path.closed {
                points.push(points[0]);
            }
            points
        })
        .collect();
    result.extend(super::silkscreen::clipped(paths, &definition.pads, style).into_iter().map(|path| GraphicView {
        purpose: super::definition::GraphicPurpose::Silkscreen,
        side,
        points: path.into_iter().map(|point| footprint.point(point)).collect(),
        stroke_nm: Some(style.stroke_width_nm),
        closed: false,
        reference: false,
    }));
    result.extend(reference_graphics(project, footprint, reference, footprint.reference_offset.unwrap_or(super::DEFAULT_REFERENCE_OFFSET)));
    result
}

/// Place generated lettering at the automatic anchor plus the library and placement offsets. Clip after moving
/// so the displayed paths and fabrication both use the actual position relative to copper.
fn reference_graphics(project: &Project, footprint: &super::Footprint, reference: &str, offset: Point) -> Vec<GraphicView> {
    let Some(definition) = project.board.definition_for(footprint) else { return Vec::new() };
    let style = project.board.silkscreen.unwrap_or_default();
    let stroke_nm = super::reference::Resolved::new(Some(definition), reference, style, footprint.reference_text.as_ref()).size.stroke_width_nm;
    let side = if footprint.bottom { super::setup::Side::Back } else { super::setup::Side::Front };
    definition
        .reference_paths_with(reference, style, offset, footprint.reference_text.as_ref())
        .into_iter()
        .map(|path| GraphicView {
            purpose: super::definition::GraphicPurpose::Silkscreen,
            side,
            points: path.into_iter().map(|point| footprint.point(point)).collect(),
            stroke_nm: Some(stroke_nm),
            closed: false,
            reference: true,
        })
        .collect()
}

/// Resolve rectangle corners from immutable local geometry, preserving exact odd dimensions
/// before rotation. Pad and graphic orientation share the placement's transform.
fn rectangle_points(footprint: &super::Footprint, center: Point, width: i64, height: i64, rotation: u16) -> [Point; 4] {
    let left = -width / 2;
    let top = -height / 2;
    [(left, top), (left + width, top), (left + width, top + height), (left, top + height)].map(|(x, y)| {
        let (x, y) = super::rotate(x, y, rotation);
        footprint.point(Point { x_nm: center.x_nm + x, y_nm: center.y_nm + y })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Session,
        persistence::{decode_documents, encode_documents},
    };
    use serde_json::json;

    /// Lock the published compatibility derivation so a refactor cannot silently change addresses.
    #[test]
    fn legacy_board_identity_has_a_fixed_compatibility_value() {
        assert_eq!(legacy_board_id(Uuid::from_u128(1)).to_string(), "f42e536f-9ed5-8fdc-bcec-20fab0ad35a9");
    }

    /// Board IDs survive rename, ordinary edits and no-op round trips; scoped views and pads
    /// cannot resolve the same object ordinal in another project.
    #[test]
    fn board_addresses_and_correspondence_survive_edits_and_storage() {
        let mut session = Session::default();
        session.apply("project.new", &json!({"create_board":true,})).unwrap();
        let target = session.project().unwrap().board_target().unwrap();
        session.apply(crate::PLACE_RESISTOR, &json!({"x_nm":0,"y_nm":0,"expected_revision":1})).unwrap();
        let selected = Selection::one_symbol("symbol-2".into());
        assert!(session.project().unwrap().board_view(target).unwrap().footprints_for_selection(&selected).is_empty());
        session.apply(crate::PLACE_FOOTPRINT, &json!({"board":target.board,"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":2,"x_nm":20_000_000,"y_nm":0,"expected_revision":2})).unwrap();
        session.apply("project.rename", &json!({"name":"Renamed"})).unwrap();
        session.apply(crate::SET_SYMBOL_FIELDS, &json!({"symbol":"symbol-2","reference":"R20","value":"22k","expected_revision":4})).unwrap();
        let snapshot = session.snapshot().unwrap();
        let bytes = encode_documents(snapshot.clone()).unwrap();
        let restored = decode_documents(&bytes).unwrap();
        assert_eq!(restored.board_target(), Some(target));
        assert_eq!(encode_documents(restored.clone()).unwrap(), bytes);
        let board = restored.board_view(target).unwrap();
        let ids = board.footprints_for_selection(&selected);
        assert_eq!(ids, BTreeSet::from(["footprint-3".into()]));
        assert_eq!(board.selection_for_footprints(&ids), selected);
        assert_eq!(board.footprints().next().unwrap().reference, "R20");
        assert_eq!(board.footprints().next().unwrap().board, target);
        let pad = restored.pad_for_terminal(restored.symbols()[0].terminals()[0]).unwrap();
        assert_eq!(pad.inspect()["board"], target.board.to_string());
        let before = session.inspect();
        for (command, params) in [
            (crate::PLACE_FOOTPRINT, json!({"symbol":"symbol-2","definition":"generic-through-hole-2","pin_1_pad":1,"x_nm":0,"y_nm":0})),
            (crate::INITIALIZE_BOARD_SETUP, json!({"copper_layers":2})),
            (crate::SET_BOARD_SETUP, json!({"setup":{}})),
        ] {
            let mut params = params;
            params["board"] = json!("00000000-0000-0000-0000-000000000001");
            params["expected_revision"] = json!(session.revision());
            assert!(session.apply(command, &params).unwrap_err().to_string().contains("unknown board"));
            assert_eq!(session.inspect(), before);
        }
        session.apply("project.new", &json!({"create_board":true,})).unwrap();
        let other = session.project().unwrap();
        assert_ne!(other.board_target(), Some(target));
        assert!(other.board_view(target).is_none());
        assert!(other.terminal_for_pad(pad).is_none());
        assert!(other.pad_position(pad).is_none());
    }
    /// Copper UUID membership comes from the stackup, survives catalog rename/reorder, and never
    /// upgrades a legacy marker or invents setup while projecting unconfigured physical pads.
    #[test]
    fn physical_pad_layers_resolve_in_the_core_without_changing_storage() {
        use super::super::{
            definition::CopperBinding,
            setup::{LayerPurpose, Side, Stratum},
        };
        use crate::{INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT, PLACE_SYMBOL, SET_BOARD_SETUP};
        for configured in [false, true] {
            let mut session = Session::default();
            session.apply("project.new", &json!({"create_board":true,})).unwrap();
            if configured {
                session.apply(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":4,"expected_revision":session.revision()})).unwrap();
            }
            for (index, definition) in ["resistor-0402", "header-1x02-254"].into_iter().enumerate() {
                session
                    .apply(
                        PLACE_SYMBOL,
                        &json!({"definition":"resistor","x_nm":index as i64*20000000,"y_nm":0,"expected_revision":session.revision()}),
                    )
                    .unwrap();
                let symbol = session.project().unwrap().symbols().last().unwrap().id();
                session.apply(PLACE_FOOTPRINT,&json!({"symbol":symbol,"definition":definition,"mapping":{"1":"1","2":"2"},"x_nm":index as i64*20000000,"y_nm":0,"expected_revision":session.revision()})).unwrap();
            }
            let before = encode_documents(session.snapshot().unwrap()).unwrap();
            let board = session.project().unwrap().boards().next().unwrap();
            let all: Vec<_> = board
                .setup()
                .into_iter()
                .flat_map(|setup| &setup.stackup().strata)
                .filter_map(|stratum| if let Stratum::Copper { layer, .. } = stratum { Some(*layer) } else { None })
                .collect();
            let front = board
                .setup()
                .and_then(|setup| setup.layers().iter().find(|layer| layer.purpose == LayerPurpose::Copper && layer.side == Some(Side::Front)))
                .map(|layer| layer.id);
            for footprint in board.footprints() {
                let binding = if footprint.definition == "resistor-0402" { CopperBinding::ComponentSide } else { CopperBinding::AllCopper };
                for pad in footprint.pads {
                    if configured {
                        let expected = if binding == CopperBinding::AllCopper { all.clone() } else { vec![front.unwrap()] };
                        assert_eq!(pad.copper_layers, Some(CopperLayers::Resolved { binding, layers: expected }));
                    } else {
                        assert_eq!(pad.copper_layers, Some(CopperLayers::Unconfigured { binding }));
                    }
                }
            }
            assert_eq!(encode_documents(session.snapshot().unwrap()).unwrap(), before);
            if configured {
                let expected: Vec<_> = board.footprints().flat_map(|footprint| footprint.pads.into_iter().map(|pad| pad.copper_layers)).collect();
                let mut setup = serde_json::to_value(board.setup().unwrap()).unwrap();
                setup["layers"].as_array_mut().unwrap().reverse();
                for layer in setup["layers"].as_array_mut().unwrap() {
                    layer["name"] = json!(format!("Renamed {}", layer["name"].as_str().unwrap()));
                }
                session.apply(SET_BOARD_SETUP, &json!({"setup":setup,"expected_revision":session.revision()})).unwrap();
                let actual: Vec<_> = session
                    .project()
                    .unwrap()
                    .boards()
                    .next()
                    .unwrap()
                    .footprints()
                    .flat_map(|footprint| footprint.pads.into_iter().map(|pad| pad.copper_layers))
                    .collect();
                assert_eq!(actual, expected);
                assert!(session.inspect()["project"]["board"]["footprints"][0]["pins"][0]["copper_layers"]["layers"].is_array());
            }
        }
    }
}
