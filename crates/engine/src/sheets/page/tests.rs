use super::*;
use crate::persistence::{decode_documents, encode_documents, prepare_opened};
use crate::sheets::CREATE_SHEET;

/// Issue commands at the current revision, retaining their ordinary strict validation.
fn apply(session: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

/// A new project with one resistor on its only sheet.
fn project_with_resistor() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    apply(&mut s, crate::PLACE_RESISTOR, json!({"x_nm":0,"y_nm":0}));
    s
}

/// The paper record inside an encoded sheet document.
fn stored_page(documents: &crate::persistence::Documents, sheet: uuid::Uuid) -> Value {
    let document: Value = serde_json::from_slice(&documents[&format!("sheets/{sheet}.json")]).unwrap();
    assert_eq!(document["format"], "pcbcraft-sheet/9");
    document["page"].clone()
}

#[test]
fn paper_sizes_follow_iso_and_orientation() {
    let a3 = Page::new(Paper::A3, Orientation::Landscape, Point { x_nm: 0, y_nm: 0 }).unwrap();
    assert_eq!(a3.size(), Point { x_nm: 420_000_000, y_nm: 297_000_000 });
    let portrait = Page::new(Paper::A3, Orientation::Portrait, Point { x_nm: 0, y_nm: 0 }).unwrap();
    assert_eq!(portrait.size(), Point { x_nm: 297_000_000, y_nm: 420_000_000 });
    for paper in Paper::ALL {
        assert_eq!(Paper::parse(paper.as_str()), Some(paper));
    }
    assert_eq!(Paper::parse("A4"), None);
    assert!(Page::new(Paper::A0, Orientation::Landscape, Point { x_nm: 0, y_nm: 0 }).is_err());
    for extreme in [i64::MAX, i64::MIN] {
        assert!(Page::new(Paper::A4, Orientation::Landscape, Point { x_nm: extreme, y_nm: 0 }).is_err(), "far corners are rejected without overflow");
        assert!(Page::new(Paper::A4, Orientation::Portrait, Point { x_nm: 0, y_nm: extreme }).is_err());
    }
}

/// New sheets start on centered A4; set_page keeps the corner, is one undo step and a no-op
/// when unchanged, and rejects paper outside the coordinate range without changing anything.
#[test]
fn set_page_is_one_undo_step_and_keeps_the_corner() {
    let mut s = project_with_resistor();
    let target = s.project().unwrap().sheet_target();
    let page = s.project().unwrap().page().unwrap();
    assert_eq!(page, Page::new_sheet());
    assert_eq!((page.paper(), page.orientation()), (Paper::A4, Orientation::Landscape));
    let [min, max] = page.corners();
    assert!(min.x_nm < 0 && min.y_nm < 0 && max.x_nm > 0 && max.y_nm > 0);
    assert_eq!(min.x_nm % crate::schematic::GRID_NM, 0);
    let before = s.snapshot().unwrap();
    let revision = s.revision();
    apply(&mut s, SET_PAGE, json!({"sheet":target.id(),"paper":"a4","orientation":"landscape"}));
    assert_eq!(s.revision(), revision);
    apply(&mut s, SET_PAGE, json!({"sheet":target.id(),"paper":"a2","orientation":"portrait"}));
    let changed = s.project().unwrap().page().unwrap();
    assert_eq!((changed.paper(), changed.orientation(), changed.at()), (Paper::A2, Orientation::Portrait, page.at()));
    assert_eq!(s.project().unwrap().symbols(), before.symbols());
    for params in [
        json!({"sheet":target.id(),"paper":"a5","orientation":"portrait"}),
        json!({"sheet":target.id(),"paper":"a4","orientation":"sideways"}),
        json!({"sheet":target.id(),"paper":"a0","orientation":"landscape","at":{"x_nm":500_000_000,"y_nm":0}}),
        json!({"sheet":uuid::Uuid::nil(),"paper":"a4","orientation":"landscape"}),
    ] {
        let mut params = params;
        params["expected_revision"] = json!(s.revision());
        let snapshot = s.snapshot().unwrap();
        assert!(s.apply(SET_PAGE, &params).is_err());
        assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    }
    apply(&mut s, SET_PAGE, json!({"sheet":target.id(),"paper":"a3","orientation":"landscape","at":{"x_nm":0,"y_nm":0}}));
    assert_eq!(s.project().unwrap().page().unwrap().at(), Point { x_nm: 0, y_nm: 0 });
    s.apply("edit.undo", &Value::Null).unwrap();
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    let inspected = s.inspect();
    assert_eq!(inspected["project"]["sheets"][0]["page"]["paper"], "a4");
}

