//! Original command-built acceptance across capture, reviewed PCB updates and native storage.
// Fixture helpers fail immediately when a production command or required record is unavailable.
#![allow(clippy::unwrap_used)]
use pcbcraft_engine::{
    ADD_DRAWING, ASSIGN_FOOTPRINT, CONNECT_PINS, DELETE_SYMBOL, DELETE_WIRE, DRAW_WIRE, PLACE_ASSIGNED_FOOTPRINT, PLACE_RESISTOR, Project,
    SET_FOOTPRINT_TRANSFORM, Session, SheetTarget,
    board::{
        identity::CREATE_BOARD,
        ratsnest::{MissingReason, Ratsnest},
        setup::LayerPurpose,
        update::{APPLY_UPDATE, Baseline, CANCEL_UPDATE, PREVIEW_UPDATE},
        view::BoardView,
    },
    hierarchy, net_labels,
    persistence::{Documents, encode_documents, encode_packed},
    schematic::{Point, Terminal, WireEndpoint},
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::Path, sync::Arc};

#[derive(Clone, Copy, Debug)]
enum Capture {
    Flat,
    Hierarchical,
}

#[derive(Clone, Debug)]
struct Part {
    sheet: SheetTarget,
    symbol: String,
    component: String,
}

/// Every authored fixture edit goes through the same revision-checked command path as GUI/MCP.
fn edit(session: &mut Session, command: &str, mut params: Value) {
    if command.starts_with("pcb.") && command != CREATE_BOARD {
        params["board"] = json!(board(session).target.board);
    }
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

/// Resolve only the explicit board registered in this project; schematic-only state is asserted separately.
fn board(session: &Session) -> BoardView<'_> {
    let project = session.project().unwrap();
    project.board_view(project.board_target().unwrap()).unwrap()
}

/// Read canonical bytes without depending on view projection or transient preview inspection.
fn documents(session: &Session) -> Documents {
    encode_documents(session.snapshot().unwrap()).unwrap()
}

/// Inspect exact persisted accepted membership through the production baseline type;
/// neither array position nor net names establish the electrical group identity.
fn accepted_groups(session: &Session) -> BTreeSet<BTreeSet<(String, String)>> {
    let board: Value = serde_json::from_slice(&documents(session)["board.json"]).unwrap();
    let baseline: Baseline = serde_json::from_value(board["baseline"].clone()).unwrap();
    baseline
        .nets
        .into_iter()
        .map(|net| net.terminals.into_iter().map(|key| (format!("component-{}", key.component), key.terminal)).collect())
        .collect()
}

/// Return an owned placement identity/pose so acceptance cannot accidentally compare borrowed old data.
fn pose(session: &Session, component: &str) -> (String, Point, u16) {
    let placement = board(session).footprints().find(|footprint| footprint.component == component).unwrap();
    (placement.id, placement.position, placement.rotation)
}

/// Construct a resistor on an explicitly addressed sheet and retain stable command identities.
fn resistor(session: &mut Session, sheet: SheetTarget, x_nm: i64) -> Part {
    edit(session, PLACE_RESISTOR, json!({"sheet":sheet.id(),"x_nm":x_nm,"y_nm":0}));
    let projection = session.project().unwrap().for_sheet(sheet).unwrap();
    let symbol = projection.symbols().last().unwrap();
    Part { sheet, symbol: symbol.id(), component: symbol.component_id() }
}

/// Locate the exact logical terminal on its original sheet after offline reopening.
fn terminal(project: &Project, part: &Part, number: &str) -> Terminal {
    project
        .for_sheet(part.sheet)
        .unwrap()
        .symbols()
        .iter()
        .find(|symbol| symbol.id() == part.symbol)
        .unwrap()
        .terminals()
        .into_iter()
        .find(|terminal| terminal.number() == number)
        .unwrap()
}

