//! Copper zone pours in the Gerber copper layers (#233), read back with the test reader and
//! independent line/arc geometry: contours equal to the analytic fill, deterministic bytes,
//! the same clearance findings as DRC, conductor attributes and the refusals.
use super::*;
use crate::board::{
    fill::{INSPECT_FILL, REFILL_ZONES, UNFILL_ZONES},
    zones::{ADD_ZONE, Connection, DELETE_ZONES, Options},
};
use pcbcraft_geometry::planar::Edge;
use std::f64::consts::TAU;

const MM: i64 = 1_000_000;
const NET_CLEARANCE: &str = "Filled copper violates another net's clearance.";
const ZONE_CLEARANCE: &str = "Filled copper violates another zone's clearance.";

fn at(x: i64, y: i64) -> Point {
    Point { x_nm: x, y_nm: y }
}

/// The class target of a pad on `ACCEPTED`, or of a pad on another named net.
fn pad_target(s: &Session, accepted: bool) -> crate::nets::ClassTarget {
    let v = view(s);
    let (footprint, number) = v
        .footprints()
        .flat_map(|footprint| footprint.pads.into_iter().map(move |pad| (footprint.id.clone(), pad)))
        .find(|(_, pad)| pad.net.as_deref().is_some_and(|net| !net.is_empty() && (net == "ACCEPTED") == accepted))
        .map(|(footprint, pad)| (footprint, pad.number.as_str().to_owned()))
        .unwrap();
    v.net_class_target_for_copper(&format!("{footprint}:{number}")).unwrap().unwrap()
}

/// A notched two-layer board with foreign tracks: one runs across top copper from edge to edge,
/// another changes layer through a via. No zones yet.
fn notched_board() -> Session {
    let mut s = routed_fixture();
    let notched = [(-5, -5), (25, -5), (25, 35), (12, 35), (12, 26), (9, 26), (9, 35), (-5, 35)].map(|(x, y)| at(x * MM, y * MM));
    add_fabrication_drawing(&mut s, LayerPurpose::Outline, &notched, true);
    let [front, back] = copper_layers(&s)[..] else { panic!("two copper layers") };
    route(&mut s, front, &[at(4 * MM, -4_950_000), at(4 * MM, 34_950_000)]).unwrap();
    route_with_vias(&mut s, front, &[at(14 * MM, 20 * MM), at(18 * MM, 20 * MM), at(22 * MM, 20 * MM)], &[(1, back)]).unwrap();
    s
}

/// Add an ACCEPTED pour larger than the board on each copper layer, so the outline and its notch
/// clip it, and a priority-1 pour of the other net on top copper; then pour copper if asked.
fn add_pours(s: &mut Session, connection: Connection, pour: bool) {
    let [front, back] = copper_layers(s)[..] else { panic!("two copper layers") };
    let (accepted, other) = (pad_target(s, true), pad_target(s, false));
    let around = [(-10, -10), (30, -10), (30, 40), (-10, 40)].map(|(x, y)| at(x * MM, y * MM));
    let small = [(10, 8), (22, 8), (22, 16), (10, 16)].map(|(x, y)| at(x * MM, y * MM));
    for (target, layer, points, priority) in [(&accepted, front, &around, 0), (&accepted, back, &around, 0), (&other, front, &small, 1)] {
        let options = Options { connection, priority, ..Options::default() };
        s.apply(ADD_ZONE, &json!({"target":target,"layer":layer,"points":points,"options":options,"expected_revision":s.revision()})).unwrap();
    }
    if pour {
        assert_eq!(s.execute(REFILL_ZONES, &json!({})).unwrap()["zone_fill"]["result"]["state"], "ready");
    }
}

fn pour_board(connection: Connection) -> Session {
    let mut s = notched_board();
    add_pours(&mut s, connection, true);
    s
}

/// One boundary piece in output nanometres: a line from `a` to `b`, or an arc about `c` with
/// radius `r` from angle `start` sweeping `sweep` (positive: counter-clockwise), ending at `b`.
#[derive(Clone, Copy, Debug)]
struct Seg {
    a: [f64; 2],
    b: [f64; 2],
    arc: Option<([f64; 2], f64, f64, f64)>,
}

fn dist(p: [f64; 2], q: [f64; 2]) -> f64 {
    (p[0] - q[0]).hypot(p[1] - q[1])
}

/// Whether angle `theta` lies on the sweep from `start`.
fn on_sweep(theta: f64, start: f64, sweep: f64) -> bool {
    if sweep >= 0. { (theta - start).rem_euclid(TAU) <= sweep + 1e-12 } else { (start - theta).rem_euclid(TAU) <= -sweep + 1e-12 }
}

fn angle(p: [f64; 2], c: [f64; 2]) -> f64 {
    (p[1] - c[1]).atan2(p[0] - c[0])
}

