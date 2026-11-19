use super::*;
use crate::{
    APPLY_UPDATE, ASSIGN_FOOTPRINT, CONNECT_PINS, DELETE_SYMBOL, DELETE_WIRE, PLACE_ASSIGNED_FOOTPRINT, PLACE_RESISTOR, PLACE_SYMBOL, PREVIEW_UPDATE,
    SET_SYMBOL_FIELDS,
    board::definition::basic_components,
    netlist::FORMAT,
    netlist::tests::{Fixture, apply, check, fixture, fixture_with_symbol, integer_spellings, schema, schema_v2, terminal, validate, wire},
    persistence::{decode_documents, encode_documents},
    schematic::WireEndpoint,
};
use std::collections::BTreeSet;
use uuid::Uuid;

const SIX_PAD: &str = include_str!("../../../../../examples/native-parts/footprints/illustrative-six-pad.json");

/// The schematic fixture with every part given a package, the board updated once and U1 placed.
/// The two resistors use different packages so their pads stay distinguishable.
fn synced() -> Fixture {
    with_packages(fixture())
}

/// Assign every component's package, accept the update and place U1 on the board.
fn with_packages(mut fixture: Fixture) -> Fixture {
    let s = &mut fixture.session;
    s.execute(
        "library.load_source",
        &json!({"source":"packages","documents":{
            "library.json":r#"{"format":"pcbcraft-library/1","id":"units","name":"Unit tests","license":"MIT OR Apache-2.0"}"#,
            "footprints/six-pad.json":SIX_PAD.replace("parts-demo", "units"),
        }}),
    )
    .unwrap();
    let six: Value = (1..=6).map(|pad| (pad.to_string(), json!(pad.to_string()))).collect::<serde_json::Map<_, _>>().into();
    apply(
        s,
        ASSIGN_FOOTPRINT,
        json!({"component":fixture.u1,"library":"units","definition":"illustrative-six-pad","definition_revision":1,"mapping":six}),
    );
    for (component, package) in [(&fixture.r1, "resistor-0402"), (&fixture.r2, "resistor-0603")] {
        let definition = basic_components(package).unwrap();
        apply(
            s,
            ASSIGN_FOOTPRINT,
            json!({"component":component,"library":definition.library,"definition":definition.id,
                "definition_revision":definition.revision,"mapping":{"1":"1","2":"2"}}),
        );
    }
    update(s);
    apply(s, PLACE_ASSIGNED_FOOTPRINT, json!({"component":fixture.u1,"x_nm":0,"y_nm":0}));
    fixture
}

/// Review and accept the pending board update, returning the reviewed changes.
fn update(s: &mut Session) -> Value {
    let preview = s.execute(PREVIEW_UPDATE, &json!({"expected_revision":s.revision()})).unwrap()["board_update_preview"].clone();
    s.apply(APPLY_UPDATE, &json!({"plan_token":preview["plan_token"],"expected_revision":s.revision()})).unwrap();
    preview["changes"].clone()
}

/// Read the preview's changes without accepting them.
fn pending(s: &mut Session) -> Value {
    s.execute(PREVIEW_UPDATE, &json!({"expected_revision":s.revision()})).unwrap()["board_update_preview"]["changes"].clone()
}

fn board(s: &Session) -> Export {
    let project = s.project().unwrap();
    project.board_view(project.board_target().unwrap()).unwrap().netlist().unwrap()
}

fn schematic(s: &Session) -> Netlist {
    s.project().unwrap().netlist().unwrap().netlist
}

/// The logical content both documents share: components without their schematic units or
/// physical package, and every net and open terminal. The format tag differs by basis.
fn logical(netlist: &Netlist) -> Value {
    let mut value = serde_json::to_value(netlist).unwrap();
    for key in ["format", "basis", "board"] {
        value.as_object_mut().unwrap().remove(key);
    }
    for component in value["components"].as_array_mut().unwrap() {
        component["units"] = json!([]);
        component.as_object_mut().unwrap().remove("package");
        for terminal in component["terminals"].as_array_mut().unwrap() {
            terminal.as_object_mut().unwrap().remove("pads");
        }
    }
    value
}

/// Each terminal with the set of terminals sharing its net (only itself when unconnected).
fn pin_nets(netlist: &Netlist) -> BTreeMap<(String, String), BTreeSet<(String, String)>> {
    let pair = |t: &TerminalRef| (t.part.clone(), t.terminal.clone());
    let mut map = BTreeMap::new();
    for net in &netlist.nets {
        let members: BTreeSet<_> = net.terminals.iter().map(pair).collect();
        for terminal in &net.terminals {
            map.insert(pair(terminal), members.clone());
        }
    }
    for terminal in &netlist.unconnected {
        map.insert(pair(terminal), BTreeSet::from([pair(terminal)]));
    }
    map
}

fn kinds(findings: &[Value]) -> Vec<&str> {
    findings.iter().map(|finding| finding["kind"].as_str().unwrap()).collect()
}

fn net_sets(nets: &Value) -> BTreeSet<BTreeSet<(String, String)>> {
    nets.as_array()
        .unwrap()
        .iter()
        .map(|net| {
            net["terminals"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| (format!("component-{}", t["component"]), t["terminal"].as_str().unwrap().to_owned()))
                .collect()
        })
        .collect()
}

fn symbol_of(s: &Session, sheet: crate::SheetTarget, component: &str) -> String {
    s.project().unwrap().for_sheet(sheet).unwrap().symbols().iter().find(|symbol| symbol.component_id() == component).unwrap().id()
}

