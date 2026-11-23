//! Copper appearance follows engine membership and shared local settings, not the active tool.
use super::*;
use pcbcraft_engine::{INITIALIZE_BOARD_SETUP, PLACE_FOOTPRINT, PLACE_SYMBOL, persistence::encode_documents};

/// Original surface and plated fixtures exercise four distinct physical copper layers.
fn board<'a>() -> Harness<'a, PcbApp> {
    let mut h = harness();
    h.set_size(egui::vec2(1400., 900.));
    h.state_mut().run("project.new", &json!({"create_board":true}));
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
    click(&mut h, "PCB Layout");
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

/// Hiding Top leaves plated pads on a visible bright layer with readable drill labels.
#[test]
fn plated_pad_labels_keep_contrast_when_top_is_hidden() {
    let mut h = board();
    swatch(&mut h, "Bottom");
    hex_field(&mut h, "#80D484");
    click(&mut h, "Apply color");
    click(&mut h, "Activate layer Bottom");
    click(&mut h, "Show layer Top");
    assert_eq!(lands(&h, egui::Color32::from_rgb(128, 212, 132)), 2);

    let canvas = h.get_by_label("PCB canvas").rect();
    for number in ["1", "2"] {
        assert!(
            h.output().shapes.iter().any(|clipped| {
                let egui::Shape::Vec(shapes) = &clipped.shape else { return false };
                clipped.clip_rect == canvas
                    && shapes.iter().any(|shape| {
                        matches!(shape, egui::Shape::Text(text)
                    if text.galley.job.text == number && text.override_text_color.is_none() && text.fallback_color == egui::Color32::WHITE)
                    })
                    && shapes.iter().any(|shape| {
                        matches!(shape, egui::Shape::Text(text)
                    if text.galley.job.text == number && text.override_text_color == Some(Tokens::get().board_canvas))
                    })
            }),
            "plated pad {number} should have bright text with a contrasting outline"
        );
    }
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
    assert!(matches!(h.state().dialog, Dialog::BoardColor(_)));
    click(&mut h, "Cancel");
    swatch(&mut h, "Top");
    click(&mut h, "Reset to default");
    assert_eq!(lands(&h, Tokens::get().board_front), 4);
    assert_eq!(h.state().session.inspect(), before);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
}

/// Same-net zones, pads and a contact-connected track share one local color across copper layers.
#[test]
fn zone_net_color_follows_copper_and_restores_layer_colors() {
    use pcbcraft_engine::{board::zones::ADD_ZONE, schematic::Point};

    let mut h = super::board_fill::board();
    let (zone, from) = {
        let project = h.state().session.project().unwrap();
        let board = project.board_view(project.board_target().unwrap()).unwrap();
        let from = board.footprints().next().unwrap().pads.into_iter().find(|pad| pad.number == "2").unwrap().center;
        (board.zones()[0].clone(), from)
    };
    let bottom = layer(&h, "Bottom");
    let revision = h.state().session.revision();
    h.state_mut().run(ADD_ZONE, &json!({"net":zone.net,"layer":bottom,"points":zone.points,"options":zone.options,"expected_revision":revision}));
    assert!(h.state().message.is_none(), "{:?}", h.state().message);
    let to = Point { y_nm: from.y_nm + 2_540_000, ..from };
    super::board_tracks::add_track(&mut h, from, to);
    let before = h.state().session.inspect();
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let selected = zone.id.to_string();
    h.state_mut().views.active.board.zones.select(Some(selected.clone()));
    h.run();
    click(&mut h, "Net color…");
    hex_field(&mut h, "#29B6C8");
    click(&mut h, "Apply color");
    h.state_mut().views.active.board.zones.select(None);
    h.run();

    let chosen = egui::Color32::from_rgb(0x29, 0xB6, 0xC8);
    let mesh_count = |h: &Harness<'_, PcbApp>, color: egui::Color32| {
        let canvas = h.get_by_label("PCB canvas").rect();
        let ink = color.gamma_multiply(0.32);
        h.output()
            .shapes
            .iter()
            .filter(|shape| {
                shape.clip_rect == canvas
                    && matches!(&shape.shape, egui::Shape::Mesh(mesh)
                        if !mesh.indices.is_empty() && mesh.vertices.iter().all(|vertex| vertex.color == ink))
            })
            .count()
    };
    let outlines = |h: &Harness<'_, PcbApp>, color| {
        let canvas = h.get_by_label("PCB canvas").rect();
        h.output()
            .shapes
            .iter()
            .filter(|shape| {
                shape.clip_rect == canvas
                    && matches!(&shape.shape, egui::Shape::Path(path)
                        if path.closed && path.stroke.color == egui::epaint::ColorMode::Solid(color))
            })
            .count()
    };
    let track_has_color = |h: &Harness<'_, PcbApp>, color| {
        let canvas = h.get_by_label("PCB canvas").rect();
        let camera = &h.state().views.active.camera;
        let endpoints = [camera.screen(from, canvas), camera.screen(to, canvas)];
        h.output().shapes.iter().any(|shape| {
            shape.clip_rect == canvas
                && matches!(&shape.shape, egui::Shape::LineSegment { points, stroke }
                    if points == &endpoints && stroke.color == color)
        })
    };
    assert_eq!(mesh_count(&h, chosen), 2);
    assert_eq!(outlines(&h, chosen), 2);
    assert_eq!(lands(&h, chosen), 2);
    assert_eq!(lands(&h, Tokens::get().board_front), 2);
    assert!(track_has_color(&h, chosen));

    let saved = h.state().board_appearance.storage_text();
    assert!(saved.is_some());
    h.state_mut().board_appearance = crate::board_appearance::Appearance::from_storage(saved.clone());
    h.run();
    assert_eq!(mesh_count(&h, chosen), 2);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let ctx = h.ctx.clone();
        let view = h.state().views.active.id().0;
        let inspect = crate::window_commands::inspect_editor(h.state());
        let board = inspect["views"].as_array().unwrap().iter().find(|view| view["workspace"] == "board").unwrap();
        assert_eq!(board["board"]["net_colors"], json!([{"net":zone.net,"color":"#29B6C8"}]));
        assert_eq!(board["board"]["net_colors_visible"], true);
        let unknown = json!({"view":view,"net":uuid::Uuid::nil(),"color":"#112233"});
        assert!(h.state_mut().run_window_command(&ctx, "view.set_net_color", &unknown).is_err());
        let known = json!({"view":view,"net":zone.net,"color":"#29B6C8"});
        h.state_mut().run_window_command(&ctx, "view.set_net_color", &known).unwrap();
        assert_eq!(h.state().board_appearance.storage_text(), saved);
    }
    click(&mut h, "Net colors");
    assert_eq!(mesh_count(&h, chosen), 0);
    assert_eq!(mesh_count(&h, Tokens::get().board_front), 1);
    assert_eq!(mesh_count(&h, Tokens::get().board_back), 1);
    assert!(track_has_color(&h, Tokens::get().board_front));
    assert_eq!(h.state().board_appearance.storage_text(), saved);
    click(&mut h, "Net colors");
    assert_eq!(mesh_count(&h, chosen), 2);
    h.state_mut().views.active.board.zones.select(Some(selected));
    h.run();
    click(&mut h, "Net color…");
    click(&mut h, "Use layer color");
    assert_eq!(mesh_count(&h, chosen), 0);

    let track = {
        let project = h.state().session.project().unwrap();
        project.board_view(project.board_target().unwrap()).unwrap().tracks()[0].id
    };
    let state = &mut h.state_mut().views.active.board;
    state.zones.select(None);
    state.tracks.selected.insert(track);
    h.run();
    click(&mut h, "Net color…");
    hex_field(&mut h, "#29B6C8");
    click(&mut h, "Apply color");
    assert_eq!(mesh_count(&h, chosen), 2);
    assert_eq!(h.state().session.inspect(), before);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
}

