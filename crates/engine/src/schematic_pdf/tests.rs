use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::*;
use crate::{
    Session,
    persistence::{decode_documents, encode_documents, prepare_opened},
};

/// Build an ordinary document through the command catalog, with the selected sheet not first.
fn fixture() -> (Session, SheetTarget) {
    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let first = session.project().unwrap().sheet_target();
    apply(&mut session, crate::net_labels::ADD, json!({"sheet":first.id(),"name":"OTHER_SHEET_ONLY","style":"local","x_nm":0,"y_nm":0}));
    apply(&mut session, crate::sheets::CREATE_SHEET, json!({"name":"Synthetic Output"}));
    let target = session.project().unwrap().sheets().nth(1).unwrap().target;
    let id = target.id();
    apply(&mut session, crate::sheets::page::SET_PAGE, json!({"sheet":id,"paper":"a4","orientation":"landscape","at":{"x_nm":0,"y_nm":0}}));
    apply(&mut session, crate::parameters::SET, json!({"scope":"sheet","sheet":id,"name":"Title","value":"Révision (B)"}));
    for x in [20, 40, 60] {
        apply(&mut session, crate::PLACE_SYMBOL, json!({"sheet":id,"definition":"resistor","x_nm":x*GRID_NM,"y_nm":20*GRID_NM}));
    }
    // Use the same placement identities as the command contract, not guessed symbol ordinals.
    let view = session.project().unwrap().for_sheet(target).unwrap();
    let a = view.symbols()[0].id();
    let b = view.symbols()[1].id();
    apply(&mut session, crate::DRAW_WIRE, json!({"sheet":id,"from":{"symbol":a,"terminal":"2"},"to":{"symbol":b,"terminal":"1"},"bends":[]}));
    for (name, style, x) in [("CLOCK_IN", "local", 15), ("POWER_OUT", "global", 65)] {
        apply(&mut session, crate::net_labels::ADD, json!({"sheet":id,"name":name,"style":style,"x_nm":x*GRID_NM,"y_nm":20*GRID_NM}));
    }
    (session, target)
}

