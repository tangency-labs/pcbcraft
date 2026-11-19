//! Resumable read-only checker work shared by native workers and browser frame budgets.
use super::*;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(clippy::unwrap_used, reason = "opt-in diagnostic fails on invalid fixture setup; preserve the matched measurement source")]
#[path = "release_board_checks.rs"]
mod release_board_checks;
use crate::board::{
    pair_index::{PairStep, PairWalk},
    track::{Copper, IslandBuilder, lands_from_footprints},
};

pub use super::super::work::{Progress as CheckProgress, Source as CheckSource};

/// One captured read-only DRC. Dropping it cancels unfinished work without publishing a
/// partial report. Pair-heavy phases yield after a caller-supplied work budget.
pub struct CheckJob {
    source: CheckSource,
    phase: u8,
    lands: Vec<Land>,
    builder: Option<IslandBuilder>,
    copper: Copper,
    members: Vec<Vec<usize>>,
    geometry: Option<BoardGeometry>,
    class_rules: Option<super::super::class_rules::Context>,
    clearance_walk: Option<PairWalk>,
    cursor: (usize, usize),
    work: usize,
    found: Findings,
    courtyards: Vec<super::courtyard::Courtyard>,
    report: Option<Report>,
}

impl Session {
    /// Validate board/rules and capture an immutable checker job without geometry work,
    /// project serialization, authored changes or an undo entry.
    pub fn design_check_job(&self, params: &Value) -> Result<CheckJob, CommandError> {
        self.validate_project_command(CHECK_DESIGN, params)?;
        let (view, effective) = self.design_check_inputs(params)?;
        let target = view.target;
        let snapshot = self.snapshot().ok_or_else(|| CommandError::Unavailable("no project is open".into()))?;
        Ok(CheckJob {
            source: CheckSource {
                snapshot,
                session: self.clearance_identity.clone(),
                opening: self.history_root_revision(),
                revision: self.revision(),
                target,
                effective,
            },
            phase: 0,
            lands: Vec::new(),
            builder: None,
            copper: Copper::default(),
            members: Vec::new(),
            geometry: None,
            class_rules: None,
            clearance_walk: None,
            cursor: (0, 1),
            work: 0,
            found: Findings::default(),
            courtyards: Vec::new(),
            report: None,
        })
    }
}

