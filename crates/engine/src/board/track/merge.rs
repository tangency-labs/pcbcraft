//! Merge collinear and stacked duplicate track segments of existing copper.
//!
//! Only the addressed, unlocked segments take part. Two of them on the same layer with the
//! same width become one when they lie on one straight line and either share an end that no
//! other copper uses (no third track end, via or pad there) or one covers the other. The
//! merged copper outline is the same as before, so connectivity does not change. The segment
//! with the lowest UUID survives with the merged ends; the others are deleted. The whole edit
//! is one undo step, and merging an already merged selection changes nothing.
use super::*;
use crate::board::{geometry::continues_straight, view::BoardTarget, work::yield_unit};
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

/// Planned merge output ready for copper validation and publication.
pub(super) struct PreparedMerge {
    /// Next project when applying a non-empty merge; `None` for dry runs and no-ops.
    pub candidate: Option<Project>,
    /// Reshaped survivor track IDs requiring contact and net-class validation.
    pub added: BTreeSet<Uuid>,
    /// Full command response (`revision` + `merge_tracks`), with `revision` updated on commit.
    pub report: Value,
}

/// Own the immutable inputs while any native or browser work queue advances the merge planner.
/// Dropping the continuation cancels all remaining work without producing a candidate.
pub(super) struct Planner {
    work: Pin<Box<dyn Future<Output = Result<PreparedMerge, CommandError>> + Send>>,
    completed: usize,
}

impl Planner {
    /// Capture only owned inputs; the first planning unit runs when polled.
    pub(super) fn new(snapshot: Arc<Project>, target: BoardTarget, revision: u64, params: Value, report_budget: usize) -> Self {
        Self { work: Box::pin(async move { prepare(&snapshot, target, revision, &params, report_budget).await }), completed: 0 }
    }

    /// Consume at most `budget` units, retaining planning state between polls.
    pub(super) fn step(&mut self, budget: usize) -> Option<Result<PreparedMerge, CommandError>> {
        let mut context = Context::from_waker(Waker::noop());
        for _ in 0..budget {
            match self.work.as_mut().poll(&mut context) {
                Poll::Ready(result) => return Some(result),
                Poll::Pending => self.completed = self.completed.saturating_add(1),
            }
        }
        None
    }

