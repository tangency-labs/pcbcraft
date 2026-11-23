use super::*;
use egui_kittest::kittest::Queryable;
use pcbcraft_engine::{
    Session,
    authoring::{Action, Item, Kind, Primitive, publishing},
    library_graphics::{Geometry, Graphic},
    persistence::{ENTRY_POINT, decode_documents, encode_documents},
    schematic::{AlternatePinFunction, ElectricalType, GRID_NM, MAX_CENTER_NM, Shape},
    selection::Selection,
};
use serde_json::Value;
use std::sync::Arc;

const SIDE: usize = 64;

/// Build one pinned symbol through normal commands, then expand a synthetic in-memory native
/// fixture in one pass. The production decoder validates every placement and stable ID.
fn large_project() -> Arc<pcbcraft_engine::Project> {
    let item = Item::new(Kind::Symbol).unwrap();
    let mut primitive = item.objects[&1].clone();
    let Primitive::Pin { name, alternates, .. } = &mut primitive else { panic!("expected starter pin") };
    *name = Some("GPIO".into());
    *alternates = vec![AlternatePinFunction { name: "ADC_IN".into(), electrical_type: ElectricalType::Input }];
    let mut editor = pcbcraft_engine::authoring::Session::new(item, false).unwrap();
    editor.execute(0, Action::Replace { object: 1, primitive }).unwrap();
    let manifest = publishing::new_library("large-fixture", "Synthetic fixture", "MIT OR Apache-2.0").unwrap();
    let published = publishing::prepare_copy(editor.snapshot(), &manifest).unwrap();
    let item = published.item();
    let mut library = published.documents().clone();
    library.insert("library.json".into(), manifest);
    let mut session = Session::default();
    session.apply("project.new", &json!({"create_board":false})).unwrap();
    session.execute("library.load_source", &json!({"source":"fixture","documents":library})).unwrap();
    // The loaded catalog adds portable provenance, which is part of the placement digest.
    let symbol = session.libraries().symbol(&item.library, &item.id, item.revision).unwrap();
    let digest = pcbcraft_engine::library::sources::symbol_digest(&symbol).unwrap();
    session.apply(pcbcraft_engine::PLACE_SYMBOL, &json!({"library":item.library,"definition":item.id,"definition_revision":item.revision,"definition_digest":digest,"x_nm":0,"y_nm":0,"expected_revision":session.revision()})).unwrap();
    let mut docs = encode_documents(session.snapshot().unwrap()).unwrap();
    let mut entry: Value = serde_json::from_slice(&docs[ENTRY_POINT]).unwrap();
    let path = entry["sheets"][0]["file"].as_str().unwrap().to_owned();
    let mut sheet: Value = serde_json::from_slice(&docs[&path]).unwrap();
    let component = entry["components"][0].clone();
    let symbol = sheet["symbols"][0].clone();
    let reference = component["reference"].as_str().unwrap();
    let prefix = reference.trim_end_matches(|c: char| c.is_ascii_digit());
    for index in 1..SIDE * SIDE {
        let component_id = 2 * index + 1;
        let mut part = component.clone();
        part["id"] = json!(component_id);
        part["reference_number"] = json!(index + 1);
        part["reference"] = json!(format!("{prefix}{}", index + 1));
        entry["components"].as_array_mut().unwrap().push(part);
        let mut placement = symbol.clone();
        placement["id"] = json!(component_id + 1);
        placement["component"] = json!(component_id);
        let x = (index % SIDE) as i64 * 10 * GRID_NM;
        let y = (index / SIDE) as i64 * 10 * GRID_NM;
        assert!(x.max(y) <= MAX_CENTER_NM);
        placement["x_nm"] = json!(x);
        placement["y_nm"] = json!(y);
        sheet["symbols"].as_array_mut().unwrap().push(placement);
    }
    entry["numeric_id_high_water"] = json!(2 * SIDE * SIDE);
    docs.insert(ENTRY_POINT.into(), serde_json::to_vec(&entry).unwrap());
    docs.insert(path, serde_json::to_vec(&sheet).unwrap());
    decode_documents(&docs).unwrap()
}

/// The live properties frame resolves the last placement by address. The same canvas projection
/// used to paint pin labels produces only one graphic list in a tightly clipped viewport.
#[test]
fn pin_functions_modal_and_canvas_project_one_visible_owner_in_large_sheet() {
    let project = large_project();
    assert_eq!(project.symbols().len(), SIDE * SIDE);
    let last = project.symbols().last().unwrap();
    let id = last.id();
    let part = last.component_id();
    let mut h = crate::tests::harness();
    {
        let app = h.state_mut();
        app.session.open_snapshot(project);
        app.session
            .apply(
                pcbcraft_engine::pin_functions::SET,
                &json!({"target":{"part":part,"terminal":"1"},"function":"ADC_IN","expected_revision":app.session.revision()}),
            )
            .unwrap();
        let sheet = app.session.project().unwrap().sheet_target();
        app.views.active.open_document(crate::editor_view::DocumentTarget::Sheet(sheet), &app.session);
        app.home = false;
        app.browser.open = false;
        app.properties_open = false;
        app.views.active.schematic.selection = Selection::one_symbol(id.clone());
        let mut edit = app.read_property_edit().unwrap();
        edit.tab = Tab::Terminals;
        app.dialog = crate::Dialog::EditFields(Box::new(edit));
    }
    h.run();
    PIN_MODAL_LOOKUPS.set(0);
    h.step();
    assert!((1..=4).contains(&PIN_MODAL_LOOKUPS.get()), "one indexed owner query per modal render, including any egui layout passes");
    h.get_by_label("input");
    assert_eq!(h.get_by_role_and_label(egui::Role::ComboBox, "Pin function for terminal 1").value().as_deref(), Some("ADC_IN"));

    let project = h.state().session.project().unwrap();
    let target = project.symbol(&id).unwrap().position;
    let visible = egui::Rect::from_center_size(egui::Pos2::ZERO, egui::vec2(2., 2.));
    let projected: Vec<_> = project
        .symbols()
        .iter()
        .filter_map(|symbol| {
            let center = egui::pos2(
                (symbol.position.x_nm - target.x_nm) as f32 / GRID_NM as f32,
                (symbol.position.y_nm - target.y_nm) as f32 / GRID_NM as f32,
            );
            crate::schematic_canvas::visible_symbol_graphics(project, symbol, center, visible)
        })
        .collect();
    assert_eq!(projected.len(), 1);
    let names = |shapes: &[Shape], wanted: &str| {
        shapes.iter().filter(|shape| matches!(shape, Shape::Graphic(Graphic { geometry: Geometry::Text { text, .. }, .. }) if text == wanted)).count()
    };
    assert_eq!(names(&projected[0], "ADC_IN"), 1);
    assert_eq!(names(&projected[0], "GPIO"), 0);
}
