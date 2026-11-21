use super::*;
use crate::persistence::{decode_documents, encode_documents};

const DIVIDER: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../kicad/tests/fixtures/divider.kicad_sch"));

/// The nets of a project as sorted groups of `reference.pin`, unconnected pins left out.
fn nets(project: &Project) -> BTreeSet<BTreeSet<String>> {
    let connectivity = project.connectivity();
    let mut groups: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    for symbol in project.symbols() {
        let reference = project.reference(symbol);
        for terminal in symbol.terminals() {
            if let Some(net) = connectivity.net_index_for(terminal) {
                groups.entry(net).or_default().insert(format!("{reference}.{}", terminal.number()));
            }
        }
    }
    groups.into_values().filter(|g| g.len() > 1).collect()
}

/// `["R1.2", "R2.1"]` as a net.
fn net(pins: &[&str]) -> BTreeSet<String> {
    pins.iter().map(|p| (*p).to_owned()).collect()
}

#[test]
fn imports_the_divider_with_its_parts_values_and_connections() {
    let imported = import(DIVIDER, "Divider").unwrap();
    let project = &imported.project;
    assert_eq!(project.name, "Divider");
    let mut parts: Vec<(String, String, String)> =
        project.symbols().iter().map(|s| (project.reference(s), s.unit_id().to_owned(), project.value(s).to_owned())).collect();
    parts.sort();
    assert_eq!(
        parts,
        [
            ("#PWR01".into(), "main".into(), "+3V3".into()),
            ("#PWR02".into(), "main".into(), "GND".into()),
            ("R1".into(), "main".into(), "10k".into()),
            ("R2".into(), "main".into(), "10k".into()),
            ("U1".into(), "a".into(), "Buffer2".into()),
            ("U1".into(), "b".into(), "Buffer2".into()),
        ]
    );
    // U1's two units are one part; its shared supply pins appear in both.
    assert_eq!(project.components().count(), 5);
    let found = nets(project);
    assert!(found.contains(&net(&["#PWR01.1", "R1.1"])), "{found:?}");
    assert!(found.contains(&net(&["R1.2", "R2.1"])), "{found:?}");
    assert!(found.contains(&net(&["#PWR02.1", "R2.2"])), "{found:?}");
    let report: Vec<_> = imported.report.notes().collect();
    assert!(report.contains(&("sheet texts left out", 1)), "{report:?}");
}

#[test]
fn an_imported_project_saves_and_reopens_unchanged() {
    let imported = import(DIVIDER, "Divider").unwrap();
    let saved = encode_documents(imported.project.clone()).unwrap();
    let reopened = decode_documents(&saved).unwrap();
    assert_eq!(encode_documents(reopened.clone()).unwrap(), saved);
    assert_eq!(nets(&reopened), nets(&imported.project));
    let symbol = &reopened.symbols()[0].definition;
    assert!(symbol.library.starts_with(LIBRARY_PREFIX) && symbol.library.len() == LIBRARY_PREFIX.len() + 16, "{}", symbol.library);
    // The same file gives the same library identity; a different one does not.
    assert_eq!(import(DIVIDER, "Again").unwrap().project.symbols()[0].definition.library, symbol.library);
    let other = import(&file(&place("T:R", "R1", 0.0, 0.0, 0, "")), "Other").unwrap();
    assert_ne!(other.project.symbols()[0].definition.library, symbol.library);
    assert_eq!(symbol.provenance.as_ref().unwrap().license, "NOASSERTION");
}