    /// Logical progress, not an estimate of total work on a dense board.
    pub(super) fn completed(&self) -> usize {
        self.completed
    }
}

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
    /// Capture the board's via and pad copper once per plan in bounded batches.
    async fn new_async(project: &Project, view: BoardView<'_>) -> Self {
        let mut vias = Vec::with_capacity(view.vias().len());
        for via in view.vias() {
            yield_unit().await;
            let contour = PadContour::capsule(via.at, via.at, via.diameter_nm as f64 / 2.);
            vias.push((contour.bounds(), contour));
        }
        vias.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
        let mut pads = Vec::new();
        let total_footprints = project.board.footprints.len();
        let mut next = 0;
        while next < total_footprints {
            yield_unit().await;
            for land in lands_from_footprints(view.footprints_range(next, 32)) {
                let contour = land.contour();
                pads.push((contour.bounds(), contour, land.layers));
            }
            next = next.saturating_add(32);
        }
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

/// Whether `report` encodes as compact JSON in at most `budget` bytes. Encoding stops at the
/// limit without keeping a copy of the bytes.
pub(crate) fn report_fits(report: &Value, budget: usize) -> bool {
    struct Budget(usize);
    impl std::io::Write for Budget {
        /// Count each encoded chunk, refusing before it crosses the remaining byte budget.
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.checked_sub(bytes.len()).ok_or_else(|| std::io::Error::other("merge report byte limit exceeded"))?;
            Ok(bytes.len())
        }
        /// Counting has no buffered output to flush.
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Budget(budget), report).is_ok()
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

/// Parse and validate `tracks` against the board's existing tracks, yielding per item.
async fn selected_ids(params: &Value, tracks: &[Track]) -> Result<BTreeSet<Uuid>, CommandError> {
    if params.get("tracks").is_none() {
        return Ok(BTreeSet::new());
    }
    let ids: Vec<Uuid> = param(params, "tracks")?;
    let mut unique = BTreeSet::new();
    for id in ids {
        yield_unit().await;
        if !unique.insert(id) || tracks.binary_search_by_key(&id, |track| track.id).is_err() {
            return Err(CommandError::InvalidParams("tracks must be unique IDs of existing board items".into()));
        }
    }
    Ok(unique)
}

/// Plan the merge of the addressed segments on one board snapshot without changing it.
/// `selected` holds existing track UUIDs; locked ones are reported and left alone. Track ends
/// are counted over the whole board, so unaddressed and locked copper still blocks a join.
async fn plan(project: &Project, view: BoardView<'_>, selected: &BTreeSet<Uuid>) -> Plan {
    let tracks = view.tracks();
    let mut ends: BTreeMap<End, usize> = BTreeMap::new();
    let mut lines: BTreeMap<Line, Vec<Span>> = BTreeMap::new();
    let mut locked = Vec::new();
    for track in tracks {
        yield_unit().await;
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
    let mut obstacles = None;
    let mut removed = BTreeSet::new();
    // Repeat across all line groups until a full pass changes nothing, because absorbing a
    // covered segment on one line can drop a shared end's count to 2 and free a join on an
    // earlier line group. Every merge removes at least one span, so the loop always ends.
    loop {
        let mut changed = false;
        for (&(layer, width_nm, ..), spans) in &mut lines {
            if spans.len() > 1 {
                let mut join = Join { layer, width_nm, project, view, obstacles: &mut obstacles };
                changed |= join.merge_line(spans, &mut ends, &mut removed).await;
            }
        }
        if !changed {
            break;
        }
    }
    let survivors = lines.into_values().flatten();
    let original = move |id: &Uuid| tracks.binary_search_by_key(id, |track| track.id).ok().map(|index| &tracks[index]);
    let mut plan = Plan { locked, removed: removed.into_iter().collect(), ..Plan::default() };
    for span in survivors {
        yield_unit().await;
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
    project: &'a Project,
    view: BoardView<'a>,
    /// Via and pad copper, captured only when a join first needs it.
    obstacles: &'a mut Option<Obstacles>,
}

impl Join<'_> {
    /// Merge the spans of one line until none can merge, returning whether any span merged.
    /// Each pass sweeps the spans in order along the line, tracking the furthest-reaching kept
    /// span and indexing kept spans by their far end. A span covered by the furthest-reaching
    /// kept span is absorbed into it; otherwise a span starting at any earlier kept span's far
    /// end joins that span when `continues_straight` holds and the shared end is free (see
    /// [`Self::free_end`]). Every merge removes a span, so the passes end; a later pass absorbs
    /// any intermediate overlapping spans now covered by the extended run and adds joins that
    /// earlier absorptions unblocked.
    async fn merge_line(&mut self, spans: &mut Vec<Span>, ends: &mut BTreeMap<End, usize>, removed: &mut BTreeSet<Uuid>) -> bool {
        let mut any = false;
        loop {
            spans.sort_by(|a, b| a.low_t.cmp(&b.low_t).then(b.high_t.cmp(&a.high_t)).then(a.id.cmp(&b.id)));
            let mut merged = false;
            let mut kept: Vec<Span> = Vec::with_capacity(spans.len());
            let mut by_high: BTreeMap<i128, usize> = BTreeMap::new();
            let mut furthest = 0usize;
            for span in spans.drain(..) {
                yield_unit().await;
                if !kept.is_empty() {
                    if span.high_t <= kept[furthest].high_t {
                        self.absorb(&mut kept[furthest], span, ends, removed);
                        merged = true;
                        continue;
                    }
                    if let Some(&idx) = by_high.get(&span.low_t)
                        && continues_straight(kept[idx].low, kept[idx].high, span.high)
                        && self.free_end(kept[idx].high, ends).await
                    {
                        by_high.remove(&span.low_t);
                        self.absorb(&mut kept[idx], span, ends, removed);
                        by_high.insert(kept[idx].high_t, idx);
                        furthest = idx;
                        merged = true;
                        continue;
                    }
                }
                let idx = kept.len();
                by_high.insert(span.high_t, idx);
                kept.push(span);
                furthest = idx;
            }
            *spans = kept;
            if !merged {
                return any;
            }
            any = true;
        }
    }

    /// Whether exactly two track ends meet at `at` on this layer and no via or pad copper
    /// touches the round track end there, so joining the two segments hides no junction.
    async fn free_end(&mut self, at: Point, ends: &BTreeMap<End, usize>) -> bool {
        if ends.get(&(self.layer, at.x_nm, at.y_nm)) != Some(&2) {
            return false;
        }
        if self.obstacles.is_none() {
            *self.obstacles = Some(Obstacles::new_async(self.project, self.view).await);
        }
        self.obstacles.as_ref().is_none_or(|obstacles| !obstacles.touch(at, self.layer, self.width_nm))
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

/// Plan and validate a track merge against an immutable board snapshot without changing it.
/// Preview and apply both check the selection and report byte budget; applying builds and
/// validates the candidate board for copper contact and class-rule checks before publication.
async fn prepare(project: &Project, target: BoardTarget, revision: u64, params: &Value, report_budget: usize) -> Result<PreparedMerge, CommandError> {
    let view = project.board_view(target).ok_or_else(|| CommandError::Unavailable("Board unavailable.".into()))?;
    let selected = selected_ids(params, view.tracks()).await?;
    if selected.is_empty() {
        return Err(CommandError::InvalidParams("name at least one track to merge".into()));
    }
    let plan = plan(project, view, &selected).await;
    let dry_run = params["dry_run"].as_bool().unwrap_or(false);
    let report = plan.report(target.board, dry_run);
    yield_unit().await;
    if !report_fits(&report, report_budget) {
        return Err(CommandError::CapExceeded(format!("the merge report would exceed {report_budget} bytes; merge fewer tracks at a time")));
    }
    let envelope = json!({"revision": revision, "merge_tracks": report});
    if !plan.changed() || dry_run {
        return Ok(PreparedMerge { candidate: None, added: BTreeSet::new(), report: envelope });
    }
    let (candidate, added) = build_candidate(project, &plan).await?;
    Ok(PreparedMerge { candidate, added, report: envelope })
}

/// Apply the planned deletions and reshapes to a private board copy, validating geometry and
/// lock protection before copper contact validation runs.
async fn build_candidate(project: &Project, plan: &Plan) -> Result<(Option<Project>, BTreeSet<Uuid>), CommandError> {
    let removed: BTreeSet<Uuid> = plan.removed.iter().copied().collect();
    let mut reshaped: BTreeMap<Uuid, &Track> = BTreeMap::new();
    for track in &plan.after {
        yield_unit().await;
        reshaped.insert(track.id, track);
    }
    let mut tracks = Vec::with_capacity(project.board.tracks.len().saturating_sub(removed.len()));
    for track in &project.board.tracks {
        yield_unit().await;
        if removed.contains(&track.id) {
            continue;
        }
        if let Some(updated) = reshaped.get(&track.id) {
            tracks.push((*updated).clone());
        } else {
            tracks.push(track.clone());
        }
    }
    let mut vias = project.board.vias.clone();
    yield_unit().await;
    validate(project, &mut tracks, &mut vias).map_err(CommandError::InvalidParams)?;
    if tracks == project.board.tracks && vias == project.board.vias {
        return Ok((None, BTreeSet::new()));
    }
    let mut next = project.clone();
    let board = Arc::make_mut(&mut next.board);
    board.tracks = tracks;
    board.vias = vias;
    protect_locked(project, &next)?;
    let added: BTreeSet<Uuid> = plan.after.iter().map(|track| track.id).collect();
    Ok((Some(next), added))
}

impl Session {
    /// Merge the addressed segments, or only report the merge on a dry run. The result lists
    /// the original and final records of every changed segment, the deleted and reshaped
    /// UUIDs, the locked segments left alone and whether anything changed. Applying validates
    /// the merged copper through the shared preparation job and publishes one undo step. With
    /// nothing to merge, revision and history stay unchanged. A report larger than
    /// [`MERGE_REPORT_MAX_BYTES`] is refused with `cap_exceeded` before anything changes, for
    /// a dry run as for an applied merge.
    pub(crate) fn merge_tracks(&mut self, params: &Value) -> Result<Value, CommandError> {
        self.merge_tracks_within(params, MERGE_REPORT_MAX_BYTES)
    }

    /// [`Self::merge_tracks`] with an explicit report byte budget.
    pub(crate) fn merge_tracks_within(&mut self, params: &Value, report_budget: usize) -> Result<Value, CommandError> {
        let mut job = self.prepare_merge_tracks_within(params, report_budget)?;
        while !job.step(1024) {}
        let prepared = job.take_result()?.ok_or_else(|| CommandError::Unavailable("Merge preparation returned no result.".into()))?;
        self.commit_copper_edit_with_report(prepared)?.1.ok_or_else(|| CommandError::Unavailable("Merge preparation returned no report.".into()))
    }
}
