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
    layer: Layer,
    pad: bool,
}

fn copper(project: &Project) -> Vec<Copper> {
    let mut result = Vec::new();
    for f in &project.board.footprints {
        for pad in &f.pads {
            if let Some(position) = f.pad_position(&pad.number) {
                result.push(Copper {
                    id: f.id,
                    net: pad.net.unwrap_or(0),
                    start: position,
                    end: position,
                    radius: pad.diameter / 2.0,
                    layer: Layer::Front,
                    pad: true,
                });
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
            layer: t.layer,
            pad: false,
        });
    }
    result
}
fn gap(a: &Copper, b: &Copper) -> f64 {
    segment_distance(a.start, a.end, b.start, b.end) - a.radius - b.radius
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
            if a.net == b.net && a.layer == b.layer && gap(a, b) <= 1e-6 {
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
                let separation = gap(a, b);
                if separation + 1e-6 < board.clearance {
                    result.push(Finding {
                        code: if separation <= 0.0 {
                            "copper_short"
                        } else {
                            "clearance"
                        },
                        severity: "error",
                        message: format!(
                            "{} / {}: {:.3} mm clearance, {:.3} mm required",
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
        let edge = [
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
            - a.radius;
        if edge + 1e-6 < board.clearance {
            result.push(Finding {
                code: "board_edge",
                severity: "error",
                message: format!("Copper is {:.3} mm from the board edge", edge),
                objects: vec![a.id],
            });
        }
        for cut in &board.cutouts {
            let clearance = point_segment(cut.center, a.start, a.end) - cut.radius - a.radius;
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
                }],
            });
        }
        p
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