/// A file with an asymmetric three-pin symbol `T:Tri`, a two-pin `T:R`, a `power:GND`, and `items`.
fn file(items: &str) -> String {
    let pin = |kind: &str, x: f64, y: f64, angle: u32, number: &str| {
        format!("(pin {kind} line (at {x} {y} {angle}) (length 1.27) (name \"~\") (number \"{number}\"))")
    };
    format!(
        r##"(kicad_sch (version 20231120) (uuid "aaaaaaaa-0000-4000-8000-000000000000")
        (lib_symbols
          (symbol "T:Tri" (property "Reference" "Q") (property "Value" "Tri")
            (symbol "Tri_1_1" (rectangle (start -1.27 -1.27) (end 1.27 1.27)) {} {} {}))
          (symbol "T:R" (property "Reference" "R") (property "Value" "R") (symbol "R_1_1" {} {}))
          (symbol "power:GND" (power) (property "Reference" "#PWR") (property "Value" "GND")
            (symbol "GND_1_1" (pin power_in line (at 0 0 270) (length 0) hide (name "GND") (number "1"))))
          (symbol "power:PWR_FLAG" (power) (property "Reference" "#FLG") (property "Value" "PWR_FLAG")
            (symbol "PWR_FLAG_1_1" (pin power_out line (at 0 0 90) (length 0) (name "pwr") (number "1"))))
          (symbol "T:Dual" (property "Reference" "U") (property "Value" "Dual")
            (symbol "Dual_0_1" (pin passive line (at 0 5.08 270) (length 1.27) (number "5")))
            (symbol "Dual_1_1" (pin passive line (at -5.08 0 0) (length 1.27) (number "1")))
            (symbol "Dual_2_1" (pin passive line (at -5.08 0 0) (length 1.27) (number "2")))))
        {items})"##,
        pin("input", -3.81, 0.0, 0, "1"),
        pin("output", 3.81, 1.27, 180, "2"),
        pin("passive", 0.0, -3.81, 90, "3"),
        pin("passive", 0.0, 3.81, 270, "1"),
        pin("passive", 0.0, -3.81, 90, "2"),
    )
}

