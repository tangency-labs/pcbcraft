use super::*;

const DIVIDER: &str = include_str!("../../tests/fixtures/divider.kicad_sch");

/// A point from millimetres, for readable expectations.
fn mm(x: f64, y: f64) -> Point {
    Point { x_nm: nm(x).unwrap(), y_nm: nm(y).unwrap() }
}

/// A minimal file around `items`, with a fixed identity.
fn file(items: &str) -> String {
    format!("(kicad_sch (version 20231120) (uuid \"aaaaaaaa-0000-4000-8000-000000000000\") {items})")
}

#[test]
fn reads_the_divider_fixture() {
    let s = read_schematic(DIVIDER).unwrap();
    assert_eq!(s.version, Some(20_231_120));
    assert_eq!(s.uuid.as_deref(), Some("0d1e0000-0000-4000-8000-000000000001"));
    assert_eq!(s.paper, Some(Paper { name: "A4".into(), width_nm: 297_000_000, height_nm: 210_000_000, portrait: false }));
    assert_eq!(s.lib_symbols.keys().map(String::as_str).collect::<Vec<_>>(), ["Device:R", "Sample:Buffer2", "power:+3V3", "power:GND"]);
    let references: Vec<_> = s.symbols.iter().map(|p| (p.reference.as_str(), p.unit)).collect();
    assert_eq!(references, [("R1", 1), ("R2", 1), ("#PWR01", 1), ("#PWR02", 1), ("U1", 1), ("U1", 2)]);
    assert_eq!((s.wires.len(), s.junctions.len(), s.no_connects.len(), s.labels.len(), s.texts.len()), (6, 1, 1, 2, 1));
    assert!(s.buses.is_empty() && s.bus_entries.is_empty() && s.sheets.is_empty() && s.graphics.is_empty());
    assert!(s.notes.is_empty() && s.lost.is_empty(), "{:?} {:?}", s.notes, s.lost);

    let r1 = &s.symbols[0];
    assert_eq!((r1.lib_symbol.as_str(), r1.at, r1.rotation_mdeg, r1.mirror), ("Device:R", mm(101.6, 50.8), 0, Mirror::None));
    assert_eq!(
        (r1.property("Value"), r1.property("Footprint"), r1.property("Tolerance")),
        (Some("10k"), Some("Resistor_SMD:R_0603_1608Metric"), Some("1%"))
    );
    assert!(r1.in_bom && r1.on_board && !r1.dnp);
    let footprint = r1.properties.iter().find(|p| p.key == "Footprint").unwrap();
    assert!(footprint.hidden && footprint.rotation_mdeg == 90_000);

    let vout = &s.labels[0];
    assert_eq!((vout.kind, vout.name.as_str(), vout.at, vout.shape.as_deref()), (LabelKind::Local, "VOUT", mm(109.22, 57.15), None));
    let input = &s.labels[1];
    assert_eq!((input.kind, input.rotation_mdeg, input.shape.as_deref()), (LabelKind::Global, 180_000, Some("input")));
    assert_eq!(s.wires[3].points, [mm(101.6, 57.15), mm(109.22, 57.15)]);
    assert_eq!(s.no_connects[0].at, mm(132.08, 50.8));
}

#[test]
fn library_symbols_keep_units_pins_and_power() {
    let s = read_schematic(DIVIDER).unwrap();
    let r = &s.lib_symbols["Device:R"];
    assert!(!r.power && r.pin_numbers_hidden && !r.pin_names_hidden);
    assert_eq!(r.property("Reference"), Some("R"));
    assert_eq!(r.units.iter().map(|u| (u.unit, u.style)).collect::<Vec<_>>(), [(0, 1), (1, 1)]);
    let pins = &r.units[1].pins;
    assert_eq!(pins.len(), 2);
    // Symbol frame: y grows upward, so pin 1 sits above the body.
    assert_eq!((pins[0].number.as_str(), pins[0].at, pins[0].rotation_mdeg, pins[0].length_nm), ("1", mm(0.0, 3.81), 270_000, 1_270_000));
    assert_eq!((pins[0].electrical, pins[0].name.as_str(), pins[0].style.as_str()), (Electrical::Passive, "", "line"));
    assert_eq!(r.units[0].graphics[0].shape, Shape::Rectangle { start: mm(-1.016, -2.54), end: mm(1.016, 2.54) });
    assert_eq!(r.units[0].graphics[0].stroke_nm, 254_000);

    let buffer = &s.lib_symbols["Sample:Buffer2"];
    assert_eq!(buffer.units.iter().map(|u| (u.unit, u.name.as_deref())).collect::<Vec<_>>(), [(0, None), (1, Some("A")), (2, Some("B"))]);
    assert_eq!(
        buffer.units[0].pins.iter().map(|p| (p.number.as_str(), p.electrical)).collect::<Vec<_>>(),
        [("5", Electrical::PowerIn), ("6", Electrical::PowerIn)]
    );
    assert_eq!(buffer.units[1].graphics[0].fill, Fill::Background);

    let power = &s.lib_symbols["power:+3V3"];
    assert!(power.power && power.pin_names_hidden);
    let pin = &power.units[1].pins[0];
    assert_eq!((pin.name.as_str(), pin.electrical, pin.hidden, pin.length_nm), ("+3V3", Electrical::PowerIn, true, 0));
}

