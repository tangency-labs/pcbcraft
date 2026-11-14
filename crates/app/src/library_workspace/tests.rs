use super::*;
use crate::tests::click;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use pcbcraft_engine::library::sources;
use std::sync::Arc;

/// Real native records exercise browsing independently of starter identities.
#[allow(clippy::unwrap_used)]
fn fixture(width: f32) -> Harness<'static, PcbApp> {
    let mut app = PcbApp::default();
    app.run("project.new", &json!({}));
    let source = pcbcraft_storage::library::load(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/native-parts"))).unwrap();
    app.session.install_library_source("example-parts", Arc::new(source)).unwrap();
    app.place_library_workspace(&egui::Context::default(), Placement::Here).unwrap();
    let mut h = Harness::builder().with_max_steps(64).with_size(egui::vec2(width, 950.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    h
}

/// Expansion and selection are independent; nested and flat views address identical records.
#[test]
fn browse_select_expand_and_flat_view_preserve_project_state() {
    for width in [1800., 620.] {
        let mut h = fixture(width);
        let before = h.state().session.inspect();
        let project = h.state().session.snapshot().unwrap();
        click(&mut h, "Native Parts Demo");
        if width < 1050. {
            click(&mut h, "Browser");
        }
        click(&mut h, "Generic parts in Native Parts Demo");
        click(&mut h, "Generic part parts-demo/resistor r1");
        assert!(h.state().views.active.library.as_ref().unwrap().expanded.is_empty());
        click(&mut h, "Expand Generic resistor");
        click(&mut h, "Orderable part parts-demo/test-resistor-100m r1");
        let selected = h.state().views.active.library.as_ref().unwrap().selected.clone();
        if width < 1050. {
            click(&mut h, "Browser");
        }
        click(&mut h, "Orderable parts in Native Parts Demo");
        click(&mut h, "Orderable part parts-demo/test-resistor-100m r1");
        assert_eq!(h.state().views.active.library.as_ref().unwrap().selected, selected, "width={width}, state={:?}", h.state().views.active.library);
        assert_eq!(h.state().session.inspect(), before);
        assert!(Arc::ptr_eq(&project, &h.state().session.snapshot().unwrap()));
        if width < 1050. {
            click(&mut h, "Preview");
        }
        assert!(h.query_all_by_label("Illustrative Manufacturer").next().is_some());
        h.get_by_label("Specifications");
        h.get_by_label("Depends on");
    }
}

/// MCP layout operations support Library beside a definition editor without an open project.
#[test]
fn mcp_mixed_views_detach_and_source_protection() {
    let mut app = PcbApp::default();
    let ctx = egui::Context::default();
    app.run_window_command(&ctx, "app.set_workspace", &json!({"workspace":"library"})).unwrap();
    assert_eq!(app.workspace_key(), "library");
    assert!(app.views.active.target().is_none());
    assert!(app.views.active.input(&app.session).is_none());
    let root = app.views.active.id();
    app.run_window_command(&ctx, "view.open_beside", &json!({"view":root.0})).unwrap();
    assert!(app.views.is_split());
    let other = app.views.hosts[&app.views.main].layout.views().into_iter().find(|id| *id != root).unwrap();
    assert!(app.views.view(other).unwrap().library.is_some());
    app.run_window_command(&ctx, "app.set_workspace", &json!({"view":other.0,"workspace":"symbol_editor"})).unwrap();
    assert!(app.views.view(other).unwrap().definition.is_some());
    assert!(app.views.view(root).unwrap().library.is_some());
    app.run_window_command(&ctx, "view.detach", &json!({"view":root.0})).unwrap();
    assert_ne!(app.views.owner(root).unwrap(), app.views.main);
    app.run_window_command(&ctx, "view.dock", &json!({"view":root.0})).unwrap();
    assert_eq!(app.views.owner(root), Some(app.views.main));
    let source =
        app.session.libraries().inspect().as_array().unwrap().iter().find(|s| s["builtin"] == true).unwrap()["source"].as_str().unwrap().to_owned();
    for action in ["remove", "delete", "refresh", "relocate"] {
        assert!(app.run_window_command(&ctx, "app.library_source", &json!({"view":root.0,"source":source,"action":action})).is_err());
    }
    let inspect = crate::window_commands::inspect_editor(&app);
    assert!(inspect["views"].as_array().unwrap().iter().any(|v| v["workspace"] == "library"));
    assert!(app.session.project().is_none());
    app.run_window_command(&ctx, "view.set_panel_position", &json!({"view":root.0,"panel":"preview","position":"floating","width":640,"height":600}))
        .unwrap();
    assert_eq!(app.views.view(root).unwrap().panels[crate::workspace_panels::PROPERTIES].saved_state().size, Some([640., 600.]));
    assert!(app.run_window_command(&ctx, "view.set_panel_position", &json!({"view":root.0,"panel":"selection_filters","position":"left"})).is_err());
}

/// Failed browsing validates before changing pane state, and sibling views retain independent queries.
#[test]
fn mcp_browse_is_pane_local_and_rejects_missing_items_atomically() {
    let mut h = fixture(1800.);
    let ctx = h.ctx.clone();
    let app = h.state_mut();
    let root = app.views.active.id();
    let other = app.views.open_beside(root).unwrap();
    let before = app.session.inspect();
    app.run_window_command(&ctx, "app.library_browse", &json!({"view":root.0,"source":"example-parts","type":3,"item":"resistor","query":"res"}))
        .unwrap();
    let state = app.views.view(root).unwrap().library.as_ref().unwrap().inspect();
    assert_ne!(app.views.view(other).unwrap().library.as_ref().unwrap().inspect(), state);
    assert!(
        app.run_window_command(&ctx, "app.library_browse", &json!({"view":root.0,"source":"example-parts","type":3,"item":"missing","query":"bad"}))
            .is_err()
    );
    assert_eq!(app.views.view(root).unwrap().library.as_ref().unwrap().inspect(), state);
    assert_eq!(app.session.inspect(), before);
}

/// All resolves actual items with or without an explicit Type, while a specific
/// incompatible Type still refuses without changing the pane or project.
#[test]
fn review_all_type_resolves_items() {
    let mut h = fixture(1800.);
    let ctx = h.ctx.clone();
    let app = h.state_mut();
    let before = app.session.inspect();
    let view = app.views.active.id().0;
    for request in [
        json!({"view":view,"source":"example-parts","item":"resistor-0603"}),
        json!({"view":view,"source":"example-parts","type":0,"item":"test-resistor-100m"}),
    ] {
        app.run_window_command(&ctx, "app.library_browse", &request).unwrap();
        assert_eq!(app.views.active.library.as_ref().unwrap().selected.as_ref().unwrap().id, request["item"]);
    }
    let state = app.views.active.library.as_ref().unwrap().inspect();
    assert!(app.run_window_command(&ctx, "app.library_browse", &json!({"view":view,"type":1,"item":"resistor-0603"})).is_err());
    assert_eq!(app.views.active.library.as_ref().unwrap().inspect(), state);
    assert_eq!(app.session.inspect(), before);
}

/// An orderable record previews its chosen package even when another sorts first;
/// generic browsing keeps its first-package default and missing choices stay explicit.
#[test]
fn review_orderable_preview_uses_selected_package() {
    let mut documents = sources::Documents::from([
        ("library.json".into(), include_str!("../../../../examples/native-parts/library.json").into()),
        ("symbols/resistor.json".into(), include_str!("../../../../examples/native-parts/symbols/resistor.json").into()),
        ("categories/resistor.json".into(), include_str!("../../../../examples/native-parts/categories/resistor.json").into()),
        ("footprints/resistor.json".into(), include_str!("../../../../examples/native-parts/footprints/resistor-0603.json").into()),
    ]);
    let mut part: Value = serde_json::from_str(include_str!("../../../../examples/native-parts/parts/resistor.json")).unwrap();
    let mut package = part["packages"]["0603"].clone();
    package["name"] = json!("Chosen package");
    part["packages"]["zz-selected"] = package;
    documents.insert("parts/resistor.json".into(), part.to_string());
    let mut record: Value =
        serde_json::from_str(include_str!("../../../../examples/native-parts/manufacturer-parts/test-resistor-100m.json")).unwrap();
    record["package"] = json!("zz-selected");
    documents.insert("manufacturer-parts/resistor.json".into(), record.to_string());
    let mut app = PcbApp::default();
    app.run("project.new", &json!({}));
    app.session.install_library_source("packages", Arc::new(sources::NativeSource::load(&documents).unwrap())).unwrap();
    app.place_library_workspace(&egui::Context::default(), Placement::Here).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1800., 950.)).build_ui_state(|ui, app: &mut PcbApp| app.frame(ui), app);
    h.run();
    let before = h.state().session.inspect();
    let ctx = h.ctx.clone();
    let view = h.state().views.active.id().0;
    h.state_mut()
        .run_window_command(&ctx, "app.library_browse", &json!({"view":view,"source":"packages","type":4,"item":"test-resistor-100m"}))
        .unwrap();
    h.run();
    // One label is package metadata; the second is the footprint preview caption.
    assert_eq!(h.query_all_by_label("Chosen package").count(), 2);
    h.state_mut().run_window_command(&ctx, "app.library_browse", &json!({"view":view,"type":3,"item":"resistor"})).unwrap();
    h.run();
    assert_eq!(h.query_all_by_label("Chosen package").count(), 1);
    let part = h.state().session.libraries().source_items("packages").iter().find_map(|item| item.part()).unwrap();
    let mut state = State::default();
    let mut missing = Harness::builder().with_size(egui::vec2(1000., 900.)).build_ui(|ui| {
        h.state().generic_preview(ui, &part, Some("missing"), &mut Vec::new(), &mut state);
    });
    missing.run();
    missing.get_by_label("Package missing is unavailable");
    assert_eq!(h.state().session.inspect(), before);
}

