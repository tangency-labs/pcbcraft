//! One revision-checked transaction for a batch of straight-via intent changes.
use super::{
    Record, ViaTeardrop,
    edit::{self, Addition, Edit, Rule, RuleBlock},
};
use crate::{
    CommandError, Session,
    board::{
        clearance::{ClearanceQuery, ObstacleId, Overlay},
        drc::{self, Outline},
        geometry::Contour,
        view::{BoardTarget, BoardView},
    },
    schematic::MAX_CENTER_NM,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
use uuid::Uuid;

/// Add and remove checked straight-via teardrop intents.
pub const EDIT_TEARDROPS: &str = "pcb.edit_teardrops";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Batch {
    add: Vec<Record>,
    remove: Vec<Uuid>,
}

impl Batch {
    /// Bound both lists and refuse parameters outside the stored dimension vocabulary.
    pub(crate) fn parse(value: &Value) -> Result<Self, CommandError> {
        let batch: Self = serde_json::from_value(value.clone()).map_err(|error| CommandError::InvalidParams(error.to_string()))?;
        if batch.add.len() > edit::MAX_INTENTS || batch.remove.len() > edit::MAX_INTENTS {
            return Err(CommandError::CapExceeded("too many teardrop edits".into()));
        }
        for record in &batch.add {
            record.parameters.validate().map_err(|_| CommandError::InvalidParams("invalid teardrop dimensions".into()))?;
        }
        Ok(batch)
    }
}

/// Publish the bounded board-local record vocabulary in the shared catalog.
pub(crate) fn schema() -> Value {
    let uuid = json!({"type":"string","format":"uuid"});
    let dimension = json!({"type":"integer","minimum":1,"maximum":MAX_CENTER_NM});
    let percent = json!({"type":"integer","minimum":1,"maximum":u16::MAX});
    let cap = json!({"anyOf":[dimension,{"type":"null"}]});
    json!({"type":"object","additionalProperties":false,"required":["add","remove"],"properties":{
        "add":{"type":"array","maxItems":edit::MAX_INTENTS,"items":{
            "type":"object","additionalProperties":false,"required":["id","via","track","parameters"],"properties":{
                "id":uuid,"via":uuid,"track":uuid,"parameters":{"type":"object","additionalProperties":false,
                    "required":["length","width_pct"],"properties":{"width_pct":percent,"max_length_nm":cap,"max_width_nm":cap,
                    "length":{"oneOf":[
                        {"type":"object","additionalProperties":false,"required":["percent"],"properties":{"percent":percent}},
                        {"type":"object","additionalProperties":false,"required":["nanometres"],"properties":{"nanometres":dimension}}
                    ]}}}
            }}},
        "remove":{"type":"array","maxItems":edit::MAX_INTENTS,"items":uuid}
    }})
}

struct Evaluator<'a> {
    overlay: Overlay<'a>,
    sources: BTreeMap<Uuid, usize>,
    added_islands: BTreeMap<Uuid, usize>,
    outline: Outline,
    initialized: bool,
}

impl<'a> Evaluator<'a> {
    /// Start with one immutable query and the outline rules used by board DRC.
    fn new(query: &'a ClearanceQuery, board: BoardView<'_>) -> Self {
        let sources =
            query.objects().iter().enumerate().filter_map(|(index, object)| Uuid::parse_str(&object.name).ok().map(|id| (id, index))).collect();
        Self { overlay: query.overlay(), sources, added_islands: BTreeMap::new(), outline: board.outline_edges(), initialized: false }
    }

    /// Resolve the attached track as the membership and class-rule source.
    fn source(&self, intent: ViaTeardrop) -> Result<(usize, usize), CommandError> {
        let source =
            *self.sources.get(&intent.track).ok_or_else(|| CommandError::NotFound("teardrop track is absent from the clearance query".into()))?;
        let island = self
            .overlay
            .object(ObstacleId::Source(source))
            .ok_or_else(|| CommandError::Unavailable("missing teardrop clearance source".into()))?
            .island;
        Ok((source, island))
    }

    /// Add a derived contour to the disposable overlay with its source track's intended net.
    fn stage(&mut self, addition: &Addition, source: usize, island: usize) -> Result<(), CommandError> {
        self.overlay.add(addition.intent.id, contour(addition), vec![addition.copper.layer], Some(source))?;
        self.added_islands.insert(addition.intent.id, island);
        Ok(())
    }