/// Done test: in sync, the board netlist carries the schematic's components, nets and net IDs.
/// One wire not yet applied to the board is exactly the difference `pcb.preview_update`
/// reviews; applying it brings the two back together.
#[test]
fn board_netlist_matches_schematic_in_sync_and_shows_one_pending_change() {
    let Fixture { mut session, sheets, u1, r1, .. } = synced();
    let Export { netlist, findings } = board(&session);
    assert_eq!(netlist.basis, Basis::Accepted);
    assert_eq!(logical(&netlist), logical(&schematic(&session)));
    assert!(netlist.nets.iter().all(|net| net.id.is_some()));
    assert_eq!(findings, Vec::<Value>::new());
    validate_v2(&serde_json::to_value(&netlist).unwrap());

    // The package and pads come from the accepted footprint choices and the placement.
    let amplifier = netlist.components.iter().find(|part| part.id == u1).unwrap();
    let package = amplifier.package.as_ref().unwrap();
    assert_eq!((package.library.as_deref(), package.id.as_str(), package.revision), (Some("units"), "illustrative-six-pad", 1));
    assert!(package.footprint.as_deref().is_some_and(|id| id.starts_with("footprint-")));
    assert!(amplifier.terminals.iter().all(|t| t.pads.as_deref() == Some(std::slice::from_ref(&t.terminal))));
    let resistor = netlist.components.iter().find(|part| part.id == r1).unwrap().package.as_ref().unwrap();
    assert_eq!((resistor.id.as_str(), resistor.footprint.as_deref()), ("resistor-0402", None));

    // Join the open U1.4 to R1.1 without updating the board.
    let (unit_a, r1_symbol) = (symbol_of(&session, sheets[0], &u1), symbol_of(&session, sheets[0], &r1));
    apply(
        &mut session,
        CONNECT_PINS,
        json!({"sheet":sheets[0].id(),"from":{"symbol":unit_a,"terminal":"4"},"to":{"symbol":r1_symbol,"terminal":"1"}}),
    );
    let Export { netlist: accepted, findings } = board(&session);
    let current = schematic(&session);
    let (before, after) = (pin_nets(&accepted), pin_nets(&current));
    let differing: BTreeSet<_> = after.iter().filter(|(pin, net)| before.get(*pin) != Some(net)).map(|(pin, _)| pin.clone()).collect();
    let pin = |part: &str, terminal: &str| (part.to_owned(), terminal.to_owned());
    assert_eq!(differing, BTreeSet::from([pin(&u1, "2"), pin(&u1, "4"), pin(&r1, "1")]));
    assert_eq!(before.len(), after.len());

    // The nets that differ are the ones the update preview would remove and add.
    let old_net = BTreeSet::from([pin(&u1, "2"), pin(&r1, "1")]);
    let new_net = BTreeSet::from([pin(&u1, "2"), pin(&u1, "4"), pin(&r1, "1")]);
    let joined = |netlist: &Netlist| -> BTreeSet<BTreeSet<_>> {
        netlist.nets.iter().map(|net| net.terminals.iter().map(|t| (t.part.clone(), t.terminal.clone())).collect()).collect()
    };
    let (accepted_nets, current_nets) = (joined(&accepted), joined(&current));
    assert_eq!(accepted_nets.difference(&current_nets).collect::<Vec<_>>(), vec![&old_net]);
    assert_eq!(current_nets.difference(&accepted_nets).collect::<Vec<_>>(), vec![&new_net]);
    let changes = pending(&mut session);
    assert_eq!(net_sets(&changes["removed_nets"]), BTreeSet::from([old_net.clone()]));
    assert_eq!(net_sets(&changes["added_nets"]), BTreeSet::from([new_net]));
    assert_eq!(changes["components"], json!([]));

    // Only the stale net loses its ID; both findings name the difference.
    for net in &accepted.nets {
        let members: BTreeSet<_> = net.terminals.iter().map(|t| (t.part.clone(), t.terminal.clone())).collect();
        assert_eq!(net.id.is_none(), members == old_net, "{}", net.name);
    }
    assert_eq!(kinds(&findings), vec!["pending_net", "pending_update"]);
    assert_eq!((findings[1]["removed_nets"].as_u64(), findings[1]["added_nets"].as_u64()), (Some(1), Some(1)));

    update(&mut session);
    let Export { netlist, findings } = board(&session);
    assert_eq!(logical(&netlist), logical(&schematic(&session)));
    assert_eq!(findings, Vec::<Value>::new());
}

/// Repeated exports, a save and reopen, and the execute and apply paths give the same bytes
/// without changing the project or its history. Unknown boards retain the shared NotFound category.
#[test]
fn board_export_is_byte_identical_and_edits_nothing() {
    let Fixture { mut session, .. } = synced();
    let first = session.export_board_netlist(&json!({})).unwrap().0;
    assert_eq!(session.export_board_netlist(&json!({})).unwrap().0, first);
    let project = session.project().unwrap();
    assert!(first.name.ends_with("-netlist.json"));
    assert!(first.name.contains(&file_part(project.board_view(project.board_target().unwrap()).unwrap().name())));
    let reopened = decode_documents(&encode_documents(session.snapshot().unwrap()).unwrap()).unwrap();
    let view = reopened.board_view(reopened.board_target().unwrap()).unwrap();
    assert_eq!(view.netlist().unwrap().netlist.to_text().unwrap(), first.contents);

    let snapshot = session.snapshot().unwrap();
    let revision = session.revision();
    let result = session.execute(EXPORT_BOARD_NETLIST, &json!({})).unwrap();
    assert_eq!(result["netlist"]["files"], json!([{"name":first.name,"contents":first.contents}]));
    let dry = session.execute(EXPORT_BOARD_NETLIST, &json!({"dry_run":true})).unwrap();
    assert_eq!(dry["netlist"]["files"], json!([{"name":first.name}]));
    assert_eq!(dry["netlist"]["findings"], result["netlist"]["findings"]);
    assert_eq!(session.apply(EXPORT_BOARD_NETLIST, &json!({})).unwrap(), revision);
    assert!(Arc::ptr_eq(&snapshot, &session.snapshot().unwrap()));
    assert_eq!(session.revision(), revision);
    let unknown = session.execute(EXPORT_BOARD_NETLIST, &json!({"board":Uuid::nil().to_string()}));
    assert!(matches!(unknown, Err(CommandError::NotFound(ref message)) if message == "unknown board ID"), "{unknown:?}");

    let text = &first.contents;
    validate_v2(&serde_json::from_str(text).unwrap());
    assert_eq!(Netlist::parse(text).unwrap(), board(&session).netlist);
}

/// A dry apply can project a board even when rendering the document would fail.
#[test]
fn board_netlist_apply_dry_run_skips_rendering() {
    let Fixture { mut session, .. } = synced();
    let mut project = (*session.snapshot().unwrap()).clone();
    let baseline = Arc::make_mut(Arc::make_mut(&mut project.board).baseline.as_mut().unwrap());
    // Deliberately make the projected capture fail document serialization, not projection.
    baseline.source.capture_id = Uuid::nil();
    session.open_snapshot(Arc::new(project));
    let revision = session.revision();
    let dry = json!({"dry_run":true});
    assert!(session.execute(EXPORT_BOARD_NETLIST, &dry).is_ok());
    assert_eq!(session.apply(EXPORT_BOARD_NETLIST, &dry).unwrap(), revision);
    for params in [json!({}), json!({"dry_run":false})] {
        let full = session.apply(EXPORT_BOARD_NETLIST, &params);
        assert!(matches!(full, Err(CommandError::Unavailable(ref message)) if message.contains("invalid board source capture identity")), "{full:?}");
    }
    assert_eq!(session.revision(), revision);
}

/// Before its first update a board without placements has accepted nothing; with placements
/// but no baseline its accepted state is unknown and the export refuses.
#[test]
fn board_without_update_is_empty_or_refused() {
    let Fixture { session, .. } = fixture();
    let Export { netlist, findings } = board(&session);
    assert!(netlist.components.is_empty() && netlist.nets.is_empty() && netlist.unconnected.is_empty());
    assert_eq!(netlist.board.as_ref().unwrap().source, None);
    assert_eq!(kinds(&findings), vec!["pending_update"]);
    validate_v2(&serde_json::to_value(&netlist).unwrap());

    let Fixture { session, .. } = synced();
    let mut project = (*session.snapshot().unwrap()).clone();
    Arc::make_mut(&mut project.board).baseline = None;
    let view = project.board_view(project.board_target().unwrap()).unwrap();
    assert!(view.netlist().unwrap_err().contains("baseline"));
}