/// Invalid native files and quarantined revisions remain visible beside the selected source.
#[test]
fn source_diagnostics_and_browser_collapse_use_real_source_state() {
    let mut h = fixture(1800.);
    let documents = sources::Documents::from([
        (
            "library.json".into(),
            json!({"format":"pcbcraft-library/1","id":"test-diag","name":"Diagnostic fixture","license":"MIT OR Apache-2.0"}).to_string(),
        ),
        ("symbols/resistor.json".into(), include_str!("../../../../libraries/starter/symbols/resistor.json").into()),
        ("symbols/bad.json".into(), "invalid native fixture".into()),
    ]);
    let source = Arc::new(sources::NativeSource::load(&documents).unwrap());
    h.state_mut().session.install_library_source("diag", source).unwrap();
    let mut other = documents.clone();
    let mut symbol: Value = serde_json::from_str(&other["symbols/resistor.json"]).unwrap();
    symbol["name"] = json!("Conflicting resistor");
    other.insert("symbols/resistor.json".into(), symbol.to_string());
    other.insert(
        "library.json".into(),
        json!({"format":"pcbcraft-library/1","id":"test-diag","name":"Conflicting fixture","license":"MIT OR Apache-2.0"}).to_string(),
    );
    h.state_mut().session.install_library_source("conflict", Arc::new(sources::NativeSource::load(&other).unwrap())).unwrap();
    h.run();
    click(&mut h, "Diagnostic fixture");
    let data = h.state().session.libraries().inspect();
    let problems = data.as_array().unwrap().iter().find(|s| s["source"] == "diag").unwrap()["problems"].as_array().unwrap();
    assert!(problems.iter().any(|p| p.as_str().unwrap().contains("bad.json")));
    assert!(problems.iter().any(|p| p.as_str().unwrap().contains("conflicting")));
    for problem in problems {
        h.get_by_label(problem.as_str().unwrap());
    }
    click(&mut h, "Collapse Browser");
    assert!(h.state().views.active.library.as_ref().unwrap().browser_collapsed);
    click(&mut h, "Expand Browser");
    assert!(!h.state().views.active.library.as_ref().unwrap().browser_collapsed);
}