/// A written segment of a read contour; an arc's radius is the mean of its two end radii.
fn written(segment: &ReadSegment) -> Seg {
    let (a, b) = ([segment.from.x_nm as f64, segment.from.y_nm as f64], [segment.to.x_nm as f64, segment.to.y_nm as f64]);
    let arc = segment.arc.map(|(center, clockwise)| {
        let c = [center.x_nm as f64, center.y_nm as f64];
        let (start, end) = (angle(a, c), angle(b, c));
        let sweep = if clockwise { -(start - end).rem_euclid(TAU) } else { (end - start).rem_euclid(TAU) };
        (c, (dist(a, c) + dist(b, c)) / 2., start, sweep)
    });
    Seg { a, b, arc }
}

/// The analytic loop in output nanometres, straight from the fill's boundary.
fn analytic(edges: &[Edge], frame: crate::board::origin::Frame) -> Vec<Seg> {
    let start = |edge: &Edge| match *edge {
        Edge::Line { start_nm } | Edge::Arc { start_nm, .. } => frame.to_output_nm(start_nm),
    };
    (0..edges.len())
        .map(|i| {
            let (a, b) = (start(&edges[i]), start(&edges[(i + 1) % edges.len()]));
            let arc = match edges[i] {
                Edge::Line { .. } => None,
                Edge::Arc { center_nm, radius_nm, sweep_rad, .. } => {
                    let c = frame.to_output_nm(center_nm);
                    Some((c, radius_nm, angle(a, c), -sweep_rad))
                }
            };
            Seg { a, b, arc }
        })
        .collect()
}

fn point_line(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx * dx + dy * dy;
    let t = if length == 0. { 0. } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length).clamp(0., 1.) };
    dist(p, [a[0] + t * dx, a[1] + t * dy])
}

/// Distance from a point to a line or arc piece.
fn point_seg(p: [f64; 2], s: &Seg) -> f64 {
    match s.arc {
        None => point_line(p, s.a, s.b),
        Some((c, r, start, sweep)) => {
            if dist(p, c) > 0. && on_sweep(angle(p, c), start, sweep) {
                (dist(p, c) - r).abs()
            } else {
                let end = [c[0] + r * (start + sweep).cos(), c[1] + r * (start + sweep).sin()];
                dist(p, [c[0] + r * start.cos(), c[1] + r * start.sin()]).min(dist(p, end))
            }
        }
    }
}