/// Paper is stored as sheet/9, round-trips byte for byte, and changing one sheet's paper
/// changes only that sheet's document. Older formats can't carry paper.
#[test]
fn paper_is_stored_per_sheet_and_round_trips() {
    let mut s = project_with_resistor();
    apply(&mut s, CREATE_SHEET, json!({"name":"Power"}));
    let power = s.project().unwrap().sheets().nth(1).unwrap().target;
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    assert_eq!(stored_page(&documents, power.id())["paper"], "a4");
    let restored = decode_documents(&documents).unwrap();
    assert_eq!(encode_documents(restored.clone()).unwrap(), documents);
    assert!(Arc::ptr_eq(&prepare_opened(restored.clone()).unwrap(), &restored));
    apply(&mut s, SET_PAGE, json!({"sheet":power.id(),"paper":"a3","orientation":"portrait"}));
    let changed = encode_documents(s.snapshot().unwrap()).unwrap();
    let paths: Vec<_> = documents.keys().filter(|path| documents.get(*path) != changed.get(*path)).collect();
    assert_eq!(paths, vec![&format!("sheets/{}.json", power.id())]);
    assert_eq!(stored_page(&changed, power.id()), json!({"paper":"a3","orientation":"portrait","at_nm":[-148_590_000,-105_410_000]}));

    let path = format!("sheets/{}.json", power.id());
    let edits: [fn(&mut Value); 6] = [
        |doc: &mut Value| doc["format"] = json!("pcbcraft-sheet/8"),
        |doc: &mut Value| drop(doc.as_object_mut().unwrap().remove("page")),
        |doc: &mut Value| doc["page"]["paper"] = json!("letter"),
        |doc: &mut Value| doc["page"]["at_nm"] = json!([900_000_000, 0]),
        |doc: &mut Value| doc["page"]["extra"] = json!(1),
        |doc: &mut Value| doc["page"] = Value::Null,
    ];
    for edit in edits {
        let mut forged = changed.clone();
        let mut doc: Value = serde_json::from_slice(&forged[&path]).unwrap();
        edit(&mut doc);
        forged.insert(path.clone(), serde_json::to_vec(&doc).unwrap());
        assert!(decode_documents(&forged).is_err());
    }
}

/// An older sheet opens with the smallest paper around its drawing, without moving anything or
/// recording history; the raw decode keeps the old bytes until the next save writes sheet/9.
#[test]
fn opening_an_older_sheet_adopts_paper_around_its_drawing() {
    let mut s = project_with_resistor();
    apply(&mut s, crate::PLACE_RESISTOR, json!({"x_nm":300_000_000,"y_nm":100_000_000}));
    apply(&mut s, CREATE_SHEET, json!({"name":"Empty"}));
    let mut documents = encode_documents(s.snapshot().unwrap()).unwrap();
    for (path, bytes) in documents.iter_mut().filter(|(path, _)| path.starts_with("sheets/")) {
        let mut doc: Value = serde_json::from_slice(bytes).unwrap();
        doc["format"] = json!("pcbcraft-sheet/3");
        doc.as_object_mut().unwrap().remove("page");
        *bytes = serde_json::to_vec_pretty(&doc).unwrap();
        assert!(path.ends_with(".json"));
    }
    let legacy = decode_documents(&documents).unwrap();
    assert!(legacy.sheets.iter().all(|sheet| sheet.page.is_none()));
    let unchanged = encode_documents(legacy.clone()).unwrap();
    // The raw codec never invents paper; only the open step adopts it.
    assert!(unchanged.iter().filter(|(path, _)| path.starts_with("sheets/")).all(|(_, bytes)| !bytes.windows(6).any(|w| w == b"\"page\"")));

    let opened = prepare_opened(legacy.clone()).unwrap();
    assert!(!Arc::ptr_eq(&opened, &legacy));
    assert_eq!(opened.symbols(), legacy.symbols());
    let main = opened.page().unwrap();
    let [min, max] = main.corners();
    let [drawn_min, drawn_max] = opened.drawing_bounds().unwrap();
    assert!(min.x_nm < drawn_min.x_nm && min.y_nm < drawn_min.y_nm && max.x_nm > drawn_max.x_nm && max.y_nm > drawn_max.y_nm);
    // 300 mm between the parts doesn't fit A4 with its margins, but fits A3.
    assert_eq!((main.paper(), main.orientation()), (Paper::A3, Orientation::Landscape));
    let empty = opened.sheets().nth(1).unwrap().target;
    assert_eq!(opened.for_sheet(empty).unwrap().page(), Some(Page::new_sheet()));
    let saved = encode_documents(opened).unwrap();
    assert!(
        saved.iter().filter(|(path, _)| path.starts_with("sheets/")).all(|(_, bytes)| bytes.starts_with(b"{\n  \"format\": \"pcbcraft-sheet/9\""))
    );
}

#[test]
fn adopted_paper_falls_back_to_a0_and_stays_in_range() {
    let huge = [Point { x_nm: -990_000_000, y_nm: -990_000_000 }, Point { x_nm: 990_000_000, y_nm: 990_000_000 }];
    let page = Page::around(Some(huge));
    assert_eq!((page.paper(), page.orientation()), (Paper::A0, Orientation::Landscape));
    let corner = [Point { x_nm: 990_000_000, y_nm: 990_000_000 }, Point { x_nm: 990_000_000, y_nm: 990_000_000 }];
    let page = Page::around(Some(corner));
    assert!(Page::new(page.paper(), page.orientation(), page.at()).is_ok());
    let tall = [Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 100_000_000, y_nm: 250_000_000 }];
    assert_eq!(Page::around(Some(tall)).orientation(), Orientation::Portrait);
}
