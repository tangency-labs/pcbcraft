//! Merge collinear and stacked duplicate track segments of existing copper.
//!
//! Only the addressed, unlocked segments take part. Two of them on the same layer with the
//! same width become one when they lie on one straight line and either share an end that no
//! other copper uses (no third track end, via or pad there) or one covers the other. The
//! merged copper outline is the same as before, so connectivity does not change. The segment
//! with the lowest UUID survives with the merged ends; the others are deleted. The whole edit
//! is one undo step, and merging an already merged selection changes nothing.
use super::*;
use crate::board::{geometry::continues_straight, view::BoardTarget};
use std::cell::OnceCell;

/// A track end on one copper layer, in board nanometres.
type End = (Uuid, i64, i64);

/// The line a segment lies on, with its layer and width: the primitive direction (pointing
/// right, or down when vertical) and the offset across that direction.
type Line = (Uuid, i64, i64, i64, i128);

/// The planned result for one selection. Nothing changes until it is published.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    /// Original records of every segment the merge deletes or reshapes, in UUID order.
    before: Vec<Track>,
    /// Final records of every surviving segment whose ends change, in UUID order.
    after: Vec<Track>,
    /// Segments absorbed into a survivor, in UUID order.
    removed: Vec<Uuid>,
    /// Addressed segments left alone because they are locked, in UUID order.
    locked: Vec<Uuid>,
}

impl Plan {
    /// Whether publishing the plan would change the board.
    fn changed(&self) -> bool {
        !self.removed.is_empty() || !self.after.is_empty()
    }

    /// The full diff returned to command callers, for a dry run and for an applied merge.
    fn report(&self, board: Uuid, dry_run: bool) -> Value {
        json!({
            "board": board,
            "before": self.before.iter().map(Track::inspect).collect::<Vec<_>>(),
            "after": self.after.iter().map(Track::inspect).collect::<Vec<_>>(),
            "removed": self.removed,
            "reshaped": self.after.iter().map(|track| track.id).collect::<Vec<_>>(),
            "locked": self.locked,
            "changed": self.changed(),
            "dry_run": dry_run,
        })
    }
}

/// One straight run of merged copper on a line, measured along the line's direction.
#[derive(Clone, Debug)]
struct Span {
    /// The surviving segment: the lowest UUID of all segments merged into this run.
    id: Uuid,
    /// Whether the survivor's original start lies at the low end of the run.
    forward: bool,
    /// Positions of `low` and `high` along the line, with `low_t < high_t`.
    low_t: i128,
    high_t: i128,
    low: Point,
    high: Point,
}

impl Span {
    /// The survivor's record with the run's ends, keeping its original direction.
    fn track(&self, original: &Track) -> Track {
        let (start, end) = if self.forward { (self.low, self.high) } else { (self.high, self.low) };
        Track { start, end, ..original.clone() }
    }
}

/// Copper that is not a track: vias on every layer and pads on their own layers. Each list is
/// sorted by left edge and keeps its widest item, so a point query visits only nearby copper.
struct Obstacles {
    vias: Vec<([f64; 4], PadContour)>,
    via_width: f64,
    pads: Vec<([f64; 4], PadContour, Vec<Uuid>)>,
    pad_width: f64,
}

impl Obstacles {
    /// Capture the board's via and pad copper once per plan.
    fn new(view: BoardView<'_>) -> Self {
        let mut vias: Vec<_> = view
            .vias()
            .iter()
            .map(|via| {
                let contour = PadContour::capsule(via.at, via.at, via.diameter_nm as f64 / 2.);
                (contour.bounds(), contour)
            })
            .collect();
        vias.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
        let mut pads: Vec<_> = lands(view)
            .into_iter()
            .map(|land| {
                let contour = land.contour();
                (contour.bounds(), contour, land.layers)
            })
            .collect();
        pads.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
        let via_width = vias.iter().map(|v| v.0[2] - v.0[0]).fold(0., f64::max);
        let pad_width = pads.iter().map(|p| p.0[2] - p.0[0]).fold(0., f64::max);
        Self { vias, via_width, pads, pad_width }
    }

    /// Whether a via, or a pad on `layer`, touches the round end of a `width_nm` track at `at`.
    fn touch(&self, at: Point, layer: Uuid, width_nm: i64) -> bool {
        let radius = width_nm as f64 / 2.;
        let (x, y) = (at.x_nm as f64, at.y_nm as f64);
        // Only items whose left edge lies within [x - radius - widest, x + radius] can reach.
        let reaches = |b: &[f64; 4]| b[0] <= x + radius && b[2] >= x - radius && b[1] <= y + radius && b[3] >= y - radius;
        let first = self.vias.partition_point(|v| v.0[0] < x - radius - self.via_width);
        let via = self.vias[first..]
            .iter()
            .take_while(|(bounds, _)| bounds[0] <= x + radius)
            .any(|(bounds, contour)| reaches(bounds) && contour.overlaps_segment(at, at, radius));
        let first = self.pads.partition_point(|p| p.0[0] < x - radius - self.pad_width);
        via || self.pads[first..]
            .iter()
            .take_while(|(bounds, ..)| bounds[0] <= x + radius)
            .any(|(bounds, contour, layers)| layers.contains(&layer) && reaches(bounds) && contour.overlaps_segment(at, at, radius))
    }
}