fn cross(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// Points where the circle of `arc` meets the infinite line through `p` and `q`.
fn line_circle(p: [f64; 2], q: [f64; 2], c: [f64; 2], r: f64) -> Vec<(f64, [f64; 2])> {
    let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
    let (fx, fy) = (p[0] - c[0], p[1] - c[1]);
    let a = dx * dx + dy * dy;
    if a == 0. {
        return Vec::new();
    }
    let (b, k) = (2. * (fx * dx + fy * dy), fx * fx + fy * fy - r * r);
    let disc = b * b - 4. * a * k;
    if disc < 0. {
        return Vec::new();
    }
    [-1., 1.].map(|sign| (-b + sign * disc.sqrt()) / (2. * a)).into_iter().map(|t| (t, [p[0] + t * dx, p[1] + t * dy])).collect()
}

/// Exact distance between two line or arc pieces: zero when they meet, otherwise the least of
/// the end point distances and the interior critical points along normals through arc centres.
fn seg_seg(s: &Seg, t: &Seg) -> f64 {
    let ends = |s: &Seg| match s.arc {
        None => [s.a, s.b],
        Some((c, r, start, sweep)) => [start, start + sweep].map(|a| [c[0] + r * a.cos(), c[1] + r * a.sin()]),
    };
    let [sa, sb] = ends(s);
    let [ta, tb] = ends(t);
    let mut best = point_seg(sa, t).min(point_seg(sb, t)).min(point_seg(ta, s)).min(point_seg(tb, s));
    match (s.arc, t.arc) {
        (None, None) => {
            let (d1, d2, d3, d4) = (cross(ta, tb, sa), cross(ta, tb, sb), cross(sa, sb, ta), cross(sa, sb, tb));
            if d1 * d2 < 0. && d3 * d4 < 0. {
                best = 0.;
            }
        }
        (Some(arc), None) | (None, Some(arc)) => {
            let line = if s.arc.is_none() { s } else { t };
            let (c, r, start, sweep) = arc;
            let foot = {
                let (dx, dy) = (line.b[0] - line.a[0], line.b[1] - line.a[1]);
                let length = dx * dx + dy * dy;
                let u = if length == 0. { 0. } else { (((c[0] - line.a[0]) * dx + (c[1] - line.a[1]) * dy) / length).clamp(0., 1.) };
                [line.a[0] + u * dx, line.a[1] + u * dy]
            };
            if dist(foot, c) > 0. && on_sweep(angle(foot, c), start, sweep) {
                best = best.min((dist(foot, c) - r).abs());
            }
            if line_circle(line.a, line.b, c, r).iter().any(|&(u, p)| (0. ..=1.).contains(&u) && on_sweep(angle(p, c), start, sweep)) {
                best = 0.;
            }
        }
        (Some((c1, r1, s1, w1)), Some((c2, r2, s2, w2))) => {
            let d = dist(c1, c2);
            if d > 0. {
                let u = [(c2[0] - c1[0]) / d, (c2[1] - c1[1]) / d];
                for k1 in [-1., 1.] {
                    for k2 in [-1., 1.] {
                        let p = [c1[0] + k1 * r1 * u[0], c1[1] + k1 * r1 * u[1]];
                        let q = [c2[0] + k2 * r2 * u[0], c2[1] + k2 * r2 * u[1]];
                        if on_sweep(angle(p, c1), s1, w1) && on_sweep(angle(q, c2), s2, w2) {
                            best = best.min(dist(p, q));
                        }
                    }
                }
                // Circle intersections inside both sweeps.
                let a = (r1 * r1 - r2 * r2 + d * d) / (2. * d);
                let h2 = r1 * r1 - a * a;
                if h2 >= 0. {
                    let m = [c1[0] + a * u[0], c1[1] + a * u[1]];
                    for k in [-1., 1.] {
                        let p = [m[0] - k * h2.sqrt() * u[1], m[1] + k * h2.sqrt() * u[0]];
                        if on_sweep(angle(p, c1), s1, w1) && on_sweep(angle(p, c2), s2, w2) {
                            best = 0.;
                        }
                    }
                }
            }
        }
    }
    best
}

/// Crossing-number containment for the union of contours (cut-in pairs cancel).
fn inside(contours: &[Vec<Seg>], p: [f64; 2]) -> bool {
    contours.iter().any(|contour| {
        let mut crossings = 0;
        for s in contour {
            match s.arc {
                None => {
                    if (s.a[1] > p[1]) != (s.b[1] > p[1]) {
                        let x = s.a[0] + (p[1] - s.a[1]) * (s.b[0] - s.a[0]) / (s.b[1] - s.a[1]);
                        if x > p[0] {
                            crossings += 1;
                        }
                    }
                }
                // Written arcs stay within one quadrant, so like a line they cross a horizontal
                // line at most once, exactly when their end points straddle it.
                Some((c, r, start, sweep)) => {
                    if (s.a[1] > p[1]) != (s.b[1] > p[1]) {
                        let reach = (r * r - (p[1] - c[1]).powi(2)).max(0.).sqrt();
                        let x = [c[0] - reach, c[0] + reach].into_iter().find(|&x| on_sweep(angle([x, p[1]], c), start, sweep)).unwrap_or(f64::NAN);
                        if x > p[0] {
                            crossings += 1;
                        }
                    }
                }
            }
        }
        crossings % 2 == 1
    })
}

/// Signed area of one contour of lines and arcs (positive counter-clockwise).
fn contour_area(contour: &[Seg]) -> f64 {
    contour
        .iter()
        .map(|s| {
            let chord = (s.a[0] * s.b[1] - s.a[1] * s.b[0]) / 2.;
            match s.arc {
                None => chord,
                Some((_, r, _, sweep)) => chord + r * r / 2. * (sweep - sweep.sin()),
            }
        })
        .sum()
}

/// Horizontal or vertical segments that a later segment retraces in reverse: the cut-ins.
fn cut_ins(contour: &[ReadSegment]) -> usize {
    contour
        .iter()
        .enumerate()
        .filter(|(i, s)| {
            s.arc.is_none()
                && (s.from.y_nm == s.to.y_nm || s.from.x_nm == s.to.x_nm)
                && contour[i + 1..].iter().any(|t| t.arc.is_none() && t.from == s.to && t.to == s.from)
        })
        .count()
}

/// The largest distance from a written contour to the analytic loops of one region, measured at
/// every vertex and at every arc's midpoint, after checking each analytic start is a vertex.
fn check_contour(contour: &[ReadSegment], boundary: &pcbcraft_geometry::planar::Boundary, frame: crate::board::origin::Frame) -> f64 {
    let loops: Vec<Vec<Seg>> = std::iter::once(&boundary.outer).chain(&boundary.holes).map(|edges| analytic(edges, frame)).collect();
    assert_eq!(cut_ins(contour), boundary.holes.len(), "one cut-in per hole");
    let vertices: BTreeSet<_> = contour.iter().map(|s| (s.from.x_nm, s.from.y_nm)).collect();
    for s in loops.iter().flatten() {
        let rounded = ((s.a[0] + 0.5).floor() as i64, (s.a[1] + 0.5).floor() as i64);
        assert!(vertices.contains(&rounded), "analytic vertex {rounded:?} is written");
    }
    let near = |p: [f64; 2]| loops.iter().flatten().map(|s| point_seg(p, s)).fold(f64::INFINITY, f64::min);
    let mut worst: f64 = 0.;
    for segment in contour {
        let s = written(segment);
        worst = worst.max(near(s.a));
        if let Some((c, r, start, sweep)) = s.arc {
            assert!(sweep.abs() <= std::f64::consts::FRAC_PI_2 + 1e-9, "arcs are at most a quarter turn");
            assert!(dist(s.a, s.b) >= 1_000., "no arc is shorter than 1 µm");
            let middle = start + sweep / 2.;
            worst = worst.max(near([c[0] + r * middle.cos(), c[1] + r * middle.sin()]));
        }
    }
    let written_area = contour_area(&contour.iter().map(written).collect::<Vec<_>>());
    // The outer loop runs counterclockwise in the file, so its area is positive; holes subtract.
    assert!(written_area > 0., "the written outer loop is counterclockwise: {written_area}");
    let exact = contour_area(&loops[0]).abs() - loops[1..].iter().map(|lp| contour_area(lp).abs()).sum::<f64>();
    let perimeter: f64 = contour.iter().map(|s| dist(written(s).a, written(s).b)).sum();
    assert!((written_area - exact).abs() <= perimeter * 2.2, "area {written_area} against {exact}");
    worst
}

/// The read copper file of one layer: top is L1 and bottom L2 on this board.
fn copper(outputs: &[Output], top: bool) -> ReadGerber {
    read_gerber(outputs, if top { TOP_COPPER } else { BOTTOM[0] })
}

/// Pour contours equal the current analytic fill island by island, at the 1 nm file resolution:
/// every analytic vertex is written, every written point lies on the analytic boundary within
/// the documented bound, and each hole gets one cut-in. Covers clearance holes around foreign
/// pads, tracks and a via, islands split by a track, thermal and solid connections, a pour on
/// each layer, two zones with priority and a pour clipped by a notched outline.
#[test]
fn pour_contours_equal_the_current_fill_at_the_file_resolution() {
    for connection in [Connection::Thermal, Connection::Solid] {
        let s = pour_board(connection);
        let v = view(&s);
        let fabrication = v.fabrication().unwrap();
        let frame = fabrication.origin.frame();
        let fill = v.filled_zones().unwrap().unwrap();
        let mut worst: f64 = 0.;
        let mut holes = 0;
        for (top, layer) in [(true, copper_layers(&s)[0]), (false, copper_layers(&s)[1])] {
            let read = copper(&fabrication.files, top);
            let islands: Vec<_> = fill.zones.iter().filter(|zone| zone.layer == layer).flat_map(|zone| &zone.islands).collect();
            assert_eq!(read.pours.len(), islands.len(), "one region per island ({connection:?}, top {top})");
            for (pour, island) in read.pours.iter().zip(islands) {
                let boundaries = island.region.boundaries().unwrap();
                assert_eq!(pour.len(), boundaries.len());
                for (contour, boundary) in pour.iter().zip(&boundaries) {
                    worst = worst.max(check_contour(contour, boundary, frame));
                    holes += boundary.holes.len();
                }
            }
        }
        let top_zone = v.zones().iter().find(|zone| zone.layer == copper_layers(&s)[0] && zone.options.priority == 0).unwrap().id;
        let top_accepted = fill.zones.iter().find(|zone| zone.id == top_zone).unwrap();
        assert!(top_accepted.islands.len() >= 2, "the edge-to-edge track splits the top pour into islands");
        assert!(holes >= 5, "clearance holes around pads, the via and the priority zone: {holes}");
        assert!(worst <= 2.2, "written pours stay within the documented bound: {worst} nm");
        println!("{connection:?}: largest distance from the analytic fill {worst:.3} nm, {holes} holes");
    }
}

/// A foreign copper object read back from a copper file: its DRC item name, the core it is
/// drawn around (a point, a stroke or a closed polygon of macro circle centres) and the radius.
struct Read {
    name: String,
    net: Option<String>,
    core: Vec<[f64; 2]>,
    radius: f64,
}

impl Read {
    /// The core as pieces: a point or stroke is one piece, a polygon is closed.
    fn pieces(&self) -> Vec<Seg> {
        match self.core.len() {
            1 | 2 => vec![Seg { a: self.core[0], b: self.core[self.core.len() - 1], arc: None }],
            n => (0..n).map(|i| Seg { a: self.core[i], b: self.core[(i + 1) % n], arc: None }).collect(),
        }
    }
}

/// Each pour's loops with its net name, and every other copper object read back.
type Copper = (Vec<(Vec<Vec<Seg>>, Option<String>)>, Vec<Read>);

/// Read one copper file's pours (with their .N) and every other copper object, naming each the
/// way DRC does: `footprint:pad` from the .C/.P attributes, and track or via IDs from their
/// exported coordinates.
fn read_copper(s: &Session, read: &ReadGerber, frame: crate::board::origin::Frame) -> Copper {
    let v = view(s);
    let out = |p: Point| {
        let p = frame.to_output(p);
        [p.x_nm as f64, p.y_nm as f64]
    };
    let f = |p: &Point| [p.x_nm as f64, p.y_nm as f64];
    let (mut pours, mut objects) = (Vec::new(), Vec::new());
    for ((function, object), drawn) in read.objects.iter().zip(&read.drawn) {
        match *drawn {
            Drawn::Pour(index) => {
                pours.push((read.pours[index].iter().map(|contour| contour.iter().map(written).collect()).collect(), object.net.clone()))
            }
            Drawn::Flash(point, code) => {
                let p = f(&point);
                let (core, radius) = if let Some(aperture) = read.apertures.get(&code) {
                    assert_eq!(aperture.kind, 'C', "round flashes on this board");
                    (vec![p], aperture.width as f64 / 2.)
                } else {
                    let rounded = &read.rounded[&code];
                    (rounded.circles.iter().map(|(_, c)| [p[0] + c.x_nm as f64, p[1] + c.y_nm as f64]).collect(), rounded.circles[0].0 as f64 / 2.)
                };
                let name = if *function == Some("ViaPad") {
                    v.vias().iter().find(|via| out(via.at) == p).unwrap().id.to_string()
                } else {
                    let (reference, pin) = object.pin.clone().unwrap();
                    format!("{}:{pin}", v.footprints().find(|footprint| footprint.reference == reference).unwrap().id)
                };
                objects.push(Read { name, net: object.net.clone(), core, radius });
            }
            Drawn::Stroke(a, b, code) => {
                let (a, b) = (f(&a), f(&b));
                let track = v.tracks().iter().find(|t| (out(t.start), out(t.end)) == (a, b) || (out(t.start), out(t.end)) == (b, a)).unwrap();
                objects.push(Read {
                    name: track.id.to_string(),
                    net: object.net.clone(),
                    core: vec![a, b],
                    radius: read.apertures[&code].width as f64 / 2.,
                });
            }
            Drawn::Region => panic!("no pad regions on this board"),
        }
    }
    (pours, objects)
}

/// Distance between pour material and a core with radius: zero when a core point is in it.
fn pour_distance(pour: &[Vec<Seg>], pieces: &[Seg], points: &[[f64; 2]], radius: f64) -> f64 {
    if points.iter().any(|&p| inside(pour, p)) {
        return 0.;
    }
    pour.iter().flatten().flat_map(|s| pieces.iter().map(move |t| seg_seg(s, t))).fold(f64::INFINITY, f64::min) - radius
}

/// Zone-object and zone-zone finding pairs.
type Findings = (BTreeSet<(String, String)>, BTreeSet<(String, String)>);

/// Clearance findings computed only from the exported files: (zone, object) pairs closer than
/// `clearance` between a pour and copper of another net, and unordered zone pairs for pours.
fn file_findings(s: &Session, outputs: &[Output], clearance: f64) -> Findings {
    let v = view(s);
    let frame = v.fabrication().unwrap().origin.frame();
    let fill = v.filled_zones().unwrap().unwrap();
    let (mut objects, mut zones) = (BTreeSet::new(), BTreeSet::new());
    for (top, layer) in [(true, copper_layers(s)[0]), (false, copper_layers(s)[1])] {
        let (pours, others) = read_copper(s, &copper(outputs, top), frame);
        let ids: Vec<_> =
            fill.zones.iter().filter(|zone| zone.layer == layer).flat_map(|zone| zone.islands.iter().map(move |_| zone.id.to_string())).collect();
        assert_eq!(ids.len(), pours.len());
        for ((pour, net), zone) in pours.iter().zip(&ids) {
            for object in others.iter().filter(|object| object.net != *net) {
                if pour_distance(pour, &object.pieces(), &object.core, object.radius) < clearance {
                    objects.insert((zone.clone(), object.name.clone()));
                }
            }
            for ((other, other_net), other_zone) in pours.iter().zip(&ids) {
                let vertices: Vec<_> = other.iter().flatten().map(|s| s.a).collect();
                if other_net != net && zone < other_zone && pour_distance(pour, &other.concat(), &vertices, 0.) < clearance {
                    zones.insert((zone.clone(), other_zone.clone()));
                }
            }
        }
    }
    (objects, zones)
}

/// Reading the exported pours, pads, tracks and vias back and measuring clearance gives the same
/// findings as DRC on the board: none at the authored rules, and the same object and zone
/// pairs under a deliberately tight 0.5 mm rule.
#[test]
fn clearance_read_back_from_the_files_matches_drc() {
    let s = pour_board(Connection::Thermal);
    let v = view(&s);
    let outputs = v.fabrication_outputs().unwrap();
    let authored = v.effective_rules().values;
    for (clearance, tight) in [(authored.clearance_nm, false), (500_000, true)] {
        let report = v.check_design(crate::board::rules::Values { clearance_nm: clearance, ..authored });
        assert_eq!(report.violations.len(), report.total, "the report is complete");
        let pair = |items: &[String]| (items[0].clone(), items[1].clone());
        let drc_objects: BTreeSet<_> = report.violations.iter().filter(|f| f.message == NET_CLEARANCE).map(|f| pair(&f.items)).collect();
        let drc_zones: BTreeSet<_> = report
            .violations
            .iter()
            .filter(|f| f.message == ZONE_CLEARANCE)
            .map(|f| if f.items[0] < f.items[1] { pair(&f.items) } else { (f.items[1].clone(), f.items[0].clone()) })
            .collect();
        let (objects, zones) = file_findings(&s, &outputs, clearance as f64);
        assert_eq!(objects, drc_objects, "pour to copper findings at {clearance} nm");
        assert_eq!(zones, drc_zones, "pour to pour findings at {clearance} nm");
        assert_eq!(!objects.is_empty() && !zones.is_empty(), tight, "{objects:?} {zones:?}");
        println!("clearance {clearance} nm: {} pour-object and {} pour-pour findings in both", objects.len(), zones.len());
    }
}

/// Two exports of one board give identical bytes, also after the fill is recalculated.
#[test]
fn pour_exports_are_byte_identical() {
    let mut s = pour_board(Connection::Thermal);
    let first = s.export_fabrication_archive(&json!({})).unwrap();
    assert_eq!(first.bytes(), s.export_fabrication_archive(&json!({})).unwrap().bytes());
    assert_eq!(s.execute(REFILL_ZONES, &json!({})).unwrap()["zone_fill"]["result"]["state"], "ready");
    assert_eq!(first.bytes(), s.export_fabrication_archive(&json!({})).unwrap().bytes(), "a fresh fill writes the same bytes");
}

/// Pours are Conductor regions carrying their zone's net name, the same .N as that net's pads,
/// and no component attributes.
#[test]
fn pours_are_conductor_regions_named_like_their_pads() {
    let s = pour_board(Connection::Thermal);
    let outputs = view(&s).fabrication_outputs().unwrap();
    let top = copper(&outputs, true);
    let pad_nets: BTreeSet<_> =
        top.objects.iter().filter(|(_, object)| object.component.is_some()).filter_map(|(_, object)| object.net.clone()).collect();
    let mut pour_nets = BTreeSet::new();
    for read in [top, copper(&outputs, false)] {
        let pours: Vec<_> =
            read.objects.iter().zip(&read.drawn).filter(|(_, drawn)| matches!(drawn, Drawn::Pour(_))).map(|(object, _)| object).collect();
        assert!(!pours.is_empty());
        for (function, object) in pours {
            assert_eq!(*function, Some("Conductor"));
            assert!(object.component.is_none() && object.pin.is_none(), "{object:?}");
            let net = object.net.clone().unwrap();
            assert!(pad_nets.contains(&net), "{net} is a pad net in {pad_nets:?}");
            pour_nets.insert(net);
        }
    }
    assert!(pour_nets.contains("ACCEPTED") && pour_nets.len() == 2, "{pour_nets:?}");
}

/// A foreign track whose clearance leaves a 500 nm copper strip beside another one: the facing
/// edges could touch under the ±0.5 µm perturbation of Gerber §4.14.2, so export is refused
/// with a message naming the zone, and nothing about the fill changes.
#[test]
fn pours_that_cannot_be_written_safely_are_refused_naming_the_zone() {
    let mut s = notched_board();
    let front = copper_layers(&s)[0];
    let v = view(&s);
    let margin = v.effective_rules().values.clearance_nm + crate::board::fill::CLEARANCE_RESERVE_NM;
    let width = v.tracks()[0].width_nm;
    // Beside the foreign track along x = 4 mm.
    let x = 4 * MM + width + 2 * margin + 500;
    route(&mut s, front, &[at(x, 0), at(x, 10 * MM)]).unwrap();
    add_pours(&mut s, Connection::Solid, true);
    let error = s.export_fabrication(&json!({})).unwrap_err();
    let layer = view(&s).layers().into_iter().find(|layer| layer.id == front).unwrap().name;
    for part in ["ACCEPTED", layer.as_str(), "1 µm"] {
        assert!(format!("{error:?}").contains(part), "{error:?} names {part:?}");
    }
}

/// The test reader refuses a pour contour that retraces one line and encloses nothing, and
/// still reads a two-segment contour that encloses area: an arc closed by its chord.
#[test]
fn reader_rejects_a_retraced_two_line_pour_contour() {
    let s = pour_board(Connection::Thermal);
    let outputs = view(&s).fabrication_outputs().unwrap();
    let (suffix, function, polarity) = TOP_COPPER;
    let text = file(&outputs, suffix);
    // A pour with arcs, so circular mode is enabled before it.
    let open = "G75*\n%TA.AperFunction,Conductor*%\nG36*\n";
    let start = text.find(open).unwrap() + open.len();
    let end = start + text[start..].find("G37*\n").unwrap();
    let with = |contour: &str| format!("{}{contour}{}", &text[..start], &text[end..]);
    let retraced = with("X0Y0D02*\nX1000000Y0D01*\nX0Y0D01*\n");
    assert_eq!(try_read_gerber(&retraced, function, polarity).unwrap_err(), "region contour not closed");
    let half_disc = with("X0Y0D02*\nG03*\nX2000000Y0I1000000J0D01*\nG01*\nX0Y0D01*\n");
    assert!(try_read_gerber(&half_disc, function, polarity).is_ok());
}

/// Remove pour regions (each with its .N scope) from a copper file's text.
fn without_pours(text: &str) -> String {
    let lines: Vec<_> = text.lines().collect();
    let mut kept = String::new();
    let mut i = 0;
    while i < lines.len() {
        let start = if lines[i].starts_with("%TO.N,") { i + 1 } else { i };
        let skip = usize::from(lines.get(start) == Some(&"G75*"));
        if lines.get(start + skip) == Some(&"%TA.AperFunction,Conductor*%") && lines.get(start + skip + 1) == Some(&"G36*") {
            let end = if start > i { "%TD.N*%" } else { "%TD.AperFunction*%" };
            i = start + lines[start..].iter().position(|line| *line == end).unwrap() + 1;
            continue;
        }
        kept.push_str(lines[i]);
        kept.push('\n');
        i += 1;
    }
    kept
}

/// Pours only add regions to the copper files. Removing the zones gives the same drill files,
/// mask, paste, silkscreen and outline, and copper files equal to the zoned ones without their
/// pour regions.
#[test]
fn pours_leave_drills_and_other_layers_unchanged() {
    let mut s = pour_board(Connection::Thermal);
    let zoned = view(&s).fabrication_outputs().unwrap();
    let zones: Vec<_> = view(&s).zones().iter().map(|zone| zone.id).collect();
    s.apply(DELETE_ZONES, &json!({"zones":zones,"expected_revision":s.revision()})).unwrap();
    let plain = view(&s).fabrication_outputs().unwrap();
    assert_eq!(zoned.iter().map(|o| &o.name).collect::<Vec<_>>(), plain.iter().map(|o| &o.name).collect::<Vec<_>>());
    for (with, without) in zoned.iter().zip(&plain) {
        if with.name.ends_with("-Top.gbr") || with.name.ends_with("-Bottom.gbr") {
            assert_ne!(with.contents, without.contents);
            assert_eq!(without_pours(&with.contents), without.contents, "{}", with.name);
        } else {
            assert_eq!(with.contents, without.contents, "{}", with.name);
        }
    }
    assert!(zoned.iter().any(|o| o.name.ends_with("-PTH.drl") && o.contents.contains("T1C")), "the via is drilled");
}

/// SHA-256 of every output of `plain_board()` exported by upstream main af0c6376, before pours.
const MAIN_PLAIN_BOARD_SHA256: &str = "af33801abdbe146c418a7c6c2d466df6566cac973c36cd341278b88006dc16d8";

/// The standard routed fixture with its rectangular outline and no zones or tracks.
fn plain_board() -> Session {
    let mut s = routed_fixture();
    outline(&mut s);
    s
}

/// A board without zones exports byte for byte as upstream main did.
#[test]
fn boards_without_zones_export_exactly_as_before() {
    let mut hasher = Sha256::new();
    for output in view(&plain_board()).fabrication_outputs().unwrap() {
        hasher.update(output.name.as_bytes());
        hasher.update([0]);
        hasher.update(output.contents.as_bytes());
        hasher.update([0]);
    }
    assert_eq!(format!("{:x}", hasher.finalize()), MAIN_PLAIN_BOARD_SHA256);
}

/// Export writes only the fill DRC uses for the current board: a pending fill (also after an
/// edit makes the previous one stale), removed pours and a failed fill are refused, naming Pour
/// copper as the way to retry, without editing the board; a fill completed by DRC is exported.
#[test]
fn fabrication_writes_only_the_current_pour() {
    use std::sync::Arc;

    let mut s = notched_board();
    add_pours(&mut s, Connection::Solid, false);
    let params = json!({});
    let refused = |s: &mut Session, text: &str| {
        for error in [s.export_fabrication_archive(&params).unwrap_err(), s.execute(EXPORT_FABRICATION, &params).unwrap_err()] {
            assert!(matches!(&error, CommandError::Unavailable(message) if message.contains(text)), "{error:?} names {text:?}");
        }
    };
    let pending = "Copper zones are not poured for the current board yet. Use Pour copper";
    let revision = s.revision();
    let snapshot = s.snapshot().unwrap();
    assert_eq!(s.execute(INSPECT_FILL, &params).unwrap()["zone_fill"]["result"]["state"], "pending");
    refused(&mut s, pending);
    assert_eq!(s.execute(REFILL_ZONES, &params).unwrap()["zone_fill"]["result"]["state"], "ready");
    assert!(file(&s.export_fabrication(&params).unwrap(), "-Top.gbr").contains("G36*"));
    assert_eq!(s.execute(UNFILL_ZONES, &params).unwrap()["zone_fill"]["result"]["state"], "unfilled");
    refused(&mut s, "Copper pours are removed. Use Pour copper");
    assert_eq!(s.revision(), revision);
    assert!(Arc::ptr_eq(&snapshot, &s.snapshot().unwrap()), "export and refusals edit nothing");

    s.execute(REFILL_ZONES, &params).unwrap();
    let back = copper_layers(&s)[1];
    route(&mut s, back, &[at(0, 30 * MM), at(2 * MM, 30 * MM)]).unwrap();
    refused(&mut s, pending);
    let v = view(&s);
    let _ = v.check_design(v.effective_rules().values);
    let drc_fill = v.filled_zones().unwrap().unwrap();
    assert!(s.export_fabrication(&params).is_ok(), "DRC poured the current board");
    assert!(Arc::ptr_eq(&drc_fill, &view(&s).filled_zones().unwrap().unwrap()));

    route(&mut s, back, &[at(0, 32 * MM), at(2 * MM, 32 * MM)]).unwrap();
    let job = s.prepare_zone_fill(view(&s).target, false).unwrap();
    assert!(s.fail_zone_fill(job.source(), "Kernel refused the pour.".into()));
    refused(&mut s, "Copper zones could not be poured: Kernel refused the pour. Use Pour copper to try again.");
}

/// The Manufacturing review opens a package with pours and draws their copper: the marks a pour
/// adds to each copper layer cover the analytic fill area of that layer, within the display
/// flattening of arcs.
#[test]
fn manufacturing_review_draws_pour_copper() {
    let marks = |s: &Session| -> Vec<Vec<review::Mark>> {
        let archive = s.export_fabrication_archive(&json!({})).unwrap();
        let review = review::Package::read(archive.bytes()).expect("Manufacturing opens the package");
        [review::LayerRole::TopCopper, review::LayerRole::BottomCopper]
            .map(|role| review.layers.iter().find(|layer| layer.role == role).unwrap().marks.clone())
            .into()
    };
    let polygon_area = |vertices: &[Point]| {
        let doubled: i128 = vertices
            .iter()
            .zip(vertices.iter().cycle().skip(1))
            .map(|(a, b)| i128::from(a.x_nm) * i128::from(b.y_nm) - i128::from(a.y_nm) * i128::from(b.x_nm))
            .sum();
        doubled as f64 / 2.
    };
    let before = marks(&notched_board());
    let s = pour_board(Connection::Thermal);
    let after = marks(&s);
    let v = view(&s);
    let frame = v.fabrication().unwrap().origin.frame();
    let fill = v.filled_zones().unwrap().unwrap();
    for (index, layer) in copper_layers(&s).into_iter().enumerate() {
        assert!(after[index].len() > before[index].len(), "pours add marks");
        let drawn: f64 = after[index][before[index].len()..]
            .iter()
            .map(|mark| match mark {
                review::Mark::Polygon(vertices) => polygon_area(vertices),
                other => panic!("a pour is drawn as polygons: {other:?}"),
            })
            .sum();
        let exact: f64 = fill
            .zones
            .iter()
            .filter(|zone| zone.layer == layer)
            .flat_map(|zone| &zone.islands)
            .flat_map(|island| island.region.boundaries().unwrap())
            .map(|boundary| {
                contour_area(&analytic(&boundary.outer, frame)).abs()
                    - boundary.holes.iter().map(|hole| contour_area(&analytic(hole, frame)).abs()).sum::<f64>()
            })
            .sum();
        assert!(drawn > 0. && (drawn - exact).abs() <= exact * 1e-4, "layer {index}: drawn {drawn} against {exact}");
    }
}
