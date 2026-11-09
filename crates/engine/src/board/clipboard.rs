//! Portable board selections with explicit schematic ownership and destination layer mapping.
use super::{
    Footprint,
    assignment::{Assignment, SchematicPackages},
    drawing::{Drawing, Drawings},
    setup::{Layer, LayerPurpose},
    track::{Track, Via},
};
use crate::{
    CommandError, Project, Session,
    clipboard::{Fragment, PasteOptions},
    schematic::{Point, WireEndpoint},
    selection::Selection,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};
use uuid::Uuid;

mod connectivity;

/// Export a board selection without changing the project or system clipboard.
pub const COPY: &str = "pcb.copy";
/// Export and remove physical objects, retaining their logical component identities.
pub const CUT: &str = "pcb.cut";
/// Paste physical objects and explicitly addressed new schematic counterparts in one transaction.
pub const PASTE: &str = "pcb.paste";
/// Delete a mixed physical selection without deleting its logical components.
pub const DELETE: &str = "pcb.delete_items";

/// One bounded physical selection. Component correspondence highlights are never editing targets.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardSelection {
    #[serde(default)]
    /// Canonical physical placement IDs.
    pub footprints: BTreeSet<String>,
    #[serde(default)]
    /// Board graphic UUIDs.
    pub drawings: BTreeSet<Uuid>,
    #[serde(default)]
    /// Copper track segment UUIDs.
    pub tracks: BTreeSet<Uuid>,
    #[serde(default)]
    /// Through-via UUIDs.
    pub vias: BTreeSet<Uuid>,
}
impl BoardSelection {
    /// Count selected physical objects independently of their owning components.
    pub fn len(&self) -> usize {
        self.footprints.len() + self.drawings.len() + self.tracks.len() + self.vias.len()
    }
    /// Whether the selection contains no physical objects.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Parse canonical bounded arrays, rejecting duplicates rather than silently coalescing them.
    pub fn parse(value: &Value) -> Option<Self> {
        let selection: Self = serde_json::from_value(value.clone()).ok()?;
        for (key, count, limit) in [
            ("footprints", selection.footprints.len(), 4096),
            ("drawings", selection.drawings.len(), 512),
            ("tracks", selection.tracks.len(), super::track::MAX_TRACKS),
            ("vias", selection.vias.len(), super::track::MAX_VIAS),
        ] {
            if count > limit || value.get(key).is_some_and(|v| v.as_array().is_none_or(|a| a.len() != count)) {
                return None;
            }
        }
        if selection.footprints.iter().any(|id| crate::schematic::wiring::parse_id(id, "footprint-").is_none()) {
            return None;
        }
        Some(selection)
    }
}

/// A board paste addresses the sheet and anchor for each copied source circuit explicitly.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchematicDestination {
    /// Existing destination sheet UUID; no sheet is inferred from a name or collection position.
    pub sheet: Uuid,
    /// Anchor for the copied symbols/wiring, independently of their physical board coordinates.
    pub at: Point,
}

/// Options distinguish physical relocation from creating new logical components.
#[derive(Clone, Debug, Default)]
pub struct BoardPasteOptions {
    /// Reuse source components only in their original project when still compatible and unplaced.
    pub reuse_components: bool,
    /// Explicit source-sheet to destination-sheet/anchor mapping for newly created components.
    pub schematic_targets: BTreeMap<Uuid, SchematicDestination>,
    /// Explicit source layer UUID to destination layer UUID mapping; omitted entries resolve only
    /// by an exact compatible identity or one unambiguous purpose/side match.
    pub layers: BTreeMap<Uuid, Uuid>,
    /// Reference and named-net policy; rotation applies to the physical group only.
    pub items: PasteOptions,
}

