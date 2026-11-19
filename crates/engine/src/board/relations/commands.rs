//! The closed P1.1 command envelopes and private, candidate-only preview dispatch.

use super::{
    Kind, Point, Reference, Relation, Relations, State, catalog,
    transaction::{self, Base, Prepared, RelationInput, ReviewSlot},
};
use crate::{
    Command, CommandError, Param, ParamKind, Session,
    board::clipboard::{ClipboardNotice, ClipboardResult},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

pub(crate) const ADD: &str = "pcb.add_relation";
pub(crate) const EDIT: &str = "pcb.edit_relation";
pub(crate) const DELETE: &str = "pcb.delete_relations";
pub(crate) const PREVIEW: &str = "pcb.preview_relation";
pub(crate) const APPLY: &str = "pcb.apply_relation_preview";
pub(crate) const CANCEL: &str = "pcb.cancel_relation_preview";
pub(crate) const LIST: &str = "pcb.relations";
pub(crate) const FEATURES: &str = "pcb.feature_catalog";
const RESULT_BYTES: usize = 8 * 1024 * 1024;

pub(crate) const REMAP_CHOICES: Param = p("remap_choices", Parameter::Remaps, false);
pub(crate) const ACCEPT_LOSS: Param = p("accept_loss", Parameter::Ids, false);
pub(crate) const CONFIRM_REMAPS: Param = p("confirm_remaps", Parameter::RemapIds, false);
const TOKEN: Param = p("token", Parameter::Id, true);

/// Keep the runtime validation and published parameter vocabulary together.
const fn p(name: &'static str, kind: Parameter, required: bool) -> Param {
    Param { name, required, kind: ParamKind::Relation(kind), summary: "Typed reference relation input." }
}

pub(crate) const ADD_COMMAND: Command = Command {
    id: ADD,
    title: "Add Reference Relation",
    summary: "Measure two ordered native features without moving either.",
    params: &[crate::BOARD, crate::EXPECTED_REVISION, p("relation", Parameter::Draft, true)],
};
pub(crate) const EDIT_COMMAND: Command = Command {
    id: EDIT,
    title: "Edit Reference Relation",
    summary: "Re-pick, label or suppress a reference without moving the board.",
    params: &[crate::BOARD, crate::EXPECTED_REVISION, p("id", Parameter::Id, true), p("edit", Parameter::Edit, true)],
};
pub(crate) const DELETE_COMMAND: Command = Command {
    id: DELETE,
    title: "Delete Reference Relations",
    summary: "Remove the exact selected relation IDs.",
    params: &[crate::BOARD, crate::EXPECTED_REVISION, p("ids", Parameter::Ids, true)],
};
pub(crate) const PREVIEW_COMMAND: Command = Command {
    id: PREVIEW,
    title: "Preview Relation Effects",
    summary: "Prepare one supported edit and disclose complete loss and remap decisions without publishing.",
    params: &[crate::EXPECTED_REVISION, p("intent", Parameter::Intent, true), REMAP_CHOICES],
};
pub(crate) const APPLY_COMMAND: Command = Command {
    id: APPLY,
    title: "Apply Relation Preview",
    summary: "Publish exactly the current checked candidate after both independently exact decisions.",
    params: &[crate::EXPECTED_REVISION, TOKEN, p("accept_loss", Parameter::Ids, true), p("confirm_remaps", Parameter::RemapIds, true)],
};
pub(crate) const CANCEL_COMMAND: Command =
    Command { id: CANCEL, title: "Cancel Relation Preview", summary: "Revoke only the corresponding generic review capability.", params: &[TOKEN] };
pub(crate) const LIST_COMMAND: Command = Command {
    id: LIST,
    title: "List Board Relations",
    summary: "Inspect current reference measurements and lost display witnesses.",
    params: &[crate::BOARD],
};
pub(crate) const FEATURES_COMMAND: Command = Command {
    id: FEATURES,
    title: "List Reference Features",
    summary: "List currently offered native features for a placement, or the native board datum when omitted.",
    params: &[crate::BOARD, Param { name: "fp", kind: ParamKind::FootprintId, required: false, summary: "Native placed footprint ID." }],
};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Parameter {
    Id,
    Ids,
    RemapIds,
    Remaps,
    Draft,
    Edit,
    Intent,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Draft {
    kind: Kind,
    refs: [Reference; 2],
    label: Option<Point>,
    #[serde(default)]
    suppressed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Changes {
    #[serde(default, deserialize_with = "crate::board::reference::present")]
    repick: Option<Draft>,
    #[serde(default, deserialize_with = "crate::board::reference::present")]
    suppressed: Option<bool>,
    #[serde(default, deserialize_with = "crate::board::reference::present")]
    label: Option<Option<Point>>,
}

/// Command ID carried as a closed tag; supplied arguments are then validated against that
/// exact native command's schema and normalized board/sheet targets before dispatch.
#[derive(Clone, Copy, Debug, Deserialize)]
enum Source {
    #[serde(rename = "pcb.cut")]
    BoardCut,
    #[serde(rename = "pcb.delete_items")]
    BoardDelete,
    #[serde(rename = "pcb.delete_footprints")]
    DeleteFootprints,
    #[serde(rename = "schematic.cut")]
    SchematicCut,
    #[serde(rename = "schematic.delete_items")]
    SchematicDelete,
    #[serde(rename = "schematic.delete_symbol")]
    DeleteSymbol,
    #[serde(rename = "sheet.delete")]
    DeleteSheet,
    #[serde(rename = "pcb.move_items")]
    Move,
    #[serde(rename = "pcb.set_footprint_transform")]
    Transform,
    #[serde(rename = "pcb.align_footprints")]
    Align,
    #[serde(rename = "pcb.add_drawing")]
    AddDrawing,
    #[serde(rename = "pcb.set_drawing")]
    SetDrawing,
    #[serde(rename = "pcb.delete_drawing")]
    DeleteDrawing,
    #[serde(rename = "pcb.set_placement_origin")]
    Origin,
    #[serde(rename = "pcb.assign_footprint")]
    Assign,
    #[serde(rename = "part.set_selection")]
    Part,
}

const SOURCES: &[Source] = &[
    Source::BoardCut,
    Source::BoardDelete,
    Source::DeleteFootprints,
    Source::SchematicCut,
    Source::SchematicDelete,
    Source::DeleteSymbol,
    Source::DeleteSheet,
    Source::Move,
    Source::Transform,
    Source::Align,
    Source::AddDrawing,
    Source::SetDrawing,
    Source::DeleteDrawing,
    Source::Origin,
    Source::Assign,
    Source::Part,
];

impl Source {
    /// Map only enumerated commands to their native handlers; never execute arbitrary requests.
    fn id(self) -> &'static str {
        match self {
            Self::BoardCut => crate::board::clipboard::CUT,
            Self::BoardDelete => crate::board::clipboard::DELETE,
            Self::DeleteFootprints => crate::DELETE_FOOTPRINTS,
            Self::SchematicCut => crate::clipboard::CUT,
            Self::SchematicDelete => crate::DELETE_ITEMS,
            Self::DeleteSymbol => crate::DELETE_SYMBOL,
            Self::DeleteSheet => crate::sheets::DELETE_SHEET,
            Self::Move => crate::board::movement::MOVE_ITEMS,
            Self::Transform => crate::SET_FOOTPRINT_TRANSFORM,
            Self::Align => crate::ALIGN_FOOTPRINTS,
            Self::AddDrawing => crate::ADD_DRAWING,
            Self::SetDrawing => crate::SET_DRAWING,
            Self::DeleteDrawing => crate::DELETE_DRAWING,
            Self::Origin => crate::SET_PLACEMENT_ORIGIN,
            Self::Assign => crate::ASSIGN_FOOTPRINT,
            Self::Part => crate::part_instances::SET_PART,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    command: Source,
    params: serde_json::Map<String, Value>,
}

/// A JSON object with all declared input fields and no unrecognized extensions.
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

/// Describe the exact closed feature tags and their individual fields.
fn ref_schema() -> Value {
    let segment = json!({"type":"string","maxLength":64,"pattern":"^[a-z0-9]([a-z0-9-]*[a-z0-9])?$"});
    let revision = json!({"type":"integer","minimum":1,"maximum":u32::MAX});
    let key = json!({"oneOf":[
        object(json!({"library":segment,"id":segment,"revision":revision}), &["library","id","revision"]),
        object(json!({"id":{"const":crate::board::FOOTPRINT_DEFINITION},"revision":{"const":crate::board::FOOTPRINT_REVISION}}), &["id","revision"]),
    ]});
    let mut tags = vec![object(json!({"tag":{"const":"board.origin"}}), &["tag"])];
    for tag in ["fp.origin", "pad.center", "pad.corner", "pad.edge", "courtyard.edge"] {
        let mut properties = json!({"tag":{"const":tag},"fp":{"type":"integer","minimum":1,"maximum":crate::MAX_OBJECT_ID},"def":key});
        let mut required = vec!["tag", "fp", "def"];
        if tag.starts_with("pad.") {
            properties["pad"] = json!({"type":"string","pattern":"^[A-Za-z0-9+_-]{1,8}$"});
            required.push("pad");
        }
        if tag == "pad.corner" {
            properties["corner"] = json!({"enum":["ll","lr","ur","ul"]});
            required.push("corner");
        }
        if tag.ends_with(".edge") {
            properties["side"] = json!({"enum":["left","right","top","bottom"]});
            required.push("side");
        }
        if tag == "courtyard.edge" {
            properties["rect"] = json!({"type":"integer","minimum":0,"maximum":63});
            required.push("rect");
        }
        tags.push(object(properties, &required));
    }
    json!({"oneOf":tags})
}

/// Reference authoring has no driving, literal target, historical witness or placement fields.
fn draft_schema() -> Value {
    object(
        json!({"kind":{"enum":["horizontal","vertical","aligned","point_edge","gap","align_h","align_v"]},
        "refs":{"type":"array","minItems":2,"maxItems":2,"items":ref_schema()},"suppressed":{"type":"boolean"},
        "label":{"anyOf":[crate::point_schema(),{"type":"null"}]}}),
        &["kind", "refs"],
    )
}

impl Parameter {
    /// Schema is shared by discovery and the committed compatibility snapshot.
    pub(crate) fn schema(self) -> Value {
        let id = json!({"type":"string","format":"uuid"});
        match self {
            Self::Id => id,
            Self::Ids | Self::RemapIds => json!({"type":"array","maxItems":self.cap(),"uniqueItems":true,"items":id}),
            Self::Remaps => {
                let alternatives = [
                    object(json!({"kind":{"const":"correspond"}}), &["kind"]),
                    object(json!({"kind":{"const":"keep_lost"}}), &["kind"]),
                    object(json!({"kind":{"const":"repick"},"reference":ref_schema()}), &["kind", "reference"]),
                ];
                json!({"type":"array","maxItems":self.cap(),"items":object(json!({"relation":id,"pick":{"enum":[0,1]},
                    "choice":{"oneOf":alternatives}}), &["relation","pick","choice"])})
            }
            Self::Draft => draft_schema(),
            Self::Edit => object(
                json!({"repick":draft_schema(),"suppressed":{"type":"boolean"},"label":{"anyOf":[crate::point_schema(),{"type":"null"}]}}),
                &[],
            ),
            Self::Intent => {
                let items: Vec<_> = SOURCES
                    .iter()
                    .filter_map(|source| crate::command(source.id()))
                    .map(|native| {
                        object(
                            json!({"command":{"const":native.id},"params":native.parameter_schema(Some("expected_revision"))}),
                            &["command", "params"],
                        )
                    })
                    .collect();
                json!({"oneOf":items})
            }
        }
    }

    /// The ID-array cap is measured in emitted rows, at most two remaps per relation.
    fn cap(self) -> usize {
        if matches!(self, Self::Remaps | Self::RemapIds) { 2 * super::MAX_RELATIONS } else { super::MAX_RELATIONS }
    }

    /// Validate actual typed envelopes, not merely an outer object check.
    pub(crate) fn validate(self, value: &Value) -> Result<(), &'static str> {
        let valid = match self {
            Self::Id => id(value).is_ok(),
            Self::Ids | Self::RemapIds => value
                .as_array()
                .is_some_and(|v| v.len() <= self.cap() && v.iter().map(id).collect::<Result<BTreeSet<_>, _>>().is_ok_and(|ids| ids.len() == v.len())),
            Self::Remaps => transaction::choices(Some(value)).is_ok(),
            Self::Draft => serde_json::from_value::<Draft>(value.clone()).is_ok_and(|draft| draft.refs.iter().all(Reference::valid)),
            Self::Edit => serde_json::from_value::<Changes>(value.clone())
                .is_ok_and(|edit| edit.repick.is_none_or(|draft| draft.refs.iter().all(Reference::valid))),
            Self::Intent => serde_json::from_value::<Intent>(value.clone()).is_ok_and(|i| !i.params.contains_key("expected_revision")),
        };
        valid.then_some(()).ok_or("must be a bounded typed relation input")
    }
}

pub(crate) fn invalid(error: impl std::fmt::Display) -> CommandError {
    CommandError::InvalidParams(error.to_string())
}

/// Parse a non-nil relation or review ID without treating it as a native object address.
fn id(value: &Value) -> Result<Uuid, CommandError> {
    value.as_str().and_then(|s| Uuid::parse_str(s).ok()).filter(|id| !id.is_nil()).ok_or_else(|| invalid("invalid relation or review ID"))
}

/// Author a reference from live geometry only; the historical witness is engine-generated.
fn picked(project: &crate::Project, id: Uuid, draft: Draft) -> Result<Relation, CommandError> {
    use transaction::{pick_literal, reason};
    let measure = catalog::measure(project, draft.kind, &draft.refs)
        .ok_or_else(|| reason("lost_reference", "Pick two distinct native bodies and compatible currently available features.", Value::Null))?;
    let (value, side) = pick_literal(draft.kind, measure)?;
    Ok(Relation {
        id,
        kind: draft.kind,
        refs: draft.refs,
        datum: 0,
        value,
        side,
        driving: false,
        suppressed: draft.suppressed,
        state: State::Ok,
        label: draft.label,
        last_resolved: measure.witnesses(),
    })
}

#[derive(Debug)]
pub(crate) struct Generic {
    base: Base,
    command: &'static str,
    args: Value,
    intent_digest: String,
    prepared: Prepared,
    clipboard: Option<ClipboardResult>,
}

/// The same success envelope is sized at preview and at Apply. The decoded text is borrowed
/// from the capture; the advisory is a separate bounded engine field, not clipboard data.
#[derive(Serialize)]
struct ApplyResult<'a> {
    revision: u64,
    intent_digest: &'a str,
    candidate_digest: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clipboard: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clipboard_notice: Option<ClipboardNotice>,
}

impl Generic {
    /// Freeze exactly the fields returned by successful Apply, including the published revision.
    fn result(&self) -> ApplyResult<'_> {
        ApplyResult {
            revision: (self.base.revision + 1).max(self.prepared.highest),
            intent_digest: &self.intent_digest,
            candidate_digest: self.prepared.digest.as_deref(),
            clipboard: self.clipboard.as_ref().map(|result| result.text.as_str()),
            clipboard_notice: self.clipboard.as_ref().and_then(|result| result.notice),
        }
    }
}

#[derive(Serialize)]
struct IntentKey<'a> {
    command: &'a str,
    project: Uuid,
    board: Uuid,
    args: &'a Value,
    choices: &'a [transaction::Choice],
}

