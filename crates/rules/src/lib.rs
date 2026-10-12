//! Initial electrical and geometric checks, with object-addressable diagnostics.
use pcbcraft_doc::{Id, Layer, Project};
use pcbcraft_geom::{Point, point_segment, segment_distance};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub code: &'static str,
    pub severity: &'static str,
    pub message: String,
    pub objects: Vec<Id>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Airwire {
    pub net: Id,
    pub from: Point,
    pub to: Point,
    pub from_object: Id,
    pub to_object: Id,
}
#[derive(Clone)]
struct Copper {
    id: Id,
    net: Id,
    start: Point,
    end: Point,
    radius: f64,
    contact_radius: f64,
    conservative: bool,
    outline: Option<Vec<Point>>,
    outline_error: f64,
    layer: Layer,
    pad: bool,
    bridge: Option<(Id, String)>,
    hole: Option<Hole>,
}
#[derive(Clone)]
struct Hole {
    start: Point,
    end: Point,
    radius: f64,
}

fn copper(project: &Project) -> Vec<Copper> {
    let mut result = Vec::new();
    for f in &project.board.footprints {
        for pad in &f.pads {
            if let Some(position) = f.pad_position(&pad.number) {
                let plated = pad
                    .geometry
                    .as_ref()
                    .is_some_and(|g| g.pad_type == "thru_hole");
                let hole = pad.geometry.as_ref().and_then(|g| {
                    g.drill.as_ref().map(|d| {
                        let radius = d.size.x.min(d.size.y) / 2.0;
                        let axis = Point::new(
                            (d.size.x / 2.0 - radius).max(0.0),
                            (d.size.y / 2.0 - radius).max(0.0),
                        );
                        let transform = |local: Point| {
                            f.position.offset(
                                pad.offset
                                    .offset(local.rotate(-g.rotation))
                                    .rotate(-f.rotation),
                            )
                        };
                        Hole {
                            start: transform(d.offset.offset(axis)),
                            end: transform(d.offset.offset(Point::new(-axis.x, -axis.y))),
                            radius,
                        }
                    })
                });
                let item = Copper {
                    id: f.id,
                    net: pad.net.unwrap_or(0),
                    start: position,
                    end: position,
                    radius: pad.bounding_radius(),
                    contact_radius: pad.diameter / 2.0,
                    conservative: pad.geometry.as_ref().is_some_and(|p| p.shape != "circle"),
                    outline: pad
                        .geometry
                        .as_ref()
                        .filter(|p| p.shape != "circle")
                        .map(|_| {
                            pad.outline_points()
                                .into_iter()
                                .map(|p| f.position.offset(p.rotate(-f.rotation)))
                                .collect()
                        }),
                    outline_error: pad.outline_error(),
                    layer: Layer::Front,
                    pad: true,
                    bridge: if plated {
                        Some((f.id, pad.number.clone()))
                    } else {
                        None
                    },
                    hole,
                };
                if plated {
                    let mut back = item.clone();
                    back.layer = Layer::Back;
                    result.push(back);
                }
                result.push(item);
            }
        }
    }
    for t in &project.board.tracks {
        result.push(Copper {
            id: t.id,
            net: t.net,
            start: t.start,
            end: t.end,
            radius: t.width / 2.0,
            contact_radius: t.width / 2.0,
            conservative: false,
            outline: None,
            outline_error: 0.0,
            layer: t.layer,
            pad: false,
            bridge: None,
            hole: None,
        });
    }
    result
}
fn hole_contains(hole: &Hole, other: &Copper) -> bool {
    // The distance-to-segment function is convex. Endpoint discs contained by
    // the hole guarantee containment of the complete track capsule. Retain
    // conservative contact bounds for pad shapes rather than inventing copper.
    let radius = if other.pad {
        other.radius
    } else {
        other.contact_radius
    };
    [other.start, other.end]
        .into_iter()
        .all(|point| point_segment(point, hole.start, hole.end) + radius < hole.radius - 1e-6)
}
fn touching(a: &Copper, b: &Copper) -> bool {
    if a.net != b.net {
        return false;
    }
    if a.bridge.is_some() && a.bridge == b.bridge {
        return true;
    }
    a.layer == b.layer
        && segment_distance(a.start, a.end, b.start, b.end) - a.contact_radius - b.contact_radius
            <= 1e-6
        && !a.hole.as_ref().is_some_and(|hole| hole_contains(hole, b))
        && !b.hole.as_ref().is_some_and(|hole| hole_contains(hole, a))
}
fn gap(a: &Copper, b: &Copper) -> f64 {
    match (&a.outline, &b.outline) {
        (Some(a_points), Some(b_points)) => {
            let mut distance = f64::INFINITY;
            if a_points.first().is_some_and(|p| inside(*p, b_points))
                || b_points.first().is_some_and(|p| inside(*p, a_points))
            {
                return -a.outline_error - b.outline_error;
            }
            for a_edge in a_points.windows(2) {
                for b_edge in b_points.windows(2) {
                    distance =
                        distance.min(segment_distance(a_edge[0], a_edge[1], b_edge[0], b_edge[1]));
                }
            }
            distance - a.outline_error - b.outline_error
        }
        (Some(points), None) => {
            polygon_segment(points, b.start, b.end) - b.radius - a.outline_error
        }
        (None, Some(points)) => {
            polygon_segment(points, a.start, a.end) - a.radius - b.outline_error
        }
        (None, None) => segment_distance(a.start, a.end, b.start, b.end) - a.radius - b.radius,
    }
}
fn inside(point: Point, points: &[Point]) -> bool {
    let mut result = false;
    for edge in points.windows(2) {
        let (a, b) = (edge[0], edge[1]);
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            result = !result;
        }
    }
    result
}
fn polygon_segment(points: &[Point], start: Point, end: Point) -> f64 {
    if inside(start, points) || inside(end, points) {
        return 0.0;
    }
    points
        .windows(2)
        .map(|edge| segment_distance(edge[0], edge[1], start, end))
        .fold(f64::INFINITY, f64::min)
}
fn root(parent: &[usize], mut node: usize) -> usize {
    for _ in 0..parent.len() {
        let Some(next) = parent.get(node).copied() else {
            return node;
        };
        if next == node {
            return node;
        }
        node = next;
    }
    node
}
pub fn airwires(project: &Project) -> Vec<Airwire> {
    let items = copper(project);
    let mut parents: Vec<_> = (0..items.len()).collect();
    for (i, a) in items.iter().enumerate() {
        for (j, b) in items.iter().enumerate().skip(i + 1) {
            if touching(a, b) {
                let ra = root(&parents, i);
                let rb = root(&parents, j);
                if let Some(p) = parents.get_mut(rb) {
                    *p = ra;
                }
            }
        }
    }
    let mut groups: BTreeMap<Id, BTreeMap<usize, Vec<&Copper>>> = BTreeMap::new();
    for (i, item) in items
        .iter()
        .enumerate()
        .filter(|(_, c)| c.pad && c.net != 0)
    {
        groups
            .entry(item.net)
            .or_default()
            .entry(root(&parents, i))
            .or_default()
            .push(item);
    }
    let mut result = vec![];
    for (net, islands) in groups {
        let Some(first) = islands.keys().next().copied() else {
            continue;
        };
        let mut reached = BTreeSet::from([first]);
        while reached.len() < islands.len() {
            let mut nearest: Option<(f64, usize, &Copper, &Copper)> = None;
            for (a_key, a_items) in &islands {
                if !reached.contains(a_key) {
                    continue;
                }
                for (b_key, b_items) in &islands {
                    if reached.contains(b_key) {
                        continue;
                    }
                    for a in a_items {
                        for b in b_items {
                            let d = a.start.distance(b.start);
                            if nearest.as_ref().is_none_or(|(best, _, _, _)| d < *best) {
                                nearest = Some((d, *b_key, a, b));
                            }
                        }
                    }
                }
            }
            let Some((_, key, a, b)) = nearest else {
                break;
            };
            reached.insert(key);
            result.push(Airwire {
                net,
                from: a.start,
                to: b.start,
                from_object: a.id,
                to_object: b.id,
            });
        }
    }
    result
}

