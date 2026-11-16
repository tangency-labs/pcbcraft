use super::*;
use crate::persistence::{decode_documents, encode_documents};
use crate::sheets::page::{LIST_TEMPLATES, Orientation, Paper, SET_PAGE, SET_TEMPLATE};
use crate::{Session, sheets::CREATE_SHEET};
use std::sync::Arc;

/// Issue a command at the current revision.
fn apply(session: &mut Session, command: &str, mut params: Value) {
    params["expected_revision"] = json!(session.revision());
    session.apply(command, &params).unwrap();
}

/// A new project whose only sheet has a title and a project revision.
fn titled_project() -> Session {
    let mut s = Session::default();
    s.apply("project.new", &json!({})).unwrap();
    let sheet = s.project().unwrap().sheet_target().id();
    apply(&mut s, crate::parameters::SET, json!({"scope":"sheet","sheet":sheet,"name":"Title","value":"Power supply"}));
    apply(&mut s, crate::parameters::SET, json!({"scope":"project","name":"Revision","value":"B"}));
    s
}

/// The bundled Default template parses, and its title block shows every standard sheet value.
#[test]
fn the_default_template_is_bundled_and_shows_the_sheet_values() {
    let default = default_template().unwrap();
    assert_eq!(default.reference(), TemplateRef { library: "basic-frames".into(), name: "default".into(), revision: 1 });
    assert_eq!(default.title(), "Default");
    assert!(bundled().iter().any(|t| std::ptr::eq(t, default)));
    let texts: Vec<_> = default.items.iter().filter_map(|item| if let Item::Text { text, .. } = item { Some(text.as_str()) } else { None }).collect();
    for reference in
        ["@sheet.Title", "@sheet.Name", "@project.Name", "@sheet.Company", "@sheet.DrawnBy", "@sheet.Revision", "@sheet.Date", "@instance.Page"]
    {
        assert!(texts.iter().any(|text| text.contains(reference)), "{reference}");
    }
}

/// On every paper size the frame stays on the paper, has zones about 50 mm long numbered and
/// lettered on both sides, and keeps the title block in the bottom-right corner.
#[test]
fn the_frame_fits_every_paper_with_zones_and_a_corner_title_block() {
    let none = |text: &str| Resolved { text: text.to_owned(), ..Resolved::default() };
    for paper in Paper::ALL {
        for orientation in Orientation::ALL {
            let page = Page::new(paper, orientation, Point { x_nm: -590_000_000, y_nm: -590_000_000 }).unwrap();
            let frame = default_template().unwrap().frame(page, &none);
            let [min, max] = page.corners();
            let inside = |p: &Point| (min.x_nm..=max.x_nm).contains(&p.x_nm) && (min.y_nm..=max.y_nm).contains(&p.y_nm);
            assert!(frame.lines.iter().flatten().all(inside));
            assert!(frame.texts.iter().map(|text| &text.at).all(inside));
            let markers: Vec<_> = frame.texts.iter().filter(|t| t.style == TextStyle::Label && t.align == Align::Center).collect();
            let size = page.size();
            let (columns, rows) = (zone_count(size.x_nm - 10_000_000, 50_000_000), zone_count(size.y_nm - 10_000_000, 50_000_000));
            assert_eq!(markers.len() as i64, 2 * (columns + rows));
            assert!(markers.iter().any(|t| t.text == columns.to_string()) && markers.iter().any(|t| t.text == "A"));
            let title = frame.texts.iter().find(|t| t.text == "@sheet.Title").unwrap();
            assert_eq!(title.at, Point { x_nm: max.x_nm - 128_000_000, y_nm: max.y_nm - 34_500_000 });
        }
    }
    let a4 = Page::new(Paper::A4, Orientation::Landscape, Point { x_nm: 0, y_nm: 0 }).unwrap();
    let frame = default_template().unwrap().frame(a4, &none);
    let markers: Vec<_> = frame.texts.iter().filter(|t| t.align == Align::Center).map(|t| t.text.as_str()).collect();
    assert_eq!(markers, ["1", "1", "2", "2", "3", "3", "4", "4", "5", "5", "6", "6", "A", "A", "B", "B", "C", "C", "D", "D"]);
}