/// A part deleted from the schematic stays on the board, with its accepted terminals and
/// pads, until the next update removes it.
#[test]
fn deleted_part_stays_until_the_update() {
    let Fixture { mut session, sheets, r2, .. } = synced();
    assert_eq!(board(&session).netlist.components.iter().find(|part| part.id == r2).unwrap().kind, ComponentKind::Part);
    let symbol = symbol_of(&session, sheets[2], &r2);
    apply(&mut session, DELETE_SYMBOL, json!({"sheet":sheets[2].id(),"symbol":symbol}));
    let Export { netlist, findings } = board(&session);
    let load = netlist.components.iter().find(|part| part.id == r2).unwrap();
    assert_eq!(load.reference, "R2");
    assert_eq!(load.kind, ComponentKind::Unknown);
    let terminals: Vec<_> = load.terminals.iter().map(|t| (t.terminal.as_str(), t.name.as_deref(), t.electrical_type, t.pads.clone())).collect();
    assert_eq!(terminals, vec![("1", None, None, Some(vec!["1".to_owned()])), ("2", None, None, Some(vec!["2".to_owned()]))]);
    assert_eq!(kinds(&findings), vec!["pending_removal", "pending_net", "pending_update"]);
    assert_eq!(findings[0]["part"], json!(r2));
    validate_v2(&serde_json::to_value(&netlist).unwrap());
    assert_eq!(Netlist::parse(&netlist.to_text().unwrap()).unwrap(), netlist);
}

/// Include a power port in the board explicitly and accept its two-pad package through
/// the shared commands. Power ports are excluded from boards by default.
fn included_power() -> (Session, String, String) {
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":true})).unwrap();
    let definition = session.libraries().symbol("power", "gnd", 1).unwrap();
    let digest = crate::library::sources::symbol_digest(&definition).unwrap();
    apply(
        &mut session,
        PLACE_SYMBOL,
        json!({"library":"power","definition":"gnd","definition_revision":1,"definition_digest":digest,"x_nm":0,"y_nm":0}),
    );
    let project = session.project().unwrap();
    let symbol = &project.symbols()[0];
    let component = project.component(symbol).unwrap();
    let (symbol, id, reference, value, mut metadata) =
        (symbol.id(), component.id(), component.reference().to_owned(), component.value().to_owned(), component.metadata.clone());
    assert!(metadata.attributes.exclude_from_board);
    metadata.attributes.exclude_from_board = false;
    apply(&mut session, SET_SYMBOL_FIELDS, json!({"symbol":symbol,"reference":reference,"value":value,"metadata":metadata}));
    let package = basic_components("resistor-0603").unwrap();
    apply(
        &mut session,
        ASSIGN_FOOTPRINT,
        json!({"component":id,"library":package.library,"definition":package.id,"definition_revision":package.revision,"mapping":{"1":["1","2"]}}),
    );
    update(&mut session);
    (session, id, symbol)
}

/// Deleting an included power port cannot invent an ordinary-part role. The baseline
/// retains its terminals, pads and authored net through save/reopen until removal is accepted.
#[test]
fn included_power_role_is_unknown_until_pending_removal_is_accepted() {
    let (mut session, id, symbol) = included_power();
    let Export { netlist: before, findings } = board(&session);
    assert!(findings.is_empty(), "{findings:?}");
    assert_eq!(before.components.len(), 1);
    assert_eq!(before.components[0].kind, ComponentKind::Power);
    assert_eq!(schematic(&session).components[0].kind, ComponentKind::Power);
    assert!(before.components[0].terminals[0].electrical_type.is_some());
    assert_eq!(before.nets.len(), 1);
    assert_eq!(before.nets[0].names, ["GND"]);
    assert!(before.nets[0].id.is_some());
    apply(&mut session, DELETE_SYMBOL, json!({"symbol":symbol}));
    let Export { netlist, findings } = board(&session);
    assert_eq!(netlist.components.len(), 1);
    let component = &netlist.components[0];
    assert_eq!((component.id.as_str(), component.kind), (id.as_str(), ComponentKind::Unknown));
    assert_eq!(component.definition, before.components[0].definition);
    assert_eq!(component.package, before.components[0].package);
    assert_eq!(component.terminals.len(), 1);
    let terminal = &component.terminals[0];
    assert_eq!((terminal.terminal.as_str(), terminal.name.as_deref(), terminal.electrical_type), ("1", None, None));
    assert_eq!(terminal.pads, Some(vec!["1".to_owned(), "2".to_owned()]));
    assert_eq!(netlist.nets[0].terminals, before.nets[0].terminals);
    assert_eq!(netlist.nets[0].names, before.nets[0].names);
    assert_eq!(netlist.nets[0].id, None);
    assert_eq!(kinds(&findings), ["pending_removal", "pending_net", "pending_update"]);
    assert!(schematic(&session).components.is_empty());
    let text = netlist.to_text().unwrap();
    validate_v2(&serde_json::from_str(&text).unwrap());
    assert_eq!(Netlist::parse(&text).unwrap(), netlist);
    let reopened = decode_documents(&encode_documents(session.snapshot().unwrap()).unwrap()).unwrap();
    let reopened_export = reopened.board_view(reopened.board_target().unwrap()).unwrap().netlist().unwrap();
    assert_eq!(reopened_export.netlist.to_text().unwrap(), text);
    assert_eq!(reopened_export.findings, findings);
    session.open_snapshot(reopened);
    update(&mut session);
    let Export { netlist, findings } = board(&session);
    assert!(netlist.components.is_empty() && netlist.nets.is_empty() && netlist.unconnected.is_empty());
    assert!(findings.is_empty(), "{findings:?}");
}

/// Fault-inject each accepted-definition mismatch into the live component of a command-built
/// snapshot. The live role must not replace the unknown accepted role. This is not a symbol
/// replacement command or a native-file fixture; the baseline and stored history are unchanged.
#[test]
fn mismatching_live_definitions_do_not_supply_the_accepted_role() {
    let (session, id, _) = included_power();
    let snapshot = session.snapshot().unwrap();
    let ordinal = snapshot.components.values().find(|part| part.id() == id).unwrap().ordinal;
    let before = board(&session).netlist;
    #[derive(Clone, Copy, Debug)]
    enum Mismatch {
        Library,
        Id,
        Revision,
        Terminals,
    }
    for mismatch in [Mismatch::Library, Mismatch::Id, Mismatch::Revision, Mismatch::Terminals] {
        let mut definition = crate::library::parse_symbol(include_str!("../../../../../libraries/power/symbols/gnd.json")).unwrap();
        definition.library = "power".to_owned();
        definition.behavior = None;
        match mismatch {
            Mismatch::Library => definition.library = "different".to_owned(),
            Mismatch::Id => definition.id = "different".to_owned(),
            Mismatch::Revision => definition.revision += 1,
            Mismatch::Terminals => {
                definition.pins[0].number = "2".to_owned();
                definition.units[0].pins[0].number = "2".to_owned();
            }
        }
        assert!(definition.power_terminal().is_none());
        let mut project = (*snapshot).clone();
        Arc::make_mut(&mut project.components).get_mut(&ordinal).unwrap().definition = Arc::new(definition);
        let export = project.board_view(project.board_target().unwrap()).unwrap().netlist().unwrap();
        let component = &export.netlist.components[0];
        assert_eq!(component.kind, ComponentKind::Unknown, "{mismatch:?}");
        assert_eq!(component.definition, before.components[0].definition, "{mismatch:?}");
        assert_eq!(component.package, before.components[0].package, "{mismatch:?}");
        assert_eq!(component.terminals[0].terminal, "1", "{mismatch:?}");
        assert!(component.terminals[0].name.is_none() && component.terminals[0].electrical_type.is_none(), "{mismatch:?}");
        validate_v2(&serde_json::to_value(&export.netlist).unwrap());
    }
}

