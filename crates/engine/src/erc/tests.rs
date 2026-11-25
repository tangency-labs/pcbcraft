use super::*;
use crate::{DRAW_WIRE, PLACE_SYMBOL, no_connect::CHECK, persistence::encode_documents};

/// A resistor with one pin wired to an open end: one unconnected pin and one dangling wire end.
fn stub() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    s.apply(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    let pin = s.project().unwrap().symbols()[0].terminals()[0];
    s.apply(DRAW_WIRE, &json!({"from":pin.inspect(),"to":{"x_nm":-25_400_000,"y_nm":0},"bends":[],"expected_revision":s.revision()})).unwrap();
    s
}

/// The registry has one rule today, with a stable id and title, and a run reports the rules in
/// registry order with exact per-rule and overall totals.
#[test]
fn registry_order_is_fixed_and_runs_follow_it() {
    let ids = rules().iter().map(|rule| rule.id).collect::<Vec<_>>();
    assert_eq!(ids, ["no_connect"]);
    assert_eq!(rules()[0].title, "No-connect and unconnected pins");
    let s = stub();
    let report = run(s.project().unwrap(), s.revision());
    assert_eq!(report.rules.iter().map(|run| run.rule).collect::<Vec<_>>(), ids);
    assert_eq!(report.rules[0].title, rules()[0].title);
    assert_eq!(report.rules[0].total_findings, 2);
    assert!(!report.rules[0].truncated);
    assert_eq!(report.total_findings, 2);
    assert_eq!(report.project, s.project().unwrap().id());
    assert_eq!(report.revision, s.revision());
}

/// Two runs on one revision give byte-identical JSON, through the function and the command.
#[test]
fn runs_on_one_revision_are_byte_identical() {
    let mut s = stub();
    let first = serde_json::to_vec(&run(s.project().unwrap(), s.revision())).unwrap();
    let second = serde_json::to_vec(&run(s.project().unwrap(), s.revision())).unwrap();
    assert_eq!(first, second);
    let ran = serde_json::to_vec(&s.execute(RUN, &json!({})).unwrap()).unwrap();
    let again = serde_json::to_vec(&s.execute(RUN, &json!({})).unwrap()).unwrap();
    assert_eq!(ran, again);
    let reply: Value = serde_json::from_slice(&ran).unwrap();
    assert_eq!(serde_json::to_vec(&reply["erc_report"]).unwrap(), serde_json::to_vec(&serde_json::from_slice::<Value>(&first).unwrap()).unwrap());
}

/// Findings in the ERC report keep the keys the focused check has always produced:
/// `<rule>/<part>/<terminal>` for pins and `<rule>/<junction>` for wire ends.
#[test]
fn report_findings_keep_the_existing_keys() {
    let s = stub();
    let p = s.project().unwrap();
    let symbol = &p.symbols()[0];
    let unwired = symbol.terminals()[1];
    let junction = p.wire_end_targets().next().unwrap().0.junction;
    let report = run(p, s.revision());
    let check = report.no_connect().unwrap();
    let keys = check.findings.iter().map(|f| f.key.clone()).chain(check.wire_findings.iter().map(|f| f.key.clone())).collect::<Vec<_>>();
    assert_eq!(keys, [format!("unconnected_pin/{}/{}", symbol.component_id(), unwired.number()), format!("dangling_wire/{junction}")]);
    let direct = p.check_no_connects(s.revision());
    let direct_keys = direct.findings.iter().map(|f| f.key.clone()).chain(direct.wire_findings.iter().map(|f| f.key.clone())).collect::<Vec<_>>();
    assert_eq!(keys, direct_keys);
    assert_eq!(serde_json::to_value(check).unwrap(), serde_json::to_value(&direct).unwrap());
}

/// `schematic.check_no_connects` still answers with exactly the bytes it always has. The
/// expected text is written out here; only the fixture's generated identities are filled in.
#[test]
fn check_no_connects_response_is_byte_compatible() {
    let mut s = stub();
    let p = s.snapshot().unwrap();
    let symbol = &p.symbols()[0];
    let unwired = symbol.terminals()[1];
    let at = p.terminal_position(unwired).unwrap();
    let (end, end_at) = p.wire_end_targets().next().unwrap();
    let sheet = text(&p.sheets().next().unwrap().target);
    let project = text(&p.id());
    let (part, terminal, symbol_id, wire) = (symbol.component_id(), unwired.number().to_owned(), symbol.id(), p.wires()[0].id());
    let revision = s.revision();
    let expected = format!(
        concat!(
            r#"{{"no_connect_check":{{"findings":[{{"appearances":[{{"at":{{"x_nm":{x},"y_nm":{y}}},"sheet":{sheet},"symbol":"{symbol_id}"}}],"#,
            r#""key":"unconnected_pin/{part}/{terminal}","rule":"unconnected_pin","severity":"warning","target":{{"part":"{part}","terminal":"{terminal}"}}}}],"#,
            r#""markers":[],"project":{project},"revision":{revision},"total_findings":2,"total_markers":0,"truncated":false,"#,
            r#""wire_findings":[{{"appearance":{{"at":{{"x_nm":{ex},"y_nm":{ey}}},"sheet":{sheet},"wire":"{wire}"}},"#,
            r#""key":"dangling_wire/{junction}","rule":"dangling_wire","severity":"warning","target":{{"junction":"{junction}"}}}}],"#,
            r#""wire_markers":[]}},"revision":{revision}}}"#
        ),
        x = at.x_nm,
        y = at.y_nm,
        ex = end_at.x_nm,
        ey = end_at.y_nm,
        junction = end.junction,
        // concat! output cannot capture identifiers implicitly.
        sheet = sheet,
        symbol_id = symbol_id,
        part = part,
        terminal = terminal,
        project = project,
        revision = revision,
        wire = wire,
    );
    let response = s.execute(CHECK, &json!({})).unwrap();
    assert_eq!(serde_json::to_string(&response).unwrap(), expected);
    assert_eq!(s.apply(CHECK, &json!({})).unwrap(), revision);
    assert!(Arc::ptr_eq(&p, &s.snapshot().unwrap()));
}

