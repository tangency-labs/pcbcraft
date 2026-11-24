use super::{board_views::board_harness, *};

/// Open a real export beside its project, verify painted overlays and keep the project editable
/// from its own pane. A rejected replacement must leave the first package visible.
#[test]
fn manufacturing_reviews_export_beside_pcb_and_preserves_project() {
    let mut h = board_harness();
    let ctx = h.ctx.clone();
    let revision = h.state().session.revision();
    h.state_mut()
        .session
        .apply(pcbcraft_engine::INITIALIZE_BOARD_SETUP, &json!({"copper_layers":2,"resolve_existing_footprints":true,"expected_revision":revision}))
        .unwrap();
    click(&mut h, "Choose workspace");
    click(&mut h, "PCB Layout");
    let outline = h
        .state()
        .session
        .project()
        .and_then(|p| p.board_view(p.board_target()?))
        .unwrap()
        .layers()
        .iter()
        .find(|layer| layer.name == "Board Outline")
        .unwrap()
        .id;
    let at = |x_nm, y_nm| json!({"x_nm": x_nm, "y_nm": y_nm});
    let revision = h.state().session.revision();
    h.state_mut().session.apply(pcbcraft_engine::ADD_DRAWING, &json!({
        "layer": outline, "points": [at(-4_000_000, -3_000_000), at(4_000_000, -3_000_000), at(4_000_000, 3_000_000), at(-4_000_000, 3_000_000)],
        "closed": true, "expected_revision": revision,
    })).unwrap();
    let revision = h.state().session.revision();
    h.state_mut()
        .session
        .apply(
            pcbcraft_engine::ADD_VIA,
            &json!({"at":at(2_500_000,1_000_000),"via_diameter_nm":700_000,"via_drill_nm":356_000,"expected_revision":revision}),
        )
        .unwrap();
    h.run();
    let before = h.state().session.snapshot().unwrap();
    let revision = h.state().session.revision();
    let board = h.state().views.active.id();
    click_tab(&mut h, "Manufacture");
    click(&mut h, "Generate fabrication outputs");
    let archive = &h.state().fabrication_exports.captured.as_ref().unwrap().1;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("build.zip");
    std::fs::write(&path, archive.bytes()).unwrap();
    h.state_mut().fabrication_exports.complete_captured(Ok(crate::board_outputs::Outcome::Saved(path.clone()))).unwrap();
    h.run();
    assert_eq!(crate::window_commands::inspect_editor(h.state())["last_fabrication_export"], json!(path));

    click(&mut h, "Choose workspace");
    click(&mut h, "Open Manufacturing beside current view");
    let view = h.state().views.active.id();
    assert_ne!(view, board);
    assert!(h.state().views.active.input(&h.state().session).is_none());
    click(&mut h, "Open the last saved fabrication ZIP");
    let inspect = |app: &PcbApp| app.views.view(view).unwrap().manufacturing.as_ref().unwrap().inspect(&app.manufacturing_files, view);
    for _ in 0..120 {
        h.run();
        if !inspect(h.state())["package"].is_null() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let state = inspect(h.state());
    assert_eq!(state["package"]["filename"], "build.zip", "{state:#}");
    assert!(
        state["package"]["layers"].as_array().unwrap().iter().any(|layer| {
            layer["role"] == "plated_drill"
                && layer["drill_tools"].as_array().unwrap().iter().any(|tool| tool["diameter_nm"] == 356_000 && tool["hits"] == 1)
        }),
        "{state:#}",
    );
    let layer = state["package"]["layers"].as_array().unwrap().iter().find(|layer| layer["role"] == "top_copper").unwrap();
    assert!(h.query_by_label("Top copper").is_some());
    let has_copper = |h: &egui_kittest::Harness<'_, PcbApp>| {
        let color = pcbcraft_chrome::Tokens::get().board_front;
        let canvas = h.get_by_label("Fabrication canvas").rect();
        h.output().shapes.iter().any(|shape| {
            shape.clip_rect.intersect(canvas).width() > canvas.width() / 2.0
                && match &shape.shape {
                    egui::Shape::Circle(circle) => circle.fill == color,
                    egui::Shape::Rect(rect) => rect.fill == color,
                    _ => false,
                }
        })
    };
    assert!(has_copper(&h));
    h.state_mut()
        .run_window_command(&ctx, "app.set_fabrication_layer", &json!({"view":view.0,"filename":layer["filename"],"visible":false}))
        .unwrap();
    h.run();
    assert!(!has_copper(&h));
    let state = inspect(h.state());
    assert!(state["package"]["layers"].as_array().unwrap().iter().any(|layer| layer["role"] == "top_copper" && layer["visible"] == false));

    let invalid = dir.path().join("invalid.zip");
    std::fs::write(&invalid, b"invalid").unwrap();
    h.state_mut().run_window_command(&ctx, "app.open_fabrication_package", &json!({"view":view.0,"path":invalid})).unwrap();
    for _ in 0..120 {
        h.run();
        if !inspect(h.state())["error"].is_null() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let state = inspect(h.state());
    assert_eq!(state["package"]["filename"], "build.zip");
    assert!(state["error"].as_str().unwrap().contains("Invalid fabrication ZIP"));
    assert_eq!(h.state().session.revision(), revision);
    assert!(Arc::ptr_eq(&h.state().session.snapshot().unwrap(), &before));
    assert!(h.state().views.view(board).unwrap().input(&h.state().session).is_some());
}
