//! #281 board Properties length fields: draft ownership across revisions, selections and views,
//! typed edits against the canonical `_nm` commands, and exact values through real files.
use super::board_tracks::{add_track, routable, tracks, vias};
use super::*;
use pcbcraft_chrome::units::Unit;
use pcbcraft_engine::{persistence::encode_documents, schematic::Point};

/// Click a labelled field and type into it, without confirming.
fn type_into(h: &mut Harness<'_, PcbApp>, label: &str, text: &str) {
    h.get_by_label(label).click();
    h.run();
    h.get_by_label(label).type_text(text);
    h.run();
}

/// Type into a labelled field and confirm with Enter.
fn enter(h: &mut Harness<'_, PcbApp>, label: &str, text: &str) {
    type_into(h, label, text);
    h.key_press(Key::Enter);
    h.run();
}

fn first_footprint(h: &Harness<'_, PcbApp>) -> String {
    h.state().session.project().unwrap().boards().next().unwrap().footprints().next().unwrap().id.clone()
}

fn position(h: &Harness<'_, PcbApp>, id: &str) -> Point {
    h.state().session.project().unwrap().boards().next().unwrap().footprints().find(|f| f.id == id).unwrap().position
}

/// Review probe: an unfinished draft typed before a peer's edit is dropped, so Enter cannot turn
/// it into a command against the new revision.
#[test]
fn a_peer_revision_discards_an_unfinished_length_draft() {
    let mut h = routable();
    h.state_mut().preferences.units = Unit::Mm;
    let id = first_footprint(&h);
    h.state_mut().views.active.board.selection.insert(id.clone());
    h.run();
    let original = position(&h, &id);
    type_into(&mut h, "Footprint X", "100 mil");
    h.state_mut().session.apply("project.rename", &json!({"name":"Peer revision"})).unwrap();
    h.run();
    let peer_revision = h.state().session.revision();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!((position(&h, &id), h.state().session.revision()), (original, peer_revision));
    assert_eq!(h.get_by_label("Footprint X").value().as_deref(), Some("0 mm"));
    // A fresh entry at the new revision applies once.
    enter(&mut h, "Footprint X", "100 mil");
    assert_eq!((position(&h, &id).x_nm, h.state().session.revision()), (2_540_000, peer_revision + 1));
}

/// A width typed for one track is not applied when the selection grows before confirmation,
/// even though the first selected track and its width are unchanged.
#[test]
fn a_changed_track_selection_discards_an_unfinished_width() {
    let mut h = routable();
    h.state_mut().preferences.units = Unit::Mm;
    let a = add_track(&mut h, Point { x_nm: 2_540_000, y_nm: 2_540_000 }, Point { x_nm: 7_620_000, y_nm: 2_540_000 });
    let b = add_track(&mut h, Point { x_nm: 2_540_000, y_nm: 5_080_000 }, Point { x_nm: 7_620_000, y_nm: 5_080_000 });
    let (first, second) = (a.min(b), a.max(b));
    h.state_mut().views.active.board.tracks.selected = std::collections::BTreeSet::from([first]);
    h.run();
    let revision = h.state().session.revision();
    type_into(&mut h, "Selected track width", "0.4");
    h.state_mut().views.active.board.tracks.selected.insert(second);
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(tracks(&h).iter().map(|t| t.width_nm).collect::<Vec<_>>(), [250_000, 250_000]);
    assert_eq!(h.state().session.revision(), revision);
}

