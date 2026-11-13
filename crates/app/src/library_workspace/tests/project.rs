use super::*;

/// Place an original native part through the same validated transaction used by the catalog.
#[allow(clippy::unwrap_used)]
fn place(app: &mut PcbApp) {
    let resolved = app.session.libraries().resolve_part(&Reference { library: "parts-demo".into(), id: "resistor".into(), revision: 1 }).unwrap();
    let digest = sources::part_digest(resolved);
    app.session.apply(pcbcraft_engine::part_instances::PLACE_PART, &json!({"library":"parts-demo","definition":"resistor","definition_revision":1,"definition_digest":digest,"x_nm":0,"y_nm":0,"expected_revision":app.session.revision()})).unwrap();
}

/// Shared tree rows and project previews work in both themes and compact/wide layouts offline.
#[test]
fn project_tree_browses_pins_without_mutating_or_requiring_a_source() {
    for (width, theme) in [(1800., crate::ThemeChoice::Dark), (620., crate::ThemeChoice::Light)] {
        let mut h = fixture(width);
        h.state_mut().preferences.theme = theme;
        place(h.state_mut());
        h.state_mut().session.execute(sources::REMOVE, &json!({"source":"example-parts"})).unwrap();
        h.run();
        let before = h.state().session.inspect();
        let snapshot = h.state().session.snapshot().unwrap();
        click(&mut h, "Project libraries");
        if width < 1050. {
            click(&mut h, "Browser");
        }
        click(&mut h, "Project Generic parts in Native Parts Demo");
        click(&mut h, "Project Generic part parts-demo/resistor r1");
        if width < 1050. {
            click(&mut h, "Preview");
        }
        h.get_by_label("Depends on");
        assert!(h.query_all_by_label("Source unavailable").next().is_some());
        assert_eq!(h.state().session.inspect(), before);
        assert!(Arc::ptr_eq(&snapshot, &h.state().session.snapshot().unwrap()));
        assert!(h.state().views.active.library.as_ref().unwrap().source_id().is_none());
    }
}

/// Project selection has no source authority; explicit copies use pinned geometry when offline.
#[test]
fn project_navigation_and_copy_are_owner_bound_and_source_actions_are_safe() {
    let mut h = fixture(1800.);
    place(h.state_mut());
    let ctx = h.ctx.clone();
    let app = h.state_mut();
    let view = app.views.active.id();
    app.run_window_command(&ctx, "app.library_browse", &json!({"view":view.0,"scope":"project","library":"parts-demo","type":1,"item":"resistor"}))
        .unwrap();
    let state = app.views.active.library.as_ref().unwrap().inspect();
    for request in [
        json!({"view":view.0,"scope":"project","source":"example-parts"}),
        json!({"view":view.0,"scope":"project","library":"missing"}),
        json!({"view":view.0,"scope":"project","item":"missing","query":"changed"}),
    ] {
        assert!(app.run_window_command(&ctx, "app.library_browse", &request).is_err());
        assert_eq!(app.views.active.library.as_ref().unwrap().inspect(), state);
    }
    let before = app.session.snapshot().unwrap();
    for action in ["remove", "delete", "relocate", "refresh"] {
        assert!(app.run_window_command(&ctx, "app.library_source", &json!({"view":view.0,"action":action})).is_err());
    }
    assert!(app.session.libraries().source_items("example-parts").len() > 1);
    assert!(app.edit_catalog_selection(&ctx, false).is_err());
    app.session.execute(sources::REMOVE, &json!({"source":"example-parts"})).unwrap();
    app.edit_catalog_selection(&ctx, true).unwrap();
    let doc = app.library_editors.active.unwrap();
    assert_eq!(
        app.library_editors.inspect()["documents"].as_array().unwrap().iter().find(|entry| entry["document"] == doc).unwrap()["state"]["item"]["library"],
        "user"
    );
    assert!(Arc::ptr_eq(&before, &app.session.snapshot().unwrap()));
    app.place_library_workspace(&ctx, Placement::Here).unwrap();
    app.run_window_command(&ctx, "app.library_browse", &json!({"view":view.0,"scope":"project","type":1,"item":"resistor"})).unwrap();
    app.session.apply("project.new", &json!({})).unwrap();
    assert!(app.edit_catalog_selection(&ctx, true).is_err());
    h.run();
    assert!(h.state().views.active.library.as_ref().unwrap().selected.is_none());
    assert!(h.state().project_library_inventory().unwrap().entries().next().is_none());
}

/// Idle panes share the projection; edits and Undo select their own immutable snapshot cache.
#[test]
fn project_inventory_cache_tracks_snapshot_identity() {
    let mut h = fixture(1800.);
    let first = h.state().project_library_inventory().unwrap();
    assert!(Arc::ptr_eq(&first, &h.state().project_library_inventory().unwrap()));
    place(h.state_mut());
    let placed = h.state().project_library_inventory().unwrap();
    assert!(!Arc::ptr_eq(&first, &placed));
    assert!(placed.entries().next().is_some());
    h.state_mut().session.apply("edit.undo", &Value::Null).unwrap();
    assert!(h.state().project_library_inventory().unwrap().entries().next().is_none());
}