#[test]
fn derived_symbols_draw_as_their_parent() {
    let s = read_schematic(&file(
        r#"(lib_symbols
            (symbol "Lib:Base" (property "Reference" "Q") (symbol "Base_1_1" (pin passive line (at 0 2.54 270) (length 1.27) (number "1"))))
            (symbol "Lib:Child" (extends "Base") (property "Reference" "QC") (property "Value" "Child"))
            (symbol "Lib:Grandchild" (extends "Lib:Child") (property "Value" "Grand"))
            (symbol "Lib:Orphan" (extends "Missing")))"#,
    ))
    .unwrap();
    let child = &s.lib_symbols["Lib:Child"];
    assert_eq!((child.property("Reference"), child.property("Value")), (Some("QC"), Some("Child")));
    assert_eq!(child.units, s.lib_symbols["Lib:Base"].units);
    assert_eq!(s.lib_symbols["Lib:Grandchild"].units, s.lib_symbols["Lib:Base"].units);
    assert!(!s.lib_symbols.contains_key("Lib:Orphan"));
    assert_eq!(s.notes.iter().collect::<Vec<_>>(), [("library symbol whose `extends` chain is broken left out", 1)]);
}

#[test]
fn an_extends_loop_is_broken_not_followed() {
    let s = read_schematic(&file(r#"(lib_symbols (symbol "L:A" (extends "L:B")) (symbol "L:B" (extends "L:A")))"#)).unwrap();
    assert!(s.lib_symbols.is_empty());
    assert_eq!(s.notes.iter().map(|(_, n)| n).sum::<usize>(), 2);
}

#[test]
fn references_come_from_the_record_for_this_file_then_older_sources() {
    let symbol =
        |uuid: &str, extra: &str| format!(r#"(symbol (lib_id "L:R") (at 0 0 0) (unit 1) (uuid "{uuid}") (property "Reference" "R?") {extra})"#);
    let text = file(&[
        // KiCad 7+: the record for this file's own path wins over an earlier record for another.
        symbol(
            "bbbbbbbb-0000-4000-8000-000000000001",
            r#"(instances (project "p" (path "/other" (reference "R9") (unit 1)) (path "/aaaaaaaa-0000-4000-8000-000000000000" (reference "R1") (unit 2))))"#,
        ),
        // Another project's record only: it is still the best this file has.
        symbol("bbbbbbbb-0000-4000-8000-000000000002", r#"(instances (project "p" (path "/other" (reference "R7"))))"#),
        // KiCad 6: no records of its own; the file's `symbol_instances` gives it.
        symbol("bbbbbbbb-0000-4000-8000-000000000003", ""),
        // Nothing but its field.
        symbol("bbbbbbbb-0000-4000-8000-000000000004", ""),
        r#"(symbol_instances (path "/BBBBBBBB-0000-4000-8000-000000000003" (reference "R3") (unit 1)))"#.to_owned(),
    ]
    .concat());
    let s = read_schematic(&text).unwrap();
    let found: Vec<_> = s.symbols.iter().map(|p| (p.reference.as_str(), p.unit)).collect();
    assert_eq!(found, [("R1", 2), ("R7", 1), ("R3", 1), ("R?", 1)]);
    let sources: Vec<_> = s.symbols.iter().map(|p| p.reference_from).collect();
    assert_eq!(sources, [ReferenceSource::OwnRecord, ReferenceSource::OtherRecord, ReferenceSource::SymbolInstances, ReferenceSource::Field]);
    assert_eq!(s.symbols[0].instances.len(), 2);
    assert_eq!(s.symbols[2].instances, [Instance { path: "/bbbbbbbb-0000-4000-8000-000000000003".into(), reference: "R3".into(), unit: Some(1) }]);
}

#[test]
fn placement_rotation_mirror_and_flags_are_read_as_written() {
    let s = read_schematic(&file(
        r#"(symbol (lib_id "L:R") (lib_name "R_1") (at 10 -5 -90) (mirror y) (unit 3) (in_bom no) (on_board no) (dnp yes))
           (symbol (lib_id "L:R") (at 1 2 180) (mirror x))
           (symbol (lib_id "") (at 1 2 0))"#,
    ))
    .unwrap();
    let a = &s.symbols[0];
    assert_eq!(
        (a.lib_symbol.as_str(), a.lib_id.as_str(), a.at, a.rotation_mdeg, a.mirror, a.unit),
        ("R_1", "L:R", mm(10.0, -5.0), 270_000, Mirror::Y, 3)
    );
    assert!(!a.in_bom && !a.on_board && a.dnp);
    assert_eq!((s.symbols[1].rotation_mdeg, s.symbols[1].mirror, s.symbols[1].reference.as_str()), (180_000, Mirror::X, "?"));
    assert_eq!(s.symbols.len(), 2);
    assert_eq!(s.lost.iter().collect::<Vec<_>>(), [("placed symbol without a library symbol or out of range left out", 1)]);
}

#[test]
fn sheets_buses_and_unsupported_items_are_kept_apart_or_noted() {
    let s = read_schematic(&file(
        r#"(bus (pts (xy 0 0) (xy 10 0))) (bus_entry (at 10 0) (size 2.54 2.54))
           (sheet (at 20 20) (size 30 20) (property "Sheetname" "Power") (property "Sheetfile" ".\\sub\\power.kicad_sch")
                  (pin "VIN" input (at 20 25 180)))
           (text_box "x") (image (at 0 0)) (rectangle (start 0 0) (end 5 5)) (mystery 1)
           (wire (pts (xy 0 0))) (label "" (at 0 0 0)) (junction (at 1e30 0))"#,
    ))
    .unwrap();
    assert_eq!((s.buses.len(), s.bus_entries.len(), s.graphics.len()), (1, 1, 1));
    assert_eq!(s.sheets[0].name, "Power");
    assert_eq!(s.sheets[0].file, "sub/power.kicad_sch");
    assert_eq!(s.sheets[0].pins, [SheetPin { name: "VIN".into(), shape: Some("input".into()), at: mm(20.0, 25.0) }]);
    // Drawings and text are notes; whatever can carry a connection is lost, which callers refuse.
    let notes: Vec<_> = s.notes.iter().map(|(what, _)| what).collect();
    assert_eq!(notes, ["image left out", "text box left out"]);
    let lost: Vec<_> = s.lost.iter().map(|(what, _)| what).collect();
    assert_eq!(
        lost,
        [
            "junction out of range left out",
            "label that is blank or out of range left out",
            "mystery left out",
            "wire without two points in range left out",
        ]
    );
}

#[test]
fn other_files_and_oversized_input_are_refused() {
    assert_eq!(read_schematic("(kicad_pcb (version 1))"), Err(Error::WrongFormat("a KiCad schematic")));
    assert!(matches!(read_schematic("(kicad_sch (wire"), Err(Error::Syntax { .. })));
    let huge = format!("(kicad_sch {})", " ".repeat(MAX_BYTES));
    assert!(matches!(read_schematic(&huge), Err(Error::TooLarge(_))));
}

#[test]
fn paper_sizes_turn_for_portrait_and_users_give_their_own() {
    let paper = |p: &str| read_schematic(&file(p)).unwrap().paper;
    assert_eq!(paper("(paper \"A3\" portrait)").map(|p| (p.width_nm, p.height_nm)), Some((297_000_000, 420_000_000)));
    assert_eq!(paper("(paper \"User\" 100 50)").map(|p| (p.width_nm, p.height_nm)), Some((100_000_000, 50_000_000)));
    assert_eq!(paper("(paper \"Napkin\")"), None);
}

#[test]
fn symbol_text_angles_read_tenths_unless_only_degrees_make_a_quarter_turn() {
    let s = read_schematic(&file(
        r#"(lib_symbols (symbol "L:T" (symbol "T_1_1" (text "a" (at 0 0 900)) (text "b" (at 0 0 90)) (text "c" (at 0 0 455)))))
           (text "d" (at 0 0 90))"#,
    ))
    .unwrap();
    let angles: Vec<_> = s.lib_symbols["L:T"].units[0].texts.iter().map(|t| t.rotation_mdeg).collect();
    assert_eq!(angles, [90_000, 90_000, 45_500]);
    assert_eq!(s.texts[0].rotation_mdeg, 90_000);
}
