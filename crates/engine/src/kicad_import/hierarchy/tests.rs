use super::*;

const ROOT: &str = "30000000-0000-4000-8000-000000000000";
const A: &str = "30000000-0000-4000-8000-000000000001";
const B: &str = "30000000-0000-4000-8000-000000000002";

fn block(id: &str, file: &str) -> String {
    format!(r#"(sheet (uuid "{id}") (at 10 20) (size 25 25) (property "Sheetname" "Child") (property "Sheetfile" "{file}"))"#)
}

fn project(items: &str) -> String {
    format!(r#"(kicad_sch (uuid "{ROOT}") {items})"#)
}

#[test]
fn repeated_source_is_read_once_and_each_occurrence_uses_its_own_record() {
    let root = project(&format!("{} {}", block(A, "child.kicad_sch"), block(B, "./child.kicad_sch")));
    let child = format!(
        r#"(kicad_sch (symbol (lib_id "Example:TwoUnit") (at 40 40 0) (unit 2)
           (instances (project "Root" (path "/{ROOT}/{B}" (reference "U3") (unit 1))
                                      (path "/{ROOT}/{A}" (reference "U2"))))))"#
    );
    let mut reads = Vec::new();
    let sheets = load("project/root.kicad_sch", |file, _| {
        reads.push(file.to_owned());
        match file {
            "project/root.kicad_sch" => Ok(root.clone()),
            "project/child.kicad_sch" => Ok(child.clone()),
            _ => Err("missing sheet".into()),
        }
    })
    .unwrap();
    assert_eq!(reads, ["project/root.kicad_sch", "project/child.kicad_sch"]);
    assert_eq!(sheets[1].parent, Some((0, 0)));
    assert_eq!(sheets[2].parent, Some((0, 1)));
    assert_eq!(sheets[1].instance_path, format!("/{ROOT}/{A}"));
    assert_eq!(sheets[2].instance_path, format!("/{ROOT}/{B}"));
    assert_eq!(sheets[1].placements, [("U2".to_owned(), 2)]);
    assert_eq!(sheets[2].placements, [("U3".to_owned(), 1)]);
    assert!(Arc::ptr_eq(&sheets[1].source, &sheets[2].source));
}

#[test]
fn missing_paths_cycles_and_escapes_report_the_source_file() {
    let root = project(&block(A, "detail/child.kicad_sch"));
    let read_root = |file: &str| (file == "root.kicad_sch").then(|| root.clone());
    let missing = load("root.kicad_sch", |file, _| read_root(file).ok_or_else(|| "missing sheet".into())).unwrap_err();
    assert!(missing.to_string().contains("detail/child.kicad_sch: missing sheet"));

    let cycle = project(&block(B, "../root.kicad_sch"));
    let looped = load("root.kicad_sch", |file, _| Ok(read_root(file).unwrap_or_else(|| cycle.clone()))).unwrap_err();
    assert!(looped.to_string().contains("root.kicad_sch: a sheet cycle"));
    assert!(looped.to_string().contains(&format!("/{ROOT}/{A}/{B}")));

    let escape = project(&block(B, "../../private.kicad_sch"));
    let invalid = load("root.kicad_sch", |file, _| Ok(read_root(file).unwrap_or_else(|| escape.clone()))).unwrap_err();
    assert!(invalid.to_string().contains("detail/child.kicad_sch: invalid child path"));
}

#[test]
fn invalid_or_duplicate_occurrences_are_rejected() {
    let duplicate = project(&format!("{} {}", block(A, "first.kicad_sch"), block(A, "second.kicad_sch")));
    let error = load("root.kicad_sch", |file, _| Ok(if file == "root.kicad_sch" { duplicate.clone() } else { "(kicad_sch)".into() })).unwrap_err();
    assert!(error.to_string().contains("second.kicad_sch: duplicate sheet occurrence"));

    let one = project(&block(A, "child.kicad_sch"));
    let other_project = format!(
        r#"(kicad_sch (symbol (lib_id "Example:R") (at 0 0 0)
           (instances (project "Elsewhere" (path "/{ROOT}/{B}" (reference "R8") (unit 1))))))"#
    );
    let error = load("root.kicad_sch", |file, _| Ok(if file == "root.kicad_sch" { one.clone() } else { other_project.clone() })).unwrap_err();
    assert!(error.to_string().contains(&format!("child.kicad_sch (/{ROOT}/{A}): no matching instance record")));
}