/// Greatest common divisor of two non-negative integers; zero only when both are zero.
fn gcd(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// The line key of a validated (non-zero-length) segment and its ends' positions along it.
fn line(track: &Track) -> (Line, i128, i128) {
    let (dx, dy) = (track.end.x_nm - track.start.x_nm, track.end.y_nm - track.start.y_nm);
    let divisor = gcd(dx.abs(), dy.abs()).max(1);
    let (mut ux, mut uy) = (dx / divisor, dy / divisor);
    if ux < 0 || (ux == 0 && uy < 0) {
        (ux, uy) = (-ux, -uy);
    }
    let along = |p: Point| i128::from(ux) * i128::from(p.x_nm) + i128::from(uy) * i128::from(p.y_nm);
    let offset = i128::from(ux) * i128::from(track.start.y_nm) - i128::from(uy) * i128::from(track.start.x_nm);
    ((track.layer, track.width_nm, ux, uy, offset), along(track.start), along(track.end))
}

/// Plan the merge of the addressed segments on one board snapshot without changing it.
/// `selected` holds existing track UUIDs; locked ones are reported and left alone. Track ends
/// are counted over the whole board, so unaddressed and locked copper still blocks a join.
pub(crate) fn plan(view: BoardView<'_>, selected: &BTreeSet<Uuid>) -> Plan {
    let tracks = view.tracks();
    let mut ends: BTreeMap<End, usize> = BTreeMap::new();
    let mut lines: BTreeMap<Line, Vec<Span>> = BTreeMap::new();
    let mut locked = Vec::new();
    for track in tracks {
        for end in [track.start, track.end] {
            *ends.entry((track.layer, end.x_nm, end.y_nm)).or_default() += 1;
        }
        if !selected.contains(&track.id) {
            continue;
        }
        if track.locked {
            locked.push(track.id);
            continue;
        }
        let (key, start_t, end_t) = line(track);
        let forward = start_t < end_t;
        let (low, high) = if forward { (track.start, track.end) } else { (track.end, track.start) };
        lines.entry(key).or_default().push(Span { id: track.id, forward, low_t: start_t.min(end_t), high_t: start_t.max(end_t), low, high });
    }
    let obstacles = OnceCell::new();
    let mut removed = BTreeSet::new();
    let mut survivors = Vec::new();
    for ((layer, width_nm, ..), mut spans) in lines {
        if spans.len() > 1 {
            let join = Join { layer, width_nm, view, obstacles: &obstacles };
            join.merge_line(&mut spans, &mut ends, &mut removed);
        }
        survivors.extend(spans);
    }
    let original = move |id: &Uuid| tracks.binary_search_by_key(id, |track| track.id).ok().map(|index| &tracks[index]);
    let mut plan = Plan { locked, removed: removed.into_iter().collect(), ..Plan::default() };
    for span in survivors {
        let Some(track) = original(&span.id) else { continue };
        let merged = span.track(track);
        if merged != *track {
            plan.before.push(track.clone());
            plan.after.push(merged);
        }
    }
    plan.before.extend(plan.removed.iter().filter_map(original).cloned());
    plan.before.sort_by_key(|track| track.id);
    plan.after.sort_by_key(|track| track.id);
    plan
}

/// The layer, width and board copper shared by every span of one line.
struct Join<'a> {
    layer: Uuid,
    width_nm: i64,
    view: BoardView<'a>,
    /// Via and pad copper, captured only when a join first needs it.
    obstacles: &'a OnceCell<Obstacles>,
}