impl CheckJob {
    /// Captured source authority; a host must additionally validate its view/request generation.
    pub fn source(&self) -> &CheckSource {
        &self.source
    }
    /// Stage-local progress, available without waiting on worker completion.
    pub fn progress(&self) -> CheckProgress {
        let stage = [
            "Preparing board",
            "Copper contacts",
            "Pad contacts",
            "Resolving nets",
            "Dimensions and shorts",
            "Copper clearance",
            "Unrouted connections",
            "Board outline",
            "Courtyards",
            "Finishing report",
            "Complete",
        ];
        let count = self
            .geometry
            .as_ref()
            .map_or_else(|| self.lands.len() + self.source.snapshot.board.tracks.len() + self.source.snapshot.board.vias.len(), |g| g.items.len());
        let total = match self.phase {
            0 => self.source.snapshot.board.footprints.len(),
            1 => count.saturating_mul(count.saturating_sub(1)) / 2,
            2 => self.lands.len().saturating_mul(self.lands.len().saturating_sub(1)) / 2,
            5 => count.saturating_mul(count.saturating_sub(1)) / 2,
            7 => count,
            8 => self.courtyards.len().saturating_mul(self.courtyards.len().saturating_sub(1)) / 2,
            _ => 1,
        };
        CheckProgress { stage: stage[self.phase as usize], completed: self.work.min(total), total }
    }
    /// Advance at most `budget` pair/outline units, yielding at every stage boundary.
    /// Footprint projection uses ranges of at most 32 placements; remaining preparation/
    /// finalization stages yield at phase boundaries and retain supported model/report bounds.
    /// A zero budget does no work. Completion returns no partial findings; take the report once.
    pub fn step(&mut self, budget: usize) -> bool {
        if self.phase == 10 {
            return true;
        }
        if budget == 0 {
            return false;
        }
        let Some(view) = self.source.snapshot.board_view(self.source.target) else {
            return false;
        };
        let rules = self.source.effective.values;
        match self.phase {
            0 => {
                let footprints: Vec<_> = view.footprints_range(self.work, budget.min(32)).collect();
                self.courtyards.extend(footprints.iter().filter_map(super::courtyard::Courtyard::capture));
                self.lands.extend(lands_from_footprints(footprints.into_iter()));
                self.work = self.work.saturating_add(budget.min(32));
                if self.work < self.source.snapshot.board.footprints.len() {
                    return false;
                }
                self.builder = Some(IslandBuilder::new(&self.lands, view.tracks(), view.vias()));
            }
            1 => {
                if !self.builder.as_mut().is_some_and(|b| b.step(budget)) {
                    self.work = self.work.saturating_add(budget);
                    return false;
                }
                if let Some(builder) = self.builder.take() {
                    (self.copper, self.members) = builder.finish(&self.lands, view.tracks(), view.vias());
                }
                self.builder = Some(IslandBuilder::pad_contacts(&self.lands, view.tracks(), view.vias(), &self.members));
            }
            2 => {
                if !self.builder.as_mut().is_some_and(|b| b.step_pads(self.lands.len(), budget)) {
                    self.work = self.work.saturating_add(budget);
                    return false;
                }
                if let Some(builder) = self.builder.take() {
                    let (_, merged) = builder.finish(&self.lands, view.tracks(), view.vias());
                    self.geometry = Some(BoardGeometry::from_contacts(
                        view,
                        std::mem::take(&mut self.lands),
                        self.copper.clone(),
                        std::mem::take(&mut self.members),
                        merged,
                    ));
                }
            }
            3 => {
                if let Some(g) = &self.geometry {
                    self.class_rules = capture_class_rules(view, g, &self.source.effective, &mut self.found);
                    self.clearance_walk = Some(g.clearance_walk(rules.clearance_nm, self.class_rules.as_ref()));
                }
            }
            4 => {
                view.check_dimensions(rules, self.class_rules.as_ref(), &mut self.found);
                if let Some(g) = &self.geometry {
                    view.check_shorts(&g.lands, &g.members, &g.items, &mut self.found);
                }
            }
            5 => {
                if let (Some(g), Some(walk)) = (&self.geometry, &mut self.clearance_walk) {
                    let mut done = false;
                    for _ in 0..budget {
                        let (i, j) = match walk.step() {
                            PairStep::Pending => continue,
                            PairStep::Pair(i, j) => (i, j),
                            PairStep::Done => {
                                done = true;
                                break;
                            }
                        };
                        self.work += 1;
                        if let Some(d) = g.clearance_decision(i, j, rules.clearance_nm, self.class_rules.as_ref())
                            && d.clearance_violation
                        {
                            let (a, b) = (&g.items[i], &g.items[j]);
                            let nets = match (&a.net, &b.net) {
                                (Some(x), Some(y)) => format!(" between nets {x} and {y}"),
                                _ => String::new(),
                            };
                            let message = if d.gap_nm <= 0. {
                                format!("Copper touches{nets}")
                            } else {
                                format!("Clearance {} is below the {} rule{nets}", mm(d.gap_nm), mm(d.clearance_nm as f64))
                            };
                            let mut names = vec![a.name.clone(), b.name.clone()];
                            names.sort();
                            self.found.push(Violation { kind: Kind::Clearance, message, at: midpoint(a.at, b.at), items: names, nets: Vec::new() });
                        }
                    }
                    if !done {
                        return false;
                    }
                }
                self.clearance_walk = None;
            }
            6 => {
                if let Ok(r) = view.ratsnest_with_copper(&self.copper) {
                    for l in r.lines {
                        self.found.push(Violation {
                            kind: Kind::Unconnected,
                            message: "Unrouted connection".into(),
                            at: midpoint(l.from.position, l.to.position),
                            items: vec![
                                format!("{}:{}", l.from.footprint, l.from.pad.number.as_str()),
                                format!("{}:{}", l.to.footprint, l.to.pad.number.as_str()),
                            ],
                            nets: Vec::new(),
                        });
                    }
                }
            }
            7 => {
                if let Some(g) = &self.geometry {
                    let end = self.work.saturating_add(budget).min(g.items.len());
                    view.check_outline(&g.items[self.work..end], rules, self.class_rules.as_ref(), &mut self.found);
                    self.work = end;
                    if end < g.items.len() {
                        return false;
                    }
                    // Empty boards still check authored outlines.
                    if g.items.is_empty() {
                        view.check_outline(&[], rules, self.class_rules.as_ref(), &mut self.found);
                    }
                }
                self.cursor = (0, 1);
            }
            8 => {
                for _ in 0..budget {
                    let (i, j) = self.cursor;
                    if i >= self.courtyards.len() {
                        break;
                    }
                    if j >= self.courtyards.len() {
                        self.cursor = (i + 1, i + 2);
                        continue;
                    }
                    self.cursor.1 += 1;
                    self.work += 1;
                    if let Some(violation) = self.courtyards[i].overlap(&self.courtyards[j]) {
                        self.found.push(violation);
                    }
                }
                if self.cursor.0 < self.courtyards.len() {
                    return false;
                }
            }
            9 => {
                self.report = Some(std::mem::take(&mut self.found).finish(view, self.source.effective.clone(), self.class_rules.take()));
            }
            _ => {}
        }
        self.phase += 1;
        self.work = 0;
        self.phase == 10
    }
    /// Consume a complete report once; pending and canceled work never masquerades as a pass.
    pub fn take_report(&mut self) -> Option<Report> {
        self.report.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        board::track::tests::{routed_fixture, view},
        persistence::encode_documents,
    };
    use std::sync::Arc;