/// Display metadata for one self-contained source circuit, with identity separate from its name.
#[derive(Clone, Debug)]
pub struct Circuit {
    /// Source sheet identity used by the explicit destination map.
    pub source_sheet: Uuid,
    /// Source sheet name for presentation only.
    pub name: String,
    fragment: Fragment,
}
impl Circuit {
    /// Number of copied symbols, wires and labels, including excluded native power annotations.
    pub fn counts(&self) -> (usize, usize, usize) {
        self.fragment.counts()
    }
    /// Bounds of symbol/pin anchors, wire vertices and label anchors, excluding text glyph extents.
    pub fn bounds(&self) -> [Point; 2] {
        let points = self
            .fragment
            .project
            .symbols()
            .iter()
            .flat_map(|s| std::iter::once(s.position).chain(s.pins()))
            .chain(self.fragment.project.wires().iter().flat_map(|w| self.fragment.project.wire_path(w)))
            .chain(self.fragment.project.net_labels().iter().map(|l| l.at));
        let mut bounds = [self.anchor(); 2];
        for point in points {
            bounds[0].x_nm = bounds[0].x_nm.min(point.x_nm);
            bounds[0].y_nm = bounds[0].y_nm.min(point.y_nm);
            bounds[1].x_nm = bounds[1].x_nm.max(point.x_nm);
            bounds[1].y_nm = bounds[1].y_nm.max(point.y_nm);
        }
        bounds
    }
    /// Original schematic anchor used to preserve the circuit's relative geometry.
    pub fn anchor(&self) -> Point {
        self.fragment.anchor()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CircuitEnvelope {
    sheet: Uuid,
    name: String,
    text: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    format: String,
    kind: String,
    source_project: Uuid,
    source_board: Uuid,
    cut: bool,
    anchor: Point,
    circuits: Vec<CircuitEnvelope>,
    footprints: Vec<Footprint>,
    layers: Vec<Layer>,
    drawings: Vec<Drawing>,
    tracks: Vec<Track>,
    vias: Vec<Via>,
}

/// Immutable, validated clipboard data. No source session, catalog, view or filesystem is retained.
#[derive(Clone, Debug)]
pub struct BoardFragment {
    source_project: Uuid,
    source_board: Uuid,
    cut: bool,
    anchor: Point,
    circuits: Vec<Circuit>,
    footprints: Vec<Footprint>,
    layers: Vec<Layer>,
    drawings: Vec<Drawing>,
    tracks: Vec<Track>,
    vias: Vec<Via>,
}

/// A disposable physical preview caches the logical/package candidate once. Moving its ghost
/// allocates no identities, recomputes no board acceptance and grants no commit authority.
#[derive(Clone, Debug)]
pub struct BoardPastePreview {
    project: Arc<Project>,
    selected: BoardSelection,
    anchor: Point,
}
impl BoardPastePreview {
    /// Identities used only to highlight the transient copied geometry.
    pub fn selection(&self) -> &BoardSelection {
        &self.selected
    }
    /// Move/rotate only copied physical objects in the cached snapshot. Bounds remain checked;
    /// copper short checks and whole-project publication still belong to the paste command.
    pub fn at(&self, at: Point, turns: u8) -> Result<Arc<Project>, CommandError> {
        let mut next = (*self.project).clone();
        let board = Arc::make_mut(&mut next.board);
        for footprint in &mut board.footprints {
            if self.selected.footprints.contains(&format!("footprint-{}", footprint.id)) {
                let point = crate::clipboard::transformed(Point { x_nm: footprint.x_nm, y_nm: footprint.y_nm }, self.anchor, at, turns)?;
                footprint.x_nm = point.x_nm;
                footprint.y_nm = point.y_nm;
                footprint.rotation = (footprint.rotation + u16::from(turns % 4) * 90) % 360;
                if !footprint.valid() {
                    return Err(invalid("pasted footprint exceeds board bounds"));
                }
            }
        }
        if let Some(drawings) = &mut board.drawings {
            for item in &mut drawings.items {
                if self.selected.drawings.contains(&item.id) {
                    item.points = item.points.iter().map(|p| crate::clipboard::transformed(*p, self.anchor, at, turns)).collect::<Result<_, _>>()?;
                }
            }
        }
        for track in &mut board.tracks {
            if self.selected.tracks.contains(&track.id) {
                track.start = crate::clipboard::transformed(track.start, self.anchor, at, turns)?;
                track.end = crate::clipboard::transformed(track.end, self.anchor, at, turns)?;
            }
        }
        for via in &mut board.vias {
            if self.selected.vias.contains(&via.id) {
                via.at = crate::clipboard::transformed(via.at, self.anchor, at, turns)?;
            }
        }
        Ok(Arc::new(next))
    }
}

/// Keep clipboard failures in the shared command error vocabulary.
fn invalid(error: impl ToString) -> CommandError {
    CommandError::InvalidParams(error.to_string())
}

/// Test exact contact with an orthogonal source route, without a screen-space tolerance.
fn on_route(point: Point, route: &[Point]) -> bool {
    route.windows(2).any(|pair| {
        let [a, b] = [pair[0], pair[1]];
        (a.x_nm == b.x_nm && point.x_nm == a.x_nm && (a.y_nm.min(b.y_nm)..=a.y_nm.max(b.y_nm)).contains(&point.y_nm))
            || (a.y_nm == b.y_nm && point.y_nm == a.y_nm && (a.x_nm.min(b.x_nm)..=a.x_nm.max(b.x_nm)).contains(&point.x_nm))
    })
}

/// Retain internal flat wiring between copied components and their named-net boundaries. Prune
/// external terminal/free stubs; retained shared nodes keep topology when capture remaps them.
fn circuit_selection(project: &Project, components: &BTreeSet<u64>) -> Selection {
    let mut selected = Selection {
        symbols: project.symbols().iter().filter(|s| components.contains(&s.component)).map(|s| s.id()).collect(),
        ..Selection::default()
    };
    let mut pins: BTreeSet<_> = project.symbols().iter().filter(|s| components.contains(&s.component)).flat_map(|s| s.terminals()).collect();
    let mut adjacency = BTreeMap::<WireEndpoint, Vec<usize>>::new();
    for (index, wire) in project.wires().iter().enumerate() {
        for end in wire.endpoints() {
            adjacency.entry(end).or_default().push(index);
        }
    }
    let mut reached = BTreeSet::new();
    let mut nodes: BTreeSet<_> = pins.iter().copied().map(WireEndpoint::Terminal).collect();
    let mut queue: VecDeque<_> = nodes.iter().copied().collect();
    while let Some(node) = queue.pop_front() {
        for &index in adjacency.get(&node).into_iter().flatten() {
            if reached.insert(index) {
                for end in project.wires()[index].endpoints() {
                    if nodes.insert(end) {
                        queue.push_back(end);
                    }
                }
            }
        }
    }
    let positions: Vec<_> = pins.iter().filter_map(|pin| project.terminal_position(*pin)).collect();
    for symbol in project
        .symbols()
        .iter()
        .filter(|s| s.definition.power_terminal().is_some() && project.component(s).is_some_and(|c| c.metadata.attributes.exclude_from_board))
    {
        if symbol.terminals().iter().any(|pin| {
            nodes.contains(&WireEndpoint::Terminal(*pin))
                || project.terminal_position(*pin).is_some_and(|point| {
                    positions.contains(&point) || reached.iter().any(|&index| on_route(point, &project.wire_path(&project.wires()[index])))
                })
        }) {
            selected.symbols.insert(symbol.id());
            pins.extend(symbol.terminals());
        }
    }
    let protected: BTreeSet<_> = reached
        .iter()
        .copied()
        .filter(|&index| project.net_labels().iter().any(|label| on_route(label.at, &project.wire_path(&project.wires()[index]))))
        .collect();
    let mut degree: BTreeMap<_, _> =
        nodes.iter().map(|node| (*node, adjacency.get(node).into_iter().flatten().filter(|index| reached.contains(index)).count())).collect();
    let keep = |node: WireEndpoint| matches!(node,WireEndpoint::Terminal(pin) if pins.contains(&pin)) || matches!(node, WireEndpoint::NetLabel(_));
    let mut leaves: VecDeque<_> = degree.iter().filter(|(node, n)| **n == 1 && !keep(**node)).map(|(node, _)| *node).collect();
    while let Some(node) = leaves.pop_front() {
        if degree.get(&node) != Some(&1) || keep(node) {
            continue;
        }
        let edge = adjacency.get(&node).into_iter().flatten().copied().find(|i| reached.contains(i));
        let Some(edge) = edge.filter(|i| !protected.contains(i)) else {
            continue;
        };
        reached.remove(&edge);
        for end in project.wires()[edge].endpoints() {
            if let Some(n) = degree.get_mut(&end) {
                *n = n.saturating_sub(1);
                if *n == 1 && !keep(end) {
                    leaves.push_back(end);
                }
            }
        }
    }
    selected.wires = reached.iter().map(|&index| project.wires()[index].id()).collect();
    selected.net_labels = project
        .net_labels()
        .iter()
        .filter(|label| {
            pins.iter().any(|pin| project.terminal_position(*pin) == Some(label.at))
                || reached.iter().any(|&index| on_route(label.at, &project.wire_path(&project.wires()[index])))
        })
        .map(|label| label.object_id())
        .collect();
    selected
}

impl BoardFragment {
    /// Capture selected physical objects plus their exact placed packages and closed logical
    /// component scope. Circuit fragments retain source sheet identities; no flattening is implicit.
    pub fn capture(project: &Project, selection: &BoardSelection, cut: bool) -> Result<Self, CommandError> {
        let target = project.resolve_board(None)?;
        let view = project.board_view(target).ok_or_else(|| invalid("board unavailable"))?;
        let footprints: Vec<_> =
            project.board.footprints().iter().filter(|f| selection.footprints.contains(&format!("footprint-{}", f.id))).cloned().collect();
        let drawings: Vec<_> = view.drawings().into_iter().filter(|d| selection.drawings.contains(&d.id)).collect();
        let tracks: Vec<_> = view.tracks().iter().filter(|t| selection.tracks.contains(&t.id)).cloned().collect();
        let vias: Vec<_> = view.vias().iter().filter(|v| selection.vias.contains(&v.id)).cloned().collect();
        if selection.is_empty()
            || footprints.len() != selection.footprints.len()
            || drawings.len() != selection.drawings.len()
            || tracks.len() != selection.tracks.len()
            || vias.len() != selection.vias.len()
        {
            return Err(invalid("copy needs existing selected board objects"));
        }
        let components: BTreeSet<_> = footprints.iter().map(|f| f.component).collect();
        if components.iter().any(|id| !project.component_is_board_eligible(*id)) {
            return Err(invalid("a copied footprint must have a live board-eligible schematic component"));
        }
        let mut circuits = Vec::new();
        for sheet in project.sheets() {
            let projection = project.for_sheet(sheet.target).ok_or_else(|| invalid("source sheet unavailable"))?;
            let items = circuit_selection(&projection, &components);
            if items.symbols.is_empty() {
                continue;
            }
            let mut fragment = Fragment::capture(&projection, &items)?;
            let mut packages = SchematicPackages::default();
            for footprint in footprints.iter().filter(|f| fragment.project.components.contains_key(&f.component)) {
                let definition = &fragment.project.components[&footprint.component].definition;
                if !project.board.mapping_valid(footprint, definition) {
                    return Err(invalid("copied footprint no longer matches its schematic terminals; review its package first"));
                }
                if let Some(key) = footprint.key() {
                    packages.definitions.insert(
                        key.clone(),
                        project.board.definitions.get(&key).ok_or_else(|| invalid("missing copied footprint definition"))?.clone(),
                    );
                    packages.assignments.insert(
                        footprint.component,
                        Assignment {
                            component: footprint.component,
                            library: key.0,
                            definition: key.1,
                            definition_revision: key.2,
                            mapping: footprint.mapping.clone().ok_or_else(|| invalid("missing copied pad map"))?,
                        },
                    );
                }
            }
            Arc::make_mut(&mut fragment.project).schematic_packages = Arc::new(packages);
            circuits.push(Circuit { source_sheet: sheet.target.id(), name: sheet.name.into(), fragment });
        }
        connectivity::validate(project, &circuits, &components)?;
        let used: BTreeSet<_> = drawings.iter().map(|d| d.layer).chain(tracks.iter().map(|t| t.layer)).collect();
        let layers = view.layers().into_iter().filter(|l| used.contains(&l.id)).collect();
        let anchor = footprints
            .first()
            .map(|f| Point { x_nm: f.x_nm, y_nm: f.y_nm })
            .or_else(|| drawings.first().and_then(|d| d.points.first().copied()))
            .or_else(|| tracks.first().map(|t| t.start))
            .or_else(|| vias.first().map(|v| v.at))
            .ok_or_else(|| invalid("nothing to copy"))?;
        let result =
            Self { source_project: project.id(), source_board: target.board, cut, anchor, circuits, footprints, layers, drawings, tracks, vias };
        Self::from_text(&result.to_text()?)
    }

    /// Decode untrusted bounded text, using native schematic codecs and the existing physical
    /// geometry validators before any destination is addressed or any identity is allocated.
    pub fn from_text(text: &str) -> Result<Self, CommandError> {
        if text.len() > crate::clipboard::MAX_BYTES {
            return Err(invalid("clipboard exceeds the 8 MiB limit"));
        }
        let envelope: Envelope =
            serde_json::from_str(text).map_err(|_| invalid("Clipboard does not contain a supported PCBcraft board fragment."))?;
        if envelope.format != "pcbcraft-clipboard/1"
            || envelope.kind != "board"
            || envelope.source_project.is_nil()
            || envelope.source_board.is_nil()
            || !crate::wire::valid_point(&envelope.anchor)
        {
            return Err(invalid("unsupported board clipboard format or source address"));
        }
        if envelope.circuits.len() > crate::sheets::MAX_SHEETS
            || envelope.footprints.len() > 4096
            || envelope.layers.len() > 128
            || envelope.drawings.len() > 512
            || envelope.tracks.len() > super::track::MAX_TRACKS
            || envelope.vias.len() > super::track::MAX_VIAS
        {
            return Err(invalid("board clipboard resource limit exceeded"));
        }
        let mut circuits = Vec::new();
        let mut sheets = BTreeSet::new();
        let mut components = BTreeMap::new();
        let mut numeric = BTreeSet::new();
        for circuit in envelope.circuits {
            let fragment = Fragment::from_text(&circuit.text)?;
            if fragment.has_hierarchy() {
                return Err(invalid("board circuit fragments must be flat"));
            }
            if circuit.sheet != fragment.project.sheet_target().id()
                || fragment.project.id() != envelope.source_project
                || !sheets.insert(circuit.sheet)
                || circuit.name.trim() != circuit.name
                || circuit.name.is_empty()
                || circuit.name.chars().count() > 200
                || circuit.name.chars().any(char::is_control)
            {
                return Err(invalid("invalid or duplicate source circuit"));
            }
            for component in fragment.project.components.values() {
                if components.insert(component.ordinal, component.clone()).is_some() {
                    return Err(invalid("component occurs in two clipboard circuits"));
                }
            }
            circuits.push(Circuit { source_sheet: circuit.sheet, name: circuit.name, fragment });
        }
        let mut owners = BTreeSet::new();
        for footprint in &envelope.footprints {
            if footprint.id == 0
                || footprint.id > crate::MAX_OBJECT_ID
                || !numeric.insert(footprint.id)
                || !owners.insert(footprint.component)
                || !footprint.valid()
            {
                return Err(invalid("invalid or duplicate copied footprint"));
            }
            let component = components.get(&footprint.component).ok_or_else(|| invalid("footprint has no copied schematic counterpart"))?;
            if let Some(key) = footprint.key() {
                let assignment = circuits
                    .iter()
                    .find_map(|c| c.fragment.project.schematic_packages.assignments.get(&footprint.component))
                    .ok_or_else(|| invalid("missing copied package assignment"))?;
                if assignment.key() != key || Some(&assignment.mapping) != footprint.mapping.as_ref() {
                    return Err(invalid("copied placement and package assignment differ"));
                }
            } else if !component.definition.fits_two_pad_footprint() {
                return Err(invalid("invalid copied legacy marker"));
            }
        }
        if components.values().any(|component| {
            !owners.contains(&component.ordinal)
                && !(component.definition.power_terminal().is_some() && component.metadata.attributes.exclude_from_board)
        }) {
            return Err(invalid("clipboard has unrelated logical components"));
        }
        for circuit in &circuits {
            let expected = circuit_selection(&circuit.fragment.project, &owners);
            let actual = Selection {
                symbols: circuit.fragment.project.symbols().iter().map(|s| s.id()).collect(),
                wires: circuit.fragment.project.wires().iter().map(|w| w.id()).collect(),
                net_labels: circuit.fragment.project.net_labels().iter().map(|l| l.object_id()).collect(),
            };
            if expected != actual {
                return Err(invalid("copied circuit contains objects outside the selected footprint scope"));
            }
        }
        let mut layers = BTreeMap::new();
        let mut uuids = BTreeSet::new();
        for layer in &envelope.layers {
            if layer.id.is_nil()
                || layers.insert(layer.id, layer).is_some()
                || layer.name.trim() != layer.name
                || layer.name.is_empty()
                || layer.name.chars().count() > 80
                || layer.name.chars().any(char::is_control)
                || !matches!(layer.purpose, LayerPurpose::Copper | LayerPurpose::Outline)
                || (layer.purpose == LayerPurpose::Outline && layer.side.is_some())
            {
                return Err(invalid("invalid or duplicate clipboard layer"));
            }
        }
        let mut used = BTreeSet::new();
        for drawing in &envelope.drawings {
            drawing.validate().map_err(invalid)?;
            if !uuids.insert(drawing.id) || !layers.get(&drawing.layer).is_some_and(|l| l.purpose == LayerPurpose::Outline) {
                return Err(invalid("invalid copied drawing layer/identity"));
            }
            used.insert(drawing.layer);
        }
        for track in &envelope.tracks {
            track.validate().map_err(invalid)?;
            if !uuids.insert(track.id) || !layers.get(&track.layer).is_some_and(|l| l.purpose == LayerPurpose::Copper) {
                return Err(invalid("invalid copied track layer/identity"));
            }
            used.insert(track.layer);
        }
        for via in &envelope.vias {
            via.validate().map_err(invalid)?;
            if !uuids.insert(via.id) {
                return Err(invalid("duplicate copied via"));
            }
        }
        if used.len() != layers.len()
            || envelope.footprints.is_empty() && envelope.drawings.is_empty() && envelope.tracks.is_empty() && envelope.vias.is_empty()
        {
            return Err(invalid("empty board fragment or unused source layers"));
        }
        Ok(Self {
            source_project: envelope.source_project,
            source_board: envelope.source_board,
            cut: envelope.cut,
            anchor: envelope.anchor,
            circuits,
            footprints: envelope.footprints,
            layers: envelope.layers,
            drawings: envelope.drawings,
            tracks: envelope.tracks,
            vias: envelope.vias,
        })
    }

    /// Encode exact packages, geometry and circuit fragments without source files or board setup.
    pub fn to_text(&self) -> Result<String, CommandError> {
        let circuits = self
            .circuits
            .iter()
            .map(|c| Ok(CircuitEnvelope { sheet: c.source_sheet, name: c.name.clone(), text: c.fragment.to_text()? }))
            .collect::<Result<_, CommandError>>()?;
        let text = serde_json::to_string(&Envelope {
            format: "pcbcraft-clipboard/1".into(),
            kind: "board".into(),
            source_project: self.source_project,
            source_board: self.source_board,
            cut: self.cut,
            anchor: self.anchor,
            circuits,
            footprints: self.footprints.clone(),
            layers: self.layers.clone(),
            drawings: self.drawings.clone(),
            tracks: self.tracks.clone(),
            vias: self.vias.clone(),
        })
        .map_err(invalid)?;
        if text.len() > crate::clipboard::MAX_BYTES {
            return Err(invalid("clipboard exceeds the 8 MiB limit"));
        }
        Ok(text)
    }
    /// Source project identity qualifies component reuse independently of paths or names.
    pub fn source_project(&self) -> Uuid {
        self.source_project
    }
    /// Original physical anchor for Paste at Original Position.
    pub fn anchor(&self) -> Point {
        self.anchor
    }
    /// Counts of copied footprints, drawings, tracks and through vias for a paste review.
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        (self.footprints.len(), self.drawings.len(), self.tracks.len(), self.vias.len())
    }
    /// Source circuits require explicit schematic destinations when duplicating components.
    pub fn circuits(&self) -> &[Circuit] {
        &self.circuits
    }
    /// Source layer identities and roles; names are presentation metadata only.
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }
    /// Whether this cut originated in the current project; compatibility/unplaced checks still run.
    pub fn can_reuse_in(&self, project: &Project) -> bool {
        self.cut && self.source_project == project.id() && project.board_target().is_some_and(|b| b.board == self.source_board)
    }

