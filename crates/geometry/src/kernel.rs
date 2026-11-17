//! The solidcore implementation of the kernel boundary. Nothing here is public: the crate root
//! converts between these millimetre values and its nanometre types.

use std::collections::{BTreeMap, BTreeSet};

use solidcore_record::step::product::{ItemKind, MAX_VISITS, ProductStructure};
use solidcore_step::write::product::{ExportBody, ExportError, Header};

use crate::guard::guard;
use crate::{ErrorCode, GeometryError, RefusedItem, Result};

/// Written into the STEP header as the originating system.
const ORIGINATING_SYSTEM: &str = "PCBcraft";

/// A solidcore body.
pub(crate) struct KernelBody(solidcore_ops::Body);

/// An axis-aligned block from its least corner and positive sizes, in millimetres.
pub(crate) fn cuboid(origin: [f64; 3], size: [f64; 3]) -> Result<KernelBody> {
    guard("cuboid", || solidcore_ops::make_box(origin, size).map(KernelBody).map_err(|e| ops_error("cuboid", e)))
}

/// The body's exact box in millimetres, least corner first.
pub(crate) fn bounds(body: &KernelBody) -> Result<([f64; 3], [f64; 3])> {
    guard("bounding box", || body.0.bounds().map_err(|e| ops_error("bounding box", e)))
}

/// A copy of `body` moved by the affine `matrix` (rows 0 to 2 the images of x, y and z, row 3 the
/// translation in millimetres).
pub(crate) fn transformed(body: &KernelBody, matrix: &[[f64; 4]; 4]) -> Result<KernelBody> {
    guard("transform", || {
        let copy = body.0.copy().map_err(|e| ops_error("transform", e))?;
        copy.transform(matrix).map_err(|e| ops_error("transform", e))?;
        Ok(KernelBody(copy))
    })
}

/// STEP text of `bodies` as one AP242 part named `part_name`.
pub(crate) fn write(part_name: &str, bodies: &[(&str, &KernelBody)]) -> Result<String> {
    let header = Header { file_name: part_name.to_string(), originating_system: ORIGINATING_SYSTEM.to_string(), ..Header::default() };
    write_with(
        bodies,
        |body| body.0.to_record().map(|(record, _)| record),
        |records| {
            let list =
                records.iter().map(|(name, record)| ExportBody { name: (*name).to_string(), record, color: None, faces: Vec::new() }).collect();
            solidcore_step::write::product::bodies(list, &header).map_err(export_error)
        },
    )
}

/// Makes each body's kernel record with `record`, then writes the records with `emit`, all under
/// one guard. A record that cannot be made keeps its kernel error's code, so a panic the kernel's
/// session caught stays [`ErrorCode::KernelInternal`]. solidcore-step's own `solidcore_bodies`
/// would fold that error into a writer refusal, which reads as `operation_failed`.
pub(crate) fn write_with<B, R>(
    bodies: &[(&str, B)],
    record: impl Fn(&B) -> solidcore_ops::Result<R>,
    emit: impl FnOnce(&[(&str, R)]) -> Result<String>,
) -> Result<String> {
    guard("STEP write", || {
        let mut records = Vec::with_capacity(bodies.len());
        for (name, body) in bodies {
            records.push((*name, record(body).map_err(|e| ops_error("STEP write", e))?));
        }
        emit(&records)
    })
}

/// What a file holds: its named bodies, their placements and the solid items refused.
pub(crate) struct Read {
    pub(crate) bodies: Vec<(String, KernelBody)>,
    /// Each use of a body (index into `bodies`) with its matrix into the world frame, in
    /// millimetres.
    pub(crate) placements: Vec<(usize, [[f64; 4]; 4])>,
    pub(crate) refused: Vec<RefusedItem>,
}

/// Most placements one file's product structure may expand to.
const MAX_PLACEMENTS: u64 = MAX_VISITS as u64;

/// Most work one expansion may do, in steps: ten for each placement the expansion is allowed.
/// [`Expansion`] says what one step is.
const MAX_EXPANSION_WORK: u64 = 10 * MAX_PLACEMENTS;