/// A short empty preview still owns a full-height column, and a live header drag
/// survives frames, cancellation, floating movement and both docking targets.
#[test]
fn preview_reserves_space_and_preserves_live_drag() {
    use crate::workspace_panels::PROPERTIES;
    use pcbcraft_chrome::workspace::panels::Position;
    let mut h = fixture(1800.);
    let before = h.state().session.inspect();
    let header = h.get_by_label("Move Preview").rect();
    let table_end = h.get_by_label("Rev").rect();
    assert!(table_end.right() < header.left(), "docked Preview must not cover the table");
    let mut pending: Vec<_> = h.output().shapes.iter().map(|s| &s.shape).collect();
    let mut frames = Vec::new();
    while let Some(shape) = pending.pop() {
        match shape {
            egui::Shape::Vec(shapes) => pending.extend(shapes),
            egui::Shape::Rect(r) if r.rect.height() > 600. && r.rect.width() < 500. => frames.push(r.rect),
            _ => {}
        }
    }
    let preview_frame = *frames.iter().find(|r| r.contains_rect(header)).expect("empty Preview must retain its full dock height");
    let browser_frame = frames.iter().find(|r| r.width() > 220. && r.width() < 240.).unwrap();
    assert!((preview_frame.top() - browser_frame.top()).abs() < 1.0, "{preview_frame:?}, {browser_frame:?}");
    assert!((preview_frame.bottom() - browser_frame.bottom()).abs() < 1.0, "{preview_frame:?}, {browser_frame:?}");
    let center = egui::pos2(900., 420.);
    h.drag_at(header.center());
    h.run();
    h.hover_at(center);
    h.run();
    assert_eq!(h.state().views.active.panels[PROPERTIES].position(), Position::Floating);
    let first = h.get_by_label("Move Preview").rect();
    h.hover_at(center + egui::vec2(60., 40.));
    h.run();
    let moved = h.get_by_label("Move Preview").rect();
    assert!((moved.min - first.min - egui::vec2(60., 40.)).length() < 1.0, "drag must continue across frames");
    h.key_press(egui::Key::Escape);
    h.run();
    h.drop_at(center);
    h.run();
    assert_eq!(h.state().views.active.panels[PROPERTIES].position(), Position::Right);
    for (at, position) in [(center, Position::Floating), (egui::pos2(20., 420.), Position::Left), (egui::pos2(1780., 420.), Position::Right)] {
        let from = h.get_by_label("Move Preview").rect().center();
        h.drag_at(from);
        h.run();
        h.hover_at(at);
        h.run();
        h.drop_at(at);
        h.run();
        assert_eq!(h.state().views.active.panels[PROPERTIES].position(), position);
        if position == Position::Right {
            assert!(h.get_by_label("Rev").rect().right() < h.get_by_label("Move Preview").rect().left());
        }
    }
    assert_eq!(h.state().session.inspect(), before);
}