    /// Default to compatible exact identities or an unambiguous purpose/side match. Explicit
    /// mappings preserve purpose but may change copper side; ambiguous inner layers require a
    /// choice. Layer names never determine identity.
    pub fn layer_mapping(&self, project: &Project, explicit: &BTreeMap<Uuid, Uuid>) -> Result<BTreeMap<Uuid, Uuid>, CommandError> {
        let view = project.board_view(project.resolve_board(None)?).ok_or_else(|| invalid("board unavailable"))?;
        let destination = view.layers();
        if explicit.keys().any(|id| !self.layers.iter().any(|l| l.id == *id)) {
            return Err(invalid("layer mapping contains an unknown source layer"));
        }
        let mut result = BTreeMap::new();
        for source in &self.layers {
            let compatible = |layer: &&Layer| layer.purpose == source.purpose && layer.side == source.side;
            let target = if let Some(id) = explicit.get(&source.id) {
                destination.iter().find(|l| l.id == *id && l.purpose == source.purpose)
            } else if let Some(same) = destination.iter().find(|l| l.id == source.id).filter(compatible) {
                Some(same)
            } else {
                let mut matches = destination.iter().filter(compatible);
                let first = matches.next();
                if matches.next().is_some() { None } else { first }
            };
            let target = target.ok_or_else(|| invalid(format!("Choose a compatible destination for layer {}", source.name)))?;
            result.insert(source.id, target.id);
        }
        Ok(result)
    }

