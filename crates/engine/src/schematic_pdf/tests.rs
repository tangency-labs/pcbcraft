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
    let mut at = [f64::NAN; 2];
    for line in objects[3].lines() {
        if line == "ET" {
            runs.push((std::mem::take(&mut text), at));
        } else if let Some(matrix) = line.strip_suffix(" Tm") {
            let matrix: Vec<f64> = matrix.split_whitespace().map(|n| n.parse().unwrap()).collect();
            at = [matrix[4], matrix[5]];
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