    /// Check closing outline, class-aware edge and copper gaps, and refuse new island joins.
    /// Add only approved contours to the overlay seen by later requests.
    fn check(&mut self, board: BoardView<'_>, addition: &Addition, retained: &[ViaTeardrop]) -> Result<Option<RuleBlock>, CommandError> {
        if self.outline.polygons.is_empty() {
            return Err(CommandError::Unavailable("teardrops require a closed Board Outline".into()));
        }
        if !self.initialized {
            for &intent in retained {
                let copper = intent
                    .derive(board)
                    .map_err(|error| CommandError::Unavailable(format!("cannot check retained teardrop {}: {error:?}", intent.id)))?;
                let (source, island) = self.source(intent)?;
                self.stage(&Addition { intent, copper }, source, island)?;
            }
            self.initialized = true;
        }
        let (source, island) = self.source(addition.intent)?;
        let shape = contour(addition);
        let probe = self.overlay.probe(&shape, &[addition.copper.layer], Some(ObstacleId::Source(source)))?;
        let mut pending_spacing = None;
        for decision in probe.decisions {
            let other = self.overlay.object(decision.obstacle).ok_or_else(|| CommandError::Unavailable("teardrop obstacle disappeared".into()))?;
            let same_island = match decision.obstacle {
                ObstacleId::Source(_) => other.island == island,
                ObstacleId::Added(id) => self.added_islands.get(&id) == Some(&island),
            };
            if decision.touches && !same_island {
                return Ok(Some(RuleBlock {
                    rule: Rule::ConflictingContact,
                    other_object: other.name.clone(),
                    required_nm: decision.clearance_nm,
                    actual_nm: decision.gap_nm,
                }));
            }
            if !same_island && decision.clearance_violation && pending_spacing.is_none() {
                pending_spacing = Some(RuleBlock {
                    rule: Rule::CopperClearance,
                    other_object: other.name.clone(),
                    required_nm: decision.clearance_nm,
                    actual_nm: decision.gap_nm,
                });
            }
        }
        if pending_spacing.is_some() {
            return Ok(pending_spacing);
        }
        let values = board.effective_rules().values;
        let required_nm = probe.probe_rules.as_ref().map_or(values, |rules| rules.hard_values(values)).edge_clearance_nm;
        let outside = !drc::contains(&self.outline.polygons, &shape);
        let (drawing, actual_nm, crosses) = self
            .outline
            .blocking_edge(&shape, outside)
            .ok_or_else(|| CommandError::Unavailable("teardrop outline has no addressable edges".into()))?;
        let rule = if outside {
            Some(Rule::OutsideBoardOutline)
        } else if crosses {
            Some(Rule::BoardEdgeCrossing)
        } else if actual_nm < required_nm as f64 {
            Some(Rule::BoardEdgeClearance)
        } else {
            None
        };
        if let Some(rule) = rule {
            return Ok(Some(RuleBlock { rule, other_object: drawing.to_string(), required_nm, actual_nm }));
        }
        self.stage(addition, source, island)?;
        Ok(None)
    }
}

/// A zero-radius convex core is the exact derived teardrop polygon.
fn contour(addition: &Addition) -> Contour {
    Contour { core: addition.copper.vertices, radius: 0. }
}

impl Session {
    /// Preview or publish the entire checked batch; a stale revision or evaluation error edits nothing.
    pub(crate) fn edit_teardrops(&mut self, params: &Value) -> Result<Value, CommandError> {
        self.validate_project_command(EDIT_TEARDROPS, params)?;
        self.check_revision(params)?;
        let project = self.open_project()?;
        let target = project.resolve_board(params.get("board").and_then(Value::as_str))?;
        let board = project.board_view(target).ok_or_else(|| CommandError::NotFound("board is unavailable".into()))?;
        let batch = Batch::parse(&params["edits"])?;
        let dry_run = params["dry_run"].as_bool().unwrap_or(false);
        let current: Vec<_> = board.teardrops().iter().map(|record| record.on(target)).collect();
        let mut edits: Vec<_> = batch.remove.into_iter().map(Edit::Remove).collect();
        for record in batch.add {
            if board.tracks().binary_search_by_key(&record.id, |track| track.id).is_ok()
                || board.vias().binary_search_by_key(&record.id, |via| via.id).is_ok()
            {
                return Err(CommandError::InvalidParams("teardrop identity is already used by copper".into()));
            }
            edits.push(Edit::Add(record.on(target)));
        }
        let query = edits.iter().any(|edit| matches!(edit, Edit::Add(_))).then(|| self.teardrop_base_query(target)).transpose()?;
        let mut evaluator = query.as_ref().map(|query| Evaluator::new(query, board));
        let diff = edit::plan(board, &current, &edits, |addition, retained, _| {
            evaluator.as_mut().ok_or_else(|| CommandError::Unavailable("missing teardrop clearance query".into()))?.check(board, addition, retained)
        })
        .map_err(|error| match error {
            edit::Error::RuleEvaluation { cause, .. } => cause,
            edit::Error::InvalidRuleReport(_) => CommandError::Unavailable("invalid teardrop rule report".into()),
            error => CommandError::InvalidParams(format!("invalid teardrop edit: {error:?}")),
        })?;
        let removed: Vec<_> = diff.removed.iter().copied().map(Record::from).collect();
        let added: Vec<_> = diff.added.iter().map(|addition| Record::from(addition.intent)).collect();
        let changed = !added.is_empty() || !removed.is_empty();
        let report = report(target, &diff, &added, &removed, dry_run);
        if changed && !dry_run {
            let mut next = project.clone();
            let mut records: BTreeMap<_, _> = board.teardrops().iter().map(|&record| (record.id, record)).collect();
            for record in removed {
                records.remove(&record.id);
            }
            records.extend(added.into_iter().map(|record| (record.id, record)));
            Arc::make_mut(&mut next.board).teardrops = records.into_values().collect();
            self.change(next)?;
        }
        Ok(json!({"revision":self.revision(),"teardrops":report}))
    }
}

/// Report authored edits and disposable copper separately; neither preview nor saving stores vertices.
fn report(target: BoardTarget, diff: &edit::Plan, added: &[Record], removed: &[Record], dry_run: bool) -> Value {
    let rejected: Vec<_> = diff.rejected.iter().map(|rejected| json!({"intent":Record::from(rejected.intent),"reason":rejected.reason})).collect();
    let copper: Vec<_> = diff
        .added
        .iter()
        .map(|addition| json!({"id":addition.intent.id,"layer":addition.copper.layer,"vertices":addition.copper.vertices}))
        .collect();
    json!({"board":target.board,"dry_run":dry_run,"added":added,"removed":removed,"rejected":rejected,"copper":copper})
}