#[test]
fn zone_rows_skip_i_and_o_and_continue_with_two_letters() {
    let letters: String = (0..24).map(row_letter).collect();
    assert_eq!(letters, "ABCDEFGHJKLMNPQRSTUVWXYZ");
    assert_eq!(row_letter(24), "AA");
    assert_eq!(row_letter(25), "AB");
    assert_eq!(zone_count(20_000_000, 50_000_000), 1);
}

/// Documents with a wrong schema, unknown fields or out-of-range items are refused.
#[test]
fn invalid_template_documents_are_refused() {
    let original: Value = serde_json::from_str(BUNDLED[0]).unwrap();
    let edits: [fn(&mut Value); 8] = [
        |doc| doc["schema"] = json!("pcbcraft-sheet-template/2"),
        |doc| doc["extra"] = json!(1),
        |doc| doc["revision"] = json!(0),
        |doc| doc["name"] = json!("Default"),
        |doc| doc["border"]["zones"]["pitch_nm"] = json!(1_000_000),
        |doc| doc["items"][0]["size_nm"] = json!([0, 1]),
        |doc| doc["items"][5]["text"] = json!(""),
        |doc| doc["items"][1]["colour"] = json!("red"),
    ];
    assert!(parse(&original.to_string()).is_ok());
    for edit in edits {
        let mut doc = original.clone();
        edit(&mut doc);
        assert!(parse(&doc.to_string()).is_err(), "{doc}");
    }
}

/// The title block shows the sheet's resolved values, and marks unknown references.
#[test]
fn the_title_block_resolves_sheet_parameters() {
    let mut s = titled_project();
    let sheet = s.project().unwrap().sheet_target().id();
    let frame = s.project().unwrap().frame().unwrap();
    let texts: Vec<_> = frame.texts.iter().map(|t| t.text.as_str()).collect();
    assert!(texts.contains(&"Power supply") && texts.contains(&"B") && texts.contains(&"1 of 1"));
    apply(&mut s, crate::parameters::SET, json!({"scope":"sheet","sheet":sheet,"name":"Company","value":"@project.Nope Ltd"}));
    let frame = s.project().unwrap().frame().unwrap();
    let company = frame.texts.iter().find(|t| t.text.ends_with(" Ltd")).unwrap();
    assert_eq!(company.unknown, std::iter::once(0..13).collect::<Vec<_>>());
}