#[test]
fn native_sheet_interface_keeps_pins_at_source_coordinates_after_reopen() {
    let root = project(&format!(
        r#"(sheet (uuid "{A}") (at 10 20) (size 25 25)
          (property "Sheetname" "Child") (property "Sheetfile" "child.kicad_sch")
          (pin "OUT" output (at 35 23.23 0))
          (pin "TOP" input (at 14.2 20 90)))"#
    ));
    let child = r#"(kicad_sch
        (hierarchical_label "OUT" (shape output) (at 40 45 180))
        (hierarchical_label "OUT" (shape output) (at 50 55 180))
        (hierarchical_label "TOP" (shape input) (at 50 45 0)))"#;
    let occurrences = load("root.kicad_sch", |file, _| Ok(if file == "root.kicad_sch" { root.clone() } else { child.to_owned() })).unwrap();
    let project = Arc::new(build_sheet_structure(&occurrences, "Interfaces").unwrap());
    let docs = crate::persistence::encode_documents(project).unwrap();
    let reopened = crate::persistence::decode_documents(&docs).unwrap();
    let block = &reopened.sheet_symbols()[0];
    let mut pins = reopened.sheet_pins(block);
    let right = pins.next().unwrap();
    assert_eq!(right.edge, PinEdge::Right);
    assert_eq!(right.at, Point { x_nm: 35_000_000, y_nm: 23_230_000 });
    let top = pins.next().unwrap();
    assert_eq!(top.edge, PinEdge::Top);
    assert_eq!(top.at, Point { x_nm: 14_200_000, y_nm: 20_000_000 });
    let target = reopened.child_sheet(block.id).unwrap();
    let child = reopened.for_sheet(target).unwrap();
    assert_eq!(child.hierarchical_labels().len(), 2);
    assert_eq!(child.net_labels()[0].name, "OUT");
    assert_eq!(child.net_labels()[0].at, Point { x_nm: 50_000_000, y_nm: 55_000_000 });

    let resized = reopened
        .resized_sheet_symbol(
            &format!("sheet-symbol-{}", block.id),
            Point { x_nm: 15_000_000, y_nm: 25_000_000 },
            Point { x_nm: 30_000_000, y_nm: 30_000_000 },
        )
        .unwrap();
    let points: Vec<_> = resized.sheet_pins(&resized.sheet_symbols()[0]).map(|pin| pin.at).collect();
    assert_eq!(points, [Point { x_nm: 45_000_000, y_nm: 28_230_000 }, Point { x_nm: 19_200_000, y_nm: 25_000_000 }]);

    let mut invalid = docs;
    let path = format!("sheets/{}.json", target.id());
    let document: serde_json::Value = serde_json::from_slice(&invalid[&path]).unwrap();
    assert_eq!(document["format"], "pcbcraft-sheet/11");
    let mut downgraded = document;
    downgraded["format"] = serde_json::json!("pcbcraft-sheet/10");
    invalid.insert(path, serde_json::to_vec(&downgraded).unwrap());
    assert!(crate::persistence::decode_documents(&invalid).unwrap_err().contains("explicit parent pin positions"));
}