/// Draw an explicit hierarchy interface wire with an orthogonal path through public geometry APIs.
fn interface_wire(session: &mut Session, part: &Part, number: &str, endpoint: WireEndpoint) {
    let projection = session.project().unwrap().for_sheet(part.sheet).unwrap();
    let pin = terminal(&projection, part, number);
    let from = projection.terminal_position(pin).unwrap();
    let to = projection.endpoint_position(endpoint).unwrap();
    let bends = if from.x_nm != to.x_nm && from.y_nm != to.y_nm { vec![Point { x_nm: from.x_nm, y_nm: to.y_nm }] } else { Vec::new() };
    edit(session, DRAW_WIRE, json!({"sheet":part.sheet.id(),"from":pin.inspect(),"to":endpoint.inspect(),"bends":bends,"auto_heal":false}));
}

/// Build a real cross-sheet connection using either board-scoped global labels or a child interface.
fn circuit(capture: Capture) -> (Session, [Part; 3], Option<u64>) {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Forward acceptance"})).unwrap();
    assert!(session.project().unwrap().board_target().is_none());
    let main = session.project().unwrap().sheet_target();
    match capture {
        Capture::Flat => edit(&mut session, pcbcraft_engine::sheets::CREATE_SHEET, json!({"name":"Power"})),
        Capture::Hierarchical => edit(&mut session, hierarchy::CREATE_CHILD, json!({"sheet":main.id(),"name":"Power","x_nm":0,"y_nm":0})),
    }
    let power = session.project().unwrap().sheets().find(|sheet| sheet.name == "Power").unwrap().target;
    let first = resistor(&mut session, main, 10_000_000);
    let second = resistor(&mut session, power, 10_000_000);
    let removable = resistor(&mut session, main, 30_000_000);
    edit(
        &mut session,
        CONNECT_PINS,
        json!({"sheet":main.id(),"from":{"symbol":first.symbol,"terminal":"2"},"to":{"symbol":removable.symbol,"terminal":"1"}}),
    );
    let label = match capture {
        Capture::Flat => {
            for part in [&first, &second] {
                let projection = session.project().unwrap().for_sheet(part.sheet).unwrap();
                let at = projection.terminal_position(terminal(&projection, part, "1")).unwrap();
                edit(&mut session, net_labels::ADD, json!({"sheet":part.sheet.id(),"name":"SUPPLY","style":"global","x_nm":at.x_nm,"y_nm":at.y_nm}));
            }
            None
        }
        Capture::Hierarchical => {
            edit(&mut session, hierarchy::ADD_LABEL, json!({"sheet":power.id(),"name":"SUPPLY","direction":"passive","x_nm":0,"y_nm":0}));
            let id = session.project().unwrap().for_sheet(power).unwrap().hierarchical_labels()[0].id;
            interface_wire(&mut session, &first, "1", WireEndpoint::SheetPin(id));
            interface_wire(&mut session, &second, "1", WireEndpoint::Label(id));
            Some(id)
        }
    };
    (session, [first, second, removable], label)
}

/// Select an exact original pinned catalog package; no external files or library implementations are consulted.
fn assign(session: &mut Session, part: &Part, definition: &str, reversed: bool) {
    let package = session.project().unwrap().footprint_catalog().into_iter().find(|package| package.id == definition).unwrap();
    let mapping = if reversed { json!({"1":"2","2":"1"}) } else { json!({"1":"1","2":"2"}) };
    edit(
        session,
        ASSIGN_FOOTPRINT,
        json!({"component":part.component,"library":package.library,"definition":package.id,"definition_revision":package.revision,"mapping":mapping}),
    );
}

/// Preview must preserve authored bytes and the immutable snapshot; its report is explicit query output.
fn preview(session: &mut Session) -> Value {
    let before = session.snapshot().unwrap();
    let bytes = documents(session);
    let revision = session.revision();
    let result = session.execute(PREVIEW_UPDATE, &json!({"board":board(session).target.board,"expected_revision":revision})).unwrap();
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), revision);
    assert_eq!(documents(session), bytes);
    result["board_update_preview"].clone()
}