    /// Every batch size agrees with the complete checker, including request-local provenance.
    /// Pending work and report consumption leave the source bytes and undo revision intact.
    #[test]
    fn resumable_check_agrees_with_complete_reference_and_preserves_source() {
        let mut session = routed_fixture();
        super::super::tests::outline(&mut session);
        let board = view(&session).target;
        let before = session.snapshot().unwrap();
        let bytes = encode_documents(before.clone()).unwrap();
        let revision = session.revision();
        // Field names, order and option omission of the exact pre-advisory typed envelope.
        #[derive(Serialize)]
        struct NativeEnvelope<'a> {
            rules: Rules,
            sources: &'a BTreeMap<String, crate::board::rules::Source>,
            #[serde(skip_serializing_if = "Option::is_none")]
            class_rules: Option<&'a crate::board::class_rules::Context>,
            violations: &'a [Violation],
            counts: &'a BTreeMap<Kind, usize>,
            total: usize,
        }
        for params in [json!({"board":board.board}), json!({"board":board.board,"clearance_nm":300_000})] {
            let expected = session.check_design(&params).unwrap();
            let envelope = session.execute(CHECK_DESIGN, &params).unwrap();
            for budget in [1, 7, 256, 4096] {
                let mut job = session.design_check_job(&params).unwrap();
                assert!(!job.step(0));
                assert!(job.take_report().is_none());
                let mut steps = 0;
                while !job.step(budget) {
                    steps += 1;
                    assert!(steps < 100_000);
                    assert!(job.take_report().is_none());
                }
                let report = job.take_report().unwrap();
                let native = NativeEnvelope {
                    rules: report.rules,
                    sources: &report.sources,
                    class_rules: report.class_rules.as_ref(),
                    violations: &report.violations,
                    counts: &report.counts,
                    total: report.total,
                };
                assert!(report.relations.is_none());
                assert_eq!(serde_json::to_vec(&report).unwrap(), serde_json::to_vec(&native).unwrap());
                assert_eq!(serde_json::to_vec(&json!(&native)).unwrap(), serde_json::to_vec(&expected).unwrap(), "budget={budget}");
                assert_eq!(serde_json::to_vec(&json!({"revision":revision,"design_check":&native})).unwrap(), serde_json::to_vec(&envelope).unwrap());
                assert!(job.take_report().is_none());
                assert!(job.step(1));
                assert!(job.source().is_current(&session));
            }
        }
        assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
        assert_eq!(revision, session.revision());
        assert_eq!(bytes, encode_documents(session.snapshot().unwrap()).unwrap());
    }

    /// Reopening the identical document or editing/undoing cannot revive old report authority.
    #[test]
    fn source_authority_expires_across_session_reopen_edits_and_undo() {
        let mut s = routed_fixture();
        let p = s.snapshot().unwrap();
        let job = s.design_check_job(&json!({})).unwrap();
        let mut other = Session::default();
        other.open_snapshot(p.clone());
        assert!(!job.source().is_current(&other));
        s.apply(crate::SET_RULES, &json!({"board":p.board_target().unwrap().board,"rules":Rules{min_track_width_nm:200_000,..Rules::default()},"expected_revision":s.revision()})).unwrap();
        assert!(!job.source().is_current(&s));
        s.apply("edit.undo", &Value::Null).unwrap();
        assert!(!job.source().is_current(&s));
        let job = s.design_check_job(&json!({})).unwrap();
        s.open_snapshot(p);
        assert!(!job.source().is_current(&s));
    }

    /// Existing short fixtures cover direct pads, tracks and vias with zero/equal clearances.
    #[test]
    fn resumable_short_findings_agree_for_all_contact_paths() {
        use super::super::short_tests::{Bridge, bridged_split};
        for bridge in [Bridge::Pads, Bridge::Track, Bridge::Vias] {
            let s = bridged_split(bridge);
            for clearance in [0, 200_000] {
                let params = json!({"clearance_nm":clearance});
                let expected = s.check_design(&params).unwrap();
                let mut job = s.design_check_job(&params).unwrap();
                while !job.step(3) {}
                assert_eq!(json!(job.take_report().unwrap()), expected);
            }
        }
    }

    /// A high-finding workload retains exactly the canonical first page and complete counts.
    #[test]
    fn findings_retention_is_bounded_and_canonically_ordered() {
        let mut findings = Findings::default();
        for x in (0..10_000).rev() {
            findings.push(Violation {
                kind: Kind::Clearance,
                message: format!("{x}"),
                at: Point { x_nm: x, y_nm: 0 },
                items: Vec::new(),
                nets: Vec::new(),
            });
            assert!(findings.listed.len() <= MAX_LISTED);
        }
        let s = routed_fixture();
        let view = view(&s);
        let report = findings.finish(view, view.effective_rules(), None);
        assert_eq!(report.total, 10_000);
        assert_eq!(report.counts[&Kind::Clearance], 10_000);
        assert_eq!(report.violations.len(), MAX_LISTED);
        assert_eq!(report.violations[0].at.x_nm, 0);
        assert_eq!(report.violations.last().unwrap().at.x_nm, 999);
    }
}