#[test]
fn repeated_sheets_import_wires_ports_and_scoped_nets_as_one_project() {
    const LIBRARY: &str = r##"(lib_symbols
        (symbol "Fixture:Tap" (property "Reference" "X")
            (symbol "Tap_1_1" (pin passive line (at 0 0 0) (length 2.54) (number "1"))))
        (symbol "Fixture:Bias" (power local) (property "Reference" "#PWR")
            (symbol "Bias_1_1" (pin power_in line (at 0 0 0) (length 0) (name "BIAS") (number "1")))))"##;
    let branch = |id, y| {
        format!(
            r#"(sheet (uuid "{id}") (at 10 {y}) (size 25 15)
            (property "Sheetname" "Branch") (property "Sheetfile" "branch.kicad_sch")
            (pin "SIGNAL" input (at 10 {} 180)))"#,
            y + 3
        )
    };
    let root = project(&format!(
        r#"{LIBRARY} {} {}
        (symbol (lib_id "Fixture:Tap") (at 5 23 0) (property "Reference" "X1"))
        (symbol (lib_id "Fixture:Tap") (at 5 43 0) (property "Reference" "X2"))
        (wire (pts (xy 5 23) (xy 10 23)))
        (wire (pts (xy 5 43) (xy 10 43)))"#,
        branch(A, 20),
        branch(B, 40)
    ));
    let instance = |x, y, a, b| {
        format!(
            r#"(symbol (lib_id "Fixture:Tap") (at {x} {y} 0) (unit 1)
             (instances (project "Fixture" (path "/{ROOT}/{A}" (reference "X{a}") (unit 1))
             (path "/{ROOT}/{B}" (reference "X{b}") (unit 1)))))"#
        )
    };
    let power = |x, a, b| {
        format!(
            r##"(symbol (lib_id "Fixture:Bias") (at {x} 60 0) (unit 1) (property "Value" "BIAS")
             (instances (project "Fixture" (path "/{ROOT}/{A}" (reference "#PWR0{a}") (unit 1))
             (path "/{ROOT}/{B}" (reference "#PWR0{b}") (unit 1)))))"##
        )
    };
    let child = format!(
        r#"(kicad_sch (generator_version "10.0") {LIBRARY} {} {} {} {} {} {} {} {}
          (hierarchical_label "SIGNAL" (shape input) (at 35 40 270) (effects (justify left)))
          (wire (pts (xy 35 40) (xy 40 40)))
          (global_label "GLOBAL" (shape input) (at 40 45 0))
          (label "GLOBAL" (at 45 45 0))
          (label "BIAS" (at 55 61 0))
          (label "LOCAL" (at 40 50 0))
          (label "LOCAL" (at 50 50 0)))"#,
        instance(40, 40, 3, 4),
        instance(40, 45, 5, 6),
        instance(40, 50, 7, 8),
        instance(50, 50, 9, 10),
        instance(45, 45, 11, 12),
        instance(55, 61, 13, 14),
        power(60, 1, 2),
        power(70, 3, 4)
    );
    let sources = load("root.kicad_sch", |file, _| Ok(if file == "root.kicad_sch" { root.clone() } else { child.clone() })).unwrap();
    let result = import(&sources, "Repeated sheets").unwrap();
    let docs = crate::persistence::encode_documents(result.project).unwrap();
    let project = crate::persistence::decode_documents(&docs).unwrap();
    let expected = BTreeSet::from([
        BTreeSet::from(["X1".to_owned(), "X3".to_owned()]),
        BTreeSet::from(["X2".to_owned(), "X4".to_owned()]),
        BTreeSet::from(["X5".to_owned(), "X6".to_owned(), "X11".to_owned(), "X12".to_owned()]),
        BTreeSet::from(["X7".to_owned(), "X9".to_owned()]),
        BTreeSet::from(["X8".to_owned(), "X10".to_owned()]),
        BTreeSet::from(["#PWR01".to_owned(), "#PWR03".to_owned(), "X13".to_owned()]),
        BTreeSet::from(["#PWR02".to_owned(), "#PWR04".to_owned(), "X14".to_owned()]),
    ]);
    let groups = project
        .electrical_connectivity()
        .nets
        .iter()
        .filter(|net| !net.is_empty())
        .map(|net| {
            net.iter()
                .map(|pin| project.reference(project.all_symbols().find(|symbol| symbol.ordinal == pin.symbol).unwrap()))
                .collect::<BTreeSet<_>>()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(groups, expected);
    assert_eq!(project.sheets().count(), 3);
    for child in project.sheets.iter().skip(1) {
        assert_eq!(child.labels[0].orientation(), PinOrientation::Up);
        assert_eq!(child.labels[0].parent_pin.unwrap().edge, PinEdge::Left);
    }
}

#[test]
fn root_legacy_annotations_resolve_each_child_occurrence() {
    let symbol_id = "30000000-0000-4000-8000-000000000003";
    let root = project(&format!(
        r#"{} {}
        (symbol_instances (path "/{A}/{symbol_id}" (reference "U2") (unit 2))
                          (path "/{B}/{symbol_id}" (reference "U3") (unit 1)))"#,
        block(A, "child.kicad_sch"),
        block(B, "child.kicad_sch")
    ));
    let child = format!(
        r#"(kicad_sch (symbol (lib_id "Example:TwoUnit") (at 40 40 0) (unit 1)
        (uuid "{symbol_id}") (property "Reference" "U?")))"#
    );
    let sheets = load("root.kicad_sch", |file, _| Ok(if file == "root.kicad_sch" { root.clone() } else { child.clone() })).unwrap();
    assert_eq!(sheets[1].placements, [("U2".into(), 2)]);
    assert_eq!(sheets[2].placements, [("U3".into(), 1)]);
}