/// Library content cannot paint over the shared pane tab, even though its background
/// fills the pane. Closing uses the same host action as schematic/PCB panes.
#[test]
fn split_library_keeps_shared_headers_visible_and_usable() {
    let mut h = fixture(1800.);
    let id = h.state().views.active.id();
    h.state_mut().views.open_beside(id).unwrap();
    h.run();
    let headers: Vec<_> = h.query_all_by_label("Close view Parts Library").map(|n| n.rect()).collect();
    assert_eq!(headers.len(), 2);
    for header in headers {
        for s in &h.output().shapes {
            if let egui::Shape::Rect(r) = &s.shape {
                assert!(
                    !(r.fill == pcbcraft_chrome::Tokens::get().canvas
                        && r.rect.width() < 1000.
                        && r.rect.intersect(s.clip_rect).contains(header.center())),
                    "workspace fill covers tab"
                );
            }
        }
    }
    h.get_all_by_label("Close view Parts Library").next().unwrap().click();
    h.run();
    assert!(!h.state().views.is_split());
}

/// Source acquisition is an anchored Craft menu with full-width rows; dismissing
/// or clicking future connections never opens a file picker or project modal.
#[test]
fn add_library_uses_standard_menu_rows_and_dismissal() {
    let mut h = fixture(1800.);
    let trigger = h.get_by_label("Add library").rect();
    click(&mut h, "Add library");
    assert!(matches!(h.state().dialog, Dialog::None));
    let folder = h.get_by_label("Choose folder…").rect();
    let bundle = h.get_by_label("Choose bundle…").rect();
    assert!(folder.width() > 200. && (folder.width() - bundle.width()).abs() < 1., "folder={folder:?}, bundle={bundle:?}");
    assert!(folder.top() >= trigger.bottom() - 1.);
    assert!((folder.left() - bundle.left()).abs() < 1.);
    assert!(h.get_by_label("Connect Git…").accesskit_node().is_disabled());
    assert!(h.get_by_label("Connect remote catalog…").accesskit_node().is_disabled());
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(h.query_by_label("Choose folder…").is_none());
    assert!(!h.state().library_sources.busy());
    // The caption and command fallback follow the same row style and Escape behavior.
    click(&mut h, "Sources menu");
    h.get_all_by_label("Add library").last().unwrap().click();
    h.run();
    h.get_by_label("Choose folder…");
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(matches!(h.state().dialog, Dialog::None));
}