/// The named bodies of a STEP text, where its product structure places them, and the solid items
/// refused. A panic while one solid is handed to the kernel refuses that solid only. A STEP item
/// the reader lists more than once (a closed shell two surface models share) is one body.
pub(crate) fn read(text: &str) -> Result<Read> {
    guard("STEP read", || {
        let exchange = solidcore_step::p21::parse(text).map_err(|e| GeometryError::new(ErrorCode::ModelUnreadable, String::from(e)))?;
        let solids = solidcore_step::topology::read_solids(&exchange);
        let mut refused: Vec<RefusedItem> =
            solids.failures.into_iter().map(|f| RefusedItem { entity: f.item, code: reader_refusal(&f.message), message: f.message }).collect();
        // The reader lists a surface model's closed shells in the model's reference order and then
        // the model itself, so its list is not always in entity order; the stable sort makes it so.
        let mut solids = solids.solids;
        solids.sort_by_key(|s| s.item);
        let mut bodies = Vec::with_capacity(solids.len());
        let mut body_of_item = BTreeMap::new();
        let mut tried = BTreeSet::new();
        for solid in solids {
            if !tried.insert(solid.item) {
                continue;
            }
            let made = guard("STEP solid", || {
                solidcore_ops::Body::from_record(&solid.brep, false).map(|(body, _)| body).map_err(|e| ops_error("STEP solid", e))
            });
            match made {
                Ok(body) => {
                    body_of_item.insert(solid.item, bodies.len());
                    bodies.push((solid.name, KernelBody(body)));
                }
                Err(e) => {
                    let code = match e.code() {
                        ErrorCode::KernelInternal => ErrorCode::KernelInternal,
                        _ => ErrorCode::ModelUnsupportedEntity,
                    };
                    refused.push(RefusedItem { entity: solid.item, code, message: e.message().to_string() });
                }
            }
        }
        refused.sort_by_key(|r| r.entity);
        refused.dedup_by_key(|r| r.entity);
        let placements = placements(&exchange, &body_of_item, bodies.len())?;
        Ok(Read { bodies, placements, refused })
    })
}

/// Every use of a read body in the world frame: the product structure's, then each body it does
/// not place, by the identity. A refused item's placements are dropped with it. The expansion is
/// counted before it runs and refused past [`MAX_PLACEMENTS`] or [`MAX_EXPANSION_WORK`].
///
/// Which bodies a body item gives is worked out once for each item: `topology::items` reads a
/// surface model's whole shell list on every call, so calling it for each use would cost the
/// list's length again each time the item is placed.
fn placements(exchange: &solidcore_step::p21::Exchange, body_of_item: &BTreeMap<u64, usize>, count: usize) -> Result<Vec<(usize, [[f64; 4]; 4])>> {
    let unreadable = |e: String| GeometryError::new(ErrorCode::ModelUnreadable, format!("assembly structure: {e}"));
    let structure = solidcore_record::step::product::read(exchange).map_err(unreadable)?;
    let mut bodies: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for representation in &structure.representations {
        for item in representation.items.iter().filter(|i| matches!(i.kind, ItemKind::Body(_))) {
            bodies.entry(item.entity).or_insert_with(|| {
                solidcore_step::topology::items(exchange, item.entity).iter().filter_map(|i| body_of_item.get(i).copied()).collect()
            });
        }
    }
    let bodies_of = |entity: u64| bodies.get(&entity).map_or(0, |list| list.len() as u64);
    let cost = Expansion::new(&structure, &bodies_of).total().map_err(unreadable)?;
    if cost.placements > MAX_PLACEMENTS {
        return Err(unreadable(format!("the assembly places more than {MAX_PLACEMENTS} bodies")));
    }
    if cost.work > MAX_EXPANSION_WORK {
        return Err(unreadable(format!("the assembly takes more than {MAX_EXPANSION_WORK} steps to expand")));
    }
    let placed = solidcore_ops::placed_bodies(&structure).map_err(unreadable)?;
    let mut out = Vec::with_capacity(placed.len().max(count));
    let mut seen = vec![false; count];
    for p in placed {
        // A surface model holding closed shells beside open ones is read as one body per closed
        // shell and one for the rest; the model's placement places them all.
        for &body in bodies.get(&p.entity).into_iter().flatten() {
            out.push((body, p.transform));
            if let Some(s) = seen.get_mut(body) {
                *s = true;
            }
        }
    }
    let identity = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    out.extend(seen.iter().enumerate().filter(|(_, s)| !**s).map(|(body, _)| (body, identity)));
    Ok(out)
}