    /// Build the complete physical/logical candidate without changing either session. New
    /// counterparts use explicit sheet targets; reuse preserves the existing logical components.
    pub fn placed(
        &self,
        destination: &Project,
        at: Point,
        first_id: u64,
        options: &BoardPasteOptions,
    ) -> Result<(Project, BoardSelection), CommandError> {
        self.build(destination, at, first_id, options, true)
    }

    /// Prepare logical counterparts/package acceptance once for a disposable cursor ghost.
    /// Copper overlap is checked by the authoritative commit, not while the ghost crosses a route.
    pub fn preview(&self, destination: &Project, first_id: u64, options: &BoardPasteOptions) -> Result<BoardPastePreview, CommandError> {
        let mut options = options.clone();
        options.items.quarter_turns = 0;
        let (project, selected) = self.build(destination, self.anchor, first_id, &options, false)?;
        Ok(BoardPastePreview { project: Arc::new(project), selected, anchor: self.anchor })
    }

    /// Construct one complete candidate, optionally deferring the expensive copper check for a
    /// preview. Preview snapshots cannot be submitted as project mutations by a caller.
    fn build(
        &self,
        destination: &Project,
        at: Point,
        first_id: u64,
        options: &BoardPasteOptions,
        check_copper: bool,
    ) -> Result<(Project, BoardSelection), CommandError> {
        let target = destination.resolve_board(None)?;
        let mapping = self.layer_mapping(destination, &options.layers)?;
        let mut next = destination.clone();
        let mut components = BTreeMap::new();
        let mut cursor = first_id.max(destination.allocation_high_water().saturating_add(1));
        if options.reuse_components {
            if self.source_project != destination.id() || self.source_board != target.board {
                return Err(invalid("source components can only be reused on their original project and board"));
            }
            if !options.schematic_targets.is_empty() || !options.items.net_name_suffix.is_empty() {
                return Err(invalid("component reuse cannot change schematic targets or net names"));
            }
            for source in &self.footprints {
                let component = destination
                    .components
                    .get(&source.component)
                    .ok_or_else(|| invalid("a source component no longer exists; duplicate the fragment instead"))?;
                if !destination.component_is_board_eligible(source.component)
                    || destination.board.by_component.contains_key(&source.component)
                    || !destination.board.mapping_valid(source, &component.definition)
                {
                    return Err(invalid("source components must be compatible, board-eligible and unplaced; choose duplication to create new parts"));
                }
                if let Some(assignment) = destination.board.assignments.get(&source.component)
                    && (source.key() != Some(assignment.key()) || source.mapping.as_ref() != Some(&assignment.mapping))
                {
                    return Err(invalid("source component package intent changed; review the PCB update first"));
                }
                components.insert(source.component, source.component);
            }
        } else if !self.footprints.is_empty() {
            if options.schematic_targets.len() != self.circuits.len()
                || self.circuits.iter().any(|c| !options.schematic_targets.contains_key(&c.source_sheet))
            {
                return Err(invalid("choose a destination sheet and schematic anchor for every copied circuit"));
            }
            if let Some(accepted) = &destination.board.baseline {
                let current =
                    super::update::Baseline::proposed(destination, &super::update::Baseline::current_choices(destination), accepted.source.clone())
                        .map_err(invalid)?;
                if !super::update::preview(accepted, &current).map_err(invalid)?.is_noop() {
                    return Err(invalid("Review the destination's pending PCB update before duplicating footprints."));
                }
            }
            for circuit in &self.circuits {
                let placement = &options.schematic_targets[&circuit.source_sheet];
                let sheet = next.resolve_sheet(Some(&placement.sheet.to_string()))?;
                let projection = next.for_sheet(sheet).ok_or_else(|| invalid("destination sheet unavailable"))?;
                let mut policy = options.items.clone();
                policy.quarter_turns = 0;
                let before = projection.symbols().len();
                let (candidate, _) = circuit.fragment.placed(&projection, placement.at, cursor, &policy)?;
                for (source, copy) in circuit.fragment.project.symbols().iter().zip(&candidate.symbols()[before..]) {
                    components.insert(source.component, copy.component);
                }
                cursor = candidate.allocation_high_water().checked_add(1).ok_or_else(|| invalid("clipboard exhausts object IDs"))?;
                next = candidate;
            }
            next.sheet_index = 0;
            if let Some(accepted) = destination.board.baseline.as_ref() {
                if self.footprints.iter().any(|f| f.library.is_none()) {
                    return Err(invalid("assign physical native packages before duplicating legacy markers into an accepted board"));
                }
                let source = super::update::capture_source(&next)?;
                let proposed = super::update::Baseline::proposed(&next, &super::update::Baseline::current_choices(&next), source).map_err(invalid)?;
                next = super::update::apply_candidate(&next, accepted, &proposed).map_err(invalid)?.0;
            }
        } else if !options.schematic_targets.is_empty() {
            return Err(invalid("geometry-only paste does not have schematic destinations"));
        }
        let mut selected = BoardSelection::default();
        for source in &self.footprints {
            let mut copy = source.clone();
            copy.id = crate::clipboard::allocate(&mut cursor)?;
            copy.component = components[&source.component];
            let point = crate::clipboard::transformed(Point { x_nm: source.x_nm, y_nm: source.y_nm }, self.anchor, at, options.items.quarter_turns)?;
            copy.x_nm = point.x_nm;
            copy.y_nm = point.y_nm;
            copy.rotation = (copy.rotation + u16::from(options.items.quarter_turns % 4) * 90) % 360;
            selected.footprints.insert(format!("footprint-{}", copy.id));
            Arc::make_mut(&mut next.board).append(copy).map_err(invalid)?;
        }
        if !self.drawings.is_empty() {
            let view = next.board_view(target).ok_or_else(|| invalid("destination board unavailable"))?;
            let mut data = next.board.drawings.clone().unwrap_or_else(|| Drawings {
                layers: view
                    .layers()
                    .into_iter()
                    .filter(|l| {
                        view.drawings().iter().any(|d| d.layer == l.id) && !view.setup().is_some_and(|s| s.layers().iter().any(|x| x.id == l.id))
                    })
                    .collect(),
                items: view.drawings(),
            });
            for source in &self.drawings {
                let layer = mapping[&source.layer];
                if !view.setup().is_some_and(|s| s.layers().iter().any(|l| l.id == layer)) && !data.layers.iter().any(|l| l.id == layer) {
                    data.layers
                        .push(view.layers().into_iter().find(|l| l.id == layer).ok_or_else(|| invalid("destination outline layer unavailable"))?);
                }
                let mut copy = source.clone();
                copy.id = crate::fresh_id()?;
                copy.layer = layer;
                copy.points = source
                    .points
                    .iter()
                    .map(|p| crate::clipboard::transformed(*p, self.anchor, at, options.items.quarter_turns))
                    .collect::<Result<_, _>>()?;
                selected.drawings.insert(copy.id);
                data.items.push(copy);
            }
            data.validate(&next).map_err(invalid)?;
            let board = Arc::make_mut(&mut next.board);
            board.outline = None;
            board.drawings = Some(data);
        }
        for source in &self.tracks {
            let mut copy = source.clone();
            copy.id = crate::fresh_id()?;
            copy.layer = mapping[&source.layer];
            copy.start = crate::clipboard::transformed(source.start, self.anchor, at, options.items.quarter_turns)?;
            copy.end = crate::clipboard::transformed(source.end, self.anchor, at, options.items.quarter_turns)?;
            selected.tracks.insert(copy.id);
            Arc::make_mut(&mut next.board).tracks.push(copy);
        }
        for source in &self.vias {
            let mut copy = source.clone();
            copy.id = crate::fresh_id()?;
            copy.at = crate::clipboard::transformed(source.at, self.anchor, at, options.items.quarter_turns)?;
            selected.vias.insert(copy.id);
            Arc::make_mut(&mut next.board).vias.push(copy);
        }
        let mut tracks = next.board.tracks.clone();
        let mut vias = next.board.vias.clone();
        super::track::validate(&next, &mut tracks, &mut vias).map_err(invalid)?;
        {
            let board = Arc::make_mut(&mut next.board);
            board.tracks = tracks;
            board.vias = vias;
        }
        let added = selected.tracks.union(&selected.vias).copied().collect();
        if check_copper && let Some((a, b)) = super::track::short_for_paste(&next, &added, &selected.footprints) {
            return Err(invalid(format!("pasted copper would connect nets {a} and {b}")));
        }
        if !self.footprints.is_empty() {
            next.numeric_id_high_water = Some(next.allocation_high_water().max(cursor.saturating_sub(1)));
        }
        Ok((next, selected))
    }
}