/// Color Undo/Redo retains file bytes, follows authored edits and restores an explicit reset.
#[test]
fn net_color_undo_and_redo_follow_the_editor_history() {
    let mut h = super::board_fill::board();
    let (target, zone, net, name) = {
        let project = h.state().session.project().unwrap();
        let board = project.board_view(project.board_target().unwrap()).unwrap();
        (board.target, board.zones()[0].id.to_string(), board.zones()[0].net, project.name.clone())
    };
    let bytes = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let revision = h.state().session.revision();
    let color = |h: &Harness<'_, PcbApp>| h.state().board_appearance.net_palette(target, true).and_then(|palette| palette.get(&net)).copied();
    let cyan = egui::Color32::from_rgb(0x29, 0xB6, 0xC8);
    h.state_mut().views.active.board.zones.select(Some(zone.clone()));
    h.run();
    click(&mut h, "Net color…");
    hex_field(&mut h, "#29B6C8");
    click(&mut h, "Apply color");
    assert_eq!(color(&h), Some(cyan));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(color(&h), None);
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
    click(&mut h, "Redo");
    assert_eq!(color(&h), Some(cyan));

    // A later authored edit is the first thing Undo reverts. The next step reaches the color.
    h.state_mut().run("project.rename", &json!({"name":"Changed"}));
    h.run();
    click(&mut h, "Undo");
    assert_eq!(h.state().session.project().unwrap().name, name);
    assert_eq!(color(&h), Some(cyan));
    let revision = h.state().session.revision();
    click(&mut h, "Undo");
    assert_eq!(color(&h), None);
    assert_eq!(h.state().session.revision(), revision);
    click(&mut h, "Redo");
    assert_eq!(color(&h), Some(cyan));
    h.state_mut().views.active.board.zones.select(Some(zone));
    h.run();
    click(&mut h, "Net color…");
    click(&mut h, "Use layer color");
    assert_eq!(color(&h), None);
    assert!(!h.state().session.available("edit.redo"));
    click(&mut h, "Undo");
    assert_eq!(color(&h), Some(cyan));
    assert_eq!(h.state().session.revision(), revision);
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), bytes);
}