/// Mixed track widths show the first width; entering that same value, in any spelling, still
/// sets every selected track, while focusing and confirming untouched text changes nothing.
#[test]
fn an_entered_width_normalizes_mixed_track_widths() {
    let mut h = routable();
    h.state_mut().preferences.units = Unit::Mm;
    let a = add_track(&mut h, Point { x_nm: 2_540_000, y_nm: 2_540_000 }, Point { x_nm: 7_620_000, y_nm: 2_540_000 });
    let b = add_track(&mut h, Point { x_nm: 2_540_000, y_nm: 5_080_000 }, Point { x_nm: 7_620_000, y_nm: 5_080_000 });
    // Selected widths in board order: the Properties field shows the first.
    let widths = |h: &Harness<'_, PcbApp>| tracks(h).iter().filter(|t| [a, b].contains(&t.id)).map(|t| t.width_nm).collect::<Vec<_>>();
    let second = tracks(&h).into_iter().filter(|t| [a, b].contains(&t.id)).nth(1).unwrap().id;
    let widen = |h: &mut Harness<'_, PcbApp>| {
        let revision = h.state().session.revision();
        h.state_mut().run(pcbcraft_engine::SET_TRACK_WIDTH, &json!({"tracks":[second],"width_nm":500_000,"expected_revision":revision}));
        h.run();
        assert_eq!(widths(h), [250_000, 500_000]);
    };
    widen(&mut h);
    h.state_mut().views.active.board.tracks.selected = std::collections::BTreeSet::from([a, b]);
    h.run();
    let shown = h.get_by_label("Selected track width").value().unwrap();
    let revision = h.state().session.revision();
    h.get_by_label("Selected track width").click();
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!((widths(&h), h.state().session.revision()), (vec![250_000, 500_000], revision));
    enter(&mut h, "Selected track width", "0.250 mm");
    assert_eq!((widths(&h), h.state().session.revision()), (vec![250_000, 250_000], revision + 1));

    // Retyping exactly the shown text also counts as an entry while the widths differ.
    widen(&mut h);
    enter(&mut h, "Selected track width", &shown);
    assert_eq!(widths(&h), [250_000, 250_000]);
}

/// Split views of one board share a Properties panel host but not drafts: text typed while one
/// view is active is neither submitted from nor shown in the other, and is gone on return.
#[test]
fn split_views_keep_length_drafts_to_the_view_that_typed_them() {
    let mut h = routable();
    h.state_mut().preferences.units = Unit::Mm;
    let id = first_footprint(&h);
    click(&mut h, "Choose workspace");
    click(&mut h, "Open PCB layout beside current view");
    let views = h.state().views.hosts[&h.state().views.main].layout.visible_views();
    assert_eq!(views.len(), 2);
    for view in &views {
        h.state_mut().views.view_mut(*view).unwrap().board.selection.insert(id.clone());
    }
    let ctx = h.ctx.clone();
    h.state_mut().activate_editor(&ctx, views[0]);
    h.run();
    let revision = h.state().session.revision();
    type_into(&mut h, "Footprint X", "100 mil");
    h.state_mut().activate_editor(&ctx, views[1]);
    h.run();
    assert_eq!(h.get_by_label("Footprint X").value().as_deref(), Some("0 mm"));
    h.key_press(Key::Enter);
    h.run();
    assert_eq!((position(&h, &id).x_nm, h.state().session.revision()), (0, revision));
    h.state_mut().activate_editor(&ctx, views[0]);
    h.run();
    assert_eq!(h.get_by_label("Footprint X").value().as_deref(), Some("0 mm"));
    enter(&mut h, "Footprint X", "100 mil");
    assert_eq!((position(&h, &id).x_nm, h.state().session.revision()), (2_540_000, revision + 1));
    h.state_mut().activate_editor(&ctx, views[1]);
    h.run();
    assert_eq!(h.get_by_label("Footprint X").value().as_deref(), Some("2.54 mm"));
}