impl Session {
    /// Export a revision-bound, explicitly addressed physical selection through GUI/MCP commands.
    pub(crate) fn copy_board(&self, params: &Value, cut: bool) -> Result<String, CommandError> {
        crate::check_params(crate::command(COPY).ok_or_else(|| invalid("copy command unavailable"))?, params)?;
        self.check_revision(params)?;
        let project = self.open_project()?;
        project.resolve_board(params.get("board").and_then(Value::as_str))?;
        let selection = BoardSelection::parse(&params["items"]).ok_or_else(|| invalid("invalid board selection"))?;
        BoardFragment::capture(project, &selection, cut)?.to_text()
    }
    /// Remove a mixed physical selection atomically, leaving its logical components and accepted
    /// package history available for relocation. Inner command histories are never published.
    pub(crate) fn delete_board_items(&mut self, params: &Value, cut: bool) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        let target = project.resolve_board(params.get("board").and_then(Value::as_str))?;
        let selection = BoardSelection::parse(&params["items"]).ok_or_else(|| invalid("invalid board selection"))?;
        if selection.is_empty() {
            return Err(invalid("select physical objects to delete"));
        }
        if cut {
            BoardFragment::capture(project, &selection, true)?;
        }
        let mut transaction = Self {
            libraries: self.libraries.clone(),
            project: self.snapshot(),
            revision: self.revision,
            history_root_revision: self.history_root_revision,
            ..Self::default()
        };
        if !selection.footprints.is_empty() {
            transaction.apply(
                crate::DELETE_FOOTPRINTS,
                &json!({"board":target.board,"footprints":selection.footprints,"expected_revision":transaction.revision()}),
            )?;
        }
        if !selection.tracks.is_empty() || !selection.vias.is_empty() {
            let mut args = json!({"board":target.board,"expected_revision":transaction.revision()});
            if !selection.tracks.is_empty() {
                args["tracks"] = json!(selection.tracks);
            }
            if !selection.vias.is_empty() {
                args["vias"] = json!(selection.vias);
            }
            transaction.apply(crate::DELETE_TRACKS, &args)?;
        }
        for drawing in selection.drawings {
            transaction.apply(crate::DELETE_DRAWING, &json!({"board":target.board,"drawing":drawing,"expected_revision":transaction.revision()}))?;
        }
        let next = transaction.open_project()?.clone();
        crate::clipboard::validate_candidate(&next)?;
        self.change(next);
        Ok(())
    }
    /// Revalidate a complete paste at the captured revision and publish one electrical/history
    /// transaction. No unrelated pending board update can be accepted by duplication.
    pub(crate) fn paste_board(&mut self, params: &Value) -> Result<(), CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        project.resolve_board(params.get("board").and_then(Value::as_str))?;
        let fragment = BoardFragment::from_text(params["text"].as_str().unwrap_or_default())?;
        let options = BoardPasteOptions {
            reuse_components: params["reuse_components"].as_bool().unwrap_or(false),
            schematic_targets: params
                .get("schematic_targets")
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()
                .map_err(invalid)?
                .unwrap_or_default(),
            layers: params.get("layers").map(|v| serde_json::from_value(v.clone())).transpose().map_err(invalid)?.unwrap_or_default(),
            items: PasteOptions {
                preserve_references: params["references"].as_str() == Some("preserve"),
                net_name_suffix: params["net_name_suffix"].as_str().unwrap_or_default().into(),
                quarter_turns: (params["rotation"].as_u64().unwrap_or(0) / 90) as u8,
                reuse_sheets: false,
            },
        };
        let at = Point { x_nm: params["x_nm"].as_i64().unwrap_or_default(), y_nm: params["y_nm"].as_i64().unwrap_or_default() };
        let (next, _) = fragment.placed(project, at, self.next_object_id()?, &options)?;
        crate::clipboard::validate_candidate(&next)?;
        self.change(next);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