/// A failed or category-mismatched review is not approval authority.
pub(crate) fn stale() -> CommandError {
    transaction::reason("stale_preview", "The preview is unknown, stale or replaced. Preview the edit again.", Value::Null)
}

impl Session {
    /// Read the exact certified generic snapshot only for byte equivalence assertions.
    #[cfg(test)]
    pub(crate) fn checked_generic_candidate(&self) -> Option<&Arc<crate::Project>> {
        match &self.update_plan {
            Some(ReviewSlot::GenericPrepared(p)) if p.base.matches(self) => Some(&p.prepared.snapshot),
            _ => None,
        }
    }

    /// Revision-only dispatch must not publish a checked Cut while dropping its cached text.
    pub(crate) fn checked_relation_apply_has_clipboard(&self, params: &Value) -> bool {
        matches!(&self.update_plan, Some(ReviewSlot::GenericPrepared(p))
            if p.clipboard.is_some() && id(&params["token"]).is_ok_and(|token| token == p.base.token) && p.base.matches(self))
    }

    /// Execute a P1.1 engine command and return its bounded typed result.
    pub(crate) fn relation_command(&mut self, id: &str, params: &Value, limit: usize) -> Result<Value, CommandError> {
        let limit = limit.min(RESULT_BYTES);
        self.refuse_stale(id, params)?;
        crate::check_params(crate::command(id).ok_or_else(|| CommandError::Unknown(id.into()))?, params)?;
        match id {
            PREVIEW => self.preview_relation(params, limit),
            APPLY => self.apply_relation_preview(params, limit),
            CANCEL => {
                let token = self::id(&params["token"])?;
                if !matches!(&self.update_plan, Some(ReviewSlot::GenericPrepared(p)) if p.base.token == token && p.base.matches(self)) {
                    return Err(stale());
                }
                let result = transaction::bounded_result(json!({"revision":self.revision}), limit)?;
                self.update_plan = None;
                Ok(result)
            }
            LIST | FEATURES => {
                let project = self.open_project()?;
                project.resolve_board(params.get("board").and_then(Value::as_str))?;
                let result = if id == LIST {
                    json!({"revision":self.revision,"relations":super::advisory::RelationAdvisory::capture(project).unwrap_or_default()})
                } else {
                    let features = if let Some(fp) = params.get("fp").and_then(Value::as_str) {
                        let key = crate::schematic::wiring::parse_id(fp, "footprint-").ok_or_else(|| invalid("invalid footprint ID"))?;
                        let f = catalog::footprint(&project.board, key).ok_or_else(|| CommandError::NotFound("unknown footprint ID".into()))?;
                        catalog::features(project, f)
                    } else {
                        let r = Reference::BoardOrigin {};
                        catalog::resolve(project, &r).map(|w| vec![(r, w)]).unwrap_or_default()
                    };
                    let features: Vec<_> =
                        features.into_iter().map(|(reference, witness)| json!({"reference":reference,"witness":witness})).collect();
                    json!({"revision":self.revision,"features":features})
                };
                transaction::bounded_result(result, limit)
            }
            ADD | EDIT | DELETE => self.edit_relation(id, params, limit),
            _ => Err(CommandError::Unknown(id.into())),
        }
    }