/// In a compact Properties panel, typed track width and via pad/drill edits produce exactly the
/// documents of the equivalent canonical integer-nanometre commands.
#[test]
fn compact_copper_entry_matches_the_canonical_nanometre_commands() {
    let mut h = routable();
    h.state_mut().preferences.units = Unit::Mm;
    let track = add_track(&mut h, Point { x_nm: 2_540_000, y_nm: 2_540_000 }, Point { x_nm: 7_620_000, y_nm: 2_540_000 });
    let revision = h.state().session.revision();
    h.state_mut().run(
        pcbcraft_engine::ADD_VIA,
        &json!({"at":{"x_nm":0,"y_nm":-5_080_000},"via_diameter_nm":600_000,"via_drill_nm":300_000,"expected_revision":revision}),
    );
    let via = vias(&h)[0].id;
    let board = h.state().session.project().unwrap().boards().next().unwrap().target.board;
    h.set_size(egui::vec2(420., 900.));
    let encode = |h: &Harness<'_, PcbApp>| encode_documents(h.state().session.snapshot().unwrap()).unwrap();
    let cases = [
        (track, "Selected track width", "20 mil", pcbcraft_engine::SET_TRACK_WIDTH, json!({"tracks":[track],"width_nm":508_000})),
        (via, "Selected via pad", "32 mil", pcbcraft_engine::SET_VIA_SIZE, json!({"vias":[via],"via_diameter_nm":812_800,"via_drill_nm":300_000})),
        (via, "Selected via drill", "0.25 mm", pcbcraft_engine::SET_VIA_SIZE, json!({"vias":[via],"via_diameter_nm":600_000,"via_drill_nm":250_000})),
    ];
    for (item, label, text, command, mut params) in cases {
        h.state_mut().views.active.board.tracks.selected = std::collections::BTreeSet::from([item]);
        h.run();
        assert!(h.get_by_label(label).rect().max.x <= 420., "{label} fits the compact window");
        let revision = h.state().session.revision();
        enter(&mut h, label, text);
        assert_eq!((h.state().message.as_deref(), h.state().session.revision()), (None, revision + 1), "{label}");
        let typed = encode(&h);
        h.state_mut().run("edit.undo", &json!({}));
        h.run();
        params["board"] = json!(board);
        params["expected_revision"] = json!(h.state().session.revision());
        h.state_mut().session.apply(command, &params).unwrap();
        h.run();
        assert_eq!(encode(&h), typed, "{label}");
        h.state_mut().run("edit.undo", &json!({}));
        h.run();
    }
}

/// Exact nanometre values typed in either unit survive display-unit switches and an actual
/// folder or archive save and reopen through the file path, without re-entering any value.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn exact_lengths_survive_unit_switches_and_a_file_save_and_reopen() {
    use crate::project_files::SaveFormat;
    for (format, name) in [(SaveFormat::Folder, "exact"), (SaveFormat::Archive, "exact.pc")] {
        let mut h = routable();
        h.state_mut().preferences.units = Unit::Mil;
        let id = first_footprint(&h);
        let track = add_track(&mut h, Point { x_nm: 2_540_000, y_nm: 2_540_000 }, Point { x_nm: 7_620_000, y_nm: 2_540_000 });
        h.state_mut().views.active.board.selection.insert(id.clone());
        h.run();
        enter(&mut h, "Footprint X", "1.234567 mm");
        enter(&mut h, "Footprint Y", "-0.000007 mm");
        h.state_mut().views.active.board.selection.clear();
        h.state_mut().views.active.board.tracks.selected = std::collections::BTreeSet::from([track]);
        h.run();
        enter(&mut h, "Selected track width", "0.123457 mm");
        let exact = (Point { x_nm: 1_234_567, y_nm: -7 }, 123_457);
        let read = |h: &Harness<'_, PcbApp>| (position(h, &id), tracks(h).into_iter().find(|t| t.id == track).unwrap().width_nm);
        assert_eq!(read(&h), exact);
        for unit in [Unit::Mm, Unit::Mil] {
            h.state_mut().preferences.units = unit;
            h.run();
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        h.state_mut().files.use_stand_in();
        let ctx = h.ctx.clone();
        h.state_mut().save_project_file(Some(format), &ctx);
        h.state_mut().files.finish_stand_in(path.clone());
        super::recent_projects::finish_file(&mut h);
        assert!(!h.state().files.dirty(&h.state().session), "{format:?}");
        // A first save names the project after its file; everything else is what was typed.
        let before = encode_documents(h.state().session.snapshot().unwrap()).unwrap();
        assert_eq!(read(&h), exact, "{format:?}");
        let revision = h.state().session.revision();
        h.state_mut().files.open_path_for_test(&path, revision);
        h.run();
        assert_eq!(h.state().message, None);
        assert_eq!(read(&h), exact, "{format:?}");
        assert_eq!(encode_documents(h.state().session.snapshot().unwrap()).unwrap(), before, "{format:?}");
    }
}