/// Read-only cameras own wheel and drag only over their canvas, remain independent,
/// and reset for another revision/item without touching the project or source records.
#[test]
fn geometry_navigation_is_independent_and_resets_on_selection() {
    let mut h = fixture(1800.);
    let ctx = h.ctx.clone();
    let id = h.state().views.active.id();
    h.state_mut().run_window_command(&ctx, "app.library_browse", &json!({"view":id.0,"source":"example-parts","type":3,"item":"resistor"})).unwrap();
    h.run();
    let before = h.state().session.inspect();
    for (label, symbol) in [("Symbol preview", true), ("Footprint preview", false)] {
        let initial = if symbol {
            h.state().views.active.library.as_ref().unwrap().symbol_camera.clone()
        } else {
            h.state().views.active.library.as_ref().unwrap().footprint_camera.clone()
        };
        click(&mut h, &format!("Zoom in {label}"));
        let zoomed = if symbol {
            h.state().views.active.library.as_ref().unwrap().symbol_camera.clone()
        } else {
            h.state().views.active.library.as_ref().unwrap().footprint_camera.clone()
        };
        assert_ne!(zoomed, initial);
        let canvas = h.get_by_label(label).rect();
        h.hover_at(canvas.center() + egui::vec2(15., 10.));
        h.run();
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            phase: egui::TouchPhase::Move,
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., 80.),
            modifiers: egui::Modifiers::NONE,
        });
        h.run();
        assert_eq!(h.get_by_label(label).rect(), canvas, "wheel over geometry must not scroll the inspector");
        let wheeled = if symbol {
            h.state().views.active.library.as_ref().unwrap().symbol_camera.clone()
        } else {
            h.state().views.active.library.as_ref().unwrap().footprint_camera.clone()
        };
        assert_ne!(wheeled, zoomed);
        h.drag_at(canvas.center());
        h.run();
        h.hover_at(canvas.center() + egui::vec2(35., 20.));
        h.run();
        h.drop_at(canvas.center() + egui::vec2(35., 20.));
        h.run();
        let panned = if symbol {
            h.state().views.active.library.as_ref().unwrap().symbol_camera.clone()
        } else {
            h.state().views.active.library.as_ref().unwrap().footprint_camera.clone()
        };
        assert_ne!(panned, wheeled);
        let state = h.state().views.active.library.as_ref().unwrap();
        assert_eq!(if symbol { &state.footprint_camera } else { &state.symbol_camera }, &crate::library_preview::Camera::default());
        click(&mut h, &format!("Fit {label}"));
        let state = h.state().views.active.library.as_ref().unwrap();
        assert_eq!(state.symbol_camera, crate::library_preview::Camera::default());
        assert_eq!(state.footprint_camera, crate::library_preview::Camera::default());
    }
    click(&mut h, "Zoom in Symbol preview");
    h.state_mut().run_window_command(&ctx, "app.library_browse", &json!({"view":id.0,"source":"example-parts","type":3,"item":"connector"})).unwrap();
    h.run();
    assert_eq!(h.state().views.active.library.as_ref().unwrap().symbol_camera, crate::library_preview::Camera::default());
    assert_eq!(h.state().session.inspect(), before);
}

