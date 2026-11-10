//! Bottom-side placement through the shared transform command.
use super::{
    drc::{Kind, Rules, tests::outline},
    setup::Side,
    track::tests::{copper_layers, routed_fixture, view},
    view::CopperLayers,
};
use crate::{
    SET_FOOTPRINT_TRANSFORM, Session,
    persistence::{decode_documents, encode_documents},
    schematic::Point,
};
use serde_json::json;

/// The first footprint's ID, position and rotation.
fn first(s: &Session) -> (String, Point, u16) {
    let f = view(s).footprints().next().unwrap();
    (f.id.clone(), f.position, f.rotation)
}

fn flip(s: &mut Session, id: &str, at: Point, rotation: u16, side: &str) -> Result<u64, crate::CommandError> {
    s.apply(
        SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":id,"x_nm":at.x_nm,"y_nm":at.y_nm,"rotation":rotation,"side":side,"expected_revision":s.revision()}),
    )
}

/// Flipping mirrors pad offsets about the footprint origin, moves component-side lands to back
/// copper and graphics to the back side, saves as board format 14, and flipping back restores
/// the exact earlier bytes. Undo and redo cover the flip as one edit.
#[test]
fn flipping_mirrors_geometry_onto_back_layers_and_round_trips() {
    let mut s = routed_fixture();
    let before = encode_documents(s.snapshot().unwrap()).unwrap();
    let (id, at, rotation) = first(&s);
    let front_pads: Vec<_> = view(&s).footprints().next().unwrap().pads.iter().map(|p| (p.number.clone(), p.center)).collect();
    let [front, back] = copper_layers(&s)[..] else { panic!("two copper layers") };
    let revision = s.revision();
    flip(&mut s, &id, at, rotation, "back").unwrap();
    assert_eq!(s.revision(), revision + 1);
    let flipped = view(&s).footprints().next().unwrap();
    assert!(flipped.bottom);
    for (number, center) in &front_pads {
        let pad = flipped.pads.iter().find(|p| &p.number == number).unwrap();
        assert_eq!(pad.center, Point { x_nm: 2 * at.x_nm - center.x_nm, y_nm: center.y_nm }, "pad {number} mirrors about the origin");
        assert!(matches!(&pad.copper_layers, Some(CopperLayers::Resolved { layers, .. }) if layers == &vec![back]), "{:?}", pad.copper_layers);
        assert!(!matches!(&pad.copper_layers, Some(CopperLayers::Resolved { layers, .. }) if layers.contains(&front)));
    }
    assert!(flipped.graphics.iter().all(|graphic| graphic.side == Side::Back));
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let text = String::from_utf8(saved["board.json"].clone()).unwrap();
    assert!(text.contains("\"format\": \"pcbcraft-board/14\"") && text.contains("\"bottom\":true"), "{text}");
    assert_eq!(encode_documents(decode_documents(&saved).unwrap()).unwrap(), saved);
    assert_eq!(s.inspect()["project"]["board"]["footprints"][0]["side"], "back");

    let revision = s.revision();
    flip(&mut s, &id, at, rotation, "back").unwrap();
    assert_eq!(s.revision(), revision, "the same side is not an edit");
    assert!(flip(&mut s, &id, at, rotation, "middle").is_err());
    flip(&mut s, &id, at, rotation, "front").unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), before, "flipping back restores the earlier format and bytes");
    s.apply("edit.undo", &json!({})).unwrap();
    assert_eq!(encode_documents(s.snapshot().unwrap()).unwrap(), saved);
}

/// Board format 14 requires a bottom-side footprint, and earlier formats reject one.
#[test]
fn board_format_14_and_bottom_footprints_require_each_other() {
    let mut s = routed_fixture();
    let (id, at, rotation) = first(&s);
    flip(&mut s, &id, at, rotation, "back").unwrap();
    let saved = encode_documents(s.snapshot().unwrap()).unwrap();
    let text = String::from_utf8(saved["board.json"].clone()).unwrap();
    let mut older = saved.clone();
    older.insert("board.json".into(), text.replace("pcbcraft-board/14", "pcbcraft-board/11").into_bytes());
    assert!(decode_documents(&older).is_err());
    let mut unflipped = saved;
    unflipped.insert("board.json".into(), text.replace(",\"bottom\":true", "").into_bytes());
    assert!(decode_documents(&unflipped).is_err());
}

/// A bottom part's pads appear in the bottom copper, mask and paste and its silkscreen on the
/// bottom legend; courtyards on opposite sides may overlap without a DRC finding.
#[test]
fn outputs_and_drc_follow_the_side() {
    let mut s = routed_fixture();
    outline(&mut s);
    let footprints: Vec<_> = view(&s).footprints().map(|f| (f.id.clone(), f.position, f.rotation, f.pads.len())).collect();
    let (id, at, rotation, pads) = footprints[0].clone();
    flip(&mut s, &id, at, rotation, "back").unwrap();
    let outputs = view(&s).fabrication_outputs().unwrap();
    let file = |suffix: &str| outputs.iter().find(|o| o.name.ends_with(suffix)).unwrap().contents.clone();
    let total: usize = footprints.iter().map(|f| f.3).sum();
    assert_eq!(file("-Bottom.gbr").matches("G36*").count(), pads);
    assert_eq!(file("-Top.gbr").matches("G36*").count(), total - pads);
    assert_eq!(file("-Paste_Bottom.gbr").matches("G36*").count(), pads);
    assert_eq!(file("-Mask_Bottom.gbr").matches("G36*").count(), pads);
    assert!(file("-Silkscreen_Bottom.gbr").contains("%TF.FileFunction,Legend,Bot*%"));
    assert!(file("-Silkscreen_Bottom.gbr").contains("D01*"), "the bottom part's silkscreen is drawn");

    let onto = footprints[1].1;
    s.apply(
        SET_FOOTPRINT_TRANSFORM,
        &json!({"footprint":id,"x_nm":onto.x_nm + 300_000,"y_nm":onto.y_nm,"rotation":rotation,"expected_revision":s.revision()}),
    )
    .unwrap();
    let report = view(&s).check_design(Rules::default());
    assert_eq!(report.counts.get(&Kind::CourtyardOverlap).copied().unwrap_or(0), 0, "opposite sides may overlap");
}