#[test]
fn annotated_units_on_different_sheets_share_a_component_and_reserve_future_references() {
    let lib = r#"(lib_symbols (symbol "Fixture:Pair" (property "Reference" "U")
        (symbol "Pair_1_1" (pin passive line (at 0 0 0) (length 2.54) (number "1")))
        (symbol "Pair_2_1" (pin passive line (at 0 0 0) (length 2.54) (number "2")))))"#;
    let root = project(&format!(
        r#"{lib} {}
        (symbol (lib_id "Fixture:Pair") (at 30 60 0) (unit 1) (property "Reference" "U1"))
        (symbol (lib_id "Fixture:Pair") (at 45 60 0) (unit 1) (property "Reference" "U?"))"#,
        block(A, "child.kicad_sch")
    ));
    let child = format!(
        r#"(kicad_sch {lib}
        (symbol (lib_id "Fixture:Pair") (at 30 60 0) (unit 2) (property "Reference" "U1"))
        (symbol (lib_id "Fixture:Pair") (at 45 60 0) (unit 1) (property "Reference" "U2")))"#
    );
    let sheets = load("root.kicad_sch", |file, _| Ok(if file == "root.kicad_sch" { root.clone() } else { child.clone() })).unwrap();
    let imported = import(&sheets, "Shared units").unwrap();
    let project = crate::persistence::decode_documents(&crate::persistence::encode_documents(imported.project).unwrap()).unwrap();
    let child = project.for_sheet(project.child_sheet(project.sheet_symbols()[0].id).unwrap()).unwrap();
    assert_eq!(project.components.len(), 3);
    let parent_unit = project.symbols().iter().find(|symbol| project.reference(symbol) == "U1").unwrap();
    let child_unit = child.symbols().iter().find(|symbol| child.reference(symbol) == "U1").unwrap();
    assert_eq!(parent_unit.component, child_unit.component);
    assert_ne!(parent_unit.unit, child_unit.unit);
    assert!(child.symbols().iter().any(|symbol| child.reference(symbol) == "U2"));
    assert!(project.symbols().iter().any(|symbol| project.reference(symbol) == "U3"));
}

