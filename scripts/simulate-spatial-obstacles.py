#!/usr/bin/env python3
"""Original analytic bounds stress cases for #244, not native projects or routing claims."""
import json
import math
import sys
from pathlib import Path

MM = 1_000_000.0
ALL_LAYERS = (1 << 64) - 1


def point(shape, fraction):
    """Evaluate an authored synthetic centerline without tessellating its analytic oracle."""
    kind, data, radius = shape
    if kind == "arc":
        x, y, r, angle, sweep = data
        a = angle + sweep * fraction
        return x + r * math.cos(a), y + r * math.sin(a)
    x, y, xx, yy = data
    return x + (xx - x) * fraction, y + (yy - y) * fraction


def envelope(shape, start=0.0, end=1.0):
    """Outward-rounded conservative capsule/arc bounds, including interior cardinal extrema."""
    kind, data, width = shape
    points = [point(shape, start), point(shape, end)]
    if kind == "arc":
        x, y, r, angle, sweep = data
        a, b = angle + start * sweep, angle + end * sweep
        for n in range(math.ceil(a / (math.pi / 2)), math.floor(b / (math.pi / 2)) + 1):
            theta = n * math.pi / 2
            points.append((x + r * math.cos(theta), y + r * math.sin(theta)))
    return [math.floor(min(p[0] for p in points) - width), math.floor(min(p[1] for p in points) - width),
            math.ceil(max(p[0] for p in points) + width), math.ceil(max(p[1] for p in points) + width)]


def distance(shape, x, y):
    """Exact analytic point-to-centerline distance, minus its capsule radius."""
    kind, data, width = shape
    if kind == "arc":
        cx, cy, radius, angle, sweep = data
        phase = (math.atan2(y - cy, x - cx) - angle) % math.tau
        if phase <= sweep:
            gap = abs(math.hypot(x - cx, y - cy) - radius)
        else:
            gap = min(math.hypot(x - px, y - py) for px, py in [point(shape, 0), point(shape, 1)])
    else:
        ax, ay, bx, by = data
        dx, dy = bx - ax, by - ay
        norm = dx * dx + dy * dy
        t = min(1, max(0, ((x - ax) * dx + (y - ay) * dy) / norm)) if norm else 0
        gap = math.hypot(x - ax - t * dx, y - ay - t * dy)
    return gap - width


def generate(case, split):
    """Keep 14,336 owners, 64 layers and 4,000 intent IDs across matched geometry variants."""
    shapes, masks, nets = [], [], []
    for i in range(8192):
        x, y = (i % 128) * 1.27 * MM, (i // 128) * 1.27 * MM
        shapes.append(("line", [x, y, x, y], 300_000.5))
        masks.append(1 << (i % 64))
        nets.append(i % 4000)
    for i in range(4096):
        x, y = (i % 64) * 2.54 * MM, (i // 64) * 1.27 * MM
        kind = case
        if case == "mixed":
            kind = ["straight", "diagonal", "curves"][i % 3]
        if kind == "straight":
            shape = ("line", [x, y, x + (2 + i % 4) * MM, y], 125_000.5)
        elif kind == "diagonal":
            shape = ("line", [x, y, x + 80 * MM, y + (1 if i % 2 else -1) * 80 * MM], 125_000.5)
        else:
            if kind == "crowded_curves":
                x, y = 80 * MM + (i % 8) * 125_000., 40 * MM + (i % 7) * 125_000.
            shape = ("arc", [x, y, (2 + i % 39) * MM, (i % 16) * math.pi / 8,
                             [math.pi / 2, math.pi, 7 * math.pi / 4][i % 3]], 125_000.5)
        shapes.append(shape)
        masks.append(1 << (i % 64))
        nets.append((8192 + i) % 4000)
    for i in range(2048):
        x, y = (i % 64) * 2.54 * MM, (i // 64) * 2.54 * MM
        shapes.append(("line", [x, y, x, y], 300_000.5))
        masks.append(ALL_LAYERS)
        nets.append((12288 + i) % 4000)
    bounds, entry_masks, owners = [], [], []
    for owner, shape in enumerate(shapes):
        pieces = 8 if split and 8192 <= owner < 12288 else 1
        for p in range(pieces):
            bounds.append(envelope(shape, p / pieces, (p + 1) / pieces))
            entry_masks.append(masks[owner])
            owners.append(owner)
    probes, expected, intent_counts = [], [], []
    for layer in range(64):
        track = 8192 + layer + 64 * (layer % 64)
        if layer < 32:
            x, y = point(shapes[track], 0.5)
            label = "on_trace"
        elif layer < 48:
            b = envelope(shapes[track])
            x, y = (b[0] + b[2]) / 2, (b[1] + b[3]) / 2
            label = "envelope_interior"
        elif layer < 56:
            x, y = point(shapes[12288 + layer * 31], 0)
            label = "through_via"
        else:
            x, y = -400 * MM, -400 * MM
            label = "empty"
        mask = 1 << layer
        probes.append([label, [x - 500_000, y - 500_000, x + 500_000, y + 500_000], mask])
        expected.append([i for i, shape in enumerate(shapes) if masks[i] & mask and distance(shape, x, y) <= 700_000])
        intent_counts.append(len({nets[i] for i in expected[-1]}))
    return dict(simulation="analytic-index-stress/1", case=case, split=split, copper_layers=64,
                tracks=4096, vias=2048, pads=8192, net_identities=4000, layer_masks=entry_masks,
                owners=owners, owner_nets=nets, bounds=bounds, probes=probes, exact_near_ids=expected,
                distinct_near_intents=intent_counts,
                scope="Synthetic bounds and analytic circular-probe reference only; no native arc, routing, connectivity or ERC qualification")


def main():
    """Write diagnostics to a new directory without modifying native project documents."""
    root = Path(sys.argv[1])
    root.mkdir()
    for case in ["straight", "diagonal", "curves", "crowded_curves", "mixed"]:
        for split in [False, True]:
            if split and case not in ("diagonal", "curves", "crowded_curves"):
                continue
            name = case + ("-split8" if split else "")
            (root / f"{name}.json").write_text(json.dumps(generate(case, split), separators=(",", ":")) + "\n")
            print(name, flush=True)


if __name__ == "__main__":
    main()