/// A placed symbol line for [`file`].
fn place(lib: &str, reference: &str, x: f64, y: f64, angle: u32, mirror: &str) -> String {
    let mirror = if mirror.is_empty() { String::new() } else { format!("(mirror {mirror})") };
    // A power symbol's Value is its net name, the same as its pin's.
    let value = lib.strip_prefix("power:").unwrap_or("v");
    format!(r#"(symbol (lib_id "{lib}") (at {x} {y} {angle}) {mirror} (unit 1) (property "Reference" "{reference}") (property "Value" "{value}"))"#)
}

/// A wire line for [`file`] through the given points in millimetres.
fn wire(points: &[(f64, f64)]) -> String {
    format!("(wire (pts {}))", points.iter().map(|(x, y)| format!("(xy {x} {y})")).collect::<String>())
}

#[test]
fn every_rotation_and_mirror_puts_each_pin_where_kicad_does() {
    let mut items = String::new();
    let mut expected = Vec::new();
    for (i, (angle, mirror)) in [0, 90, 180, 270].into_iter().flat_map(|a| [(a, ""), (a, "x"), (a, "y")]).enumerate() {
        let (x, y) = (25.4 * (i % 4) as f64, 25.4 * (i / 4) as f64);
        items.push_str(&place("T:Tri", &format!("Q{}", i + 1), x, y, angle, mirror));
        let symbol = PlacedSymbol {
            uuid: None,
            lib_symbol: "T:Tri".into(),
            lib_id: "T:Tri".into(),
            at: Point { x_nm: (x * 1e6).round() as i64, y_nm: (y * 1e6).round() as i64 },
            rotation_mdeg: angle as i32 * 1000,
            mirror: match mirror {
                "x" => Mirror::X,
                "y" => Mirror::Y,
                _ => Mirror::None,
            },
            unit: 1,
            reference: format!("Q{}", i + 1),
            reference_from: Default::default(),
            instances: Vec::new(),
            properties: Vec::new(),
            in_bom: true,
            on_board: true,
            dnp: false,
        };
        for (number, (px, py)) in [("1", (-3.81, 0.0)), ("2", (3.81, 1.27)), ("3", (0.0, -3.81))] {
            let library = Point { x_nm: (px * 1e6_f64).round() as i64, y_nm: (py * 1e6_f64).round() as i64 };
            expected.push((symbol.reference.clone(), number.to_owned(), kicad_pin_position(&symbol, library).unwrap()));
        }
    }
    let imported = import(&file(&items), "Poses").unwrap();
    let project = &imported.project;
    let mut found = Vec::new();
    for symbol in project.symbols() {
        for (terminal, at) in symbol.terminals().into_iter().zip(symbol.pins()) {
            found.push((project.reference(symbol), terminal.number().to_owned(), Point { x_nm: at.x_nm, y_nm: at.y_nm }));
        }
    }
    found.sort();
    expected.sort();
    assert_eq!(found, expected);
}

#[test]
fn kicad_pin_rules_turn_counterclockwise_then_mirror() {
    let symbol = |rotation_mdeg, mirror| PlacedSymbol {
        uuid: None,
        lib_symbol: String::new(),
        lib_id: String::new(),
        at: Point { x_nm: 0, y_nm: 0 },
        rotation_mdeg,
        mirror,
        unit: 1,
        reference: String::new(),
        reference_from: Default::default(),
        instances: Vec::new(),
        properties: Vec::new(),
        in_bom: true,
        on_board: true,
        dnp: false,
    };
    // A pin to the right of the origin in the library (y upward).
    let right = Point { x_nm: 10, y_nm: 0 };
    let up = Point { x_nm: 0, y_nm: 10 };
    let p = |x, y| Point { x_nm: x, y_nm: y };
    assert_eq!(kicad_pin_position(&symbol(0, Mirror::None), right), Some(p(10, 0)));
    // A quarter turn counterclockwise takes the right-hand pin to the top: y -10 on the sheet.
    assert_eq!(kicad_pin_position(&symbol(90_000, Mirror::None), right), Some(p(0, -10)));
    // The library's up is the sheet's up.
    assert_eq!(kicad_pin_position(&symbol(0, Mirror::None), up), Some(p(0, -10)));
    assert_eq!(kicad_pin_position(&symbol(0, Mirror::Y), right), Some(p(-10, 0)));
    assert_eq!(kicad_pin_position(&symbol(0, Mirror::X), up), Some(p(0, 10)));
    // Turned first, then mirrored left to right: the top pin stays on top.
    assert_eq!(kicad_pin_position(&symbol(90_000, Mirror::Y), right), Some(p(0, -10)));
}

#[test]
fn t_joints_connect_and_crossings_without_a_dot_do_not() {
    // R1 vertical at x=0: pins at y=-3.81 (1) and 3.81 (2). R2 at x=10.16.
    // A wire from R1.2 runs right and ends on the middle of a vertical wire to R2.2 (a T).
    // A horizontal wire from R3.1 crosses that vertical wire without a dot.
    let items = [
        place("T:R", "R1", 0.0, 0.0, 0, ""),
        place("T:R", "R2", 10.16, 0.0, 0, ""),
        place("T:R", "R3", -10.16, 15.24, 90, ""),
        wire(&[(0.0, 3.81), (0.0, 7.62), (10.16, 7.62)]),
        wire(&[(10.16, 3.81), (10.16, 20.32)]),
        // R3 turned a quarter: its pin 1 is at the left, pin 2 at the right.
        wire(&[(-6.35, 15.24), (15.24, 15.24)]),
    ]
    .concat();
    let imported = import(&file(&items), "T").unwrap();
    let found = nets(&imported.project);
    assert_eq!(found, BTreeSet::from([net(&["R1.2", "R2.2"])]), "{found:?}");
}

#[test]
fn a_junction_dot_joins_crossing_wires() {
    let items = [
        place("T:R", "R1", 0.0, 0.0, 0, ""),
        place("T:R", "R2", -10.16, 7.62, 90, ""),
        wire(&[(0.0, 3.81), (0.0, 12.7)]),
        wire(&[(-6.35, 7.62), (6.35, 7.62)]),
        "(junction (at 0 7.62))".into(),
    ]
    .concat();
    let found = nets(&import(&file(&items), "Dot").unwrap().project);
    assert_eq!(found, BTreeSet::from([net(&["R1.2", "R2.2"])]), "{found:?}");
}

#[test]
fn a_power_symbol_on_a_pin_or_a_label_on_a_pin_connects_like_kicad() {
    let items = [
        place("T:R", "R1", 0.0, 0.0, 0, ""),
        place("T:R", "R2", 10.16, 0.0, 0, ""),
        // GND sits right on R1's lower pin, with no wire.
        place("power:GND", "#PWR01", 0.0, 3.81, 0, ""),
        // And on a wire from R2's lower pin.
        place("power:GND", "#PWR02", 10.16, 7.62, 0, ""),
        wire(&[(10.16, 3.81), (10.16, 7.62)]),
        // Labels named alike on the two upper pins.
        r#"(label "TOP" (at 0 -3.81 0)) (label "TOP" (at 10.16 -3.81 0))"#.into(),
    ]
    .concat();
    let imported = import(&file(&items), "Power").unwrap();
    let found = nets(&imported.project);
    assert!(found.contains(&net(&["#PWR01.1", "#PWR02.1", "R1.2", "R2.2"])), "{found:?}");
    assert!(found.contains(&net(&["R1.1", "R2.1"])), "{found:?}");
}

#[test]
fn what_would_change_connections_refuses_the_whole_file() {
    let refused = |items: &str| match import(&file(items), "x") {
        Err(CommandError::InvalidParams(message)) => message,
        other => panic!("expected a refusal, got {other:?}"),
    };
    let sheet = r#"(sheet (at 0 0) (size 10 10) (property "Sheetname" "A") (property "Sheetfile" "a.kicad_sch"))"#;
    assert!(refused(sheet).contains("sub-sheet"));
    assert!(refused("(bus (pts (xy 0 0) (xy 5 0)))").contains("buses"));
    assert!(refused(r#"(hierarchical_label "X" (at 0 0 0))"#).contains("hierarchical labels"));
    assert!(refused(&wire(&[(0.0, 0.0), (2.54, 2.54)])).contains("diagonal"));
    assert!(refused(&place("T:Missing", "U1", 0.0, 0.0, 0, "")).contains("doesn't hold"));
    assert!(refused(&place("T:R", "R1", 0.0, 0.0, 45, "")).contains("quarter turn"));
    // Two resistors touching pin to pin, with no wire between them.
    let touching = [place("T:R", "R1", 0.0, 0.0, 0, ""), place("T:R", "R2", 0.0, 7.62, 0, "")].concat();
    assert!(refused(&touching).contains("pins touch without a wire"));
}

#[test]
fn symbols_the_native_format_cant_hold_are_refused_by_name() {
    let symbol = |pins: &str| {
        format!(
            r#"(kicad_sch (version 20231120) (lib_symbols (symbol "L:Wide" (property "Reference" "U") (symbol "Wide_1_1" {pins})))
            (symbol (lib_id "L:Wide") (at 0 0 0) (unit 1) (property "Reference" "U1")))"#
        )
    };
    let message = |text: String| match import(&text, "x") {
        Err(CommandError::InvalidParams(message)) => message,
        other => panic!("expected a refusal, got {other:?}"),
    };
    let far = message(symbol(r#"(pin passive line (at 15.24 0 180) (length 2.54) (number "1"))"#));
    assert!(far.contains("`L:Wide`") && far.contains("12.7 mm"), "{far}");
    let off_grid = message(symbol(r#"(pin passive line (at 1 0 180) (length 2.54) (number "1"))"#));
    assert!(off_grid.contains("grid"), "{off_grid}");
    let stacked =
        message(symbol(r#"(pin passive line (at 0 0 0) (length 2.54) (number "1")) (pin passive line (at 0 0 0) (length 2.54) (number "2"))"#));
    assert!(stacked.contains("share a position"), "{stacked}");
    let hidden = message(symbol(r#"(pin power_in line (at 0 0 0) (length 2.54) hide (name "VCC") (number "1"))"#));
    assert!(hidden.contains("hidden power pin"), "{hidden}");
    let long_number = message(symbol(r#"(pin passive line (at 0 0 0) (length 2.54) (number "123456789"))"#));
    assert!(long_number.contains("1 to 8"), "{long_number}");
}

#[test]
fn no_connect_marks_land_on_pins() {
    let items = [place("T:Tri", "Q1", 0.0, 0.0, 0, ""), "(no_connect (at 3.81 -1.27))".into(), "(no_connect (at 50 50))".into()].concat();
    let imported = import(&file(&items), "NC").unwrap();
    let part = imported.project.components().next().unwrap();
    assert_eq!(part.no_connects.keys().collect::<Vec<_>>(), ["2"]);
    assert!(imported.report.notes().any(|(what, n)| what == "no-connect marks not on a pin or wire end left out" && n == 1));
}

#[test]
fn the_command_replaces_the_project_in_one_undoable_step_or_changes_nothing() {
    let mut session = Session::default();
    session.apply("project.new", &json!({"name": "Before"})).unwrap();
    let before = session.snapshot().unwrap();
    let refused = session.execute(IMPORT, &json!({"name": "Bus", "text": file("(bus (pts (xy 0 0) (xy 5 0)))")}));
    assert!(matches!(refused, Err(CommandError::InvalidParams(m)) if m.contains("buses")));
    assert!(Arc::ptr_eq(&before, &session.snapshot().unwrap()));
    let result = session.execute(IMPORT, &json!({"name": "Divider", "text": DIVIDER})).unwrap();
    assert_eq!(result["revision"], session.revision());
    assert!(result["report"].as_array().unwrap().iter().any(|n| n["what"] == "sheet texts left out" && n["count"] == 1));
    assert_eq!(session.project().unwrap().name, "Divider");
    assert_eq!(session.project().unwrap().symbols().len(), 6);
    session.apply("edit.undo", &Value::Null).unwrap();
    crate::assert_same_content(&before, &session.snapshot().unwrap());
}

#[test]
fn read_only_sessions_cannot_import() {
    let mut session = Session::with_scope(crate::SessionScope::ReadOnly);
    let result = session.execute(IMPORT, &json!({"text": DIVIDER}));
    assert!(matches!(result, Err(CommandError::OutOfScope { .. })), "{result:?}");
}

#[test]
fn the_net_check_refuses_a_project_whose_connections_differ_from_the_file() {
    let imported = import(DIVIDER, "Divider").unwrap();
    let mut schematic = pcbcraft_kicad::read_schematic(DIVIDER).unwrap();
    let mut report = Report::default();
    let converted: BTreeMap<&str, symbols::Converted> = schematic
        .lib_symbols
        .iter()
        .map(|(name, symbol)| (name.as_str(), symbols::convert(symbol, symbols::native_id(name), &mut report).unwrap()))
        .collect();
    // The import places symbols in file order, so the n-th native placement is the n-th KiCad one.
    let mut natives: Vec<_> = imported.project.symbols().iter().collect();
    natives.sort_by_key(|s| s.ordinal);
    let placed: Vec<Placed> = natives
        .iter()
        .zip(&schematic.symbols)
        .enumerate()
        .map(|(source, (native, symbol))| Placed {
            source,
            symbol: native.ordinal,
            component: native.component,
            pins: pins_of(symbol, &schematic, &converted).unwrap(),
        })
        .collect();
    let segments = |s: &Schematic| wiring::segments(&s.wires.iter().map(|w| w.points.clone()).collect::<Vec<_>>()).unwrap();
    assert!(check_nets(&schematic, &segments(&schematic), &placed, &converted, &imported.project).is_ok());
    // Without the wire from R1 down to the junction, KiCad would leave R1.2 apart.
    schematic.wires.retain(|w| w.uuid.as_deref() != Some("0d1e0000-0000-4000-8000-000000000202"));
    let wrong = check_nets(&schematic, &segments(&schematic), &placed, &converted, &imported.project);
    assert!(matches!(&wrong, Err(CommandError::InvalidParams(m)) if m.contains("differ")), "{wrong:?}");
}

#[test]
fn an_unreadable_item_that_could_connect_refuses_the_file() {
    // A pin without a number would silently drop the wire to it.
    let text = r#"(kicad_sch (version 20231120) (lib_symbols (symbol "L:X" (property "Reference" "U")
        (symbol "X_1_1" (pin passive line (at 0 0 0) (length 1.27) (number "1")) (pin passive line (at 2.54 0 180) (length 1.27)))))
        (symbol (lib_id "L:X") (at 0 0 0) (unit 1) (property "Reference" "U1")))"#;
    let refused = import(text, "x");
    assert!(matches!(&refused, Err(CommandError::InvalidParams(m)) if m.contains("pin without a number")), "{refused:?}");
}

#[test]
fn a_shared_pin_wired_on_one_unit_only_is_one_connection() {
    // U1's common pin 5 is at y = -5.08 above each unit; only unit A's copy is wired to R1.1.
    let items = [
        r#"(symbol (lib_id "T:Dual") (at 0 0 0) (unit 1) (property "Reference" "U1") (property "Value" "Dual"))"#.to_owned(),
        r#"(symbol (lib_id "T:Dual") (at 25.4 0 0) (unit 2) (property "Reference" "U1") (property "Value" "Dual"))"#.to_owned(),
        place("T:R", "R1", 10.16, -15.24, 0, ""),
        wire(&[(0.0, -5.08), (0.0, -19.05), (10.16, -19.05)]),
    ]
    .concat();
    let found = nets(&import(&file(&items), "Shared").unwrap().project);
    assert!(found.contains(&net(&["R1.1", "U1.5"])), "{found:?}");
}

#[test]
fn a_copied_reference_becomes_a_second_renumbered_part() {
    let items = [place("T:R", "R1", 0.0, 0.0, 0, ""), place("T:R", "R1", 10.16, 0.0, 0, "")].concat();
    let imported = import(&file(&items), "Copies").unwrap();
    let mut references: Vec<String> = imported.project.symbols().iter().map(|s| imported.project.reference(s)).collect();
    references.sort();
    assert_eq!(references.len(), 2);
    assert!(references.contains(&"R1".to_owned()) && references[0] != references[1], "{references:?}");
    assert!(imported.report.notes().any(|(what, _)| what.contains("used by two different parts")));
}

#[test]
fn a_name_used_by_a_label_and_a_power_symbol_is_refused_with_its_name() {
    let items =
        [place("T:R", "R1", 0.0, 0.0, 0, ""), place("power:GND", "#PWR01", 0.0, 3.81, 0, ""), r#"(global_label "GND" (at 0 -3.81 0))"#.into()]
            .concat();
    let refused = import(&file(&items), "Clash");
    assert!(matches!(&refused, Err(CommandError::InvalidParams(m)) if m.contains("`GND`") && m.contains("power symbol")), "{refused:?}");
}

#[test]
fn power_flags_are_left_out_rather_than_refused_on_a_pin() {
    let items = [place("T:R", "R1", 0.0, 0.0, 0, ""), place("power:PWR_FLAG", "#FLG01", 0.0, 3.81, 0, "")].concat();
    let imported = import(&file(&items), "Flag").unwrap();
    assert_eq!(imported.project.symbols().len(), 1);
    assert!(imported.report.notes().any(|(what, n)| what.starts_with("power flags") && n == 1));
}

#[test]
fn an_alternate_body_style_that_moves_pins_refuses_its_symbol() {
    let text = |alternate_x: f64| {
        format!(
            r#"(kicad_sch (version 20231120) (lib_symbols (symbol "L:G" (property "Reference" "U")
            (symbol "G_1_1" (pin input line (at -2.54 0 0) (length 1.27) (number "1")))
            (symbol "G_1_2" (pin input line (at {alternate_x} 0 0) (length 1.27) (number "1")))))
            (symbol (lib_id "L:G") (at 0 0 0) (unit 1) (property "Reference" "U1")))"#
        )
    };
    let refused = import(&text(-5.08), "x");
    assert!(matches!(&refused, Err(CommandError::InvalidParams(m)) if m.contains("body style")), "{refused:?}");
    let same = import(&text(-2.54), "x").unwrap();
    assert!(same.report.notes().any(|(what, _)| what.contains("De Morgan")));
}

#[test]
fn long_staircases_and_loose_loops_are_drawn_in_pieces() {
    // 80 corners from R1.2 to R2.2, more than one native wire may bend through.
    let mut points = vec![(0.0, 3.81)];
    for i in 0..40 {
        let y = 3.81 + 2.54 * (i + 1) as f64;
        points.push((2.54 * i as f64, y));
        points.push((2.54 * (i + 1) as f64, y));
    }
    let (end_x, end_y) = *points.last().unwrap();
    let items = [
        place("T:R", "R1", 0.0, 0.0, 0, ""),
        place("T:R", "R2", end_x, end_y + 3.81, 180, ""),
        wire(&points),
        // A closed square of wire touching nothing.
        wire(&[(-20.32, 0.0), (-15.24, 0.0), (-15.24, 5.08), (-20.32, 5.08), (-20.32, 0.0)]),
    ]
    .concat();
    let found = nets(&import(&file(&items), "Pieces").unwrap().project);
    assert!(found.contains(&net(&["R1.2", "R2.2"])), "{found:?}");
}

#[test]
fn more_labels_than_a_sheet_holds_are_refused_before_building() {
    let items: String = (0..300).map(|i| format!("(label \"N{i}\" (at {} 0 0))", i as f64 * 2.54)).collect();
    assert!(matches!(import(&file(&items), "Labels"), Err(CommandError::CapExceeded(m)) if m.contains("300 labels")));
}

#[test]
fn a_wire_ending_where_the_other_mirror_order_puts_a_pin_refuses_the_pose() {
    let symbol = PlacedSymbol {
        uuid: None,
        lib_symbol: "T:Tri".into(),
        lib_id: "T:Tri".into(),
        at: Point { x_nm: 0, y_nm: 0 },
        rotation_mdeg: 90_000,
        mirror: Mirror::X,
        unit: 1,
        reference: "Q1".into(),
        reference_from: Default::default(),
        instances: Vec::new(),
        properties: Vec::new(),
        in_bom: true,
        on_board: true,
        dnp: false,
    };
    // Pin 2 sits at (3.81, 1.27) in the library; find where it lands with the other mirror axis.
    let other = PlacedSymbol { mirror: Mirror::Y, ..symbol.clone() };
    let library = Point { x_nm: 3_810_000, y_nm: 1_270_000 };
    let here = kicad_pin_position(&symbol, library).unwrap();
    let there = kicad_pin_position(&other, library).unwrap();
    assert_ne!(here, there);
    let mm = |p: Point| (p.x_nm as f64 / 1e6, p.y_nm as f64 / 1e6);
    let (x, y) = mm(there);
    let dangling = [place("T:Tri", "Q1", 0.0, 0.0, 90, "x"), wire(&[(x, y), (x + 10.16, y)])].concat();
    let refused = import(&file(&dangling), "Ambiguous");
    assert!(matches!(&refused, Err(CommandError::InvalidParams(m)) if m.contains("mirrored before turning")), "{refused:?}");
    // The same wire on the pin where this reading puts it imports.
    let (x, y) = mm(here);
    let attached = [place("T:Tri", "Q1", 0.0, 0.0, 90, "x"), wire(&[(x, y), (x + 10.16, y)])].concat();
    assert!(import(&file(&attached), "Attached").is_ok());
}

#[test]
fn nested_collinear_wires_split_once_and_still_connect() {
    // 3,000 nested runs along one line and 3,000 junction dots on it: one merged run, cut once
    // at every distinct point, rather than every run cut at every point inside it.
    let p = |mm: f64| Point { x_nm: 0, y_nm: (mm * 1e6).round() as i64 };
    let segments: Vec<wiring::Segment> = (0..3000).map(|i| (p(-(i as f64) * 0.01), p(10.0 + i as f64 * 0.01))).collect();
    let cuts: BTreeSet<Point> = (0..3000).map(|i| p(i as f64 * 0.003)).collect();
    let pieces = wiring::split(&segments, &cuts);
    // One merged run, cut at every distinct point on it: as many pieces as points less one.
    let points: BTreeSet<Point> = segments.iter().flat_map(|(a, b)| [*a, *b]).chain(cuts.iter().copied()).collect();
    assert_eq!(pieces.len(), points.len() - 1);
    // And through the import, overlapping wires join the pins at both ends.
    let items = [
        place("T:R", "R1", 0.0, 0.0, 0, ""),
        place("T:R", "R2", 0.0, 20.32, 180, ""),
        wire(&[(0.0, 3.81), (0.0, 12.7)]),
        wire(&[(0.0, 7.62), (0.0, 16.51)]),
    ]
    .concat();
    let found = nets(&import(&file(&items), "Overlap").unwrap().project);
    assert!(found.contains(&net(&["R1.2", "R2.2"])), "{found:?}");
}

#[test]
fn a_power_symbol_whose_value_and_pin_name_differ_is_refused() {
    // The library GND symbol's hidden pin is named GND; a Value of AGND would name another net in
    // KiCad 8 than in KiCad 7.
    let items = [place("T:R", "R1", 0.0, 0.0, 0, ""), place("power:GND", "#PWR01", 0.0, 3.81, 0, "")]
        .concat()
        .replace(r##"(property "Reference" "#PWR01") (property "Value" "GND")"##, r##"(property "Reference" "#PWR01") (property "Value" "AGND")"##);
    let refused = import(&file(&items), "Names");
    assert!(matches!(&refused, Err(CommandError::InvalidParams(m)) if m.contains("`AGND`") && m.contains("`GND`")), "{refused:?}");
}

#[test]
fn automatic_numbers_never_take_a_reference_a_later_symbol_keeps() {
    // An unannotated R? first, then R1; and a copy of R1 before R2.
    let items = [
        place("T:R", "R?", 0.0, 0.0, 0, ""),
        place("T:R", "R1", 10.16, 0.0, 0, ""),
        place("T:R", "R1", 20.32, 0.0, 0, ""),
        place("T:R", "R2", 30.48, 0.0, 0, ""),
    ]
    .concat();
    let imported = import(&file(&items), "Order").unwrap();
    let project = &imported.project;
    let at = |x_nm: i64| project.symbols().iter().find(|s| s.position.x_nm == x_nm).map(|s| project.reference(s)).unwrap();
    assert_eq!((at(10_160_000), at(30_480_000)), ("R1".to_owned(), "R2".to_owned()));
    let mut all: Vec<String> = project.symbols().iter().map(|s| project.reference(s)).collect();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 4, "{all:?}");
}

#[test]
fn a_power_flag_that_joins_crossing_wires_refuses_but_one_on_a_net_does_not() {
    // Two wires cross at (0, 7.62) with no dot; a power flag sits right on the crossing.
    let crossing = [
        place("T:R", "R1", 0.0, 0.0, 0, ""),
        place("T:R", "R2", -10.16, 7.62, 90, ""),
        wire(&[(0.0, 3.81), (0.0, 12.7)]),
        wire(&[(-6.35, 7.62), (6.35, 7.62)]),
        place("power:PWR_FLAG", "#FLG01", 0.0, 7.62, 0, ""),
    ]
    .concat();
    // A second, harmless flag at the far end of the vertical wire lands in the group the crossing
    // flag joins; only the crossing flag is named.
    let crossing = crossing + &place("power:PWR_FLAG", "#FLG02", 0.0, 12.7, 0, "");
    let refused = import(&file(&crossing), "Flag");
    assert!(matches!(&refused, Err(CommandError::InvalidParams(m)) if m.contains("#FLG01") && !m.contains("#FLG02")), "{refused:?}");
    // On the end of a wire it joins nothing else and is still left out.
    let on_net =
        [place("T:R", "R1", 0.0, 0.0, 0, ""), wire(&[(0.0, 3.81), (0.0, 7.62)]), place("power:PWR_FLAG", "#FLG01", 0.0, 7.62, 0, "")].concat();
    assert!(import(&file(&on_net), "Flag").is_ok());
}

#[test]
fn a_kicad_8_file_names_power_nets_by_value_even_when_the_pin_says_otherwise() {
    // KiCad 8 and later name the net after the Value, so AGND on a GND-pinned symbol is AGND.
    let items = [
        r#"(generator_version "9.0")"#.to_owned(),
        place("T:R", "R1", 0.0, 0.0, 0, ""),
        place("power:GND", "#PWR01", 0.0, 3.81, 0, ""),
        place("T:R", "R2", 10.16, 0.0, 0, ""),
        place("power:GND", "#PWR02", 10.16, 3.81, 0, ""),
    ]
    .concat()
    .replace(r##"(property "Reference" "#PWR01") (property "Value" "GND")"##, r##"(property "Reference" "#PWR01") (property "Value" "AGND")"##);
    let found = nets(&import(&file(&items), "Eight").unwrap().project);
    // R1 joins AGND alone; R2 joins GND; the two don't meet.
    assert!(found.contains(&net(&["#PWR01.1", "R1.2"])) && found.contains(&net(&["#PWR02.1", "R2.2"])), "{found:?}");
}

#[test]
fn an_alternate_body_style_that_hides_a_pin_refuses_its_symbol() {
    let text = r#"(kicad_sch (version 20231120) (lib_symbols (symbol "L:G" (property "Reference" "U")
        (symbol "G_1_1" (pin input line (at -2.54 0 0) (length 1.27) (number "1")))
        (symbol "G_1_2" (pin input line (at -2.54 0 0) (length 1.27) hide (number "1")))))
        (symbol (lib_id "L:G") (at 0 0 0) (unit 1) (property "Reference" "U1")))"#;
    let refused = import(text, "x");
    assert!(matches!(&refused, Err(CommandError::InvalidParams(m)) if m.contains("hides pins")), "{refused:?}");
}

#[test]
fn power_flags_in_the_middle_of_a_wire_or_on_a_dotted_crossing_are_left_out() {
    // In the middle of a wire from R1 to R2: it joins nothing the wire doesn't.
    let mid = [
        place("T:R", "R1", 0.0, 0.0, 0, ""),
        place("T:R", "R2", 0.0, 20.32, 180, ""),
        wire(&[(0.0, 3.81), (0.0, 16.51)]),
        place("power:PWR_FLAG", "#FLG01", 0.0, 10.16, 0, ""),
    ]
    .concat();
    let found = nets(&import(&file(&mid), "Mid").unwrap().project);
    assert!(found.contains(&net(&["R1.2", "R2.2"])), "{found:?}");
    // On a crossing that already has a dot: the dot joins the wires anyway.
    let dotted = [
        place("T:R", "R1", 0.0, 0.0, 0, ""),
        place("T:R", "R2", -10.16, 7.62, 90, ""),
        wire(&[(0.0, 3.81), (0.0, 12.7)]),
        wire(&[(-6.35, 7.62), (6.35, 7.62)]),
        "(junction (at 0 7.62))".into(),
        place("power:PWR_FLAG", "#FLG01", 0.0, 7.62, 0, ""),
    ]
    .concat();
    let found = nets(&import(&file(&dotted), "Dotted").unwrap().project);
    assert!(found.contains(&net(&["R1.2", "R2.2"])), "{found:?}");
}