/// Apply uses exactly the reviewed opaque token and explicit physical-board target.
fn accept(session: &mut Session, report: &Value) {
    edit(session, APPLY_UPDATE, json!({"plan_token":report["plan_token"]}));
}

/// Filesystem folders and archives are generated by production codecs, then independently reopened.
/// A second folder save must stage no payload/journal, and archive encoding must reproduce exact bytes.
fn round_trip(root: &Path, name: &str, session: &Session) -> Arc<Project> {
    let expected = documents(session);
    let folder = root.join(name);
    pcbcraft_storage::save(&folder, None, &expected, &mut |_| Ok(())).unwrap();
    assert_eq!(pcbcraft_storage::load(&folder).unwrap(), expected);
    let reopened = pcbcraft_storage::open(&folder.join("project.json")).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), expected);
    let mut checkpoints = Vec::new();
    pcbcraft_storage::save(&folder, Some(&expected), &expected, &mut |phase| {
        checkpoints.push(phase.to_owned());
        Ok(())
    })
    .unwrap();
    assert!(checkpoints.is_empty(), "no-op save staged a write: {checkpoints:?}");
    let archive = root.join(format!("{name}.pc"));
    let bytes = encode_packed(session.snapshot().unwrap()).unwrap();
    fs::write(&archive, &bytes).unwrap();
    let packed = pcbcraft_storage::open(&archive).unwrap();
    assert_eq!(encode_documents(packed.clone()).unwrap(), expected);
    assert_eq!(encode_packed(packed).unwrap(), bytes);
    reopened
}

