//! Practical library construction and failed-batch regressions.
use super::*;
use crate::{
    library_graphics::{Fill, Geometry, Graphic, Transform},
    schematic::{PinOrientation, PinStyle},
};
fn p(x_nm: i64, y_nm: i64) -> Point {
    Point { x_nm, y_nm }
}
fn apply(s: &mut Session, action: Action) {
    s.execute(s.revision(), action).unwrap();
}
/// An eight-pin symbol can be arrayed, mirrored and saved with editable pin appearance intact.
#[test]
fn symbol_arrays_keep_unique_terminals_and_integrated_leads() {
    let mut s = Session::new(Item::new(Kind::Symbol).unwrap(), false).unwrap();
    apply(&mut s, Action::Delete { objects: vec![3] });
    let Primitive::Pin { mut appearance, number, name, electrical_type, at } = s.snapshot().objects[&1].clone() else { panic!() };
    appearance.as_mut().unwrap().style = PinStyle::InvertedClock;
    apply(&mut s, Action::Replace { object: 1, primitive: Primitive::Pin { appearance, number, name, electrical_type, at } });
    apply(&mut s, Action::Array { objects: vec![1], columns: 1, rows: 4, column_step: p(0, 0), row_step: p(0, 2_540_000), numbering: None });
    let ids: Vec<_> = s.snapshot().objects.iter().filter(|(_, v)| matches!(v, Primitive::Pin { .. })).map(|(id, _)| *id).collect();
    apply(&mut s, Action::Duplicate { objects: ids.clone(), offset: p(10_160_000, 0) });
    let right: Vec<_> = s.snapshot().objects.keys().filter(|id| !ids.contains(id) && **id != 2).copied().collect();
    apply(
        &mut s,
        Action::Transform {
            objects: right,
            transform: Transform { pivot: p(5_080_000, 0), offset: p(0, 0), quarter_turns: 0, mirror_x: true, mirror_y: false },
        },
    );
    let item = Item::decode(&s.snapshot().encode().unwrap()).unwrap();
    let definition = library::parse_symbol(&item.export_native().unwrap().to_string()).unwrap();
    assert_eq!(definition.pins().len(), 8);
    let numbers: std::collections::BTreeSet<_> = definition.pins().iter().map(|p| p.number.as_str()).collect();
    assert_eq!(numbers.len(), 8);
    for pin in definition.pins() {
        let a = pin.appearance.as_ref().unwrap();
        assert_eq!(a.style, PinStyle::InvertedClock);
        assert_eq!(a.orientation, if pin.offset.x_nm < 0 { PinOrientation::Right } else { PinOrientation::Left });
        assert!(a.graphics(pin.offset, &pin.number, pin.name.as_deref()).len() >= 4);
    }
    assert_eq!(item.encode().unwrap(), Item::decode(&item.encode().unwrap()).unwrap().encode().unwrap());
}
/// Alphanumeric oval plated pads and layered editable curves survive native and editor round trips.
#[test]
fn footprint_arrays_and_layered_artwork_roundtrip() {
    let mut s = Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    apply(&mut s, Action::Delete { objects: vec![2, 3] });
    let Primitive::Pad { mut pad } = s.snapshot().objects[&1].clone() else { panic!() };
    pad.number = "A1".into();
    pad.shape = footprint::PadShape::Oval;
    pad.corner_radius_nm = 0;
    pad.width_nm = 1_600_000;
    pad.height_nm = 2_000_000;
    pad.copper = footprint::CopperBinding::AllCopper;
    pad.drill_nm = Some(800_000);
    apply(&mut s, Action::Replace { object: 1, primitive: Primitive::Pad { pad } });
    apply(&mut s, Action::Array { objects: vec![1], columns: 2, rows: 4, column_step: p(7_620_000, 0), row_step: p(0, 2_540_000), numbering: None });
    let geometries = [
        Geometry::Rectangle { min: p(-2_000_000, -2_000_000), max: p(9_000_000, 10_000_000) },
        Geometry::Arc { center: p(3_000_000, 0), radius_nm: 1_000_000, start_degrees: 0, sweep_degrees: 180 },
        Geometry::Text { at: p(3_000_000, 10_000_000), text: "DIP8".into(), height_nm: 1_000_000, rotation: 0, align: TextAlign::Middle },
    ];
    for (purpose, geometry) in [footprint::GraphicPurpose::Courtyard, footprint::GraphicPurpose::Silkscreen, footprint::GraphicPurpose::Fabrication]
        .into_iter()
        .zip(geometries)
    {
        apply(
            &mut s,
            Action::Insert {
                primitive: Primitive::Artwork {
                    artwork: footprint::Artwork { purpose, graphic: Graphic { geometry, stroke_nm: 150_000, fill: Fill::None } },
                },
            },
        );
    }
    let item = Item::decode(&s.snapshot().encode().unwrap()).unwrap();
    let definition: footprint::FootprintDefinition = serde_json::from_value(item.export_native().unwrap()).unwrap();
    definition.validate().unwrap();
    assert_eq!(definition.pads.len(), 8);
    assert_eq!(definition.artwork.len(), 3);
    assert_eq!(definition.pads.last().unwrap().number, "A8");
    assert!(definition.pads.iter().all(|p| p.shape == footprint::PadShape::Oval && p.drill_nm == Some(800_000)));
    assert_eq!(item.footprint().unwrap(), definition);
}
/// Rejected arrays (including errors after temporary allocations) consume neither IDs nor history.
#[test]
fn failed_batches_stale_edits_and_empty_drafts_are_safe() {
    let mut s = Session::new(Item::new(Kind::Symbol).unwrap(), false).unwrap();
    let before = s.snapshot();
    for (columns, step) in [(513, 2_540_000), (3, 50_800_000), (2, 0)] {
        assert!(
            s.execute(0, Action::Array { objects: vec![1], columns, rows: 1, column_step: p(step, 0), row_step: p(0, 0), numbering: None }).is_err()
        );
        assert_eq!(s.snapshot(), before);
        assert_eq!(s.revision(), 0);
        assert!(!s.can_undo());
    }
    apply(&mut s, Action::Duplicate { objects: vec![1], offset: p(0, 2_540_000) });
    assert!(s.snapshot().objects.contains_key(&4));
    assert!(s.execute(0, Action::Delete { objects: vec![4] }).is_err());
    apply(&mut s, Action::Undo);
    assert_eq!(s.snapshot().native().unwrap(), before.native().unwrap());
    apply(&mut s, Action::Delete { objects: vec![1, 2, 3] });
    let item = Item::decode(&s.snapshot().encode().unwrap()).unwrap();
    assert!(item.objects.is_empty());
    assert!(item.export_native().is_err());
    for kind in [Kind::Symbol, Kind::Footprint] {
        let mut s = Session::new(Item::new(kind).unwrap(), false).unwrap();
        apply(&mut s, Action::Delete { objects: vec![1, 2, 3] });
        assert!(Item::decode(&s.snapshot().encode().unwrap()).unwrap().objects.is_empty());
        assert!(s.snapshot().export_native().is_err());
    }
}
/// Number swaps are one atomic batch rather than a transient duplicate-terminal state.
#[test]
fn renumber_batch_and_legacy_fork_preserve_sources() {
    let mut s = Session::new(Item::new(Kind::Symbol).unwrap(), false).unwrap();
    let mut replacements = BTreeMap::new();
    for (id, number) in [(1, "2"), (3, "1")] {
        let mut pin = s.snapshot().objects[&id].clone();
        if let Primitive::Pin { number: n, .. } = &mut pin {
            *n = number.into();
        }
        replacements.insert(id, pin);
    }
    apply(&mut s, Action::ReplaceMany { primitives: replacements });
    assert!(matches!(&s.snapshot().objects[&1],Primitive::Pin { number,.. } if number == "2"));
    for source in library::basic_components().symbols() {
        let before = serde_json::to_value(library::saved_copy(source)).unwrap();
        let fork = Item::from_symbol(source).unwrap();
        assert!(
            fork.objects
                .values()
                .filter_map(|p| if let Primitive::Pin { appearance, .. } = p { Some(appearance) } else { None })
                .all(Option::is_some)
        );
        assert_eq!(serde_json::to_value(library::saved_copy(source)).unwrap(), before);
        fork.validate().unwrap();
    }
}

