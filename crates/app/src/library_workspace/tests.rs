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
    let headers: Vec<_> = h.query_all_by_label("Close view Library").map(|n| n.rect()).collect();
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
    h.get_all_by_label("Close view Library").next().unwrap().click();
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