/// A matching live symbol supplies native terminal order. After pending deletion, the
/// accepted baseline supplies lexical order and no longer has terminal names or types.
#[test]
fn nonlexical_native_terminal_order_falls_back_to_lexical_after_pending_deletion() {
    let mut symbol: Value = serde_json::from_str(include_str!("../../../../../examples/native-parts/symbols/dual-amplifier.json")).unwrap();
    symbol["terminals"].as_array_mut().unwrap().rotate_right(1);
    let Fixture { mut session, sheets, u1, .. } = with_packages(fixture_with_symbol(&symbol.to_string()));
    let before = board(&session).netlist;
    let component = before.components.iter().find(|part| part.id == u1).unwrap();
    assert_eq!(component.terminals.iter().map(|pin| pin.terminal.as_str()).collect::<Vec<_>>(), ["6", "1", "2", "3", "4", "5"]);
    assert!(component.terminals.iter().all(|pin| pin.electrical_type.is_some()));
    assert_eq!(logical(&before), logical(&schematic(&session)));
    for sheet in [sheets[0], sheets[1]] {
        let symbol = symbol_of(&session, sheet, &u1);
        apply(&mut session, DELETE_SYMBOL, json!({"sheet":sheet.id(),"symbol":symbol}));
    }
    let Export { netlist, findings } = board(&session);
    let component = netlist.components.iter().find(|part| part.id == u1).unwrap();
    assert_eq!(component.terminals.iter().map(|pin| pin.terminal.as_str()).collect::<Vec<_>>(), ["1", "2", "3", "4", "5", "6"]);
    assert!(component.terminals.iter().all(|pin| pin.name.is_none() && pin.electrical_type.is_none()));
    assert!(findings.iter().any(|finding| finding["kind"] == "pending_removal" && finding["part"] == u1));
    validate_v2(&serde_json::to_value(&netlist).unwrap());
    let text = netlist.to_text().unwrap();
    assert_eq!(Netlist::parse(&text).unwrap().to_text().unwrap(), text);
    let reopened = decode_documents(&encode_documents(session.snapshot().unwrap()).unwrap()).unwrap();
    assert_eq!(reopened.board_view(reopened.board_target().unwrap()).unwrap().netlist().unwrap().netlist.to_text().unwrap(), text);
}

/// Accept a three-pad mapping for each R2 terminal; R2.2 starts as a pad-only net.
fn multipad() -> Fixture {
    let mut fixture = synced();
    apply(
        &mut fixture.session,
        ASSIGN_FOOTPRINT,
        json!({"component":fixture.r2,"library":"units","definition":"illustrative-six-pad","definition_revision":1,
            "mapping":{"1":["1","4","5"],"2":["2","3","6"]}}),
    );
    update(&mut fixture.session);
    fixture
}

/// An unwired terminal mapped to several pads still needs copper between them, so the board keeps
/// a net of that one terminal. The schematic has no such net: its id is null and a `pad_net`
/// finding names it, while the board is otherwise in sync.
#[test]
fn unwired_terminal_on_several_pads_is_a_pad_net() {
    let Fixture { session, r2, .. } = multipad();
    let s = &session;
    let Export { netlist, findings } = board(s);
    let pads_only: Vec<_> = netlist.nets.iter().filter(|net| net.terminals.len() == 1).collect();
    assert_eq!(pads_only.len(), 1, "{:?}", netlist.nets);
    let net = pads_only[0];
    assert_eq!((net.terminals[0].part.as_str(), net.terminals[0].terminal.as_str()), (r2.as_str(), "2"));
    assert_eq!(net.id, None);
    assert!(!netlist.unconnected.iter().any(|t| t.part == r2 && t.terminal == "2"));
    let resistor = netlist.components.iter().find(|component| component.id == r2).unwrap();
    let pads = resistor.terminals.iter().find(|t| t.terminal == "2").unwrap().pads.clone();
    assert_eq!(pads, Some(vec!["2".to_owned(), "3".to_owned(), "6".to_owned()]));
    assert_eq!(findings.iter().map(|f| f["kind"].as_str().unwrap()).collect::<Vec<_>>(), ["pad_net"]);
    assert_eq!(findings[0]["net"], json!(net.name));
    assert!(netlist.nets.iter().filter(|other| other.terminals.len() > 1).all(|other| other.id.is_some()));
    validate_v2(&serde_json::to_value(&netlist).unwrap());
}

fn multipad_net<'a>(export: &'a Export, part: &str) -> &'a Net {
    export.netlist.nets.iter().find(|net| net.terminals.as_slice() == [TerminalRef { part: part.to_owned(), terminal: "2".into() }]).unwrap()
}

/// A native reopen does not reclassify the accepted net or alter the serialized board files.
fn reopen(s: &mut Session, expected: &Export) {
    let docs = encode_documents(s.snapshot().unwrap()).unwrap();
    let reopened = decode_documents(&docs).unwrap();
    assert_eq!(encode_documents(Arc::clone(&reopened)).unwrap(), docs);
    s.open_snapshot(reopened);
    let actual = board(s);
    assert_eq!(actual.netlist, expected.netlist);
    assert_eq!(actual.findings, expected.findings);
}

/// A pending label cannot give an accepted pad-only net a current schematic UUID.
#[test]
fn accepted_pad_net_cannot_borrow_a_pending_named_net_uuid() {
    let Fixture { mut session, sheets: [_, _, sheet], r2, .. } = multipad();
    let symbol = symbol_of(&session, sheet, &r2);
    let from = terminal(&session, sheet, &symbol, "2");
    apply(&mut session, crate::net_labels::ADD, json!({"sheet":sheet.id(),"name":"NEW","style":"local","x_nm":25_400_000,"y_nm":-12_700_000}));
    let label = session.project().unwrap().for_sheet(sheet).unwrap().net_labels()[0].id;
    wire(&mut session, sheet, from, WireEndpoint::NetLabel(label));
    let current = schematic(&session).nets.into_iter().find(|net| net.names == ["NEW"]).unwrap();
    assert!(current.id.is_some());
    let pending = board(&session);
    assert_eq!(multipad_net(&pending, &r2).id, None);
    assert_eq!(kinds(&pending.findings), ["pad_net", "pending_update"]);
    reopen(&mut session, &pending);
    update(&mut session);
    let accepted = board(&session);
    assert_eq!(multipad_net(&accepted, &r2), &current);
    assert!(accepted.findings.is_empty());
    reopen(&mut session, &accepted);
}

