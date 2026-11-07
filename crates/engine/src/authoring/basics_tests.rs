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
    apply(&mut s, Action::Array { objects: vec![1], columns: 1, rows: 4, column_step: p(0, 0), row_step: p(0, 2_540_000) });
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
    apply(&mut s, Action::Array { objects: vec![1], columns: 2, rows: 4, column_step: p(7_620_000, 0), row_step: p(0, 2_540_000) });
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
        assert!(s.execute(0, Action::Array { objects: vec![1], columns, rows: 1, column_step: p(step, 0), row_step: p(0, 0) }).is_err());
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
    for source in library::starter().symbols() {
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
