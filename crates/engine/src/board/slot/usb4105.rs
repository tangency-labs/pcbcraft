//! Shell slots and locating holes of the GCT USB4105 USB Type-C receptacle, taken from the
//! "Recommended PCB Layout" (viewed from the component side, tolerance 0.05 mm) of GCT's public
//! product drawing USB4105, revision B4 dated 18/12/23: <https://gct.co/files/drawings/usb4105.pdf>.
//! Origin: the connector centre line on the PCB edge, with positive Y down, so the connector
//! body lies at negative Y. Published dimensions used:
//! - shell slot centres 8.64 apart; front pair 2.60 from the PCB edge, rear pair 4.18 behind it;
//! - all four finished slots 0.60 wide; 1.40 long at the front and 1.70 at the rear;
//! - all four lands 1.00 wide; 1.80 long at the front and 2.10 at the rear;
//! - two locating holes of diameter 0.65, 5.78 apart and 3.68 behind the front slot centres;
//! - the A1/B12 land 0.60 by 1.15 at X -3.20 (6.40 between the outer lands), whose front edge
//!   is on the rear slot centre line.
//!
//! The remaining eleven signal lands are omitted because the test exercises slots, not USB pins.
use super::{fixture::place, *};
use crate::{
    Session,
    authoring::{Action, Item, Kind, Primitive},
    board::{
        definition::{CircularNpth, CopperBinding, PadShape},
        drc::{Kind as Finding, Rules},
        track::tests::view,
    },
};

const SLOT_PITCH_NM: i64 = 8_640_000;
const FRONT_SLOT_Y_NM: i64 = -2_600_000;
const REAR_SLOT_Y_NM: i64 = FRONT_SLOT_Y_NM - 4_180_000;
const SLOT_WIDTH_NM: i64 = 600_000;
const LAND_WIDTH_NM: i64 = 1_000_000;
const HOLE_PITCH_NM: i64 = 5_780_000;
const HOLE_Y_NM: i64 = FRONT_SLOT_Y_NM - 3_680_000;
const HOLE_DIAMETER_NM: i64 = 650_000;
// Each published land is 0.40 longer and wider than its slot, leaving 0.20 copper all round.
const PUBLISHED_RING_NM: i64 = 200_000;

/// Number, centre, finished slot length and land length of each shell land, front pair last.
const SHELLS: [(&str, i64, i64, i64, i64); 4] = [
    ("SH1", -SLOT_PITCH_NM / 2, REAR_SLOT_Y_NM, 1_700_000, 2_100_000),
    ("SH2", SLOT_PITCH_NM / 2, REAR_SLOT_Y_NM, 1_700_000, 2_100_000),
    ("SH3", -SLOT_PITCH_NM / 2, FRONT_SLOT_Y_NM, 1_400_000, 1_800_000),
    ("SH4", SLOT_PITCH_NM / 2, FRONT_SLOT_Y_NM, 1_400_000, 1_800_000),
];

/// Author the published lands through the validated editor. Each slot runs along Y, at 90
/// degrees to its local X axis, inside an unrotated oval land.
fn usb4105() -> std::sync::Arc<Item> {
    let mut editor = crate::authoring::Session::new(Item::new(Kind::Footprint).unwrap(), false).unwrap();
    editor.execute(editor.revision(), Action::Delete { objects: vec![1, 3] }).unwrap();
    let Primitive::Pad { pad: template } = editor.snapshot().objects[&2].clone() else { panic!("expected a pad") };
    let mut signal = template.clone();
    signal.center = Point { x_nm: -3_200_000, y_nm: REAR_SLOT_Y_NM - 575_000 };
    (signal.width_nm, signal.height_nm, signal.shape, signal.corner_radius_nm) = (600_000, 1_150_000, PadShape::Rectangle, 0);
    editor.execute(editor.revision(), Action::Replace { object: 2, primitive: Primitive::Pad { pad: signal } }).unwrap();
    for (number, x_nm, y_nm, slot_length_nm, land_length_nm) in SHELLS {
        let mut pad = template.clone();
        pad.number = number.into();
        pad.center = Point { x_nm, y_nm };
        (pad.width_nm, pad.height_nm, pad.rotation) = (LAND_WIDTH_NM, land_length_nm, 0);
        (pad.shape, pad.corner_radius_nm, pad.copper) = (PadShape::Oval, 0, CopperBinding::AllCopper);
        pad.plated_slot = Some(PlatedSlot::new(SLOT_WIDTH_NM, slot_length_nm, 90_000).unwrap());
        editor.execute(editor.revision(), Action::Insert { primitive: Primitive::Pad { pad } }).unwrap();
    }
    for x_nm in [-HOLE_PITCH_NM / 2, HOLE_PITCH_NM / 2] {
        let hole = CircularNpth { center: Point { x_nm, y_nm: HOLE_Y_NM }, diameter_nm: HOLE_DIAMETER_NM };
        editor.execute(editor.revision(), Action::Insert { primitive: Primitive::NonPlatedHole { hole } }).unwrap();
    }
    editor.snapshot()
}