/// Keep fixture edits subject to ordinary revision checks.
fn apply(session: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

/// Follow the trailer's xref offsets and validate that every stream's declared byte count ends
/// at its actual delimiter. Read the PDF rather than using any writer state as the oracle.
fn objects(bytes: &[u8]) -> Vec<&str> {
    let file = std::str::from_utf8(bytes).unwrap();
    let (xref, _) = file.rsplit_once("startxref\n").unwrap().1.split_once('\n').unwrap();
    let mut entries = file[xref.parse::<usize>().unwrap()..].lines();
    assert_eq!(entries.next(), Some("xref"));
    let count = entries.next().unwrap().strip_prefix("0 ").unwrap().parse::<usize>().unwrap();
    assert_eq!(entries.next(), Some("0000000000 65535 f "));
    (1..count)
        .map(|id| {
            let entry = entries.next().unwrap();
            assert_eq!(&entry[10..], " 00000 n ");
            let offset = entry[..10].parse::<usize>().unwrap();
            let object = file[offset..].strip_prefix(&format!("{id} 0 obj\n")).unwrap().split_once("\nendobj\n").unwrap().0;
            if let Some((header, content)) = object.split_once("\nstream\n") {
                let size = header.split_whitespace().nth(2).unwrap().parse::<usize>().unwrap();
                assert_eq!(&content[size..], "\nendstream");
            }
            object
        })
        .collect()
}

/// Extract shown strings using page font resources and their ToUnicode maps, not by looking
/// for metadata or expected string literals in the output. Each PDF text object starts a line.
fn extract(objects: &[&str]) -> String {
    runs(objects).into_iter().map(|(text, _)| text + "\n").collect()
}

/// Decode each PDF text object with the translation of its text matrix, in points.
fn runs(objects: &[&str]) -> Vec<(String, [f64; 2])> {
    text_runs(objects).into_iter().map(|(text, matrix)| (text, [matrix[4], matrix[5]])).collect()
}

/// Decode each PDF text object with its full 6-number text matrix (`a b c d e f Tm`).
fn text_runs(objects: &[&str]) -> Vec<(String, [f64; 6])> {
    let resources = objects[2].split_once("/Font << ").unwrap().1.split_once(">>").unwrap().0;
    let resources: Vec<_> = resources.split_whitespace().collect();
    let mut encodings = BTreeMap::new();
    let (resources, remainder) = resources.as_chunks::<4>();
    assert!(remainder.is_empty());
    for resource in resources {
        let font = objects[resource[1].parse::<usize>().unwrap() - 1];
        let cmap_id = font.split_once("/ToUnicode ").unwrap().1.split_whitespace().next().unwrap().parse::<usize>().unwrap();
        let mut encoding = BTreeMap::new();
        let mut inside = false;
        for line in objects[cmap_id - 1].lines() {
            if line.ends_with(" beginbfchar") {
                inside = true;
            } else if line == "endbfchar" {
                inside = false;
            } else if inside {
                let (code, hex) = line.trim_start_matches('<').trim_end_matches('>').split_once("> <").unwrap();
                let (units, remainder) = hex.as_bytes().as_chunks::<4>();
                assert!(remainder.is_empty());
                let utf16: Vec<_> = units.iter().map(|unit| u16::from_str_radix(std::str::from_utf8(unit).unwrap(), 16).unwrap()).collect();
                encoding.insert(u8::from_str_radix(code, 16).unwrap(), String::from_utf16(&utf16).unwrap());
            }
        }
        encodings.insert(resource[0], encoding);
    }
    let mut runs = Vec::new();
    let mut text = String::new();
    let mut matrix = [f64::NAN; 6];
    for line in objects[3].lines() {
        if line == "ET" {
            runs.push((std::mem::take(&mut text), matrix));
        } else if let Some(tm) = line.strip_suffix(" Tm") {
            let parsed: Vec<f64> = tm.split_whitespace().map(|n| n.parse().unwrap()).collect();
            matrix = parsed.try_into().unwrap();
        } else if let Some((font, run)) = line.split_once(" Tf <") {
            let font = font.split_whitespace().next().unwrap();
            let run = run.strip_suffix("> Tj").unwrap();
            let (pairs, remainder) = run.as_bytes().as_chunks::<2>();
            assert!(remainder.is_empty());
            for code in pairs {
                let code = u8::from_str_radix(std::str::from_utf8(code).unwrap(), 16).unwrap();
                text.push_str(&encodings[font][&code]);
            }
        }
    }
    runs
}

/// One vector page uses the addressed sheet's paper and resolves its frame text. Reopening
/// the command-built project and passing a different active projection produce the same bytes.
#[test]
fn addressed_sheet_has_searchable_text_and_its_saved_media_box() {
    let (session, target) = fixture();
    let before = encode_documents(session.snapshot().unwrap()).unwrap();
    let project = prepare_opened(decode_documents(&before).unwrap()).unwrap();
    let bytes = export(&project, target).unwrap();
    assert_eq!(bytes, export(&project.for_sheet(target).unwrap(), target).unwrap());
    assert_eq!(encode_documents(project).unwrap(), before);
    let objects = objects(&bytes);
    assert!(objects[1].contains("/Count 1 /Kids [3 0 R]"));
    assert!(!objects.iter().any(|object| object.contains("/Subtype /Image")));
    let media = objects[2].split_once("/MediaBox [").unwrap().1.split_once(']').unwrap().0;
    let dims: Vec<f64> = media.split_whitespace().map(|n| n.parse().unwrap()).collect();
    assert_eq!(dims[..2], [0., 0.]);
    // A4 landscape is 297 × 210 mm (ISO 216), as declared by the shared sheet-paper model.
    assert!((dims[2] * 25.4 / 72. - 297.).abs() < 0.1);
    assert!((dims[3] * 25.4 / 72. - 210.).abs() < 0.1);
    assert!(objects[3].contains(" re W n\n"));
    assert!(objects[3].contains(" l\nS\n"));
    // The first frame segment starts at its 5 mm inset from the stored top-left,
    // not at the drawing origin. Read the emitted coordinates with an independent conversion.
    let (x, y) = objects[3].lines().find_map(|line| line.strip_suffix(" m")).unwrap().split_once(' ').unwrap();
    assert!((x.parse::<f64>().unwrap() * 25.4 / 72. - 5.).abs() < 0.1);
    assert!((y.parse::<f64>().unwrap() * 25.4 / 72. - 205.).abs() < 0.1);
    let text = extract(&objects);
    for expected in ["R1", "R2", "R3", "CLOCK_IN", "POWER_OUT", "Révision (B)"] {
        assert!(text.lines().any(|line| line == expected), "missing {expected}: {text}");
    }
    assert!(!text.contains("OTHER_SHEET_ONLY"));
}

/// A quarter-turned two-pin part keeps automatic Reference and Value to the right of its body,
/// left-aligned above and below its centre line, as the canvas does. Centring them on the
/// symbol would put them on the vertical pin axis, where the connecting wires continue.
#[test]
fn quarter_turned_fields_sit_beside_the_body() {
    let (mut session, target) = fixture();
    let id = target.id();
    let (x, y) = (80 * GRID_NM, 40 * GRID_NM);
    apply(&mut session, crate::PLACE_SYMBOL, json!({"sheet":id,"definition":"resistor","x_nm":x,"y_nm":y,"rotation":90}));
    let view = session.project().unwrap().for_sheet(target).unwrap();
    let turned = view.symbols().iter().find(|s| s.turns() == 1).unwrap().id();
    apply(&mut session, crate::SET_SYMBOL_FIELDS, json!({"sheet":id,"symbol":turned,"reference":"R9","value":"4k7"}));
    let bytes = export(session.project().unwrap(), target).unwrap();
    let runs = runs(&objects(&bytes));
    let find = |shown: &str| runs.iter().find(|(text, _)| text == shown).unwrap().1;
    let (reference, value) = (find("R9"), find("4k7"));
    // The A4 paper's top-left is the sheet origin. Upright Start-aligned text begins at its anchor.
    let axis = x as f64 * PT_PER_NM;
    let middle = (210_000_000 - y) as f64 * PT_PER_NM;
    let height = Layout::default().height_nm as f64 * PT_PER_NM;
    assert_eq!(reference[0], value[0]);
    assert!(reference[0] > axis + 0.4 * GRID_NM as f64 * PT_PER_NM, "{reference:?} on the pin axis {axis}");
    assert!(reference[1] > middle, "{reference:?} not above {middle}");
    assert!(value[1] + height < middle, "{value:?} not below {middle}");
    // Unturned parts keep their centred placement on the symbol's vertical axis.
    let unturned = find("R1");
    assert!(unturned[0] < 20. * GRID_NM as f64 * PT_PER_NM);
}

/// A line-only body uses the centered two-grid canvas fallback, not the off-axis line bounds.
#[test]
fn zero_area_turned_body_uses_centered_field_fallback() {
    let (session, target) = fixture();
    let sheet = session.project().unwrap().for_sheet(target).unwrap();
    let mut symbol = sheet.symbols()[0].clone();
    symbol.rotation = 1;
    let horizontal = Shape::Line(Point { x_nm: -GRID_NM, y_nm: -3 * GRID_NM }, Point { x_nm: GRID_NM, y_nm: -3 * GRID_NM });
    let vertical = Shape::Line(Point { x_nm: 3 * GRID_NM, y_nm: -GRID_NM }, Point { x_nm: 3 * GRID_NM, y_nm: GRID_NM });
    let centered = [add(symbol.position, -GRID_NM, -GRID_NM), add(symbol.position, GRID_NM, GRID_NM)];
    assert_eq!(body_bounds(&symbol, &[horizontal]), centered);
    assert_eq!(body_bounds(&symbol, &[vertical]), centered);
}

/// A saved net-name offset follows the contact's quarter-turn without turning the text itself.
#[test]
fn saved_net_name_offset_turns_with_contact() {
    let (mut session, target) = fixture();
    let (x, y) = (80 * GRID_NM, 40 * GRID_NM);
    let text = Layout { offset: Some(Point { x_nm: GRID_NM, y_nm: 0 }), align: TextAlign::Start, ..Layout::default() };
    apply(
        &mut session,
        crate::net_labels::ADD,
        json!({"sheet":target.id(),"name":"TURNED_NET","style":"local","x_nm":x,"y_nm":y,"rotation":90,"text":text}),
    );
    let bytes = export(session.project().unwrap(), target).unwrap();
    let runs = runs(&objects(&bytes));
    let (_, [tx, ty]) = runs.iter().find(|(name, _)| name == "TURNED_NET").unwrap();
    let contact_x = x as f64 * PT_PER_NM;
    let contact_y = (210_000_000 - y) as f64 * PT_PER_NM;
    let rise = (GRID_NM as f64 - text.height_nm as f64 / 2.) * PT_PER_NM;
    assert!((tx - contact_x).abs() < 0.00001 && (ty - contact_y - rise).abs() < 0.00001, "unexpected text position: {tx}, {ty}");
}

/// Input and output names sit on opposite sides of open markers whose tips stay on the contacts.
#[test]
fn hierarchy_labels_point_to_their_contacts() {
    let (mut session, target) = fixture();
    let (x, y) = (80 * GRID_NM, 40 * GRID_NM);
    let labels = [("INPUT_PORT", "input", y, -1.), ("OUTPUT_PORT", "output", y + 4 * GRID_NM, 1.)];
    for (name, direction, y, _) in labels {
        apply(&mut session, crate::hierarchy::ADD_LABEL, json!({"sheet":target.id(),"name":name,"direction":direction,"x_nm":x,"y_nm":y}));
    }
    let bytes = export(session.project().unwrap(), target).unwrap();
    let objects = objects(&bytes);
    let runs = runs(&objects);
    let contact_x = x as f64 * PT_PER_NM;
    let grid = GRID_NM as f64 * PT_PER_NM;
    for (name, _, y, side) in labels {
        let contact_y = (210_000_000 - y) as f64 * PT_PER_NM;
        let back = contact_x + side * grid / 2.;
        let top = contact_y + grid / 4.;
        let bottom = contact_y - grid / 4.;
        let marker = format!("{back:.5} {top:.5} m\n{contact_x:.5} {contact_y:.5} l\n{back:.5} {bottom:.5} l\nS\n");
        assert!(objects[3].contains(&marker), "{name} marker does not meet the contact");
        let (_, [tx, ty]) = runs.iter().find(|(text, _)| text == name).unwrap();
        assert!((tx - back) * side > 0., "{name} is on the wrong side of its marker");
        assert!((ty + grid / 2. - contact_y).abs() < 0.00001, "{name} is not centered at its contact height");
    }
}

/// Look up the Type 3 glyph stream emitted for `target` via `/ToUnicode` and `/CharProcs`.
fn glyph_program<'a>(objects: &[&'a str], target: char) -> &'a str {
    let want = target.to_string();
    let resources = objects[2].split_once("/Font << ").unwrap().1.split_once(">>").unwrap().0;
    let resources: Vec<_> = resources.split_whitespace().collect();
    let (resources, remainder) = resources.as_chunks::<4>();
    assert!(remainder.is_empty());
    for resource in resources {
        let font = objects[resource[1].parse::<usize>().unwrap() - 1];
        let cmap_id = font.split_once("/ToUnicode ").unwrap().1.split_whitespace().next().unwrap().parse::<usize>().unwrap();
        let mut inside = false;
        for line in objects[cmap_id - 1].lines() {
            if line.ends_with(" beginbfchar") {
                inside = true;
            } else if line == "endbfchar" {
                inside = false;
            } else if inside {
                let (code, hex) = line.trim_start_matches('<').trim_end_matches('>').split_once("> <").unwrap();
                let (units, remainder) = hex.as_bytes().as_chunks::<4>();
                assert!(remainder.is_empty());
                let utf16: Vec<_> = units.iter().map(|unit| u16::from_str_radix(std::str::from_utf8(unit).unwrap(), 16).unwrap()).collect();
                if String::from_utf16(&utf16).unwrap() == want {
                    let code = u8::from_str_radix(code, 16).unwrap();
                    let procs = font.split_once("/CharProcs << ").unwrap().1.split_once(">>").unwrap().0;
                    let proc_id = procs.split_once(&format!(" /g{code} ")).unwrap().1.split_whitespace().next().unwrap().parse::<usize>().unwrap();
                    return objects[proc_id - 1].split_once("\nstream\n").unwrap().1.split_once("\nendstream").unwrap().0;
                }
            }
        }
    }
    panic!("missing glyph for {target:?}");
}