/// Serialize a fixture identity independently of the report under test.
fn text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap()
}

/// Running ERC changes no project data, history or revision. An edit makes the kept report
/// stale: the reply then carries no findings, only the revision it ran on and the rerun command.
/// A rerun is current again, Undo makes it stale once more, and opening a project discards it.
#[test]
fn a_run_is_read_only_and_goes_stale_after_an_edit() {
    let mut s = stub();
    assert_eq!(s.execute(REPORT, &json!({})).unwrap(), json!({"revision":s.revision(),"status":"not_run","rerun":RUN}));
    let before = s.snapshot().unwrap();
    let bytes = encode_documents(before.clone()).unwrap();
    let inspected = s.inspect();
    let revision = s.revision();
    let ran = s.execute(RUN, &json!({})).unwrap();
    assert_eq!(ran["status"], "current");
    assert_eq!(ran["erc_report"]["revision"], revision);
    assert_eq!(ran["erc_report"]["total_findings"], 2);
    assert_eq!(s.revision(), revision);
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), bytes);
    assert_eq!(s.inspect(), inspected, "no undo step or other session-visible change");
    assert_eq!(s.execute(REPORT, &json!({})).unwrap(), ran);
    assert_eq!(s.erc_report().unwrap().1, Status::Current);

    s.apply("project.rename", &json!({"name":"Changed"})).unwrap();
    let stale = json!({"revision":s.revision(),"status":"stale","report_revision":revision,"rerun":RUN});
    assert_eq!(s.execute(REPORT, &json!({})).unwrap(), stale);
    assert_eq!(s.erc_report().unwrap().1, Status::Stale);
    assert!(s.execute(REPORT, &json!({})).unwrap().get("erc_report").is_none());

    let rerun = s.execute(RUN, &json!({})).unwrap();
    assert_eq!(rerun["status"], "current");
    assert_eq!(rerun["erc_report"]["revision"], s.revision());
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(s.execute(REPORT, &json!({})).unwrap()["status"], "stale", "Undo is a new revision");

    s.open_snapshot(before);
    assert!(s.erc_report().is_none());
    assert_eq!(s.execute(REPORT, &json!({})).unwrap()["status"], "not_run");
}

/// Both commands need an open project and refuse parameters, like the focused check, and a
/// refused run leaves the kept report exactly as it was.
#[test]
fn erc_commands_need_a_project_and_take_no_parameters() {
    let mut s = Session::default();
    for id in [RUN, REPORT] {
        let err = s.execute(id, &json!({})).unwrap_err();
        assert!(matches!(err, CommandError::Unavailable(ref m) if m.contains("no project is open")), "{id}: {err:?}");
    }
    let mut s = stub();
    for id in [RUN, REPORT] {
        let err = s.execute(id, &json!({"extra":true})).unwrap_err();
        assert!(matches!(err, CommandError::InvalidParams(ref m) if m.contains("has no parameter `extra`")), "{id}: {err:?}");
    }
    assert!(s.erc_report().is_none(), "a refused run keeps no report");
    s.execute(RUN, &json!({})).unwrap();
    let (kept, _) = s.erc_report().unwrap();
    let err = s.execute(RUN, &json!({"extra":true})).unwrap_err();
    assert!(matches!(err, CommandError::InvalidParams(_)), "{err:?}");
    assert!(Arc::ptr_eq(&kept, &s.erc_report().unwrap().0), "a refused run never replaces the kept report");
}

/// A run covers every sheet of the project and still leaves the revision, the document bytes
/// and the undo history unchanged: the next Undo reverts the last real edit, not the run.
#[test]
fn a_run_over_three_sheets_changes_no_revision_bytes_or_history() {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    for name in ["Second", "Third"] {
        s.apply(crate::sheets::CREATE_SHEET, &json!({"name":name,"expected_revision":s.revision()})).unwrap();
    }
    let sheets = s.project().unwrap().sheets().map(|sheet| sheet.target).collect::<Vec<_>>();
    assert_eq!(sheets.len(), 3);
    for sheet in &sheets {
        s.apply(PLACE_SYMBOL, &json!({"sheet":sheet.id(),"definition":"header-4","x_nm":0,"y_nm":0,"expected_revision":s.revision()})).unwrap();
    }
    let before = s.snapshot().unwrap();
    let bytes = encode_documents(before.clone()).unwrap();
    let inspected = s.inspect();
    let revision = s.revision();
    let ran = s.execute(RUN, &json!({})).unwrap();
    assert_eq!(s.revision(), revision);
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), bytes);
    assert_eq!(s.inspect(), inspected);
    let (report, _) = s.erc_report().unwrap();
    let check = report.no_connect().unwrap();
    assert_eq!(report.total_findings, 12, "four open header pins on each of three sheets");
    assert_eq!(ran["erc_report"]["total_findings"], 12);
    for sheet in &sheets {
        assert_eq!(check.findings.iter().filter(|finding| finding.appearances.iter().any(|at| at.sheet == *sheet)).count(), 4);
    }
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(s.project().unwrap().check_no_connects(s.revision()).total_findings, 8, "Undo reverted the last placement, not the run");
}