#[test]
fn imported_port_rename_updates_only_its_repeated_anchors_and_allocates_distinct_pins() {
    let root = project(&format!(
        r#"(sheet (uuid "{A}") (at 10 20) (size 25 25)
        (property "Sheetname" "Child") (property "Sheetfile" "child.kicad_sch")
        (pin "OUT" output (at 35 25.08 0)))"#
    ));
    let child = r#"(kicad_sch
        (hierarchical_label "OUT" (shape output) (at 40 45 0))
        (hierarchical_label "OUT" (shape output) (at 50 55 0))
        (label "OUT" (at 55 55 0)))"#;
    let sheets = load("root.kicad_sch", |file, _| Ok(if file == "root.kicad_sch" { root.clone() } else { child.to_owned() })).unwrap();
    let imported = import(&sheets, "Repeat port").unwrap();
    let project = crate::persistence::decode_documents(&crate::persistence::encode_documents(imported.project).unwrap()).unwrap();
    let target = project.child_sheet(project.sheet_symbols()[0].id).unwrap();
    let port = project.for_sheet(target).unwrap().hierarchical_labels()[0].clone();
    let [alias] = port.imported_aliases.as_slice() else {
        panic!("one imported alias expected");
    };
    let mut session = Session::default();
    session.open_snapshot(project);
    session
        .apply(
            crate::hierarchy::EDIT_LABEL,
            &json!({"sheet":target.id(), "id":format!("label-{}", port.id), "name":"DATA",
        "direction":"output", "x_nm":port.at.x_nm, "y_nm":port.at.y_nm, "expected_revision":session.revision()}),
        )
        .unwrap();
    let project = session.snapshot().unwrap();
    let child = project.for_sheet(target).unwrap();
    assert_eq!(child.net_labels().iter().find(|label| label.id == *alias).unwrap().name, "DATA");
    assert_eq!(child.net_labels().iter().find(|label| label.id != *alias).unwrap().name, "OUT");
    let graph = project.electrical_connectivity();
    let port_net = graph.net_index_for_endpoint(WireEndpoint::Label(port.id)).unwrap();
    assert_eq!(graph.net_index_for_endpoint(WireEndpoint::NetLabel(*alias)), Some(port_net));
    let root = project.sheet_target();
    let block = project.sheet_symbols()[0].id;
    let clipboard = session
        .execute(
            crate::clipboard::COPY,
            &json!({
                "items":crate::selection::Selection::default(), "objects":[format!("sheet-symbol-{block}")], "sheet":root.id(),
                "expected_revision":session.revision()
            }),
        )
        .unwrap()["clipboard"]
        .as_str()
        .unwrap()
        .to_owned();
    session
        .apply(
            crate::clipboard::PASTE,
            &json!({"text":clipboard,"sheet":root.id(),"x_nm":60_000_000,"y_nm":60_000_000,
        "expected_revision":session.revision()}),
        )
        .unwrap();
    let copied = session.snapshot().unwrap();
    let new_block = copied.sheet_symbols().iter().find(|sheet| sheet.id != block).unwrap();
    let copied_child = copied.for_sheet(copied.child_sheet(new_block.id).unwrap()).unwrap();
    let copy_aliases = &copied_child.hierarchical_labels()[0].imported_aliases;
    assert_eq!(copy_aliases.len(), 1);
    assert_ne!(copy_aliases[0], *alias);
    assert_eq!(copied_child.net_labels().iter().find(|label| label.id == copy_aliases[0]).unwrap().name, "DATA");

    // Give the explicit pin a different slot in a candidate. Slot 0 is free by ordinal but its
    // automatic position is already occupied by the explicit 5.08 mm offset.
    let mut candidate = (*session.snapshot().unwrap()).clone();
    let labels = Arc::make_mut(&mut Arc::make_mut(&mut candidate.sheets)[1].labels);
    labels[0].slot = 7;
    session.open_snapshot(Arc::new(candidate));
    session
        .apply(crate::hierarchy::ADD_LABEL, &json!({"sheet":target.id(),"name":"Second","direction":"output","x_nm":60_000_000,"y_nm":55_000_000,"expected_revision":session.revision()}))
        .unwrap();
    let project = session.snapshot().unwrap();
    let pins: Vec<_> = project.sheet_pins(&project.sheet_symbols()[0]).map(|pin| pin.at).collect();
    assert_eq!(pins[0].y_nm, 25_080_000);
    assert_ne!(pins[0], pins[1]);
}

#[test]
fn invalid_interface_reports_the_child_file_and_occurrence() {
    let name = "x".repeat(201);
    let root = project(&format!(
        r#"(sheet (uuid "{A}") (at 10 20) (size 25 25)
        (property "Sheetname" "Child") (property "Sheetfile" "child.kicad_sch")
        (pin "{name}" input (at 10 25 0)))"#
    ));
    let child = format!(r#"(kicad_sch (hierarchical_label "{name}" (shape input) (at 40 45 0)))"#);
    let sheets = load("root.kicad_sch", |file, _| Ok(if file == "root.kicad_sch" { root.clone() } else { child.clone() })).unwrap();
    let error = build_sheet_structure(&sheets, "Invalid name").unwrap_err().to_string();
    assert!(error.contains(&format!("child.kicad_sch (/{ROOT}/{A})")));
    assert!(error.contains("1–200 characters"));
}