/// What expanding part of a product structure costs: the placements it emits and the work it does.
#[derive(Clone, Copy, Default)]
struct Cost {
    placements: u64,
    work: u64,
}

impl Cost {
    fn plus(self, other: Cost) -> Cost {
        Cost { placements: self.placements.saturating_add(other.placements), work: self.work.saturating_add(other.work) }
    }
}

/// A memoised node of the count: not reached yet, being counted (a cycle if reached again), or
/// counted.
#[derive(Clone, Copy)]
enum Node {
    Unseen,
    Open,
    Done(Cost),
}

/// Counts what `solidcore_ops::placed_bodies` would emit without expanding anything: each
/// definition and representation is counted once and its uses multiply, so a file that places a
/// body 2^20 times through 20 levels of two mapped items costs 20 small sums. It follows the
/// walk's rules: a definition walks every representation its shapes reach through relationships
/// without a transformation, then its occurrences; a representation emits its bodies and walks
/// each mapped item's representation and those related to it, each once for that item.
///
/// A step is one unit of the walk's work, and every loop of the walk at the pinned revision is
/// counted:
/// - before the walk, for every definition in the file, each representation its shapes reach,
///   plus one for each relationship of the file checked against it (the walk scans them all for
///   each one);
/// - each definition visited, plus each representation its shapes reach;
/// - each representation walked, plus each of its items;
/// - each mapped item walked, one for each relationship of the file (the walk scans them all to
///   find the representations related to the mapped one).
struct Expansion<'a> {
    structure: &'a ProductStructure,
    /// The placements one body item gives: the read bodies of its STEP item.
    bodies_of: &'a dyn Fn(u64) -> u64,
    /// Representations related to each without a transformation, in relationship order.
    related: Vec<Vec<usize>>,
    /// Child definitions of each definition, in occurrence order.
    children: Vec<Vec<usize>>,
    definitions: Vec<Node>,
    representations: Vec<Node>,
}