/// Floating resizing uses shared edge sashes, supports keyboard adjustment and
/// Escape, and retains sizes through docking and local layout serialization.
#[test]
fn floating_preview_resizes_expands_and_restores() {
    use crate::workspace_panels::PROPERTIES;
    let mut h = fixture(1800.);
    let before = h.state().session.inspect();
    click(&mut h, "Preview position");
    click(&mut h, "Float");
    let left = h.get_by_label("Resize Preview width from left").rect();
    let old_right = h.get_by_label("Resize Preview width").rect().right();
    h.drag_at(left.center());
    h.run();
    h.hover_at(left.center() - egui::vec2(60., 0.));
    h.run();
    h.drop_at(left.center() - egui::vec2(60., 0.));
    h.run();
    assert!(h.state().views.active.panels[PROPERTIES].saved_state().size.unwrap()[0] > 480.);
    assert!((h.get_by_label("Resize Preview width").rect().right() - old_right).abs() < 1.0);
    let width = h.get_by_label("Resize Preview width").rect();
    h.drag_at(width.center());
    h.run();
    h.hover_at(width.center() - egui::vec2(140., 0.));
    h.run();
    h.drop_at(width.center() - egui::vec2(140., 0.));
    h.run();
    let size = h.state().views.active.panels[PROPERTIES].saved_state().size.unwrap();
    assert!(size[0] > 300. && size[0] < 400.);
    let height = h.get_by_label("Resize Preview height").rect();
    h.drag_at(height.center());
    h.run();
    h.hover_at(height.center() + egui::vec2(0., 90.));
    h.run();
    h.drop_at(height.center() + egui::vec2(0., 90.));
    h.run();
    let state = h.state().views.active.panels[PROPERTIES].saved_state();
    assert!(state.size.unwrap()[1] > 500.);
    let height = h.get_by_label("Resize Preview height").rect();
    h.drag_at(height.center());
    h.run();
    h.hover_at(height.center() + egui::vec2(0., 40.));
    h.run();
    h.key_press(egui::Key::Escape);
    h.run();
    h.drop_at(height.center());
    h.run();
    assert_eq!(h.state().views.active.panels[PROPERTIES].saved_state(), state);
    h.get_by_label("Resize Preview width").click();
    h.run();
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert!(h.state().views.active.panels[PROPERTIES].saved_state().size.unwrap()[0] > state.size.unwrap()[0]);
    let saved = h.state().views.active.panels[PROPERTIES].saved_state();
    let decoded: pcbcraft_chrome::workspace::panels::PanelState = serde_json::from_value(serde_json::to_value(saved).unwrap()).unwrap();
    assert!(decoded.valid());
    assert_eq!(decoded, saved);
    click(&mut h, "Preview position");
    click(&mut h, "Dock right");
    click(&mut h, "Preview position");
    click(&mut h, "Float");
    assert_eq!(h.state().views.active.panels[PROPERTIES].saved_state().size, saved.size);
    click(&mut h, "Preview position");
    click(&mut h, "Expand Preview");
    let expanded = h.state().views.active.panels[PROPERTIES].saved_state().size.unwrap();
    assert!(expanded[0] > 1700. && expanded[1] > 650.);
    assert!(h.get_by_label("Resize Preview width").rect().right() < 1800.);
    click(&mut h, "Preview position");
    click(&mut h, "Reset Preview size");
    assert!(h.state().views.active.panels[PROPERTIES].saved_state().size.is_none());
    assert_eq!(h.state().session.inspect(), before);
}

/// Left-sash arrows move that edge in the named direction while anchoring the right edge.
#[test]
fn review_left_sash_keyboard_direction() {
    let mut h = fixture(1800.);
    click(&mut h, "Preview position");
    click(&mut h, "Float");
    h.get_by_label("Resize Preview width from left").click();
    h.run();
    let left = h.get_by_label("Resize Preview width from left").rect().left();
    let right = h.get_by_label("Resize Preview width").rect().right();
    h.key_press(egui::Key::ArrowLeft);
    h.run();
    assert!(h.get_by_label("Resize Preview width from left").rect().left() < left);
    assert!((h.get_by_label("Resize Preview width").rect().right() - right).abs() < 1.);
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert!((h.get_by_label("Resize Preview width from left").rect().left() - left).abs() < 1.);
    assert!((h.get_by_label("Resize Preview width").rect().right() - right).abs() < 1.);
}

/// Existing local arrangements omit size; invalid dimensions must not grant a
/// floating card an unbounded or non-finite layout rectangle.
#[test]
fn panel_sizes_accept_old_layouts_and_reject_invalid_dimensions() {
    use pcbcraft_chrome::workspace::panels::PanelState;
    let old: PanelState = serde_json::from_value(json!({"position":"floating","at":[12,24]})).unwrap();
    assert!(old.valid());
    assert!(old.size.is_none());
    for size in [[-1., 100.], [100., 0.], [f32::INFINITY, 100.], [100., f32::NAN], [100_001., 100.]] {
        assert!(!PanelState { size: Some(size), ..old }.valid());
    }
}