impl Join<'_> {
    /// Merge the spans of one line until none can merge. Each pass sweeps the spans in order
    /// along the line; the last kept span always reaches furthest. A span inside it is
    /// absorbed, and a span starting exactly at its far end is joined when that end is free
    /// (see [`Self::free_end`]). Every merge removes a span, so the passes end; a later pass
    /// only adds the joins that duplicates absorbed in an earlier pass were blocking.
    fn merge_line(&self, spans: &mut Vec<Span>, ends: &mut BTreeMap<End, usize>, removed: &mut BTreeSet<Uuid>) {
        loop {
            spans.sort_by(|a, b| a.low_t.cmp(&b.low_t).then(b.high_t.cmp(&a.high_t)).then(a.id.cmp(&b.id)));
            let mut merged = false;
            let mut kept: Vec<Span> = Vec::with_capacity(spans.len());
            for span in spans.drain(..) {
                if let Some(reach) = kept.last_mut()
                    && (span.high_t <= reach.high_t
                        || (span.low_t == reach.high_t && continues_straight(reach.low, reach.high, span.high) && self.free_end(reach.high, ends)))
                {
                    self.absorb(reach, span, ends, removed);
                    merged = true;
                    continue;
                }
                kept.push(span);
            }
            *spans = kept;
            if !merged {
                return;
            }
        }
    }

    /// Whether exactly two track ends meet at `at` on this layer and no via or pad copper
    /// touches the round track end there, so joining the two segments hides no junction.
    fn free_end(&self, at: Point, ends: &BTreeMap<End, usize>) -> bool {
        ends.get(&(self.layer, at.x_nm, at.y_nm)) == Some(&2)
            && !self.obstacles.get_or_init(|| Obstacles::new(self.view)).touch(at, self.layer, self.width_nm)
    }

    /// Fold `other` into `into`: the run covers both, the lower UUID survives, and the track
    /// end counts follow the copper that remains.
    fn absorb(&self, into: &mut Span, other: Span, ends: &mut BTreeMap<End, usize>, removed: &mut BTreeSet<Uuid>) {
        for end in [into.low, into.high, other.low, other.high] {
            if let Some(count) = ends.get_mut(&(self.layer, end.x_nm, end.y_nm)) {
                *count = count.saturating_sub(1);
            }
        }
        if other.low_t < into.low_t {
            (into.low_t, into.low) = (other.low_t, other.low);
        }
        if other.high_t > into.high_t {
            (into.high_t, into.high) = (other.high_t, other.high);
        }
        for end in [into.low, into.high] {
            *ends.entry((self.layer, end.x_nm, end.y_nm)).or_default() += 1;
        }
        if other.id < into.id {
            removed.insert(into.id);
            (into.id, into.forward) = (other.id, other.forward);
        } else {
            removed.insert(other.id);
        }
    }
}

impl Session {
    /// Merge the addressed segments, or only report the merge on a dry run. The result lists
    /// the original and final records of every changed segment, the deleted and reshaped
    /// UUIDs, the locked segments left alone and whether anything changed. Applying deletes
    /// the absorbed segments and reshapes the survivors through the shared track commands in
    /// one private transaction, so their validation, lock protection and net-short refusal
    /// apply, then publishes one undo step. With nothing to merge, revision and history stay
    /// unchanged.
    pub(crate) fn merge_tracks(&mut self, params: &Value) -> Result<Value, CommandError> {
        self.validate_project_command(MERGE_TRACKS, params)?;
        self.check_revision(params)?;
        let project = self.open_project()?;
        let target = project.resolve_board(params.get("board").and_then(Value::as_str))?;
        let view = project.board_view(target).ok_or_else(|| CommandError::Unavailable("Board unavailable.".into()))?;
        let selected = ids(params, "tracks", |id| view.tracks().binary_search_by_key(id, |track| track.id).is_ok())?;
        if selected.is_empty() {
            return Err(CommandError::InvalidParams("name at least one track to merge".into()));
        }
        let plan = plan(view, &selected);
        let dry_run = params["dry_run"].as_bool().unwrap_or(false);
        if plan.changed() && !dry_run {
            self.publish_merge(target, &plan)?;
        }
        Ok(json!({"revision": self.revision(), "merge_tracks": plan.report(target.board, dry_run)}))
    }

    /// Delete the absorbed segments, then reshape the survivors, in a private transaction, and
    /// publish the result as one undo step. Inner command histories stay private.
    fn publish_merge(&mut self, target: BoardTarget, plan: &Plan) -> Result<(), CommandError> {
        let mut transaction = Self {
            libraries: self.libraries.clone(),
            project: self.snapshot(),
            revision: self.revision,
            history_root_revision: self.history_root_revision,
            ..Self::default()
        };
        if !plan.removed.is_empty() {
            transaction.apply(DELETE_TRACKS, &json!({"board":target.board,"tracks":plan.removed,"expected_revision":transaction.revision()}))?;
        }
        if !plan.after.is_empty() {
            let reshape: Vec<_> = plan.after.iter().map(|track| json!({"track":track.id,"start":track.start,"end":track.end})).collect();
            transaction.apply(RESHAPE_TRACKS, &json!({"board":target.board,"tracks":reshape,"expected_revision":transaction.revision()}))?;
        }
        let next = transaction.open_project()?.clone();
        self.change(next)
    }
}