pub fn erc(project: &Project) -> Vec<Finding> {
    let mut result = vec![];
    for c in &project.components {
        for p in &c.pins {
            if p.net.is_none() {
                result.push(Finding {
                    code: "unassigned_pin",
                    severity: "warning",
                    message: format!("{} pin {} has no net", c.reference, p.number),
                    objects: vec![c.id],
                });
            }
        }
    }
    for net in &project.nets {
        let pins = project
            .components
            .iter()
            .flat_map(|c| &c.pins)
            .filter(|p| p.net == Some(net.id))
            .count();
        if pins < 2 {
            result.push(Finding {
                code: "single_pin_net",
                severity: "warning",
                message: format!("{} has {pins} connected pins", net.name),
                objects: vec![net.id],
            });
        }
    }
    result
}
pub fn drc(project: &Project) -> Vec<Finding> {
    let mut result = vec![];
    let items = copper(project);
    let board = &project.board;
    for (i, a) in items.iter().enumerate() {
        for b in items.iter().skip(i + 1) {
            if (a.net != b.net || a.net == 0) && a.layer == b.layer {
                if segment_distance(a.start, a.end, b.start, b.end) - a.radius - b.radius
                    > board.clearance
                {
                    continue;
                }
                let separation = gap(a, b);
                if separation + 1e-6 < board.clearance {
                    result.push(Finding {
                        code: if a.conservative || b.conservative {
                            "pad_clearance_bound"
                        } else if separation <= 0.0 {
                            "copper_short"
                        } else {
                            "clearance"
                        },
                        severity: "error",
                        message: format!(
                            "{} / {}: {:.3} mm clearance bound, {:.3} mm required",
                            project.net_name(a.net),
                            project.net_name(b.net),
                            separation,
                            board.clearance
                        ),
                        objects: vec![a.id, b.id],
                    });
                }
            }
        }
        let edge = if let Some(points) = &a.outline {
            points
                .iter()
                .map(|p| p.x.min(p.y).min(board.width - p.x).min(board.height - p.y))
                .fold(f64::INFINITY, f64::min)
                - a.outline_error
        } else {
            [
                a.start.x,
                a.end.x,
                a.start.y,
                a.end.y,
                board.width - a.start.x,
                board.width - a.end.x,
                board.height - a.start.y,
                board.height - a.end.y,
            ]
            .into_iter()
            .fold(f64::INFINITY, f64::min)
                - a.radius
        };
        if edge + 1e-6 < board.clearance {
            result.push(Finding {
                code: "board_edge",
                severity: "error",
                message: format!("Copper is {:.3} mm from the board edge", edge),
                objects: vec![a.id],
            });
        }
        for cut in &board.cutouts {
            let clearance = if let Some(points) = &a.outline {
                polygon_segment(points, cut.center, cut.center) - cut.radius - a.outline_error
            } else {
                point_segment(cut.center, a.start, a.end) - cut.radius - a.radius
            };
            if clearance + 1e-6 < board.clearance {
                result.push(Finding {
                    code: "cutout_clearance",
                    severity: "error",
                    message: format!("Copper violates cutout clearance ({clearance:.3} mm)"),
                    objects: vec![a.id, cut.id],
                });
            }
        }
    }
    for wire in airwires(project) {
        result.push(Finding {
            code: "unrouted",
            severity: "warning",
            message: format!(
                "{} has disconnected copper islands",
                project.net_name(wire.net)
            ),
            objects: vec![wire.from_object, wire.to_object],
        });
    }
    for c in &project.components {
        if !board.footprints.iter().any(|f| f.component == c.id) {
            result.push(Finding {
                code: "missing_footprint",
                severity: "warning",
                message: format!("{} has no PCB footprint; sync the board", c.reference),
                objects: vec![c.id],
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcbcraft_doc::{Footprint, Net, Pad, Track, component_template};
    fn fixture() -> Project {
        let mut p = Project::default();
        p.nets.push(Net {
            id: 1,
            name: "signal".into(),
        });
        for (id, x) in [(2, 10.0), (3, 30.0)] {
            if let Some(mut c) =
                component_template("resistor", id, format!("R{id}"), Point::new(x, 10.0))
            {
                for pin in &mut c.pins {
                    pin.net = Some(1);
                }
                p.components.push(c);
            }
            p.board.footprints.push(Footprint {
                id: id + 10,
                component: id,
                position: Point::new(x, 10.0),
                rotation: 0.0,
                pads: vec![Pad {
                    number: "1".into(),
                    offset: Point::default(),
                    diameter: 1.0,
                    net: Some(1),
                    geometry: None,
                }],
            });
        }
        p
    }
    fn test_copper(id: Id, point: Point, layer: Layer, net: Id, pad: bool) -> Copper {
        Copper {
            id,
            net,
            start: point,
            end: point,
            radius: if pad { 0.8 } else { 0.1 },
            contact_radius: if pad { 0.8 } else { 0.1 },
            conservative: false,
            outline: None,
            outline_error: 0.0,
            layer,
            pad,
            bridge: None,
            hole: None,
        }
    }
    #[test]
    fn plated_pad_bridges_front_and_back_but_identity_and_net_must_match() {
        let mut front = test_copper(10, Point::new(10.0, 10.0), Layer::Front, 1, true);
        front.bridge = Some((10, "1".into()));
        let mut back = front.clone();
        back.layer = Layer::Back;
        assert!(touching(&front, &back));
        let mut smd = front.clone();
        smd.bridge = None;
        assert!(!touching(&smd, &back));
        let mut different_pad = back.clone();
        different_pad.bridge = Some((10, "2".into()));
        assert!(!touching(&front, &different_pad));
        let mut different_net = back;
        different_net.net = 2;
        assert!(!touching(&front, &different_net));
    }
    #[test]
    fn hole_only_stubs_do_not_touch_annulus_but_center_routes_cross_it() {
        let center = Point::new(10.0, 10.0);
        let mut pad = test_copper(10, center, Layer::Front, 1, true);
        pad.hole = Some(Hole {
            start: center,
            end: center,
            radius: 0.4,
        });
        let mut stub = test_copper(11, center, Layer::Front, 1, false);
        stub.end = Point::new(10.1, 10.0);
        assert!(!touching(&pad, &stub));
        assert!(!touching(&stub, &pad));
        stub.end = Point::new(12.0, 10.0);
        assert!(touching(&pad, &stub));
        // A slot wholly contains a short trace even when the trace is far
        // from the slot centre; test capsule containment rather than a circle.
        pad.hole = Some(Hole {
            start: Point::new(9.5, 10.0),
            end: Point::new(10.5, 10.0),
            radius: 0.25,
        });
        stub.start = Point::new(10.3, 10.0);
        stub.end = Point::new(10.4, 10.0);
        assert!(!touching(&pad, &stub));
    }
    #[test]
    fn front_track_to_plated_pad_to_back_track_connects_physical_pads() -> Result<(), String> {
        let mut p = fixture();
        let mut bridge = component_template("resistor", 4, "R4".into(), Point::new(20.0, 10.0))
            .ok_or("missing template")?;
        for pin in &mut bridge.pins {
            pin.net = Some(1);
        }
        p.components.push(bridge);
        let g = pcbcraft_library::FootprintPad {
            number: "1".into(),
            pad_type: "thru_hole".into(),
            shape: "circle".into(),
            position: Point::default(),
            rotation: 0.0,
            size: Point::new(1.6, 1.6),
            layers: vec!["*.Cu".into(), "*.Mask".into()],
            drill: Some(pcbcraft_library::Drill {
                size: Point::new(0.8, 0.8),
                offset: Point::default(),
            }),
            roundrect_ratio: None,
            raw_source: String::new(),
        };
        p.board.footprints.push(Footprint {
            id: 14,
            component: 4,
            position: Point::new(20.0, 10.0),
            rotation: 0.0,
            pads: vec![Pad {
                number: "1".into(),
                offset: Point::default(),
                diameter: 1.6,
                net: Some(1),
                geometry: Some(g),
            }],
        });
        // Endpoints are front SMD; their separate front approach demonstrates
        // that the only cross-layer join comes from the plated package pad.
        for (id, start, end, layer) in [
            (
                20,
                Point::new(10.0, 10.0),
                Point::new(20.0, 10.0),
                Layer::Front,
            ),
            (
                21,
                Point::new(20.0, 10.0),
                Point::new(30.0, 10.0),
                Layer::Back,
            ),
            (
                22,
                Point::new(30.0, 10.0),
                Point::new(30.0, 11.0),
                Layer::Front,
            ),
        ] {
            p.board.tracks.push(Track {
                id,
                net: 1,
                start,
                end,
                width: 0.2,
                layer,
            });
        }
        // Make the far endpoint a plated pad as well, so its back approach
        // meets that package's copper instead of an unsupported layer jump.
        let mut geometry = p
            .board
            .footprints
            .last()
            .and_then(|f| f.pads.first())
            .and_then(|p| p.geometry.clone())
            .ok_or("missing bridge geometry")?;
        geometry.size = Point::new(1.0, 1.0);
        geometry.drill = Some(pcbcraft_library::Drill {
            size: Point::new(0.4, 0.4),
            offset: Point::default(),
        });
        let far = p.board.footprints.get_mut(1).ok_or("missing far pad")?;
        far.pads[0].geometry = Some(geometry);
        assert!(airwires(&p).is_empty());
        p.board
            .footprints
            .last_mut()
            .and_then(|f| f.pads.first_mut())
            .and_then(|p| p.geometry.as_mut())
            .ok_or("missing bridge")?
            .pad_type = "smd".into();
        assert!(!airwires(&p).is_empty());
        Ok(())
    }
    #[test]
    fn real_0805_copper_clearance_uses_pad_outlines() -> Result<(), String> {
        let definition = pcbcraft_library::read_footprint(include_str!(
            "../../library/tests/fixtures/R_0805_2012Metric.kicad_mod"
        ))
        .map_err(|e| e.to_string())?;
        let mut p = Project::default();
        let mut c = component_template("resistor", 1, "R1".into(), Point::default())
            .ok_or("missing template")?;
        for (i, pin) in c.pins.iter_mut().enumerate() {
            pin.net = Some(2 + i as u64);
        }
        c.library_footprint = Some(pcbcraft_doc::LibraryFootprint {
            library: "Resistor_SMD".into(),
            definition: definition.clone(),
        });
        p.components.push(c);
        p.nets = vec![
            Net {
                id: 2,
                name: "IN".into(),
            },
            Net {
                id: 3,
                name: "OUT".into(),
            },
        ];
        let pads = definition
            .pads
            .iter()
            .enumerate()
            .map(|(i, g)| Pad {
                number: g.number.clone(),
                offset: g.position,
                diameter: g.size.x.min(g.size.y),
                net: Some(2 + i as u64),
                geometry: Some(g.clone()),
            })
            .collect();
        p.board.footprints.push(Footprint {
            id: 4,
            component: 1,
            position: Point::new(20.0, 20.0),
            rotation: 0.0,
            pads,
        });
        p.validate()?;
        assert!(
            drc(&p).is_empty(),
            "standard 0805 pads are not a short at 0.2mm clearance"
        );
        p.board.clearance = 0.9;
        assert!(drc(&p).iter().any(|f| f.code == "pad_clearance_bound"));
        p.board.clearance = 0.2;
        p.board.tracks.push(Track {
            id: 5,
            net: 2,
            start: Point::new(19.0875, 20.0),
            end: Point::new(20.9125, 20.0),
            width: 0.25,
            layer: Layer::Front,
        });
        assert!(
            drc(&p)
                .iter()
                .any(|f| f.code == "pad_clearance_bound" && f.objects.contains(&5))
        );
        Ok(())
    }
    #[test]
    fn routing_connects_islands_but_back_layer_does_not() {
        let mut p = fixture();
        assert_eq!(airwires(&p).len(), 1);
        p.board.tracks.push(Track {
            id: 20,
            net: 1,
            start: Point::new(10.0, 10.0),
            end: Point::new(30.0, 10.0),
            width: 0.25,
            layer: Layer::Back,
        });
        assert_eq!(airwires(&p).len(), 1);
        if let Some(t) = p.board.tracks.first_mut() {
            t.layer = Layer::Front;
        }
        assert!(airwires(&p).is_empty());
    }
    #[test]
    fn foreign_net_crossing_is_a_short() {
        let mut p = fixture();
        p.nets.push(Net {
            id: 4,
            name: "gnd".into(),
        });
        p.board.tracks.push(Track {
            id: 20,
            net: 1,
            start: Point::new(10.0, 10.0),
            end: Point::new(30.0, 10.0),
            width: 0.25,
            layer: Layer::Front,
        });
        p.board.tracks.push(Track {
            id: 21,
            net: 4,
            start: Point::new(20.0, 5.0),
            end: Point::new(20.0, 15.0),
            width: 0.25,
            layer: Layer::Front,
        });
        assert!(drc(&p).iter().any(|f| f.code == "copper_short"));
    }
}