/// Older metadata forms omit category; only an explicit null requests removing the binding.
#[test]
fn metadata_omission_preserves_category_while_null_clears_it() {
    let definition = library::parse_symbol(include_str!("../../../../examples/native-parts/symbols/resistor.json")).unwrap();
    let mut item = Item::from_symbol(&definition).unwrap();
    item.license = "MIT OR Apache-2.0".into();
    let original = item.category.clone();
    assert!(original.is_some());
    let mut editor = Session::new(item, false).unwrap();
    for (settings, expected) in
        [(json!({"behavior":null,"defaults":null}), original), (json!({"behavior":null,"defaults":null,"category":null}), None)]
    {
        let snapshot = editor.snapshot();
        let action: Action = serde_json::from_value(json!({"operation":"metadata","name":snapshot.name,
            "license":snapshot.license,"source":snapshot.source,"reference_prefix":snapshot.reference_prefix,
            "default_value":snapshot.default_value,"symbol_settings":settings}))
        .unwrap();
        editor.execute(editor.revision(), action).unwrap();
        assert_eq!(editor.snapshot().category, expected);
        assert_eq!(Item::decode(&editor.snapshot().encode().unwrap()).unwrap().category, expected);
    }
}

/// Done test: a 26 x 26 BGA (0.8 mm pitch, JEDEC skip set on per JEDEC JEP95 / IPC-SM-782A §14.1.3)
/// generates A1..AF26 in one undoable step, previews via dry run, round-trips through save/reopen,
/// maps every ball to a matching terminal, and lists pads in natural order (A2 before A10, Y26 before AA1).
#[test]
fn bga_26x26_ball_grid_roundtrips_maps_and_sorts_naturally() {
    let mut s = Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    apply(&mut s, Action::Delete { objects: vec![2, 3] });
    let Primitive::Pad { mut pad } = s.snapshot().objects[&1].clone() else { panic!("seed pad expected") };
    pad.number = "1".into();
    pad.center = p(-10_000_000, -10_000_000);
    pad.width_nm = 400_000;
    pad.height_nm = 400_000;
    pad.shape = footprint::PadShape::Circle;
    pad.corner_radius_nm = 0;
    pad.copper = footprint::CopperBinding::ComponentSide;
    pad.drill_nm = None;
    apply(&mut s, Action::Replace { object: 1, primitive: Primitive::Pad { pad } });

    let before = s.snapshot();
    let before_rev = s.revision();
    let grid_action = Action::Array {
        objects: vec![1],
        columns: 26,
        rows: 26,
        column_step: p(800_000, 0),
        row_step: p(0, 800_000),
        numbering: Some(ArrayNumbering::BallGrid { skip_jedec: true }),
    };

    // Dry run returns a readable change record without mutating state or revision.
    let preview = s.preview(before_rev, grid_action.clone()).unwrap();
    assert!(preview.dry_run);
    assert!(preview.changed);
    assert_eq!(preview.before_revision, before_rev);
    assert_eq!(preview.after_revision, before_rev + 1);
    assert_eq!(preview.added_pads.len(), 675);
    assert_eq!(preview.renamed_pads, vec!["1 -> A1"]);
    assert_eq!(preview.added_pads.first().map(String::as_str), Some("A2"));
    assert_eq!(preview.added_pads.last().map(String::as_str), Some("AF26"));
    assert!(preview.summary.contains("26x26"));
    assert_eq!(s.snapshot(), before);
    assert_eq!(s.revision(), before_rev);

    // Applying commits in one undo step with the same readable record.
    let applied = s.execute_change(before_rev, grid_action).unwrap();
    assert!(!applied.dry_run);
    assert!(applied.changed);
    assert_eq!(applied.added_pads, preview.added_pads);
    assert_eq!(applied.renamed_pads, preview.renamed_pads);
    assert_eq!(s.snapshot().objects.len(), 676);

    // One undo step restores the exact pre-array document, and redo restores all 676 pads.
    apply(&mut s, Action::Undo);
    assert_eq!(s.snapshot().native().unwrap(), before.native().unwrap());
    apply(&mut s, Action::Redo);
    assert_eq!(s.snapshot().objects.len(), 676);

    // Save/reopen round-trip and native footprint export.
    let bytes = s.snapshot().encode().unwrap();
    let reopened_item = Item::decode(&bytes).unwrap();
    assert_eq!(reopened_item.encode().unwrap(), bytes);
    let mut native_fp = reopened_item.export_native().unwrap();
    native_fp["library"] = json!("bga-grid");
    native_fp["id"] = json!("bga-676");
    let definition: footprint::FootprintDefinition = serde_json::from_value(native_fp.clone()).unwrap();
    definition.validate().unwrap();
    assert_eq!(definition.pads.len(), 676);
    assert_eq!(definition.pads.first().map(|pad| pad.number.as_str()), Some("A1"));
    assert_eq!(definition.pads.last().map(|pad| pad.number.as_str()), Some("AF26"));
    for pad in &definition.pads {
        for forbidden in JEDEC_SKIP_LETTERS {
            assert!(!pad.number.contains(forbidden), "pad `{}` used skipped letter `{forbidden}`", pad.number);
        }
    }

    // Map every ball A1..AF26 to a matching terminal in a native part and verify catalog natural sort.
    let names: Vec<String> = definition.pads.iter().map(|pad| pad.number.clone()).collect();
    let units: Vec<_> = names
        .chunks(128)
        .enumerate()
        .map(|(unit, chunk)| {
            let pins: Vec<_> = chunk
                .iter()
                .enumerate()
                .map(|(index, name)| json!({"terminal":name,"at_nm":[(index as i64 % 16 - 8) * 1_270_000,(index as i64 / 16 - 4) * 1_270_000]}))
                .collect();
            json!({"id":format!("u{}", unit + 1),"drawing":[{"type":"rect","min_nm":[-11_430_000,-6_350_000],"max_nm":[10_160_000,5_080_000]}],"pins":pins})
        })
        .collect();
    let terminals: Vec<_> = names.iter().map(|name| json!({"id":name,"electrical_type":"passive"})).collect();
    let symbol = json!({"format":"pcbcraft-symbol/4","id":"bga-676","revision":1,"name":"BGA 26x26","reference_prefix":"U",
        "default_value":"BGA676","terminals":terminals,"units":units});
    let mapping: serde_json::Map<_, _> = names.iter().map(|name| (name.clone(), json!([name]))).collect();
    let part = json!({"format":"pcbcraft-part/1","library":"bga-grid","id":"bga-676","revision":1,"name":"BGA 26x26",
        "license":"MIT OR Apache-2.0","source":{"kind":"original","reference":"Synthetic BGA fixture","locator":"bga-676"},
        "symbol":{"library":"bga-grid","id":"bga-676","revision":1},
        "packages":{"bga":{"name":"BGA-676","footprint":{"library":"bga-grid","id":"bga-676","revision":1},"mapping":mapping}}});
    let documents: crate::library::sources::Documents = [
        ("library.json", r#"{"format":"pcbcraft-library/1","id":"bga-grid","name":"BGA grid","license":"MIT OR Apache-2.0"}"#.to_owned()),
        ("symbols/bga-676.json", serde_json::to_string_pretty(&symbol).unwrap()),
        ("footprints/bga-676.json", serde_json::to_string_pretty(&native_fp).unwrap()),
        ("parts/bga-676.json", part.to_string()),
    ]
    .into_iter()
    .map(|(path, text)| (path.into(), text))
    .collect();
    let mut project_session = crate::Session::default();
    project_session.apply("project.new", &json!({"create_board":true})).unwrap();
    let loaded = project_session.execute(crate::library::sources::LOAD, &json!({"source":"bga-grid","documents":documents})).unwrap();
    let problems = &loaded.as_array().unwrap().iter().find(|src| src["source"] == "bga-grid").unwrap()["problems"];
    assert_eq!(problems, &json!([]));
    let resolved = project_session
        .execute(crate::library::sources::RESOLVE_PART, &json!({"library":"bga-grid","definition":"bga-676","definition_revision":1}))
        .unwrap();
    project_session
        .apply(
            "schematic.place_part",
            &json!({"library":"bga-grid","definition":"bga-676","definition_revision":1,"definition_digest":resolved["digest"],"x_nm":0,"y_nm":0,"expected_revision":project_session.revision()}),
        )
        .unwrap();
    let catalog = project_session.inspect()["footprint_catalog"].clone();
    let fp_entry = catalog.as_array().unwrap().iter().find(|entry| entry["library"] == "bga-grid" && entry["id"] == "bga-676").unwrap();
    let listed_pads: Vec<&str> = fp_entry["pads"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert_eq!(listed_pads.len(), 676);
    let pos = |label: &str| listed_pads.iter().position(|p| *p == label).unwrap();
    assert!(pos("A2") < pos("A10"), "natural sort must place A2 before A10");
    assert!(pos("Y26") < pos("AA1"), "natural sort must place Y26 before AA1");
    assert_eq!(listed_pads.first().copied(), Some("A1"));
    assert_eq!(listed_pads.last().copied(), Some("AF26"));

    // Unchanged project saves byte-identically across reopen.
    let saved = crate::persistence::encode_documents(project_session.snapshot().unwrap()).unwrap();
    let reopened_project = crate::persistence::decode_documents(&saved).unwrap();
    assert_eq!(crate::persistence::encode_documents(reopened_project).unwrap(), saved);
}

/// Disabling JEDEC skipping retains all 26 letters A..Z before wrapping to AA, AB...
#[test]
fn ball_grid_array_wraps_past_z_when_jedec_skip_is_disabled() {
    let mut s = Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    apply(&mut s, Action::Delete { objects: vec![2, 3] });
    apply(
        &mut s,
        Action::Array {
            objects: vec![1],
            columns: 2,
            rows: 28,
            column_step: p(1_000_000, 0),
            row_step: p(0, 1_000_000),
            numbering: Some(ArrayNumbering::BallGrid { skip_jedec: false }),
        },
    );
    let def = s.snapshot().footprint().unwrap();
    let numbers: Vec<&str> = def.pads.iter().map(|pad| pad.number.as_str()).collect();
    assert_eq!(numbers.len(), 56);
    assert!(numbers.contains(&"I1"));
    assert!(numbers.contains(&"O1"));
    assert!(numbers.contains(&"Q1"));
    assert!(numbers.contains(&"S1"));
    assert!(numbers.contains(&"X1"));
    assert!(numbers.contains(&"Z2"));
    assert!(numbers.contains(&"AA1"));
    assert!(numbers.contains(&"AB2"));
}

/// Invalid ball-grid requests (zero dimensions, oversized grids, zero steps, non-pad or multi-object
/// selections, symbol documents and pad-name collisions) fail atomically with zero state change.
#[test]
fn ball_grid_array_refuses_bad_grids_atomically() {
    let mut s = Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    let before = s.snapshot();
    let bad_actions = [
        // Zero rows or columns
        (
            Action::Array {
                objects: vec![1],
                columns: 0,
                rows: 4,
                column_step: p(800_000, 0),
                row_step: p(0, 800_000),
                numbering: Some(ArrayNumbering::BallGrid { skip_jedec: true }),
            },
            "at least two cells",
        ),
        (
            Action::Array {
                objects: vec![1],
                columns: 4,
                rows: 0,
                column_step: p(800_000, 0),
                row_step: p(0, 800_000),
                numbering: Some(ArrayNumbering::BallGrid { skip_jedec: true }),
            },
            "at least two cells",
        ),
        // Exceeds 1,024 pad limit (33 * 32 = 1056)
        (
            Action::Array {
                objects: vec![1],
                columns: 32,
                rows: 33,
                column_step: p(800_000, 0),
                row_step: p(0, 800_000),
                numbering: Some(ArrayNumbering::BallGrid { skip_jedec: true }),
            },
            "pad limit",
        ),
        // Zero step along multi-cell axis
        (
            Action::Array {
                objects: vec![1],
                columns: 2,
                rows: 2,
                column_step: p(0, 0),
                row_step: p(0, 800_000),
                numbering: Some(ArrayNumbering::BallGrid { skip_jedec: true }),
            },
            "nonzero step",
        ),
        // Multi-object selection
        (
            Action::Array {
                objects: vec![1, 2],
                columns: 2,
                rows: 2,
                column_step: p(800_000, 0),
                row_step: p(0, 800_000),
                numbering: Some(ArrayNumbering::BallGrid { skip_jedec: true }),
            },
            "exactly one selected pad",
        ),
        // Non-pad selection (object 3 is an outline)
        (
            Action::Array {
                objects: vec![3],
                columns: 2,
                rows: 2,
                column_step: p(800_000, 0),
                row_step: p(0, 800_000),
                numbering: Some(ArrayNumbering::BallGrid { skip_jedec: true }),
            },
            "selected object to be a pad",
        ),
    ];
    for (action, expected) in bad_actions {
        let preview_err = s.preview(0, action.clone()).unwrap_err();
        assert!(preview_err.contains(expected), "expected `{expected}` in `{preview_err}`");
        let execute_err = s.execute(0, action).unwrap_err();
        assert!(execute_err.contains(expected), "expected `{expected}` in `{execute_err}`");
        assert_eq!(s.snapshot(), before);
        assert_eq!(s.revision(), 0);
        assert!(!s.can_undo());
    }
    // Symbol document refuses ball-grid numbering
    let mut sym = Session::new(Item::new(Kind::Symbol).unwrap(), false).unwrap();
    let sym_before = sym.snapshot();
    let sym_err = sym
        .execute(
            0,
            Action::Array {
                objects: vec![1],
                columns: 2,
                rows: 2,
                column_step: p(2_540_000, 0),
                row_step: p(0, 2_540_000),
                numbering: Some(ArrayNumbering::BallGrid { skip_jedec: true }),
            },
        )
        .unwrap_err();
    assert!(sym_err.contains("footprint documents"), "unexpected symbol error: {sym_err}");
    assert_eq!(sym.snapshot(), sym_before);
    assert_eq!(sym.revision(), 0);

    // Collision with an existing pad named A2 is refused atomically
    let Primitive::Pad { mut pad } = s.snapshot().objects[&2].clone() else { panic!("pad 2 expected") };
    pad.number = "A2".into();
    apply(&mut s, Action::Replace { object: 2, primitive: Primitive::Pad { pad } });
    let before_collision = s.snapshot();
    let rev_before = s.revision();
    let collision_err = s
        .execute(
            rev_before,
            Action::Array {
                objects: vec![1],
                columns: 2,
                rows: 2,
                column_step: p(800_000, 0),
                row_step: p(0, 800_000),
                numbering: Some(ArrayNumbering::BallGrid { skip_jedec: true }),
            },
        )
        .unwrap_err();
    assert!(collision_err.contains("already exists"), "unexpected collision error: {collision_err}");
    assert_eq!(s.snapshot(), before_collision);
    assert_eq!(s.revision(), rev_before);
}