/// Acceptance exercises one user's complete update cycle, in both supported sheet organizations,
/// including stale authority, physical retention, offline pending intent and no-diff persistence.
#[test]
fn schematic_to_updatable_single_board_survives_flat_and_hierarchical_offline_workflows() {
    for capture in [Capture::Flat, Capture::Hierarchical] {
        let output = tempfile::tempdir().unwrap();
        let (mut session, parts, label) = circuit(capture);
        let schematic = round_trip(output.path(), "schematic-only", &session);
        assert!(!documents(&session).contains_key("board.json"));
        session.open_snapshot(schematic);
        edit(&mut session, CREATE_BOARD, json!({"name":"Controller"}));
        for part in &parts[..2] {
            assign(&mut session, part, "resistor-0402", false);
        }
        let blocked = preview(&mut session);
        assert_eq!(blocked["can_apply"], false);
        let snapshot = session.snapshot().unwrap();
        assert!(
            session
                .apply(
                    APPLY_UPDATE,
                    &json!({"board":board(&session).target.board,"plan_token":blocked["plan_token"],"expected_revision":session.revision()})
                )
                .is_err()
        );
        assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
        session.apply(CANCEL_UPDATE, &json!({"plan_token":blocked["plan_token"]})).unwrap();
        assign(&mut session, &parts[2], "resistor-0402", false);
        let cancelled = preview(&mut session);
        assert_eq!(cancelled["can_apply"], true);
        let before = documents(&session);
        session.apply(CANCEL_UPDATE, &json!({"plan_token":cancelled["plan_token"]})).unwrap();
        assert_eq!(documents(&session), before);
        let first = preview(&mut session);
        let revision = session.revision();
        accept(&mut session, &first);
        assert_eq!(session.revision(), revision + 1);
        assert_eq!(board(&session).footprints().count(), 0);
        assert_eq!(board(&session).ratsnest().unwrap().missing.len(), 4);
        for (index, part) in parts.iter().enumerate() {
            edit(&mut session, PLACE_ASSIGNED_FOOTPRINT, json!({"component":part.component,"x_nm":index as i64*10_000_000,"y_nm":0}));
        }
        let selected = pcbcraft_engine::selection::Selection::one_symbol(parts[1].symbol.clone());
        let second_id = pose(&session, &parts[1].component).0;
        assert_eq!(board(&session).footprints_for_selection(&selected), BTreeSet::from([second_id.clone()]));
        assert_eq!(board(&session).selection_for_footprints(&BTreeSet::from([second_id])), selected);
        for part in &parts {
            let pin = terminal(session.project().unwrap(), part, "1");
            let pad = session.project().unwrap().pad_for_terminal(pin).unwrap();
            assert_eq!(session.project().unwrap().terminal_for_pad(pad), Some(pin));
            assert_eq!(pad.inspect()["board"], json!(board(&session).target.board));
        }
        let placement = pose(&session, &parts[0].component).0;
        edit(&mut session, SET_FOOTPRINT_TRANSFORM, json!({"footprint":placement,"x_nm":3_810_000,"y_nm":2_540_000,"rotation":90}));
        let edge = board(&session).layers().into_iter().find(|layer| layer.purpose == LayerPurpose::Outline).unwrap().id;
        edit(
            &mut session,
            ADD_DRAWING,
            json!({"layer":edge,"closed":true,"points":[{"x_nm":-5_000_000,"y_nm":-5_000_000},{"x_nm":30_000_000,"y_nm":-5_000_000},{"x_nm":30_000_000,"y_nm":10_000_000},{"x_nm":-5_000_000,"y_nm":10_000_000}]}),
        );
        let preserved_pose = pose(&session, &parts[0].component);
        let other_pose = pose(&session, &parts[1].component);
        let graphics = board(&session).drawings();
        let accepted: Ratsnest = board(&session).ratsnest().unwrap();
        assert_eq!(accepted.lines.len(), 2);
        assert!(accepted.missing.is_empty());
        let stale = preview(&mut session);
        let added = resistor(&mut session, parts[0].sheet, 45_000_000);
        let before_stale = session.snapshot().unwrap();
        assert!(
            session
                .apply(
                    APPLY_UPDATE,
                    &json!({"board":board(&session).target.board,"plan_token":stale["plan_token"],"expected_revision":session.revision()})
                )
                .is_err()
        );
        assert!(Arc::ptr_eq(&before_stale, &session.snapshot().unwrap()));
        assign(&mut session, &added, "resistor-0402", false);
        let package = session.project().unwrap().footprint_catalog().into_iter().find(|package| package.id == "resistor-0603").unwrap();
        let before_invalid = session.snapshot().unwrap();
        assert!(session.apply(ASSIGN_FOOTPRINT,&json!({"board":board(&session).target.board,"component":parts[0].component,"library":package.library,"definition":package.id,"definition_revision":package.revision,"mapping":{"1":"1","2":"1"},"expected_revision":session.revision()})).is_err());
        assert!(Arc::ptr_eq(&before_invalid, &session.snapshot().unwrap()));
        assign(&mut session, &parts[0], "resistor-0603", true);
        edit(
            &mut session,
            CONNECT_PINS,
            json!({"sheet":parts[0].sheet.id(),"from":{"symbol":parts[0].symbol,"terminal":"2"},"to":{"symbol":added.symbol,"terminal":"1"}}),
        );
        edit(&mut session, DELETE_SYMBOL, json!({"sheet":parts[2].sheet.id(),"symbol":parts[2].symbol}));
        match label {
            None => {
                let projection = session.project().unwrap().for_sheet(parts[1].sheet).unwrap();
                let id = projection.net_labels()[0].object_id();
                let at = projection.terminal_position(terminal(&projection, &parts[1], "2")).unwrap();
                edit(
                    &mut session,
                    net_labels::SET,
                    json!({"sheet":parts[1].sheet.id(),"id":id,"name":"SUPPLY","style":"global","x_nm":at.x_nm,"y_nm":at.y_nm}),
                );
            }
            Some(label) => {
                let wire = session.project().unwrap().for_sheet(parts[1].sheet).unwrap().wires()[0].id();
                edit(&mut session, DELETE_WIRE, json!({"sheet":parts[1].sheet.id(),"wire":wire}));
                interface_wire(&mut session, &parts[1], "2", WireEndpoint::Label(label));
            }
        }
        assert_eq!(board(&session).ratsnest().unwrap(), accepted);
        assert_eq!(pose(&session, &parts[0].component), preserved_pose);
        assert_eq!(board(&session).footprints().find(|footprint| footprint.component == parts[0].component).unwrap().definition, "resistor-0402");
        assert!(board(&session).footprints().find(|footprint| footprint.component == parts[2].component).unwrap().pending_removal);
        let pending = round_trip(output.path(), "pending-update", &session);
        session.open_snapshot(pending);
        assert!(session.cached_board_update().is_none());
        assert_eq!(board(&session).ratsnest().unwrap(), accepted);
        let review = preview(&mut session);
        assert_eq!(review["can_apply"], true);
        assert_eq!(review["component_details"][&parts[0].component]["proposed"]["package"]["definition"], "resistor-0603");
        let before_apply = documents(&session);
        let revision = session.revision();
        accept(&mut session, &review);
        assert_eq!(session.revision(), revision + 1);
        assert_eq!(pose(&session, &parts[0].component), preserved_pose);
        assert_eq!(pose(&session, &parts[1].component), other_pose);
        assert_eq!(board(&session).drawings(), graphics);
        assert!(board(&session).footprints().all(|footprint| footprint.component != parts[2].component));
        assert_eq!(board(&session).footprints().find(|footprint| footprint.component == parts[0].component).unwrap().definition, "resistor-0603");
        let pin = terminal(session.project().unwrap(), &parts[0], "1");
        assert_eq!(session.project().unwrap().pad_for_terminal(pin).unwrap().inspect()["pad"], "2");
        let updated = board(&session).ratsnest().unwrap();
        assert_ne!(updated.source, accepted.source);
        assert_eq!(updated.source.capture_id.to_string(), review["source"]["capture_id"]);
        assert_eq!((updated.net_count, updated.terminal_count, updated.lines.len()), (2, 4, 1));
        let connected: BTreeSet<_> = updated
            .lines
            .iter()
            .flat_map(|line| [&line.from, &line.to])
            .map(|endpoint| (format!("component-{}", endpoint.terminal.component), endpoint.terminal.terminal.clone()))
            .collect();
        assert_eq!(connected, BTreeSet::from([(parts[0].component.clone(), "1".into()), (parts[1].component.clone(), "2".into())]));
        assert!(board(&session).footprints().all(|footprint| footprint.component != added.component));
        assert_eq!(updated.missing.len(), 1);
        assert_eq!(updated.missing[0].reason, MissingReason::Unplaced);
        assert_eq!(updated.missing[0].terminal.component.to_string(), added.component.strip_prefix("component-").unwrap());
        assert_eq!(
            accepted_groups(&session),
            BTreeSet::from([
                BTreeSet::from([(parts[0].component.clone(), "1".into()), (parts[1].component.clone(), "2".into())]),
                BTreeSet::from([(parts[0].component.clone(), "2".into()), (added.component.clone(), "1".into())]),
            ])
        );
        let after_apply = documents(&session);
        session.apply("edit.undo", &json!({})).unwrap();
        assert_eq!(documents(&session), before_apply);
        assert_eq!(board(&session).ratsnest().unwrap(), accepted);
        session.apply("edit.redo", &json!({})).unwrap();
        assert_eq!(documents(&session), after_apply);
        let noop = preview(&mut session);
        let before_noop = session.snapshot().unwrap();
        let revision = session.revision();
        accept(&mut session, &noop);
        assert_eq!(session.revision(), revision);
        assert!(Arc::ptr_eq(&before_noop, &session.snapshot().unwrap()));
        assert_eq!(documents(&session), after_apply);
        let reopened = round_trip(output.path(), "accepted-update", &session);
        let target = reopened.board_target().unwrap();
        assert_eq!(reopened.board_view(target).unwrap().ratsnest().unwrap(), updated);
    }
}