#[test]
fn repeated_annotation_fallback_keeps_one_multi_unit_part_per_occurrence() {
    let child = r#"(kicad_sch
        (lib_symbols (symbol "Fixture:Pair" (property "Reference" "U")
            (symbol "Pair_1_1" (pin passive line (at 0 0 0) (length 2.54) (number "1")))
            (symbol "Pair_2_1" (pin passive line (at 0 0 0) (length 2.54) (number "2")))))
        (symbol (lib_id "Fixture:Pair") (at 30 60 0) (unit 2) (property "Reference" "U1"))
        (symbol (lib_id "Fixture:Pair") (at 45 60 0) (unit 1) (property "Reference" "U1")))"#;
    let second = block(B, "child.kicad_sch").replace("(at 10 20)", "(at 10 60)");
    let root = project(&format!("{} {second}", block(A, "child.kicad_sch")));
    let sheets = load("root.kicad_sch", |file, _| Ok(if file == "root.kicad_sch" { root.clone() } else { child.to_owned() })).unwrap();
    let imported = import(&sheets, "Repeated units").unwrap();
    let project = crate::persistence::decode_documents(&crate::persistence::encode_documents(imported.project).unwrap()).unwrap();
    assert_eq!(project.components.len(), 2);
    let mut components = BTreeSet::new();
    let mut references = BTreeSet::new();
    for child in project.sheets().skip(1) {
        let sheet = project.for_sheet(child.target).unwrap();
        let [left, right] = sheet.symbols() else { panic!("each child should have two unit placements") };
        assert_eq!(left.component, right.component);
        assert_ne!(left.unit, right.unit);
        assert!(components.insert(left.component));
        assert!(references.insert(sheet.reference(left).to_owned()));
    }
}

#[test]
fn joining_power_flags_are_refused_with_only_hierarchical_contacts() {
    const FLAG: &str = r##"(lib_symbols
        (symbol "power:PWR_FLAG" (power) (property "Reference" "#FLG")
            (symbol "PWR_FLAG_1_1" (pin power_out line (at 0 0 0) (length 0) (name "pwr") (number "1")))))
        (symbol (lib_id "power:PWR_FLAG") (at 25 15 0) (unit 1) (property "Reference" "#FLG01"))"##;
    const WIRES: &str = "(wire (pts (xy 10 15) (xy 35 15))) (wire (pts (xy 25 5) (xy 25 30)))";
    let parent_ports = format!(
        r#"(sheet (uuid "{A}") (at 0 10) (size 10.16 10.16)
             (property "Sheetname" "Left") (property "Sheetfile" "left.kicad_sch")
             (pin "A" output (at 10.16 15 0)))
           (sheet (uuid "{B}") (at 20 -5.16) (size 10.16 10.16)
             (property "Sheetname" "Top") (property "Sheetfile" "top.kicad_sch")
             (pin "B" output (at 25 5 90)))"#
    );
    for dot in ["", "(junction (at 25 15))"] {
        let root = project(&format!("{FLAG} {parent_ports} {} {dot}", WIRES.replace("xy 10 15", "xy 10.16 15")));
        let sheets = load("root.kicad_sch", |file, _| {
            Ok(match file {
                "root.kicad_sch" => root.clone(),
                "left.kicad_sch" => r#"(kicad_sch (hierarchical_label "A" (shape output) (at 20 20 0)))"#.into(),
                "top.kicad_sch" => r#"(kicad_sch (hierarchical_label "B" (shape output) (at 20 20 0)))"#.into(),
                _ => unreachable!(),
            })
        })
        .unwrap();
        let imported = import(&sheets, "Parent contacts");
        if dot.is_empty() {
            assert!(matches!(imported, Err(CommandError::InvalidParams(message)) if message.contains("power flag #FLG01 sits")));
        } else {
            imported.unwrap();
        }
    }

    let root = project(&format!(
        r#"(sheet (uuid "{A}") (at 10 20) (size 25 25)
          (property "Sheetname" "Child") (property "Sheetfile" "child.kicad_sch")
          (pin "A" input (at 10 25 180)) (pin "B" input (at 10 30 180)))"#
    ));
    let child = format!(
        r#"(kicad_sch {FLAG} {WIRES}
            (hierarchical_label "A" (shape input) (at 10 15 0))
            (hierarchical_label "B" (shape input) (at 25 5 90)))"#
    );
    let sheets = load("root.kicad_sch", |file, _| Ok(if file == "root.kicad_sch" { root.clone() } else { child.clone() })).unwrap();
    let imported = import(&sheets, "Child contacts");
    assert!(
        matches!(imported, Err(CommandError::InvalidParams(message)) if message.contains("child.kicad_sch") && message.contains("power flag #FLG01 sits"))
    );
}