impl<'a> Expansion<'a> {
    /// A count of `structure` not started yet, with each representation's relationships without
    /// a transformation and each definition's children gathered once. An occurrence whose parent
    /// is out of range is skipped, as the walk skips it; a relationship to a representation out
    /// of range is still listed under its other end, so the count reports it as the walk does.
    fn new(structure: &'a ProductStructure, bodies_of: &'a dyn Fn(u64) -> u64) -> Expansion<'a> {
        let mut related = vec![Vec::new(); structure.representations.len()];
        for link in structure.links.iter().filter(|l| l.transformation.is_none()) {
            if let Some(list) = related.get_mut(link.rep_1) {
                list.push(link.rep_2);
            }
            if let Some(list) = related.get_mut(link.rep_2) {
                list.push(link.rep_1);
            }
        }
        let mut children = vec![Vec::new(); structure.definitions.len()];
        for occurrence in &structure.occurrences {
            if let Some(list) = children.get_mut(occurrence.parent) {
                list.push(occurrence.child);
            }
        }
        Expansion {
            structure,
            bodies_of,
            related,
            children,
            definitions: vec![Node::Unseen; structure.definitions.len()],
            representations: vec![Node::Unseen; structure.representations.len()],
        }
    }

    /// The cost of expanding every root, after the representations reached from every
    /// definition are found; the first error any of them meets. The preliminary closure scan
    /// stops after its accumulated work exceeds [`MAX_EXPANSION_WORK`].
    fn total(&mut self) -> std::result::Result<Cost, String> {
        let scan = self.structure.links.len() as u64;
        let mut cost = Cost::default();
        for definition in &self.structure.definitions {
            let reached = self.closure(&definition.shapes).len() as u64;
            cost = cost.plus(Cost { placements: 0, work: reached.saturating_mul(1 + scan) });
            if cost.work > MAX_EXPANSION_WORK {
                return Ok(cost);
            }
        }
        for &root in &self.structure.roots {
            cost = cost.plus(self.definition(root)?);
        }
        Ok(cost)
    }

    /// The cost of expanding `definition` once: one step, one for each representation its
    /// shapes reach, those representations' costs and its occurrences' children's. An error for
    /// a definition met again inside its own expansion.
    fn definition(&mut self, definition: usize) -> std::result::Result<Cost, String> {
        match self.definitions.get(definition) {
            None => return Ok(Cost { placements: 0, work: 1 }),
            Some(Node::Open) => return Err("assembly cycle".into()),
            Some(Node::Done(cost)) => return Ok(*cost),
            Some(Node::Unseen) => {}
        }
        self.definitions[definition] = Node::Open;
        let shapes = self.structure.definitions[definition].shapes.clone();
        let reached = self.closure(&shapes);
        let mut cost = Cost { placements: 0, work: 1 + reached.len() as u64 };
        for representation in reached {
            cost = cost.plus(self.representation(representation)?);
        }
        for child in self.children[definition].clone() {
            cost = cost.plus(self.definition(child)?);
        }
        self.definitions[definition] = Node::Done(cost);
        Ok(cost)
    }

    /// The representations `start` reaches through relationships without a transformation, each
    /// once.
    fn closure(&self, start: &[usize]) -> BTreeSet<usize> {
        let mut seen = BTreeSet::new();
        let mut stack = start.to_vec();
        while let Some(representation) = stack.pop() {
            if seen.insert(representation) {
                stack.extend(self.related.get(representation).into_iter().flatten());
            }
        }
        seen
    }

    /// The cost of walking `representation` once: one step and one for each item, a placement
    /// for each read body of each body item, and for each mapped item one step for each
    /// relationship of the file and the cost of its representation and of those related to it.
    /// An error for a missing representation or map, and for a representation met again inside
    /// its own walk.
    fn representation(&mut self, representation: usize) -> std::result::Result<Cost, String> {
        match self.representations.get(representation) {
            None => return Err(format!("missing representation #{representation}")),
            Some(Node::Open) => return Err("mapped items form a cycle".into()),
            Some(Node::Done(cost)) => return Ok(*cost),
            Some(Node::Unseen) => {}
        }
        self.representations[representation] = Node::Open;
        let structure = self.structure;
        let items = &structure.representations[representation].items;
        let mut cost = Cost { placements: 0, work: 1 + items.len() as u64 };
        for item in items {
            match &item.kind {
                ItemKind::Body(_) => cost = cost.plus(Cost { placements: (self.bodies_of)(item.entity), work: 0 }),
                ItemKind::Mapped { map, .. } => {
                    let Some(map) = structure.maps.get(*map) else {
                        return Err(format!("missing representation map #{}", item.entity));
                    };
                    cost = cost.plus(Cost { placements: 0, work: structure.links.len() as u64 });
                    // The mapped representation, then each related one, each once for this item.
                    let mut walked = BTreeSet::new();
                    let related = self.related.get(map.representation).cloned().unwrap_or_default();
                    for target in std::iter::once(map.representation).chain(related) {
                        if walked.insert(target) {
                            cost = cost.plus(self.representation(target)?);
                        }
                    }
                }
                ItemKind::Other(_) => {}
            }
        }
        self.representations[representation] = Node::Done(cost);
        Ok(cost)
    }
}

/// A deliberate panic inside a solidcore call, as the kernel's session catches it.
pub(crate) fn crash_test() -> Result<()> {
    guard("crash test", || solidcore_ops::session::crash_test().map_err(|e| ops_error("crash test", e)))
}

/// The reader's refusal of a solid as a code: what it does not support yet, or a malformed file.
fn reader_refusal(message: &str) -> ErrorCode {
    if message.contains("not supported yet") { ErrorCode::ModelUnsupportedEntity } else { ErrorCode::ModelUnreadable }
}

/// A solidcore-ops failure as ours: a panic its session caught stays an internal error.
fn ops_error(what: &str, e: solidcore_ops::Error) -> GeometryError {
    match e {
        solidcore_ops::Error::Internal(message) => GeometryError::new(ErrorCode::KernelInternal, format!("{what}: {message}")),
        solidcore_ops::Error::Argument => GeometryError::new(ErrorCode::InvalidParams, format!("{what}: the kernel refused an argument")),
        other => GeometryError::new(ErrorCode::OperationFailed, format!("{what}: {other}")),
    }
}

/// The STEP writer's refusal as ours.
fn export_error(e: ExportError) -> GeometryError {
    let code = match e {
        ExportError::Nothing => ErrorCode::InvalidParams,
        _ => ErrorCode::OperationFailed,
    };
    GeometryError::new(code, format!("STEP write: {e}"))
}
