//! Copper appearance follows engine membership and shared local settings, not the active tool.
use super::*;
use pcbcraft_engine::{INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT, PLACE_SYMBOL, persistence::encode_documents};

/// Original surface and plated fixtures exercise four distinct physical copper layers.
fn board<'a>() -> Harness<'a, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    h.state_mut().run("project.new", &json!({}));
    let revision = h.state().session.revision();
    h.state_mut().run(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":4,"expected_revision":revision}));
    for (index, definition) in ["resistor-0402", "header-1x02-254"].into_iter().enumerate() {
        let revision = h.state().session.revision();
        h.state_mut().run(PLACE_SYMBOL, &json!({"definition":"resistor","x_nm":index as i64*10000000,"y_nm":0,"expected_revision":revision}));
        let symbol = h.state().session.project().unwrap().symbols().last().unwrap().id();
        let revision = h.state().session.revision();
        h.state_mut().run(PLACE_FOOTPRINT,&json!({"symbol":symbol,"definition":definition,"mapping":{"1":"1","2":"2"},"x_nm":index as i64*10000000,"y_nm":0,"expected_revision":revision}));
    }
    h.run();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB layout");
    h.query_all_by_label("Fit board").next().unwrap().click();
    h.run();
    h
}

fn layer(h: &Harness<'_, PcbApp>, name: &str) -> String {
    h.state().session.project().unwrap().boards().next().unwrap().layers().into_iter().find(|layer| layer.name == name).unwrap().id.to_string()
}
fn lands(h: &Harness<'_, PcbApp>, color: egui::Color32) -> usize {
    let canvas = h.get_by_label("PCB canvas").rect();
    h.output()
        .shapes
        .iter()
        .filter(|shape| shape.clip_rect == canvas && matches!(&shape.shape,egui::Shape::Path(path) if path.fill==color && path.points.len()>=4))
        .count()
}
fn hex_field(h: &mut Harness<'_, PcbApp>, value: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Hex").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Hex").type_text(value);
    h.run();
}
fn swatch(h: &mut Harness<'_, PcbApp>, name: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, &format!("Change color for layer {name}")).click();
    h.run();
}

/// Surface lands remain front-bound; plated lands follow actual visible/active copper colors.
#[test]
fn copper_uses_membership_visibility_and_per_layer_colors() {
    let mut h = board();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    assert_eq!(lands(&h, Tokens::get().board_front), 4);
    click(&mut h, "Activate layer Bottom");
    assert_eq!(lands(&h, Tokens::get().board_front), 2);
    assert_eq!(lands(&h, Tokens::get().board_back), 2);
    click(&mut h, "Show layer Bottom");
    assert_eq!(lands(&h, Tokens::get().board_back), 0);
    assert_eq!(lands(&h, Tokens::get().board_front), 4);
    click(&mut h, "Show layer Bottom");
    click(&mut h, "Isolate active");
    assert_eq!(lands(&h, Tokens::get().board_front), 0);
    assert_eq!(lands(&h, Tokens::get().board_back), 2);
    click(&mut h, "Show all");
    let inner: Vec<_> = h
        .state()
        .session
        .project()
        .unwrap()
        .boards()
        .next()
        .unwrap()
        .setup()
        .unwrap()
        .stackup()
        .strata
        .iter()
        .filter_map(|stratum| {
            let pcbcraft_engine::board::setup::Stratum::Copper { layer, .. } = stratum else { return None };
            h.state()
                .session
                .project()
                .unwrap()
                .boards()
                .next()
                .unwrap()
                .layers()
                .into_iter()
                .find(|record| record.id == *layer && record.side.is_none())
                .map(|record| record.id.to_string())
        })
        .collect();
    for (id, color) in inner.iter().zip([Tokens::get().board_inner, Tokens::get().board_inner_alt]) {
        h.state_mut().views.active.board.set_layer(id, true, None);
        h.run();
        click(&mut h, "Isolate active");
        assert_eq!(lands(&h, color), 2);
        click(&mut h, "Show all");
    }
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}

/// Color drafts are reversible and do not activate a layer or enter document undo/native files.
#[test]
fn swatches_apply_cancel_reset_and_persist_locally() {
    let mut h = board();
    let before = h.state().session.inspect();
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let active = h.state().views.active.board.active_layer.clone();
    swatch(&mut h, "Bottom");
    hex_field(&mut h, "#29B6C8");
    click(&mut h, "Cancel");
    assert_eq!(h.state().views.active.board.active_layer, active);
    assert_eq!(lands(&h, Tokens::get().board_front), 4);
    swatch(&mut h, "Top");
    hex_field(&mut h, "#29B6C8");
    click(&mut h, "Apply color");
    let chosen = egui::Color32::from_rgb(0x29, 0xB6, 0xC8);
    assert_eq!(lands(&h, chosen), 4);
    let saved = h.state().board_appearance.storage_text();
    h.state_mut().board_appearance = crate::board_appearance::Appearance::from_storage(saved);
    h.run();
    assert_eq!(lands(&h, chosen), 4);
    swatch(&mut h, "Top");
    hex_field(&mut h, "#💥xx");
    click(&mut h, "Apply color");
    assert!(matches!(h.state().dialog, Dialog::LayerColor(_)));
    click(&mut h, "Cancel");
    swatch(&mut h, "Top");
    click(&mut h, "Reset to default");
    assert_eq!(lands(&h, Tokens::get().board_front), 4);
    assert_eq!(h.state().session.inspect(), before);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
}