/// Live palette edits and project Undo run through the same cursor as GUI actions.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_net_color_undo_returns_the_previous_override() {
    let mut h = super::board_fill::board();
    let ctx = h.ctx.clone();
    let view = h.state().views.active.id().0;
    let (target, net) = {
        let project = h.state().session.project().unwrap();
        let board = project.board_view(project.board_target().unwrap()).unwrap();
        (board.target, board.zones()[0].net)
    };
    let revision = h.state().session.revision();
    for (command, params) in [("view.set_net_color", json!({"view":view,"net":net,"color":"#29B6C8"})), ("edit.undo", serde_json::Value::Null)] {
        let mut editor = crate::live_control::LiveEditor { app: h.state_mut(), ctx: &ctx };
        let reply = pcbcraft_mcp::live::call_editor(
            &mut editor,
            &json!({"method":"engine.execute","params":{"command":command,"params":params,"inspect":false}}),
        )
        .unwrap();
        assert_eq!(reply["revision"], revision);
    }
    assert!(h.state().board_appearance.net_palette(target, true).is_none());
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
    click(&mut h, "Open PCB Layout beside current view");
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
            layer["name"] = json!("Part copper");
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
    h.state_mut().session.apply("project.new", &json!({"create_board":true})).unwrap();
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

/// Body and reference selection lighten each configured layer, including fixed silk and generated letters.
#[test]
fn footprint_selection_preserves_custom_copper_and_silkscreen_colors() {
    let mut h = board();
    let view = h.state().views.active.id();
    let copper = egui::Color32::from_rgb(160, 32, 48);
    let silk = egui::Color32::from_rgb(176, 142, 34);
    let top = layer(&h, "Top");
    let front_silk = layer(&h, "Front silkscreen");
    h.state_mut().set_layer_color(view, &top, Some([160, 32, 48])).unwrap();
    h.state_mut().set_layer_color(view, &front_silk, Some([176, 142, 34])).unwrap();
    let snapshot = h.state().session.snapshot().unwrap();
    let footprint = snapshot.board_view(snapshot.board_target().unwrap()).unwrap().footprints().next().unwrap();
    let id = footprint.id;
    let before = encode_documents(snapshot.clone()).unwrap();
    for reference in [false, true] {
        let policy = h.state().preferences.selection_follow;
        let app = h.state_mut();
        if reference {
            app.views.select_board_reference(id.clone(), &app.session, policy);
        } else {
            app.views.select_board(std::collections::BTreeSet::from([id.clone()]), &app.session, policy);
        }
        h.run();
        assert_eq!(lands(&h, copper.lerp_to_gamma(egui::Color32::WHITE, 0.35)), 2);
        let expected = silk.lerp_to_gamma(egui::Color32::WHITE, 0.35);
        let paths = h
            .output()
            .shapes
            .iter()
            .filter(|shape| matches!(&shape.shape,egui::Shape::Path(path) if path.stroke.color==egui::epaint::ColorMode::Solid(expected)))
            .count();
        assert!(paths > 2, "Fixed silk and generated reference paths keep the silk hue");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
    }
}