/// Resistor values with `Ω` (and `Ω`) draw a visible ohm glyph rather than falling back to `?`,
/// while still extracting as the original Unicode through `/ToUnicode`. Common EE unit symbols
/// (`µ`, `μ`, `°`, `±`) likewise emit real glyphs instead of `?`.
#[test]
fn ohm_and_common_unit_symbols_draw_real_glyphs() {
    let (mut session, target) = fixture();
    let id = target.id();
    let view = session.project().unwrap().for_sheet(target).unwrap();
    let r1 = view.symbols()[0].id();
    let r2 = view.symbols()[1].id();
    let r3 = view.symbols()[2].id();
    apply(&mut session, crate::SET_SYMBOL_FIELDS, json!({"sheet":id,"symbol":r1,"reference":"R1","value":"10 kΩ"}));
    apply(&mut session, crate::SET_SYMBOL_FIELDS, json!({"sheet":id,"symbol":r2,"reference":"R2","value":"10 k\u{2126} 10 µF 10 μF"}));
    apply(&mut session, crate::SET_SYMBOL_FIELDS, json!({"sheet":id,"symbol":r3,"reference":"R3","value":"±5% 25°C ?"}));
    let bytes = export(session.project().unwrap(), target).unwrap();
    let objects = objects(&bytes);
    let text = extract(&objects);
    for expected in ["10 kΩ", "10 k\u{2126} 10 µF 10 μF", "±5% 25°C ?"] {
        assert!(text.lines().any(|line| line == expected), "missing {expected}: {text}");
    }
    let question = glyph_program(&objects, '?');
    let omega = glyph_program(&objects, 'Ω');
    let ohm_sign = glyph_program(&objects, '\u{2126}');
    assert_ne!(omega, question, "Ω fell back to '?'");
    assert_eq!(ohm_sign, omega, "U+2126 should draw the same ohm glyph as U+03A9");
    let points: Vec<[f64; 2]> = omega
        .lines()
        .filter_map(|line| line.strip_suffix(" m").or_else(|| line.strip_suffix(" l")))
        .map(|coords| {
            let (x, y) = coords.split_once(' ').unwrap();
            [x.parse().unwrap(), y.parse().unwrap()]
        })
        .collect();
    assert!(points.len() >= 10, "ohm glyph should have a smooth arch: {points:?}");
    let first = points[0];
    let second = points[1];
    let before_last = points[points.len() - 2];
    let last = points[points.len() - 1];
    // Left and right feet sit on the baseline; the arch peaks symmetrically at full cap height.
    assert!(first[1].abs() < 0.1 && second[1].abs() < 0.1 && second[0] > first[0]);
    assert!(before_last[1].abs() < 0.1 && last[1].abs() < 0.1 && last[0] > before_last[0]);
    let apex = points.iter().copied().max_by(|a, b| a[1].total_cmp(&b[1])).unwrap();
    assert!((apex[1] - 930.).abs() < 1., "unexpected ohm arch height: {apex:?}");
    assert!((apex[0] - (first[0] + last[0]) / 2.).abs() < 1., "ohm arch is not centered: {apex:?}");
    for symbol in ['µ', 'μ', '°', '±'] {
        assert_ne!(glyph_program(&objects, symbol), question, "{symbol:?} fell back to '?'");
    }
    assert_eq!(glyph_program(&objects, 'μ'), glyph_program(&objects, 'µ'));
}