/// A disconnected schematic-origin net stays pending until Apply accepts its pad-only replacement.
#[test]
fn accepted_schematic_singleton_remains_pending_until_pad_only_origin_is_accepted() {
    let Fixture { mut session, sheets: [_, _, sheet], r2, .. } = multipad();
    apply(&mut session, PLACE_RESISTOR, json!({"sheet":sheet.id(),"x_nm":35_560_000,"y_nm":10_160_000}));
    let project = session.project().unwrap();
    let projection = project.for_sheet(sheet).unwrap();
    let extra = projection.symbols().last().unwrap();
    let part = project.component(extra).unwrap();
    let (symbol, reference, value, mut metadata) = (extra.id(), part.reference().to_owned(), part.value().to_owned(), part.metadata.clone());
    metadata.attributes.exclude_from_board = true;
    apply(&mut session, SET_SYMBOL_FIELDS, json!({"sheet":sheet.id(),"symbol":symbol,"reference":reference,"value":value,"metadata":metadata}));
    let from = terminal(&session, sheet, &symbol_of(&session, sheet, &r2), "2");
    let to = terminal(&session, sheet, &symbol, "1");
    wire(&mut session, sheet, from, to);
    let wire_id = session.project().unwrap().for_sheet(sheet).unwrap().wires().last().unwrap().id();
    update(&mut session);
    let accepted = board(&session);
    assert!(multipad_net(&accepted, &r2).id.is_some());
    assert!(multipad_net(&accepted, &r2).names.is_empty());
    assert!(accepted.findings.is_empty());
    reopen(&mut session, &accepted);
    apply(&mut session, DELETE_WIRE, json!({"sheet":sheet.id(),"wire":wire_id}));
    let pending = board(&session);
    assert_eq!(multipad_net(&pending, &r2).id, None);
    assert_eq!(kinds(&pending.findings), ["pending_net", "pending_update"]);
    reopen(&mut session, &pending);
    update(&mut session);
    let pads_only = board(&session);
    assert_eq!(multipad_net(&pads_only, &r2).id, None);
    assert_eq!(kinds(&pads_only.findings), ["pad_net"]);
    reopen(&mut session, &pads_only);
}

/// Older unnamed singletons export an unknown origin without upgrading their native bytes.
#[test]
fn older_unnamed_singletons_have_unknown_origin_and_round_trip_without_upgrade() {
    use crate::board::update::{DISPLAY_FORMAT, NET_ORIGIN_FORMAT};
    let Fixture { mut session, r2, .. } = multipad();
    let mut project = (*session.snapshot().unwrap()).clone();
    let baseline = Arc::make_mut(Arc::make_mut(&mut project.board).baseline.as_mut().unwrap());
    assert_eq!(baseline.format, NET_ORIGIN_FORMAT);
    let index = baseline.nets.iter().position(|net| net.origin.is_some()).unwrap();
    assert_eq!(baseline.nets[index].origin, Some(NetOrigin::PadOnly));
    baseline.nets[index].origin = None;
    assert!(baseline.encode().unwrap_err().contains("origin"));
    baseline.format = DISPLAY_FORMAT.to_owned();
    let legacy = baseline.encode().unwrap();
    assert_eq!(Baseline::decode(&legacy).unwrap().encode().unwrap(), legacy);
    session.open_snapshot(Arc::new(project));
    let old = board(&session);
    assert_eq!(multipad_net(&old, &r2).id, None);
    assert_eq!(kinds(&old.findings), ["net_origin_unknown", "pending_update"]);
    reopen(&mut session, &old);
}

/// The accepted partition includes terminals belonging only to an unplaced unit, while
/// the schematic partition keeps shared terminals drawn on the remaining placed unit.
#[test]
fn accepted_terminals_include_unplaced_units() {
    let mut fixture = fixture();
    let unit_b = symbol_of(&fixture.session, fixture.sheets[1], &fixture.u1);
    apply(&mut fixture.session, DELETE_SYMBOL, json!({"sheet":fixture.sheets[1].id(),"symbol":unit_b}));
    let Fixture { session, u1, .. } = with_packages(fixture);
    let accepted = board(&session).netlist;
    let schematic = schematic(&session);
    let partition = |netlist: &Netlist| {
        let mut counts = BTreeMap::<String, usize>::new();
        for terminal in netlist.nets.iter().flat_map(|net| &net.terminals).chain(&netlist.unconnected).filter(|terminal| terminal.part == u1) {
            *counts.entry(terminal.terminal.clone()).or_default() += 1;
        }
        counts
    };
    assert_eq!(partition(&accepted), BTreeMap::from(["1", "2", "3", "4", "5", "6"].map(|terminal| (terminal.to_owned(), 1))));
    assert_eq!(partition(&schematic), BTreeMap::from(["1", "2", "3", "4"].map(|terminal| (terminal.to_owned(), 1))));
    let unconnected: BTreeSet<_> =
        accepted.unconnected.iter().filter(|terminal| terminal.part == u1).map(|terminal| terminal.terminal.as_str()).collect();
    assert!(unconnected.contains("5") && unconnected.contains("6"), "{unconnected:?}");
    let component = schematic.components.iter().find(|part| part.id == u1).unwrap();
    assert_eq!(component.units.iter().map(|unit| unit.unit.as_str()).collect::<Vec<_>>(), ["a"]);
    assert_eq!(component.terminals.iter().filter(|terminal| terminal.terminal == "3" || terminal.terminal == "4").count(), 2);
    validate_v2(&serde_json::to_value(&accepted).unwrap());
    assert_eq!(Netlist::parse(&accepted.to_text().unwrap()).unwrap(), accepted);
}

/// The board's definition and package revisions accept integral JSON number spellings
/// through u32::MAX, reject fractions and overflow exactly, and write plain digits.
#[test]
fn board_integer_spellings_agree_with_the_schema() {
    let Fixture { session, .. } = synced();
    let text = board(&session).netlist.to_text().unwrap();
    assert!(text.contains("\"package\""));
    integer_spellings(&text, &schema_v2());
}

/// Optional board/package/pads fields cannot be null. Omission is valid only where that
/// basis permits it: all three on a schematic, only the package on an accepted component.
#[test]
fn optional_nonnullable_fields_distinguish_absence_null_and_value_on_both_bases() {
    let Fixture { session, .. } = synced();
    let accepted = serde_json::to_value(&board(&session).netlist).unwrap();
    let plain = serde_json::to_value(schematic(&session)).unwrap();
    for (base, schema, is_board) in [(&plain, schema(), false), (&accepted, schema_v2(), true)] {
        for pointer in ["/board", "/components/0/package", "/components/0/terminals/0/pads"] {
            let (parent, field) = pointer.rsplit_once('/').unwrap();
            let mut absent = base.clone();
            absent.pointer_mut(parent).unwrap().as_object_mut().unwrap().remove(field);
            let omission_allowed = !is_board || field == "package";
            assert_eq!(Netlist::parse(&absent.to_string()).is_ok(), omission_allowed, "{pointer} omitted on board={is_board}");
            assert_eq!(check(&schema, &schema, &absent, "$").is_ok(), omission_allowed, "{pointer} omitted on board={is_board}");
            if !omission_allowed {
                let error = Netlist::parse(&absent.to_string()).unwrap_err();
                assert!(error.contains(if field == "board" { "names its board" } else { "lists its pads" }), "{error}");
            }
            let mut null = absent.clone();
            null.pointer_mut(parent).unwrap()[field] = Value::Null;
            let error = Netlist::parse(&null.to_string()).unwrap_err();
            assert!(error.starts_with("invalid netlist: invalid type: null"), "{pointer}, board={is_board}: {error}");
            let schema_error = check(&schema, &schema, &null, "$").unwrap_err();
            if is_board {
                assert!(schema_error.starts_with(&format!("{}: null is not", schema_path(pointer))), "{schema_error}");
            } else {
                assert_eq!(schema_error, format!("{}: unexpected {field}", schema_path(parent)));
            }
            let mut present = absent;
            present.pointer_mut(parent).unwrap()[field] = accepted.pointer(pointer).unwrap().clone();
            assert_eq!(check(&schema, &schema, &present, "$").is_ok(), is_board, "{pointer}");
            let result = Netlist::parse(&present.to_string());
            assert_eq!(result.is_ok(), is_board, "{pointer}: {result:?}");
            if !is_board {
                assert!(result.unwrap_err().contains(if field == "pads" { "no pads" } else { "names a board or packages" }));
            }
        }
    }
}