/// The placed connector keeps every published slot end, width and land, both locating holes,
/// and a 0.20 mm ring that the annular-ring rule measures exactly.
#[test]
fn usb4105_published_shell_slots_place_exactly() {
    let item = usb4105();
    let definition = item.footprint().unwrap();
    assert_eq!(definition.format, "pcbcraft-footprint/5");
    let session: Session = place(&item, "GCT USB4105 shell slots");
    let footprint = view(&session).footprints().next().unwrap();
    assert!(!footprint.bottom);
    assert_eq!(footprint.rotation, 0);
    let at = |x_nm: i64, y_nm: i64| Point { x_nm: footprint.position.x_nm + x_nm, y_nm: footprint.position.y_nm + y_nm };
    for (number, x_nm, y_nm, slot_length_nm, land_length_nm) in SHELLS {
        let authored = definition.pads.iter().find(|pad| pad.number == number).unwrap();
        assert_eq!((authored.width_nm, authored.height_nm, authored.shape), (LAND_WIDTH_NM, land_length_nm, PadShape::Oval));
        let pad = footprint.pads.iter().find(|pad| pad.number == number).unwrap();
        assert_eq!(pad.center, at(x_nm, y_nm));
        assert_eq!(pad.drill_nm, None);
        assert_eq!(pad.net.as_deref(), Some("GND"));
        let slot = pad.plated_slot.unwrap();
        assert_eq!(slot.width_nm(), SLOT_WIDTH_NM);
        let half_spine = (slot_length_nm - SLOT_WIDTH_NM) / 2;
        let mut ends = slot.cap_centres();
        ends.sort_by_key(|end| end.y_nm);
        assert_eq!(ends, [at(x_nm, y_nm - half_spine), at(x_nm, y_nm + half_spine)]);
    }
    let signal = footprint.pads.iter().find(|pad| pad.number == "2").unwrap();
    assert!(signal.plated_slot.is_none() && signal.drill_nm.is_none());
    assert_eq!(signal.center, at(-3_200_000, REAR_SLOT_Y_NM - 575_000));
    let mut holes: Vec<_> = footprint.non_plated_holes.iter().map(|hole| (hole.center, hole.diameter_nm)).collect();
    holes.sort_by_key(|(center, _)| center.x_nm);
    assert_eq!(holes, [(at(-HOLE_PITCH_NM / 2, HOLE_Y_NM), HOLE_DIAMETER_NM), (at(HOLE_PITCH_NM / 2, HOLE_Y_NM), HOLE_DIAMETER_NM)]);
    let at_ring = view(&session).check_design(Rules { min_annular_ring_nm: PUBLISHED_RING_NM, ..Rules::default() });
    assert!(!at_ring.counts.contains_key(&Finding::AnnularRing));
    let above_ring = view(&session).check_design(Rules { min_annular_ring_nm: PUBLISHED_RING_NM + 1, ..Rules::default() });
    assert_eq!(above_ring.counts.get(&Finding::AnnularRing), Some(&4));
}