/// The live command shares the palette handler and refuses unknown/foreign layers before mutation.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_color_changes_share_palette_and_refuse_bad_addresses() {
    let mut h = board();
    let ctx = h.ctx.clone();
    let view = h.state().views.active.id().0;
    let top = layer(&h, "Top");
    let before = h.state().session.inspect();
    h.state_mut().run_window_command(&ctx, "view.set_layer_color", &json!({"view":view,"layer":top,"color":"#80D484"})).unwrap();
    h.run();
    assert_eq!(lands(&h, egui::Color32::from_rgb(0x80, 0xD4, 0x84)), 4);
    let saved = h.state().board_appearance.storage_text();
    for params in [
        json!({"view":999,"layer":top,"color":"#FFFFFF"}),
        json!({"view":view,"layer":layer(&h,"Board Edge"),"color":"#FFFFFF"}),
        json!({"view":view,"layer":"00000000-0000-0000-0000-000000000001","color":"#FFFFFF"}),
        json!({"view":view,"layer":top,"color":"#éxxxx"}),
    ] {
        assert!(h.state_mut().run_window_command(&ctx, "view.set_layer_color", &params).is_err());
        assert_eq!(h.state().board_appearance.storage_text(), saved);
    }
    h.state_mut().run_window_command(&ctx, "view.set_layer_color", &json!({"view":view,"layer":top,"color":"#F4F4F4"})).unwrap();
    h.run();
    let canvas = h.get_by_label("PCB canvas").rect();
    assert!(h.output().shapes.iter().any(|shape|shape.clip_rect==canvas && matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text.lines().next()==Some("1") && text.fallback_color==Tokens::get().board_canvas)),"bright copper must retain legible pad labels");
    h.state_mut().run_window_command(&ctx, "view.set_layer_color", &json!({"view":view,"layer":top,"color":null})).unwrap();
    h.run();
    assert_eq!(lands(&h, Tokens::get().board_front), 4);
    assert_eq!(h.state().session.inspect(), before);
}

/// Color edits address board/layer UUIDs across view copies and survive layer renaming.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn shared_colors_survive_renaming_and_do_not_leak_to_other_boards() {
    let mut h = board();
    let ctx = h.ctx.clone();
    let original = h.state().views.active.id();
    let top = layer(&h, "Top");
    h.state_mut().set_layer_color(original, &top, Some([35, 180, 130])).unwrap();
    let before = h.state().board_appearance.storage_text();
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB layout beside current view");
    let inspected = crate::window_commands::inspect_editor(h.state());
    let palettes: Vec<_> = inspected["views"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|view| view["workspace"] == "board")
        .map(|view| view["board"]["copper_colors"].clone())
        .collect();
    assert_eq!(palettes.len(), 2);
    assert_eq!(palettes[0], palettes[1]);
    let mut setup = h.state().session.inspect()["project"]["board"]["setup"].clone();
    for layer in setup["layers"].as_array_mut().unwrap() {
        if layer["id"] == top {
            layer["name"] = json!("Component copper");
        }
    }
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::SET_BOARD_SETUP, &json!({"setup":setup,"expected_revision":revision}));
    h.run();
    assert_eq!(h.state().board_appearance.storage_text(), before);
    assert!(crate::window_commands::inspect_editor(h.state())["views"].as_array().unwrap().iter().any(|view| {
        view["board"]["copper_colors"]
            .as_array()
            .is_some_and(|colors| colors.iter().any(|color| color["layer"] == top && color["color"] == "#23B482"))
    }));
    h.state_mut().session.apply("project.new", &json!({})).unwrap();
    let revision = h.state().session.revision();
    h.state_mut().run(INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"expected_revision":revision}));
    h.run();
    assert!(h.state_mut().run_window_command(&ctx, "view.set_layer_color", &json!({"view":original.0,"layer":top,"color":"#FFFFFF"})).is_err());
    assert_eq!(h.state().board_appearance.storage_text(), before);
}

/// A project edit or reopening the same snapshot expires a pending swatch draft before Apply.
#[test]
fn stale_color_drafts_cannot_rewrite_local_settings() {
    let mut h = board();
    let saved = h.state().board_appearance.storage_text();
    swatch(&mut h, "Top");
    hex_field(&mut h, "#8899AA");
    h.state_mut().session.apply("project.rename", &json!({"name":"Changed"})).unwrap();
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(h.state().board_appearance.storage_text(), saved);
    swatch(&mut h, "Top");
    hex_field(&mut h, "#8899AA");
    let project = h.state().session.snapshot().unwrap();
    h.state_mut().session.open_snapshot(project);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
    assert_eq!(h.state().board_appearance.storage_text(), saved);
}

/// The application's actual settings save writes the personal palette while retaining native bytes.
#[test]
fn app_settings_save_retains_palette_without_writing_engineering_data() {
    #[derive(Default)]
    struct Storage(std::collections::BTreeMap<String, String>);
    impl eframe::Storage for Storage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }
    let mut h = board();
    let view = h.state().views.active.id();
    let top = layer(&h, "Top");
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    h.state_mut().set_layer_color(view, &top, Some([12, 34, 56])).unwrap();
    let mut storage = Storage::default();
    eframe::App::save(h.state_mut(), &mut storage);
    assert!(!h.state().board_appearance.dirty);
    let saved = storage.0.get(crate::board_appearance::STORAGE_KEY).cloned();
    assert!(saved.is_some());
    h.state_mut().board_appearance = crate::board_appearance::Appearance::from_storage(saved);
    h.run();
    assert_eq!(lands(&h, egui::Color32::from_rgb(12, 34, 56)), 4);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
}
