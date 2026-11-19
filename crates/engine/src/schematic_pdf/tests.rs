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
    let mut text = String::new();
    for line in objects[3].lines() {
        if line == "ET" {
            text.push('\n');
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
    text
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