/// Only an accepted netlist names a board.
#[test]
fn parse_refuses_a_board_on_the_wrong_basis() {
    let Fixture { session, .. } = synced();
    let text = board(&session).netlist.to_text().unwrap();
    let mut value: Value = serde_json::from_str(&text).unwrap();
    value.as_object_mut().unwrap().remove("board");
    assert!(Netlist::parse(&value.to_string()).unwrap_err().contains("board"));
    let mut value: Value = serde_json::from_str(&schematic(&session).to_text().unwrap()).unwrap();
    value["board"] = serde_json::from_str::<Value>(&text).unwrap()["board"].clone();
    assert!(Netlist::parse(&value.to_string()).unwrap_err().contains("board"));
}

/// The schema checker's path for a JSON pointer: `/nets/0/id` is `$.nets[0].id`.
fn schema_path(pointer: &str) -> String {
    pointer.split('/').skip(1).fold("$".to_owned(), |path, step| match step.parse::<usize>() {
        Ok(index) => format!("{path}[{index}]"),
        Err(_) => format!("{path}.{step}"),
    })
}

/// Panic unless the reader refuses the document as invalid (not as another format) and the
/// schema refuses it at `at` or inside it.
fn assert_both_refuse(result: &(Result<Netlist, String>, Result<(), String>), at: &str, case: &str) {
    let reader = result.0.as_ref().map(|_| ()).unwrap_err();
    assert!(reader.starts_with("invalid netlist: "), "{case}: reader said {reader}");
    let schema = result.1.as_ref().unwrap_err();
    assert!(
        schema.starts_with(&format!("{at}:")) || schema.starts_with(&format!("{at}.")) || schema.starts_with(&format!("{at}[")),
        "{case}: schema said {schema}"
    );
}

/// Set (`Some`) or remove (`None`) the value at a JSON pointer, then report what the reader
/// and the published schema each make of the result.
fn read_edited(text: &str, pointer: &str, value: Option<Value>) -> (Result<Netlist, String>, Result<(), String>) {
    let mut document: Value = serde_json::from_str(text).unwrap();
    let (parent, field) = pointer.rsplit_once('/').unwrap();
    let object = document.pointer_mut(parent).unwrap().as_object_mut().unwrap();
    assert!(object.contains_key(field), "{pointer} is not in the document");
    match value {
        Some(value) => object.insert(field.to_owned(), value),
        None => object.remove(field),
    };
    let schema = schema_v2();
    (Netlist::parse(&document.to_string()), check(&schema, &schema, &document, "$"))
}

/// Reader and schema agree at the edges of the board netlist's nullable, numeric and
/// formatted fields: nullable fields may be null but not omitted, revisions stop at the
/// largest u32, the source digest is a lowercase SHA-256, board UUIDs are hyphenated, the
/// added objects take no extra fields, and the reader refuses the fields of the other basis.
#[test]
fn reader_and_schema_agree_at_board_field_edges() {
    let Fixture { session, .. } = synced();
    let text = board(&session).netlist.to_text().unwrap();
    let both_accept = |result: (Result<Netlist, String>, Result<(), String>), case: &str| {
        assert!(result.0.is_ok() && result.1.is_ok(), "{case}: {result:?}");
    };
    // A refused edit is reported by the schema at the edited field's parent: an omitted
    // field as missing there, a bad value there or below it (`anyOf` reports at the parent).
    let both_refuse = |result: (Result<Netlist, String>, Result<(), String>), pointer: &str| {
        let parent = schema_path(pointer.rsplit_once('/').unwrap().0);
        assert_both_refuse(&result, &parent, pointer);
    };
    let omitted_is_missing = |result: (Result<Netlist, String>, Result<(), String>), pointer: &str| {
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        assert_eq!(result.1, Err(format!("{}: missing \"{field}\"", schema_path(parent))), "{pointer}");
        assert!(result.0.unwrap_err().starts_with("invalid netlist: "), "{pointer}");
    };
    for pointer in [
        "/board/source",
        "/components/0/package/library",
        "/components/0/package/footprint",
        "/components/0/terminals/0/name",
        "/components/0/terminals/0/electrical_type",
        "/nets/0/id",
    ] {
        both_accept(read_edited(&text, pointer, Some(Value::Null)), pointer);
        omitted_is_missing(read_edited(&text, pointer, None), pointer);
    }
    for pointer in ["/components/0/definition/revision", "/components/0/package/revision"] {
        both_accept(read_edited(&text, pointer, Some(json!(0))), pointer);
        both_accept(read_edited(&text, pointer, Some(json!(u32::MAX))), pointer);
        both_refuse(read_edited(&text, pointer, Some(json!(u64::from(u32::MAX) + 1))), pointer);
        both_refuse(read_edited(&text, pointer, Some(json!(-1))), pointer);
    }
    let digest = "/board/source/content_digest";
    both_accept(read_edited(&text, digest, Some(json!("0123456789abcdef".repeat(4)))), digest);
    for bad in ["0123456789ABCDEF".repeat(4), "0".repeat(63), "0".repeat(65)] {
        both_refuse(read_edited(&text, digest, Some(json!(bad))), digest);
    }
    let nil_capture = read_edited(&text, "/board/source/capture_id", Some(json!(Uuid::nil())));
    assert!(nil_capture.0.unwrap_err().starts_with("invalid netlist: invalid board source capture identity"));
    assert_eq!(nil_capture.1, Err("$.board.source: matches no alternative".to_owned()));
    let schema = schema_v2();
    let capture_rule = &schema["$defs"]["board"]["properties"]["source"]["anyOf"][0]["properties"]["capture_id"];
    assert_eq!(check(&schema, capture_rule, &json!(Uuid::nil()), "capture_id"), Err("capture_id: matches excluded value".to_owned()));
    check(&schema, capture_rule, &json!(Uuid::from_u128(1)), "capture_id").unwrap();
    both_accept(read_edited(&text, "/board/source/capture_id", Some(json!(Uuid::from_u128(1)))), "last nonnil capture");
    // Board UUIDs follow the rest of the document: hyphenated, in either case.
    let document: Value = serde_json::from_str(&text).unwrap();
    for pointer in ["/board/id", "/board/source/capture_id"] {
        let id = document.pointer(pointer).unwrap().as_str().unwrap().to_owned();
        both_accept(read_edited(&text, pointer, Some(json!(id.to_uppercase()))), pointer);
        for spelling in [id.replace('-', ""), format!("{{{id}}}"), format!("urn:uuid:{id}")] {
            both_refuse(read_edited(&text, pointer, Some(json!(spelling))), pointer);
        }
    }
    // The objects the board netlist adds are closed on both sides.
    for pointer in ["/board", "/board/source", "/components/0/package"] {
        let mut value = document.clone();
        value.pointer_mut(pointer).unwrap().as_object_mut().unwrap().insert("extra".into(), json!(1));
        let value = value.to_string();
        let result = (Netlist::parse(&value), check(&schema_v2(), &schema_v2(), &serde_json::from_str(&value).unwrap(), "$"));
        assert_both_refuse(&result, &schema_path(pointer), pointer);
    }
    both_accept(read_edited(&text, "/components/0/terminals/0/pads", Some(json!([]))), "empty pads");
    for pointer in ["/components/0/package/unconnected_pads", "/components/0/terminals/0/pads"] {
        both_refuse(read_edited(&text, pointer, Some(json!("1"))), pointer);
    }

    // Fields of the other basis: format 2 requires pads on every terminal and lists no units.
    let refused = |result: (Result<Netlist, String>, Result<(), String>), schema: &str| {
        assert_eq!(result.1.as_ref().unwrap_err(), schema);
        result.0.unwrap_err()
    };
    let no_pads = refused(read_edited(&text, "/components/0/terminals/0/pads", None), "$.components[0].terminals[0]: missing \"pads\"");
    assert!(no_pads.contains("pads"), "{no_pads}");
    let unit = json!([{"unit":"a","symbol":"symbol-1","sheet":Uuid::nil(),"occurrence":[]}]);
    let units = refused(read_edited(&text, "/components/0/units", Some(unit)), "$.components[0].units: more than 0 items");
    assert!(units.contains("units"), "{units}");
    let plain = schematic(&session).to_text().unwrap();
    let package = serde_json::from_str::<Value>(&text).unwrap()["components"][0]["package"].clone();
    let mut with_package: Value = serde_json::from_str(&plain).unwrap();
    with_package["components"][0]["package"] = package;
    assert!(Netlist::parse(&with_package.to_string()).unwrap_err().contains("packages"));
    let mut with_pads: Value = serde_json::from_str(&plain).unwrap();
    with_pads["components"][0]["terminals"][0]["pads"] = json!(["1"]);
    assert!(Netlist::parse(&with_pads.to_string()).unwrap_err().contains("pads"));
    let mut untyped: Value = serde_json::from_str(&plain).unwrap();
    untyped["components"][0]["terminals"][0]["electrical_type"] = Value::Null;
    assert!(Netlist::parse(&untyped.to_string()).unwrap_err().contains("electrical type"));
}

