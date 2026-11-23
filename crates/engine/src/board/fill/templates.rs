//! Reusable clearance profiles and independent thermal neighborhoods.
use super::{cuts, *};
use crate::board::clearance::index::Tree;
use std::sync::Mutex;

/// Only small analytic profiles are retained. These limits bound the memo's memory;
/// every other shape still goes through the same calculation.
const CONTOUR_MEMO_LIMIT: usize = 2_048;
const THERMAL_MEMO_LIMIT: usize = 512;
const THERMAL_PADS_PER_TEMPLATE: usize = 8;
const FOREIGN_CUTS_PER_TEMPLATE: usize = 16;

/// Pure geometry shared by native refills of this open project. No cached value can
/// publish, identify a live job, or keep an authored snapshot alive.
#[derive(Default, Debug)]
pub(super) struct Memo(Mutex<Profiles>);

#[derive(Default, Debug)]
struct Profiles {
    contours: BTreeMap<ContourProfile, Arc<Regions>>,
    thermals: BTreeMap<ThermalProfile, Arc<Vec<Regions>>>,
}

impl Memo {
    fn profiles(&self) -> std::sync::MutexGuard<'_, Profiles> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Keep the core order and exact fractional radius. Pose is never part of the key.
    fn contour(&self, key: ContourProfile) -> Result<Arc<Regions>, String> {
        if let Some(profile) = self.profiles().contours.get(&key).cloned() {
            return Ok(profile);
        }
        let local = Arc::new(Regions::rounded_core(&key.core, f64::from_bits(key.radius)).map_err(|e| e.to_string())?);
        // Clear disposable kernel preparation before sharing a template between jobs.
        if let Some(profile) = local.place_profile([0, 0]) {
            let mut entries = self.profiles();
            if entries.contours.len() < CONTOUR_MEMO_LIMIT {
                entries.contours.entry(key).or_insert_with(|| Arc::new(profile));
            }
        }
        Ok(local)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ContourProfile {
    core: [[i64; 2]; 4],
    radius: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ContourPlacement {
    profile: ContourProfile,
    at: [i64; 2],
}

fn position(point: Point) -> [i64; 2] {
    [point.x_nm, point.y_nm]
}

fn relative(p: [i64; 2], origin: [i64; 2]) -> [i64; 2] {
    [p[0] - origin[0], p[1] - origin[1]]
}

impl ContourPlacement {
    fn new(contour: &Contour, margin: i64) -> Self {
        let at = position(contour.core[0]);
        let core = contour.core.map(|point| relative(position(point), at));
        Self { profile: ContourProfile { core, radius: (contour.radius + margin as f64).to_bits() }, at }
    }

    fn relative_to(self, origin: [i64; 2]) -> Self {
        Self { at: relative(self.at, origin), ..self }
    }

    fn instantiate(self, memo: &Memo) -> Option<Regions> {
        memo.contour(self.profile).ok()?.place_profile(self.at)
    }

    fn build(self, contour: &Contour, margin: i64, memo: &Memo) -> Result<Regions, String> {
        if let Some(profile) = self.instantiate(memo) { Ok(profile) } else { region(contour, margin) }
    }
}

/// A foreign keepout stores its template coordinates only when it was generated from
/// a physical contour. Intersecting older pours use the absolute Boolean path.
pub(super) struct Keepout {
    pub(super) region: Regions,
    placement: Option<ContourPlacement>,
}

impl Keepout {
    pub(super) fn physical(contour: &Contour, margin: i64, memo: &Memo) -> Result<Self, String> {
        let placement = ContourPlacement::new(contour, margin);
        let region = placement.build(contour, margin, memo)?;
        Ok(Self { region, placement: Some(placement) })
    }

    pub(super) fn plane(region: Regions) -> Self {
        Self { region, placement: None }
    }
}

/// Four independent spokes. A blocked direction cannot count as reaching material
/// merely because the opposite direction reaches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Spokes {
    at: [i64; 2],
    length: i64,
    width: i64,
}

impl Spokes {
    fn new(object: &Object, width: i64, gap: i64) -> Self {
        let [x0, y0, x1, y1] = object.contour.bounds();
        let length = (x1 - x0).max(y1 - y0).ceil() as i64 + gap + width;
        Self { at: position(object.at), length, width }
    }

    fn relative_to(self, origin: [i64; 2]) -> Self {
        Self { at: relative(self.at, origin), ..self }
    }

    fn regions(self) -> [Result<Regions, String>; 4] {
        let Self { at: [x, y], length, width } = self;
        let lo = width / 2;
        let hi = width - lo;
        [
            [[x - lo, y - lo], [x + length, y - lo], [x + length, y + hi], [x - lo, y + hi]],
            [[x - length, y - lo], [x + hi, y - lo], [x + hi, y + hi], [x - length, y + hi]],
            [[x - lo, y - lo], [x + hi, y - lo], [x + hi, y + length], [x - lo, y + length]],
            [[x - lo, y - length], [x + hi, y - length], [x + hi, y + hi], [x - lo, y + hi]],
        ]
        .map(|points| Regions::polygon(&points).map_err(|e| e.to_string()))
    }

    fn bounds(self) -> [f64; 4] {
        let [x, y] = self.at;
        let r = self.length;
        [(x - r) as f64, (y - r) as f64, (x + r) as f64, (y + r) as f64]
    }
}

fn join(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ThermalPad {
    gap: ContourPlacement,
    spokes: Spokes,
}

pub(super) struct Thermal {
    pad: ThermalPad,
    gap: Regions,
    bounds: [f64; 4],
}

impl Thermal {
    pub(super) fn new(object: &Object, width: i64, gap: i64, memo: &Memo) -> Result<Self, String> {
        let contour = &object.contour;
        let margin = gap + CLEARANCE_RESERVE_NM;
        let placement = ContourPlacement::new(contour, margin);
        let gap_shape = placement.build(contour, margin, memo)?;
        let spokes = Spokes::new(object, width, gap);
        let bounds = gap_shape.bounds_nm().ok_or("Thermal clearance has no material.")?;
        Ok(Self { pad: ThermalPad { gap: placement, spokes }, gap: gap_shape, bounds: join(bounds, spokes.bounds()) })
    }
}

/// A neighborhood key contains every gap/spoke and every potentially interfering
/// foreign contour in the same local frame. An edit invalidates only the keys for
/// neighborhoods whose relative geometry changes.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ThermalProfile {
    pads: Vec<ThermalPad>,
    foreign: Vec<ContourPlacement>,
}

fn local_profile(key: &ThermalProfile, memo: &Memo, canceled: Option<&AtomicBool>) -> Option<Arc<Vec<Regions>>> {
    if let Some(result) = memo.profiles().thermals.get(key).cloned() {
        return Some(result);
    }
    let gaps = key.pads.iter().map(|pad| pad.gap.instantiate(memo)).collect::<Option<Vec<_>>>()?;
    let foreign = Cuts::new(key.foreign.iter().map(|cut| cut.instantiate(memo)).collect::<Option<Vec<_>>>()?);
    let candidates = key.pads.iter().flat_map(|pad| pad.spokes.regions());
    let shapes = cuts::thermal_gaps(None, &foreign, gaps, candidates, canceled).ok()?;
    // Drop the temporary prepared bodies and prove that the output can be translated.
    Some(Arc::new(shapes.iter().map(|region| region.place_profile([0, 0])).collect::<Option<_>>()?))
}

fn use_template(
    memo: &Memo,
    stock: &Regions,
    foreign: &Cuts,
    placements: &[Option<ContourPlacement>],
    group: &[&Thermal],
    canceled: Option<&AtomicBool>,
) -> Option<Vec<Regions>> {
    if group.len() > THERMAL_PADS_PER_TEMPLATE {
        return None;
    }
    let extent = group.iter().map(|thermal| thermal.bounds).reduce(join)?;
    let neighbors = foreign.candidates(extent);
    if neighbors.len() > FOREIGN_CUTS_PER_TEMPLATE || stock.covers_box(extent).ok() != Some(true) {
        return None;
    }
    let origin = group.first()?.pad.gap.at;
    let key = ThermalProfile {
        pads: group
            .iter()
            .map(|thermal| ThermalPad { gap: thermal.pad.gap.relative_to(origin), spokes: thermal.pad.spokes.relative_to(origin) })
            .collect(),
        foreign: neighbors.into_iter().map(|index| placements[index].map(|cut| cut.relative_to(origin))).collect::<Option<_>>()?,
    };
    let profile = local_profile(&key, memo, canceled)?;
    let result = profile.iter().map(|region| region.place_profile(origin)).collect::<Option<_>>()?;
    let mut entries = memo.profiles();
    if entries.thermals.len() < THERMAL_MEMO_LIMIT {
        entries.thermals.entry(key).or_insert(profile);
    }
    Some(result)
}

/// Partition thermals using the envelope of each complete gap and all its spokes.
/// A spoke can only change its component's gaps. A bounded interior component can
/// therefore use its local profile even when the rest of the plane changes.
pub(super) fn thermal_cuts(
    stock: &Regions,
    keepouts: Vec<Keepout>,
    pads: Vec<Thermal>,
    memo: &Memo,
    canceled: Option<&AtomicBool>,
) -> Result<Cuts, String> {
    let (foreign, placements): (Vec<_>, Vec<_>) =
        keepouts.into_iter().filter(|keepout| !keepout.region.is_empty()).map(|keepout| (keepout.region, keepout.placement)).unzip();
    let foreign = Cuts::new(foreign);
    let bounds: Vec<_> = pads.iter().map(|pad| pad.bounds).collect();
    let tree = Tree::new(bounds.clone());
    let ids: Vec<_> = (0..pads.len()).collect();
    let mut cuts = Vec::new();
    for group in cuts::components(&tree, &bounds, &ids, canceled)? {
        check_cancel(canceled)?;
        let group: Vec<_> = group.iter().map(|&i| &pads[i]).collect();
        if let Some(regions) = use_template(memo, stock, &foreign, &placements, &group, canceled) {
            cuts.extend(regions);
        } else {
            check_cancel(canceled)?;
            let gaps = group.iter().map(|pad| pad.gap.clone()).collect();
            let candidates = group.iter().flat_map(|pad| pad.pad.spokes.regions());
            cuts.extend(cuts::thermal_gaps(Some(stock), &foreign, gaps, candidates, canceled)?);
        }
    }
    let mut shapes = foreign.shapes;
    shapes.extend(cuts);
    Ok(Cuts::new(shapes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x_nm: i64, y_nm: i64) -> Point {
        Point { x_nm, y_nm }
    }

    fn object(x: i64, y: i64) -> Object {
        let at = point(x, y);
        Object {
            name: String::new(),
            contour: Contour::capsule(at, at, 180_000.5),
            layers: Vec::new(),
            intended_nets: Vec::new(),
            at,
            island: 0,
            net: None,
            mechanical: false,
        }
    }

    #[test]
    fn moved_copper_reuses_other_neighborhoods_and_matches_absolute_recalculation() {
        let stock = Regions::polygon(&[[0, 0], [14_000_000, 0], [14_000_000, 8_000_000], [0, 8_000_000]]).unwrap();
        let pads = [object(2_200_000, 3_000_000), object(3_700_000, 3_000_000), object(9_200_000, 3_000_000), object(10_700_000, 3_000_000)];
        let make_track = |x, y| Contour::capsule(point(x, y), point(x, y + 610_000), 90_000.5);
        let memo = Memo::default();
        let run = |moved: i64| {
            let tracks = [make_track(2_830_000, 2_730_000 + moved), make_track(9_830_000, 2_730_000)];
            let (width, gap, margin) = (260_001, 300_000, 120_002);
            let keepouts = tracks.iter().map(|track| Keepout::physical(track, margin, &memo).unwrap()).collect();
            let thermals = pads.iter().map(|pad| Thermal::new(pad, width, gap, &memo).unwrap()).collect();
            let actual = thermal_cuts(&stock, keepouts, thermals, &memo, None).unwrap().subtract(stock.clone(), None).unwrap();

            // Compare against one absolute calculation with every gap, spoke and track.
            let raw_gaps = pads.iter().map(|p| region(&p.contour, gap + CLEARANCE_RESERVE_NM).unwrap()).collect();
            let raw_spokes = pads.iter().flat_map(|p| Spokes::new(p, width, gap).regions());
            let foreign = Cuts::new(tracks.iter().map(|t| region(t, margin).unwrap()).collect());
            let mut expected_cuts = cuts::thermal_gaps(Some(&stock), &foreign, raw_gaps, raw_spokes, None).unwrap();
            expected_cuts.extend(foreign.shapes);
            let expected = Cuts::new(expected_cuts).subtract(stock.clone(), None).unwrap();
            assert_eq!(actual.region_count(), expected.region_count());
            assert!((actual.area().unwrap().value_nm2 - expected.area().unwrap().value_nm2).abs() < 10.);
            for (a, b) in [(&actual, &expected), (&expected, &actual)] {
                assert!(clip(a, b, Operation::Difference).unwrap().is_empty());
            }
            actual
        };

        let before = run(0);
        let original = {
            let entries = memo.profiles();
            assert_eq!(entries.thermals.len(), 1);
            assert_eq!(entries.thermals.first_key_value().unwrap().0.pads.len(), 2);
            entries.thermals.first_key_value().unwrap().1.clone()
        };
        let after = run(700_000);
        assert_eq!(memo.profiles().thermals.len(), 2);
        assert!(memo.profiles().thermals.values().any(|profile| Arc::ptr_eq(profile, &original)));
        let probe = [2_830_000, 3_025_000];
        assert_eq!(before.contains(probe).unwrap(), Some(false));
        assert_eq!(after.contains(probe).unwrap(), Some(true));
    }
}