/// Every registered layer has an editable palette entry; supported technical graphics use it.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn technical_layer_colors_reach_edges_silkscreen_and_ribbon_patches() {
    let mut h = board();
    let ctx = h.ctx.clone();
    let view = h.state().views.active.id().0;
    let edge = layer(&h, "Board Outline");
    let silk = layer(&h, "Front silkscreen");
    let mask = layer(&h, "Front mask");
    let revision = h.state().session.revision();
    h.state_mut().run(pcbcraft_engine::ADD_DRAWING,&json!({"layer":edge,"points":[{"x_nm":-3000000,"y_nm":-3000000},{"x_nm":13000000,"y_nm":-3000000},{"x_nm":13000000,"y_nm":3000000},{"x_nm":-3000000,"y_nm":3000000}],"closed":true,"expected_revision":revision}));
    h.run();
    let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    for (id, color) in [(&edge, "#AE63D1"), (&silk, "#40C8D0"), (&mask, "#904030")] {
        h.state_mut().run_window_command(&ctx, "view.set_layer_color", &json!({"view":view,"layer":id,"color":color})).unwrap();
    }
    h.run();
    let path_color = |h: &Harness<'_, PcbApp>, color: egui::Color32| {
        h.output()
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape,egui::Shape::Path(path) if path.stroke.color==egui::epaint::ColorMode::Solid(color)))
    };
    assert!(path_color(&h, egui::Color32::from_rgb(0xAE, 0x63, 0xD1)));
    assert!(path_color(&h, egui::Color32::from_rgb(0x40, 0xC8, 0xD0)));
    click(&mut h, "Layout");
    click(&mut h, "Active layer");
    click(&mut h, "Board Outline");
    let field = h.get_by_label("Active layer").rect();
    let color = egui::Color32::from_rgb(0xAE, 0x63, 0xD1);
    assert!(
        h.output().shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Rect(rect) if rect.fill==color && field.contains_rect(rect.rect)))
    );
    click(&mut h, "Layout");
    click(&mut h, "Active layer");
    assert!(
        h.output().shapes.iter().filter(|shape| matches!(&shape.shape,egui::Shape::Rect(rect) if rect.fill==color)).count() >= 2,
        "open picker and selected field both need a color patch"
    );
    h.key_press(Key::Escape);
    h.run();
    swatch(&mut h, "Board Outline");
    hex_field(&mut h, "#DD8822");
    click(&mut h, "Apply color");
    assert_eq!(h.state().views.active.board.active_layer, Some(edge));
    assert!(path_color(&h, egui::Color32::from_rgb(0xDD, 0x88, 0x22)));
    let inspected = crate::window_commands::inspect_editor(h.state());
    assert!(inspected["views"].as_array().unwrap().iter().any(|view| {
        view["board"]["layer_colors"].as_array().is_some_and(|colors| colors.iter().any(|item| item["layer"] == mask && item["color"] == "#904030"))
    }));
    assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before);
}