/// Panic unless the document satisfies the published format 2 schema.
fn validate_v2(value: &Value) {
    let schema = schema_v2();
    check(&schema, &schema, value, "$").unwrap();
}

/// The board netlist is format 2 and the schematic netlist stays format 1: each reads back
/// to the same bytes and satisfies its own schema, and neither satisfies the other's.
#[test]
fn board_netlist_is_format_2_and_schematic_netlist_stays_format_1() {
    let Fixture { session, .. } = synced();
    let plain = schematic(&session).to_text().unwrap();
    let read = Netlist::parse(&plain).unwrap();
    assert_eq!(read.format, FORMAT);
    assert_eq!(read.to_text().unwrap(), plain);
    validate(&serde_json::from_str(&plain).unwrap());

    let text = board(&session).netlist.to_text().unwrap();
    let read = Netlist::parse(&text).unwrap();
    assert_eq!(read.format, FORMAT_V2);
    assert_eq!(read.to_text().unwrap(), text);
    let value: Value = serde_json::from_str(&text).unwrap();
    validate_v2(&value);
    // The checker visits keywords in key order, so the first finding is the same each run.
    assert_eq!(check(&schema(), &schema(), &value, "$"), Err("$: unexpected board".to_owned()));
    let value: Value = serde_json::from_str(&plain).unwrap();
    assert_eq!(check(&schema_v2(), &schema_v2(), &value, "$"), Err("$.basis: not \"accepted\"".to_owned()));
}

/// The reader takes the tag the content needs and no other: format 1 for a schematic
/// netlist, format 2 for a board netlist, and refuses any later format by its tag.
#[test]
fn reader_checks_the_format_against_the_content() {
    let Fixture { session, .. } = synced();
    let retagged = |text: String, format: &str| {
        let mut value: Value = serde_json::from_str(&text).unwrap();
        value["format"] = json!(format);
        Netlist::parse(&value.to_string()).unwrap_err()
    };
    let plain = schematic(&session).to_text().unwrap();
    let text = board(&session).netlist.to_text().unwrap();
    assert!(retagged(plain.clone(), FORMAT_V2).contains("for the schematic basis, which takes `pcbcraft-netlist/1`"));
    assert!(retagged(text.clone(), FORMAT).contains("for the accepted basis, which takes `pcbcraft-netlist/2`"));
    for later in ["pcbcraft-netlist/3", "pcbcraft-netlist/10"] {
        assert_eq!(retagged(plain.clone(), later), format!("unsupported netlist format `{later}`"));
        assert_eq!(retagged(text.clone(), later), format!("unsupported netlist format `{later}`"));
    }
    // A later format with fields this reader does not know is still refused by its tag.
    let mut value: Value = serde_json::from_str(&text).unwrap();
    value["format"] = json!("pcbcraft-netlist/3");
    value["added_later"] = json!(true);
    assert_eq!(Netlist::parse(&value.to_string()).unwrap_err(), "unsupported netlist format `pcbcraft-netlist/3`");
    // The format 1 schema refuses a schematic netlist tagged format 2 by its tag alone.
    let mut value: Value = serde_json::from_str(&plain).unwrap();
    value["format"] = json!(FORMAT_V2);
    assert!(check(&schema(), &schema(), &value, "$").unwrap_err().starts_with("$.format"));
}

/// Direct readers and writers enforce tag and basis rules even for ordinary components.
#[test]
fn direct_serde_rejects_wrong_version_and_basis_fields_on_both_sides() {
    let Fixture { session, .. } = synced();
    let plain = schematic(&session);
    let accepted = board(&session).netlist;
    let mut retagged = accepted.clone();
    retagged.format = FORMAT.into();
    let mut untyped = plain.clone();
    untyped.components[0].terminals[0].electrical_type = None;
    let mut padded = plain.clone();
    padded.components[0].terminals[0].pads = Some(vec!["1".into()]);
    let mut packaged = plain.clone();
    packaged.components[0].package = accepted.components[0].package.clone();
    let mut absent_board = accepted.clone();
    absent_board.board = None;
    for (bad, mut raw, pointer, replacement, expected) in [
        (retagged, json!(accepted), "/format", json!(FORMAT), "for the accepted basis"),
        (untyped, json!(plain), "/components/0/terminals/0/electrical_type", Value::Null, "electrical type"),
        (padded, json!(plain), "/components/0/terminals/0/pads", json!(["1"]), "no pads"),
        (packaged, json!(plain), "/components/0/package", json!(accepted.components[0].package), "names a board or packages"),
        (absent_board, json!(accepted), "/board", Value::Null, "names its board"),
    ] {
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        let object = raw.pointer_mut(parent).unwrap().as_object_mut().unwrap();
        if pointer == "/board" {
            object.remove(field);
        } else {
            object.insert(field.into(), replacement);
        }
        let text = raw.to_string();
        assert!(serde_json::from_str::<Netlist>(&text).unwrap_err().to_string().contains(expected));
        assert!(serde_json::from_value::<Netlist>(raw).unwrap_err().to_string().contains(expected));
        assert!(serde_json::to_value(&bad).unwrap_err().to_string().contains(expected));
        assert!(bad.to_text().unwrap_err().contains(expected));
        let mut bytes = Vec::new();
        assert!(serde_json::to_writer(&mut bytes, &bad).unwrap_err().to_string().contains(expected));
        assert!(bytes.is_empty());
    }
}