/// New sheets get the Default frame; choosing none or Default again is one undo step each, a
/// repeated choice changes nothing, the paper keeps its template, and unknown templates are
/// refused without changing anything.
#[test]
fn set_template_is_one_undo_step() {
    let mut s = titled_project();
    let target = s.project().unwrap().sheet_target();
    let page = s.project().unwrap().page().unwrap();
    assert!(std::ptr::eq(page.template().unwrap(), default_template().unwrap()));
    let before = s.snapshot().unwrap();
    apply(&mut s, SET_TEMPLATE, json!({"sheet":target.id(),"template":null}));
    let plain = s.project().unwrap().page().unwrap();
    assert_eq!((plain.template(), plain.at(), plain.paper()), (None, page.at(), page.paper()));
    let frame = s.project().unwrap().frame().unwrap();
    assert_eq!((frame.lines.len(), frame.texts.len()), (4, 0));
    let revision = s.revision();
    apply(&mut s, SET_TEMPLATE, json!({"sheet":target.id(),"template":null}));
    assert_eq!(s.revision(), revision);
    apply(&mut s, SET_PAGE, json!({"sheet":target.id(),"paper":"a3","orientation":"landscape"}));
    assert_eq!(s.project().unwrap().page().unwrap().template(), None);
    for template in [json!({"library":"basic-frames","name":"default","revision":2}), json!({"library":"basic-frames"}), json!("default")] {
        let snapshot = s.snapshot().unwrap();
        let params = json!({"sheet":target.id(),"template":template,"expected_revision":s.revision()});
        assert!(s.apply(SET_TEMPLATE, &params).is_err());
        assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()));
    }
    s.apply("edit.undo", &Value::Null).unwrap();
    s.apply("edit.undo", &Value::Null).unwrap();
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    let listed = s.execute(LIST_TEMPLATES, &json!({})).unwrap();
    assert_eq!(listed["templates"][0]["name"], "default");
    assert_eq!(listed["templates"][0]["revision"], 1);
    assert!(Session::default().execute(LIST_TEMPLATES, &json!({})).is_ok());
    // `apply` accepts the same read-only query, with or without a project, and changes nothing.
    let revision = s.revision();
    assert_eq!(s.apply(LIST_TEMPLATES, &json!({})), Ok(revision));
    assert!(Arc::ptr_eq(&before, &s.snapshot().unwrap()));
    assert!(Session::default().apply(LIST_TEMPLATES, &json!({})).is_ok());
}

/// The template reference is stored in the page record only when there is one; a sheet with an
/// unknown template revision is refused rather than opened without its frame.
#[test]
fn the_template_is_stored_in_the_page_record() {
    let mut s = titled_project();
    apply(&mut s, CREATE_SHEET, json!({"name":"Plain"}));
    let plain = s.project().unwrap().sheets().nth(1).unwrap().target;
    apply(&mut s, SET_TEMPLATE, json!({"sheet":plain.id(),"template":null}));
    let documents = encode_documents(s.snapshot().unwrap()).unwrap();
    let page = |id: uuid::Uuid| serde_json::from_slice::<Value>(&documents[&format!("sheets/{id}.json")]).unwrap()["page"].clone();
    let main = s.project().unwrap().sheets().next().unwrap().target;
    assert_eq!(page(main.id())["template"], json!({"library":"basic-frames","name":"default","revision":1}));
    assert!(page(plain.id()).get("template").is_none());
    let restored = decode_documents(&documents).unwrap();
    assert_eq!(encode_documents(restored.clone()).unwrap(), documents);
    assert!(std::ptr::eq(restored.page().unwrap().template().unwrap(), default_template().unwrap()));

    let path = format!("sheets/{}.json", main.id());
    for template in [json!({"library":"basic-frames","name":"default","revision":9}), json!(null), json!({"library":"basic-frames","name":"default"})]
    {
        let mut forged = documents.clone();
        let mut doc: Value = serde_json::from_slice(&forged[&path]).unwrap();
        doc["page"]["template"] = template;
        forged.insert(path.clone(), serde_json::to_vec(&doc).unwrap());
        assert!(decode_documents(&forged).is_err());
    }
}

/// Paper adopted for an older drawing keeps the drawing clear of the title block, choosing a
/// larger size when the smallest one would put the block over it.
#[test]
fn adopted_paper_keeps_the_drawing_clear_of_the_title_block() {
    // Fits A4 landscape by size, but centred A4 would put the title block over its corner.
    let drawing = [Point { x_nm: 0, y_nm: 0 }, Point { x_nm: 250_000_000, y_nm: 165_000_000 }];
    let page = Page::around(Some(drawing));
    assert!(std::ptr::eq(page.template().unwrap(), default_template().unwrap()));
    assert_ne!((page.paper(), page.orientation()), (Paper::A4, Orientation::Landscape));
    let [a, b] = default_template().unwrap().items_bounds(page).unwrap();
    let [min, max] = drawing;
    assert!(b.x_nm < min.x_nm || a.x_nm > max.x_nm || b.y_nm < min.y_nm || a.y_nm > max.y_nm);
}