/// Live clients address the same independent cameras and floating card state as GUI
/// controls. Invalid compound requests publish neither navigation nor browsing changes.
#[test]
fn preview_commands_share_state_and_reject_partial_updates() {
    let mut h = fixture(1800.);
    let ctx = h.ctx.clone();
    let id = h.state().views.active.id();
    let before = h.state().session.inspect();
    h.state_mut()
        .run_window_command(
            &ctx,
            "app.library_browse",
            &json!({"view":id.0,"source":"example-parts","type":3,"item":"resistor","preview":"symbol","zoom_percent":125}),
        )
        .unwrap();
    h.run();
    let state = h.state().views.active.library.as_ref().unwrap();
    assert_eq!(state.inspect()["previews"]["symbol"]["zoom_percent"], json!(125.0));
    assert_eq!(state.footprint_camera, crate::library_preview::Camera::default());
    let saved = state.inspect();
    for extra in [json!({"zoom_percent":0}), json!({"pan_x":10}), json!({"fit":true,"zoom_percent":125}), json!({"pan_x":100001,"pan_y":0})] {
        let mut request = json!({"view":id.0,"source":"example-parts","type":3,"item":"connector","query":"should not commit","preview":"symbol"});
        request.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        assert!(h.state_mut().run_window_command(&ctx, "app.library_browse", &request).is_err());
        assert_eq!(h.state().views.active.library.as_ref().unwrap().inspect(), saved);
    }
    h.state_mut().run_window_command(&ctx, "app.library_browse", &json!({"view":id.0,"preview":"symbol","pan_x":25,"pan_y":-10})).unwrap();
    h.run();
    assert_eq!(h.state().views.active.library.as_ref().unwrap().inspect()["previews"]["symbol"]["pan"], json!([25.0, -10.0]));
    click(&mut h, "Fit Symbol preview");
    assert_eq!(h.state().views.active.library.as_ref().unwrap().symbol_camera, crate::library_preview::Camera::default());
    h.state_mut()
        .run_window_command(
            &ctx,
            "view.set_panel_position",
            &json!({"view":id.0,"panel":"preview","position":"floating","x":100,"y":50,"width":620,"height":600}),
        )
        .unwrap();
    h.run();
    use crate::workspace_panels::PROPERTIES;
    let panel = h.state().views.active.panels[PROPERTIES].saved_state();
    assert_eq!(panel.size, Some([620., 600.]));
    assert!(
        h.state_mut()
            .run_window_command(
                &ctx,
                "view.set_panel_position",
                &json!({"view":id.0,"panel":"preview","position":"floating","width":-1,"height":500})
            )
            .is_err()
    );
    assert_eq!(h.state().views.active.panels[PROPERTIES].saved_state(), panel);
    assert_eq!(h.state().session.inspect(), before);
}

/// Workspace titles and picker order follow the host catalogue. Floating cards share a
/// width while adapting their heights to the ribbon; narrow chooser buttons share both dimensions.
#[test]
fn workspace_names_order_and_consistent_tile_widths() {
    let order: Vec<_> = crate::WORKSPACES.entries.iter().map(|e| (e.id, e.title)).collect();
    assert_eq!(&order[..3], &[("schematic", "Schematic"), ("board", "PCB Layout"), ("library", "Parts Library")]);
    for width in [1800., 620., 520.] {
        let mut h = fixture(width);
        let ctx = h.ctx.clone();
        let mut expected: Option<egui::Vec2> = None;
        for key in ["schematic", "board", "library", "symbol_editor", "footprint_editor"] {
            h.state_mut().enter_workspace(&ctx, key, Placement::Here).unwrap();
            h.run();
            let size = h.get_by_label("Choose workspace").rect().size();
            if let Some(expected) = expected {
                assert_eq!(size.x, expected.x, "{key}, width={width}");
                if width < 600. {
                    assert_eq!(size.y, expected.y, "compact {key}, width={width}");
                }
            } else {
                expected = Some(size);
            }
        }
        click(&mut h, "Choose workspace");
        assert!(h.get_by_label("PCB Layout").rect().top() < h.get_by_label("Parts Library").rect().top());
        assert!(h.get_by_label("Parts Library").rect().top() < h.get_by_label("Symbol Editor").rect().top());
    }
}