/// All document records reject tuples in text and in serde JSON values.
#[test]
fn document_records_refuse_positional_arrays() {
    let Fixture { session, .. } = synced();
    let accepted = json!(board(&session).netlist);
    let plain = json!(schematic(&session));
    for (base, pointer, fields) in [
        (&accepted, "", &(["format", "project", "basis", "board", "components", "nets", "unconnected"])[..]),
        (&accepted, "/board", &["id", "name", "source"]),
        (&accepted, "/board/source", &["capture_id", "content_digest"]),
        (&accepted, "/components/0/package", &["library", "id", "revision", "footprint", "unconnected_pads"]),
        (&plain, "/project", &["id", "name"]),
        (&plain, "/components/0", &["id", "kind", "reference", "value", "definition", "units", "terminals"]),
        (&plain, "/components/0/definition", &["library", "id", "revision"]),
        (&plain, "/components/0/units/0", &["unit", "symbol", "sheet", "occurrence"]),
        (&plain, "/components/0/terminals/0", &["terminal", "name", "electrical_type"]),
        (&plain, "/nets/0", &["id", "name", "names", "terminals"]),
        (&plain, "/nets/0/terminals/0", &["part", "terminal"]),
        (&plain, "/unconnected/0", &["part", "terminal"]),
    ] {
        let row = base.pointer(pointer).unwrap();
        let mut mutated = base.clone();
        *mutated.pointer_mut(pointer).unwrap() = fields.iter().map(|&field| row[field].clone()).collect::<Vec<_>>().into();
        let text = mutated.to_string();
        assert!(Netlist::parse(&text).unwrap_err().contains("invalid type: sequence"), "{pointer}");
        assert!(serde_json::from_value::<Netlist>(mutated).unwrap_err().to_string().contains("invalid type: sequence"), "{pointer}");
    }
}

/// The schematic writer retains the format 1 field order and bytes for ordinary parts and
/// power ports. Unknown kinds belong to accepted format 2, including through direct serde.
#[test]
fn unknown_kinds_are_scoped_in_schema_reader_and_serde() {
    #[derive(serde::Serialize)]
    struct SchematicText<'a> {
        format: &'a str,
        project: &'a ProjectRef,
        basis: Basis,
        components: &'a [Component],
        nets: &'a [Net],
        unconnected: &'a [TerminalRef],
    }
    let reason = "unknown component kinds require an accepted format 2 netlist";
    for session in [fixture().session, included_power().0] {
        let plain = schematic(&session);
        let legacy = SchematicText {
            format: &plain.format,
            project: &plain.project,
            basis: plain.basis,
            components: &plain.components,
            nets: &plain.nets,
            unconnected: &plain.unconnected,
        };
        let expected = format!("{}\n", serde_json::to_string_pretty(&legacy).unwrap());
        assert_eq!(plain.to_text().unwrap(), expected);
        assert_eq!(Netlist::parse(&expected).unwrap(), plain);
        validate(&serde_json::from_str(&expected).unwrap());
        for (basis, format) in [(Basis::Schematic, FORMAT), (Basis::Schematic, FORMAT_V2), (Basis::Accepted, FORMAT)] {
            let mut bad = plain.clone();
            bad.basis = basis;
            bad.format = format.to_owned();
            bad.components[0].kind = ComponentKind::Unknown;
            assert_eq!(bad.to_text().unwrap_err(), reason);
            assert_eq!(serde_json::to_string(&bad).unwrap_err().to_string(), reason);
            assert_eq!(serde_json::to_value(&bad).unwrap_err().to_string(), reason);
            let mut output = Vec::new();
            assert_eq!(serde_json::to_writer(&mut output, &bad).unwrap_err().to_string(), reason);
            assert!(output.is_empty());
            let mut raw = serde_json::to_value(&plain).unwrap();
            raw["basis"] = json!(basis);
            raw["format"] = json!(format);
            raw["components"][0]["kind"] = json!("unknown");
            let text = raw.to_string();
            assert!(Netlist::parse(&text).unwrap_err().contains(reason));
            assert!(serde_json::from_str::<Netlist>(&text).unwrap_err().to_string().contains(reason));
            assert!(serde_json::from_slice::<Netlist>(text.as_bytes()).unwrap_err().to_string().contains(reason));
            assert!(serde_json::from_value::<Netlist>(raw.clone()).unwrap_err().to_string().contains(reason));
            if basis == Basis::Schematic && format == FORMAT {
                assert!(check(&schema(), &schema(), &raw, "$").unwrap_err().starts_with("$.components[0].kind:"));
            }
        }
    }
    let Fixture { session, .. } = synced();
    let mut accepted = board(&session).netlist;
    accepted.components[0].kind = ComponentKind::Unknown;
    let text = accepted.to_text().unwrap();
    let raw = serde_json::to_value(&accepted).unwrap();
    validate_v2(&raw);
    assert_eq!(Netlist::parse(&text).unwrap(), accepted);
    assert_eq!(serde_json::from_str::<Netlist>(&text).unwrap(), accepted);
    assert_eq!(serde_json::from_value::<Netlist>(raw).unwrap(), accepted);
}

/// The format 2 schema lists the board netlist's vocabulary: the `accepted` basis only, every
/// electrical type plus null for a terminal whose symbol revision is gone, and every kind.
#[test]
fn format_2_schema_vocabulary_matches_the_types() {
    let v2 = schema_v2();
    assert_eq!(v2["properties"]["format"]["const"], json!(FORMAT_V2));
    assert_eq!(v2["properties"]["basis"]["const"], json!(Basis::Accepted));
    assert_eq!(Netlist::format_for(Basis::Accepted), FORMAT_V2);
    assert_eq!(Netlist::format_for(Basis::Schematic), FORMAT);
    let v1 = schema();
    let mut types: Vec<Value> = v1["$defs"]["terminal"]["properties"]["electrical_type"]["enum"].as_array().unwrap().clone();
    types.push(Value::Null);
    assert_eq!(v2["$defs"]["terminal"]["properties"]["electrical_type"]["enum"], Value::Array(types));
    let kinds = [ComponentKind::Part, ComponentKind::Power, ComponentKind::Unknown];
    for kind in kinds {
        match kind {
            ComponentKind::Part | ComponentKind::Power | ComponentKind::Unknown => {}
        }
    }
    assert_eq!(v2["$defs"]["component"]["properties"]["kind"]["enum"], json!(kinds));
    assert_eq!(v1["$defs"]["component"]["properties"]["kind"]["enum"], json!(["part", "power"]));
    assert_eq!(v2["$defs"]["component"]["properties"]["units"]["maxItems"], json!(0));
    for required in ["board", "components", "nets", "unconnected"] {
        assert!(v2["required"].as_array().unwrap().contains(&json!(required)), "{required}");
    }
    assert!(v2["$defs"]["terminal"]["required"].as_array().unwrap().contains(&json!("pads")));
}