/// Change one component-owned attribute through its schematic placement, retaining
/// other metadata so population and physical membership remain independent.
fn attribute(session: &mut Session, part: &Part, change: impl FnOnce(&mut pcbcraft_engine::schematic::fields::Attributes)) {
    let component = session.project().unwrap().components().find(|component| component.id() == part.component).unwrap();
    let mut metadata = component.metadata.clone();
    change(&mut metadata.attributes);
    let params =
        json!({"sheet":part.sheet.id(),"symbol":part.symbol,"reference":component.reference(),"value":component.value(),"metadata":metadata});
    edit(session, pcbcraft_engine::SET_SYMBOL_FIELDS, params);
}

/// Board exclusion must filter the resolved full graph, not discard wires meeting
/// at an excluded terminal; DNP does not remove copper or waive package requirements.
#[test]
fn physical_eligibility_and_population_survive_review_and_offline_reopen() {
    let output = tempfile::tempdir().unwrap();
    let mut session = Session::default();
    session.apply("project.new", &json!({"name":"Physical eligibility acceptance"})).unwrap();
    let sheet = session.project().unwrap().sheet_target();
    let dnp = resistor(&mut session, sheet, 0);
    let excluded = resistor(&mut session, sheet, 10_000_000);
    let ordinary = resistor(&mut session, sheet, 20_000_000);
    for (from, to) in [(&dnp, &excluded), (&excluded, &ordinary)] {
        edit(
            &mut session,
            CONNECT_PINS,
            json!({"sheet":sheet.id(),"from":{"symbol":from.symbol,"terminal":"1"},"to":{"symbol":to.symbol,"terminal":"1"}}),
        );
    }
    attribute(&mut session, &dnp, |attributes| attributes.do_not_populate = true);
    attribute(&mut session, &excluded, |attributes| attributes.exclude_from_board = true);
    edit(&mut session, CREATE_BOARD, json!({"name":"Controller"}));
    assign(&mut session, &ordinary, "resistor-0402", false);
    let missing_dnp = preview(&mut session);
    assert_eq!(missing_dnp["can_apply"], false);
    let blockers = &session.cached_board_update().unwrap().changes.blockers;
    assert_eq!(blockers.len(), 1);
    assert!(
        matches!(&blockers[0], pcbcraft_engine::board::update::Blocker::MissingPackage { component } if format!("component-{component}") == dnp.component)
    );
    assign(&mut session, &dnp, "resistor-0402", false);
    let first = preview(&mut session);
    assert_eq!(first["can_apply"], true);
    assert!(session.cached_board_update().unwrap().changes.blockers.is_empty());
    assert!(session.project().unwrap().footprint_assignments().iter().all(|assignment| assignment["component"] != excluded.component));
    accept(&mut session, &first);
    assert_eq!(session.inspect()["project"]["board"]["accepted_baseline"]["component_count"], 2);
    for (index, part) in [&dnp, &ordinary].into_iter().enumerate() {
        edit(&mut session, PLACE_ASSIGNED_FOOTPRINT, json!({"component":part.component,"x_nm":index as i64 * 20_000_000,"y_nm":0}));
    }
    let accepted = board(&session).ratsnest().unwrap();
    assert_eq!((accepted.net_count, accepted.terminal_count, accepted.lines.len()), (1, 2, 1));
    assert!(accepted.missing.is_empty());
    let members: BTreeSet<_> = accepted
        .lines
        .iter()
        .flat_map(|line| [&line.from, &line.to])
        .map(|endpoint| (format!("component-{}", endpoint.terminal.component), endpoint.terminal.terminal.clone()))
        .collect();
    assert_eq!(members, BTreeSet::from([(dnp.component.clone(), "1".into()), (ordinary.component.clone(), "1".into())]));
    let dnp_pose = pose(&session, &dnp.component);
    let ordinary_pose = pose(&session, &ordinary.component);
    let loaded = round_trip(output.path(), "excluded-and-dnp", &session);
    session.open_snapshot(loaded);
    assert_eq!(board(&session).ratsnest().unwrap(), accepted);
    assert!(session.project().unwrap().components().find(|component| component.id() == dnp.component).unwrap().metadata.attributes.do_not_populate);
    assert!(
        session
            .project()
            .unwrap()
            .components()
            .find(|component| component.id() == excluded.component)
            .unwrap()
            .metadata
            .attributes
            .exclude_from_board
    );
    attribute(&mut session, &excluded, |attributes| attributes.exclude_from_board = false);
    let blocked = preview(&mut session);
    assert_eq!(blocked["can_apply"], false);
    let blockers = &session.cached_board_update().unwrap().changes.blockers;
    assert_eq!(blockers.len(), 1);
    assert!(
        matches!(&blockers[0], pcbcraft_engine::board::update::Blocker::MissingPackage { component } if format!("component-{component}") == excluded.component)
    );
    let snapshot = session.snapshot().unwrap();
    assert!(
        session
            .apply(
                APPLY_UPDATE,
                &json!({"board":board(&session).target.board,"plan_token":blocked["plan_token"],"expected_revision":session.revision()})
            )
            .is_err()
    );
    assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
    assert_eq!(board(&session).ratsnest().unwrap(), accepted);
    let loaded = round_trip(output.path(), "unexcluded-without-package", &session);
    session.open_snapshot(loaded);
    assert!(session.cached_board_update().is_none());
    assert_eq!(preview(&mut session)["can_apply"], false);
    assign(&mut session, &excluded, "resistor-0402", false);
    let review = preview(&mut session);
    assert_eq!(review["can_apply"], true);
    accept(&mut session, &review);
    assert_eq!(pose(&session, &dnp.component), dnp_pose);
    assert_eq!(pose(&session, &ordinary.component), ordinary_pose);
    assert!(board(&session).footprints().all(|footprint| footprint.component != excluded.component));
    let unplaced = board(&session).ratsnest().unwrap();
    assert_eq!((unplaced.net_count, unplaced.terminal_count, unplaced.missing.len()), (1, 3, 1));
    assert_eq!(format!("component-{}", unplaced.missing[0].terminal.component), excluded.component);
    assert_eq!(unplaced.missing[0].reason, MissingReason::Unplaced);
    assert_eq!(unplaced.missing[0].terminal.terminal, "1");
    edit(&mut session, PLACE_ASSIGNED_FOOTPRINT, json!({"component":excluded.component,"x_nm":10_000_000,"y_nm":0}));
    let placed = board(&session).ratsnest().unwrap();
    assert_eq!((placed.terminal_count, placed.lines.len()), (3, 2));
    assert!(placed.missing.is_empty());
    assert_eq!(board(&session).footprints().count(), 3);
    let members: BTreeSet<_> = placed
        .lines
        .iter()
        .flat_map(|line| [&line.from, &line.to])
        .map(|endpoint| (format!("component-{}", endpoint.terminal.component), endpoint.terminal.terminal.clone()))
        .collect();
    assert_eq!(
        members,
        BTreeSet::from([(dnp.component.clone(), "1".into()), (excluded.component.clone(), "1".into()), (ordinary.component.clone(), "1".into())])
    );
    let loaded = round_trip(output.path(), "all-eligible-with-dnp", &session);
    let target = loaded.board_target().unwrap();
    assert_eq!(loaded.board_view(target).unwrap().ratsnest().unwrap(), placed);
    assert!(loaded.components().find(|component| component.id() == dnp.component).unwrap().metadata.attributes.do_not_populate);
}