/// An independent viewer's text extractor can read the exact same command-built fixture.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "requires Poppler pdftotext on PATH"]
fn poppler_extracts_addressed_sheet_text() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };

    let (session, target) = fixture();
    let bytes = export(session.project().unwrap(), target).unwrap();
    let mut poppler = Command::new("pdftotext")
        .args(["-enc", "UTF-8", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input = poppler.stdin.take().unwrap().write_all(&bytes);
    let output = poppler.wait_with_output().unwrap();
    input.unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8(output.stdout).unwrap();
    for expected in ["R1", "R2", "R3", "CLOCK_IN", "POWER_OUT", "Révision (B)"] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert!(!text.contains("OTHER_SHEET_ONLY"));
}

/// The same request refuses an unknown address and a legacy sheet without adopted paper.
#[test]
fn refuses_foreign_address_and_missing_paper() {
    let (session, target) = fixture();
    let mut project = session.project().unwrap().for_sheet(target).unwrap();
    let foreign = SheetTarget { sheet: uuid::Uuid::nil(), ..target };
    assert_eq!(export(&project, foreign), Err(PdfError::UnknownSheet));
    assert!(export(&project, target).is_ok());
    project.sheet_mut().page = None;
    assert_eq!(export(&project, target), Err(PdfError::MissingPaper));
}

/// A run crossing a Type 3 code boundary still preserves UTF-16 surrogate pairs and the
/// original characters, even when the regular face uses a visible fallback glyph.
#[test]
fn unicode_runs_cross_font_dictionaries_without_losing_text() {
    let text: String = ('\u{100}'..='\u{200}').chain(['\u{1F680}', '\u{100}']).collect();
    let mut page = pdf::Page::new([1000., 600.]);
    page.text([10., 50.], 4., 0., TextAlign::Start, &text).unwrap();
    let bytes = page.finish().unwrap();
    let objects = objects(&bytes);
    assert!(objects[2].contains("/F1 "));
    assert_eq!(extract(&objects).trim_end(), text);
}

/// Selecting an alternate pin function replaces the primary pin label in exported schematic PDFs.
#[test]
fn selected_alternate_pin_function_appears_in_exported_pdf() {
    use crate::{
        authoring::{Action, Item, Kind, Primitive},
        library,
        schematic::{AlternatePinFunction, ElectricalType},
    };

    let item = Item::new(Kind::Symbol).unwrap();
    let mut pin = item.objects[&1].clone();
    let Primitive::Pin { name, electrical_type, alternates, .. } = &mut pin else { panic!("expected starter pin") };
    *name = Some("GPIO".into());
    *electrical_type = ElectricalType::Bidirectional;
    *alternates = vec![AlternatePinFunction { name: "ADC_IN".into(), electrical_type: ElectricalType::Input }];
    let mut editor = crate::authoring::Session::new(item, false).unwrap();
    editor.execute(0, Action::Replace { object: 1, primitive: pin }).unwrap();
    let manifest = crate::authoring::publishing::new_library("synthetic", "Synthetic", "MIT OR Apache-2.0").unwrap();
    let published = crate::authoring::publishing::prepare_copy(editor.snapshot(), &manifest).unwrap();
    let published_item = published.item();

    let mut session = Session::default();
    session.apply("project.new", &json!({})).unwrap();
    let target = session.project().unwrap().sheet_target();
    apply(&mut session, crate::sheets::page::SET_PAGE, json!({"sheet":target.id(),"paper":"a4","orientation":"landscape","at":{"x_nm":0,"y_nm":0}}));
    let mut documents = published.documents().clone();
    documents.insert("library.json".into(), manifest);
    session.execute("library.load_source", &json!({"source":"mcu","documents":documents})).unwrap();
    let definition = session.libraries.symbol(&published_item.library, &published_item.id, published_item.revision).unwrap();
    let digest = library::sources::symbol_digest(&definition).unwrap();
    apply(
        &mut session,
        crate::PLACE_SYMBOL,
        json!({
            "library": published_item.library,
            "definition": published_item.id,
            "definition_revision": published_item.revision,
            "definition_digest": digest,
            "x_nm": 20 * GRID_NM,
            "y_nm": 20 * GRID_NM,
        }),
    );

    let before_bytes = export(session.project().unwrap(), target).unwrap();
    let before_text = extract(&objects(&before_bytes));
    assert!(before_text.contains("GPIO"), "missing primary pin label before override: {before_text}");
    assert!(!before_text.contains("ADC_IN"), "unexpected alternate pin label before override: {before_text}");

    let part = session.project().unwrap().symbols()[0].component_id();
    session
        .execute(
            crate::pin_functions::SET,
            &json!({"target":{"part":part,"terminal":"1"},"function":"ADC_IN","expected_revision":session.revision()}),
        )
        .unwrap();

    let after_bytes = export(session.project().unwrap(), target).unwrap();
    let after_text = extract(&objects(&after_bytes));
    assert!(after_text.contains("ADC_IN"), "missing selected alternate pin label in PDF: {after_text}");
    assert!(!after_text.contains("GPIO"), "overridden primary pin label remained in PDF: {after_text}");
}

/// A local net label with a saved 180-degree Start-aligned text appearance preserves that exact
/// rotation and anchor in the PDF content stream instead of flipping upright to 0 degrees and End.
#[test]
fn saved_net_label_text_keeps_180_rotation_and_start_alignment() {
    let (mut session, target) = fixture();
    let (x, y) = (80 * GRID_NM, 40 * GRID_NM);
    let offset = Point { x_nm: 2 * GRID_NM, y_nm: -GRID_NM };
    let text = Layout { offset: Some(offset), rotation: 180, align: TextAlign::Start, ..Layout::default() };
    apply(
        &mut session,
        crate::net_labels::ADD,
        json!({"sheet":target.id(),"name":"INVERTED_NET","style":"local","x_nm":x,"y_nm":y,"rotation":0,"text":text}),
    );
    let bytes = export(session.project().unwrap(), target).unwrap();
    let runs = text_runs(&objects(&bytes));
    let (_, [a, b, c, d, tx, ty]) = *runs.iter().find(|(name, _)| name == "INVERTED_NET").unwrap();
    assert!(
        (a + 1.).abs() < 1e-5 && b.abs() < 1e-5 && c.abs() < 1e-5 && (d + 1.).abs() < 1e-5,
        "expected 180-degree text matrix (-1 0 0 -1), got [{a}, {b}, {c}, {d}]"
    );
    let anchor_x = (x + offset.x_nm) as f64 * PT_PER_NM;
    let anchor_y = (210_000_000 - (y + offset.y_nm)) as f64 * PT_PER_NM;
    let expected_ty = anchor_y + text.height_nm as f64 * PT_PER_NM / 2.;
    assert!(
        (tx - anchor_x).abs() < 1e-4 && (ty - expected_ty).abs() < 1e-4,
        "expected Start-aligned anchor ({anchor_x}, {expected_ty}), got ({tx}, {ty})"
    );
}