    /// Prepare once on a private Outer capture session. None of the allowlisted handlers
    /// can publish to the live session or write the external clipboard.
    fn preview_relation(&mut self, params: &Value, limit: usize) -> Result<Value, CommandError> {
        self.check_revision(params)?;
        let Intent { command, params: mut args } = serde_json::from_value(params["intent"].clone()).map_err(invalid)?;
        let command = command.id();
        let snapshot = self.snapshot().ok_or_else(|| CommandError::Unavailable("no project is open".into()))?;
        let target = snapshot.resolve_board(args.get("board").and_then(Value::as_str))?;
        if !snapshot.board.arcs.is_empty() {
            return Err(transaction::reason("arc_board", crate::board::arc::UNSUPPORTED, Value::Null));
        }
        let native = crate::command(command).ok_or_else(|| CommandError::Unknown(command.into()))?;
        if native.params.iter().any(|p| p.name == "board") && args.get("board").is_none_or(Value::is_string) {
            args.insert("board".into(), json!(target.board));
        }
        if crate::sheets::scoped_command(command) && args.get("sheet").is_none_or(Value::is_string) {
            let target = snapshot.resolve_sheet(args.get("sheet").and_then(Value::as_str))?;
            args.insert("sheet".into(), json!(target.sheet));
        }
        let choices = transaction::choices(params.get("remap_choices"))?;
        let mut args = Value::Object(args);
        let intent_digest = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&IntentKey { command, project: target.project, board: target.board, args: &args, choices: &choices })
                    .map_err(invalid)?
            )
        );
        args["expected_revision"] = json!(self.revision);
        crate::check_params(native, &args)?;
        let mut capture = Session {
            libraries: self.libraries.clone(),
            project: Some(snapshot.clone()),
            revision: self.revision,
            history_root_revision: self.history_root_revision,
            relation_capture: Some((RelationInput { review: true, choices, ..RelationInput::default() }, None)),
            ..Session::default()
        };
        let clipboard = if command == crate::board::clipboard::CUT {
            Some(capture.cut_board(&args)?)
        } else if command == crate::clipboard::CUT {
            let text = capture.copy_schematic(&args, true)?;
            capture.apply_command(command, &args)?;
            Some(ClipboardResult { text, notice: None })
        } else {
            capture.apply_command(command, &args)?;
            None
        };
        let prepared = capture.relation_capture.take().and_then(|(_, p)| p).ok_or_else(|| invalid("this intent has no committable edit"))?;
        let pending = Generic { base: Base::new(self, snapshot, target)?, command, args, intent_digest, prepared, clipboard };
        // A smaller caller budget here limits only Preview. Every public Execute Apply has
        // the fixed engine budget; never displace a prior token with an impossible result.
        if pending.clipboard.is_some() {
            transaction::check_result(&pending.result(), RESULT_BYTES)?;
        }
        let result = transaction::bounded_result(
            json!({"revision":self.revision,"relation_preview":{
            "token":pending.base.token,"project":target.project,"board":target.board,"intent_digest":pending.intent_digest,
            "candidate_digest":pending.prepared.digest,"disclosure":pending.prepared.disclosure}}),
            limit,
        )?;
        self.update_plan = Some(ReviewSlot::GenericPrepared(pending));
        Ok(result)
    }

    /// Return the exact result-bearing command envelope before consuming its one capability.
    fn apply_relation_preview(&mut self, params: &Value, limit: usize) -> Result<Value, CommandError> {
        self.check_revision(params)?;
        let token = id(&params["token"])?;
        let Some(ReviewSlot::GenericPrepared(p)) = &self.update_plan else { return Err(stale()) };
        if p.base.token != token || !p.base.matches(self) {
            return Err(stale());
        }
        p.prepared.disclosure.consent(params, false)?;
        let result = p.result();
        transaction::check_result(&result, limit)?;
        let response = serde_json::to_value(result).map_err(invalid)?;
        let Some(ReviewSlot::GenericPrepared(p)) = self.update_plan.take() else { return Err(stale()) };
        self.publish(p.prepared);
        self.report_committed(p.command, &p.args);
        Ok(response)
    }

    /// Apply bounded reference-only CRUD through the ordinary immutable prepare/publish path.
    fn edit_relation(&mut self, command: &str, params: &Value, limit: usize) -> Result<Value, CommandError> {
        self.check_revision(params)?;
        let project = self.open_project()?;
        project.resolve_board(params.get("board").and_then(Value::as_str))?;
        if !project.board.arcs.is_empty() {
            return Err(transaction::reason("arc_board", crate::board::arc::UNSUPPORTED, Value::Null));
        }
        let mut rows = project.board.relations.as_deref().map(|r| r.relations.clone()).unwrap_or_default();
        let response_id = match command {
            ADD => {
                if rows.len() == super::MAX_RELATIONS {
                    return Err(CommandError::CapExceeded("board relation count cap exceeded".into()));
                }
                let new = crate::fresh_id()?;
                rows.push(picked(project, new, serde_json::from_value(params["relation"].clone()).map_err(invalid)?)?);
                Some(new)
            }
            EDIT => {
                let edit: Changes = serde_json::from_value(params["edit"].clone()).map_err(invalid)?;
                let id = self::id(&params["id"])?;
                let row = rows.iter_mut().find(|r| r.id == id).ok_or_else(|| CommandError::NotFound("unknown relation ID".into()))?;
                if let Some(draft) = edit.repick {
                    *row = picked(project, id, draft)?;
                }
                if let Some(suppressed) = edit.suppressed {
                    row.suppressed = suppressed;
                }
                if let Some(label) = edit.label {
                    row.label = label;
                }
                Some(id)
            }
            DELETE => {
                let ids: BTreeSet<Uuid> = serde_json::from_value(params["ids"].clone()).map_err(invalid)?;
                if !ids.iter().all(|id| rows.iter().any(|r| &r.id == id)) {
                    return Err(CommandError::NotFound("unknown relation ID".into()));
                }
                rows.retain(|r| !ids.contains(&r.id));
                None
            }
            _ => return Err(CommandError::Unknown(command.into())),
        };
        if rows.as_slice() == project.board.relations.as_ref().map_or(&[][..], |r| r.relations.as_slice()) {
            return transaction::bounded_result(json!({"revision":self.revision,"id":response_id}), limit);
        }
        let mut next = project.clone();
        Arc::make_mut(&mut next.board).relations = (!rows.is_empty()).then(|| Arc::new(Relations { relations: rows }));
        let prepared = self.prepare_with(next, &RelationInput::default())?;
        prepared.disclosure.require_no_decisions()?;
        let response = transaction::bounded_result(json!({"revision":(self.revision + 1).max(prepared.highest),"id":response_id}), limit)?;
        self.publish(prepared);
        self.report_committed(command, params);
        Ok(response)
    }
}

/// Route only the P1.1 engine contracts and leave unrelated dispatch untouched.
pub(crate) fn handles(id: &str) -> bool {
    matches!(id, ADD | EDIT | DELETE | PREVIEW | APPLY | CANCEL | LIST | FEATURES)
}
