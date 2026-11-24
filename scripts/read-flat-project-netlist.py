#!/usr/bin/env python3
"""Read connectivity from a trusted, unpacked project/6 with one flat sheet/1."""

import argparse
from collections import defaultdict
import json
from pathlib import Path
import re


def require_int_id(value, label):
    """Require an integer counter ID; floats and booleans must not alias integers."""
    if type(value) is not int:
        raise ValueError(f"expected integer {label}: {value!r}")
    return value


def unique_object(pairs):
    """Reject duplicate JSON object keys rather than keeping the last value."""
    fields = dict(pairs)
    if len(fields) != len(pairs):
        raise ValueError("JSON object must not contain duplicate keys")
    return fields


def read_netlist(read_document):
    """Project the saved parts, pins and wire membership, without engine code."""
    project = read_document("project.json")
    if project["format"] != "pcbcraft-project/6" or len(project["sheets"]) != 1:
        raise ValueError("expected project/6 with exactly one sheet")
    sheet_ref = project["sheets"][0]
    sheet_id = sheet_ref["id"]
    sheet_path = f"sheets/{sheet_id}.json"
    if not re.fullmatch(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}", sheet_id) or sheet_ref["file"] != sheet_path:
        raise ValueError("invalid sheet reference")
    sheet = read_document(sheet_path)
    if sheet["format"] != "pcbcraft-sheet/1" or sheet["id"] != sheet_id:
        raise ValueError("expected the referenced sheet/1")

    parts, pins_by_part = {}, {}
    for component in project["components"]:
        library, definition = component["library"], component["definition"]
        revision = component["definition_revision"]
        if not all(re.fullmatch(r"[a-zA-Z0-9_-]+", segment) for segment in (library, definition)) or type(revision) is not int or revision < 1:
            raise ValueError("invalid pinned symbol address")
        symbol = read_document(f"library/symbols/{library}/{definition}.r{revision}.json")
        if (symbol["format"], symbol["library"], symbol["id"], symbol["revision"]) != (
            "pcbcraft-symbol/1", library, definition, revision
        ) or len(symbol["units"]) != 1 or symbol.get("behavior") is not None:
            raise ValueError("expected a pinned, ordinary single-unit symbol/1")
        unit, = symbol["units"]
        terminals = [[pin["id"], pin.get("name"), pin["electrical_type"]] for pin in symbol["terminals"]]
        pin_ids = [pin["terminal"] for pin in unit["pins"]]
        terminal_ids = {pin[0] for pin in terminals}
        pins = set(pin_ids)
        if len(terminal_ids) != len(terminals) or len(pins) != len(pin_ids) or pins != terminal_ids:
            raise ValueError("expected each logical terminal to appear once in the definition and unit")
        part = require_int_id(component["id"], "component ID")
        if part in parts:
            raise ValueError("duplicate component ID")
        parts[part] = [part, component["reference"], component["value"], sorted(terminals, key=lambda pin: pin[0])]
        pins_by_part[part] = (unit["id"], pins)

    owners = {}
    for symbol in sheet["symbols"]:
        symbol_id = require_int_id(symbol["id"], "symbol ID")
        owner_id = require_int_id(symbol["component"], "symbol component ID")
        if symbol_id in owners:
            raise ValueError("expected exactly one placement per part")
        owners[symbol_id] = owner_id
    if sorted(owners.values()) != sorted(parts):
        raise ValueError("expected exactly one placement per part")
    if any(symbol["unit"] != pins_by_part[symbol["component"]][0] for symbol in sheet["symbols"]):
        raise ValueError("placement names an unknown unit")
    placed_pins = {(part, pin) for part, (_, pins) in pins_by_part.items() for pin in pins}
    junctions = set()
    for junction in sheet["junctions"]:
        junction_id = require_int_id(junction["id"], "junction ID")
        if junction_id in junctions:
            raise ValueError("duplicate junction ID")
        junctions.add(junction_id)
    edges = defaultdict(set)
    for wire in sheet["wires"]:
        a, b = (endpoint(wire[end], owners, placed_pins, junctions) for end in ("from", "to"))
        edges[a].add(b)
        edges[b].add(a)

    unseen, nets, connected = set(edges), [], set()
    while unseen:
        pending = [unseen.pop()]
        terminals = []
        while pending:
            node = pending.pop()
            if node[0] == "pin":
                terminals.append(node[1:])
            adjacent = edges[node] & unseen
            unseen.difference_update(adjacent)
            pending.extend(adjacent)
        if terminals:
            nets.append(sorted(terminals))
            connected.update(terminals)
    return {"parts": [parts[part] for part in sorted(parts)], "nets": sorted(nets), "unconnected": sorted(placed_pins - connected)}


def endpoint(record, owners, placed_pins, junctions):
    """Translate an explicit saved endpoint; coordinates and bends do not join wires."""
    if isinstance(record, dict):
        if record.keys() == {"symbol", "pin"} and type(record["symbol"]) is int and record["symbol"] in owners:
            terminal = owners[record["symbol"]], record["pin"]
            if terminal in placed_pins:
                return ("pin", *terminal)
        if record.keys() == {"junction"} and type(record["junction"]) is int and record["junction"] in junctions:
            return ("junction", record["junction"])
    raise ValueError(f"unsupported or unresolved wire endpoint: {record!r}")


def count_fields(pairs):
    """Reject ambiguous source counts rather than keeping the last duplicate key."""
    fields = dict(pairs)
    if len(fields) != len(pairs):
        raise ValueError("expected counts must not contain duplicate keys")
    return fields


def compare_counts(netlist, expected):
    """Compare independently supplied logical counts; refuse invalid counts or any mismatch."""
    actual = {
        "parts": len(netlist["parts"]),
        "pins": sum(len(part[3]) for part in netlist["parts"]),
        "nets": len(netlist["nets"]),
    }
    if not isinstance(expected, dict) or expected.keys() != actual.keys() or any(
        type(count) is not int or count < 0 for count in expected.values()
    ):
        raise ValueError("expected counts must contain exactly parts, pins and nets as nonnegative integers")
    mismatches = [f"{key}: expected {expected[key]}, got {value}" for key, value in actual.items() if expected[key] != value]
    if mismatches:
        raise ValueError("count mismatch: " + "; ".join(mismatches))
    return actual


def main():
    """Print the projection or verified counts; report errors without a success result."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("folder", type=Path, help="trusted project directory, not a .pc archive")
    parser.add_argument("--expected-counts", type=Path, help="compare to source-side JSON with parts, pins and nets")
    args = parser.parse_args()
    try:
        result = read_netlist(
            lambda path: json.loads((args.folder / path).read_text(encoding="utf-8"), object_pairs_hook=unique_object)
        )
        if args.expected_counts is not None:
            expected = json.loads(args.expected_counts.read_text(encoding="utf-8"), object_pairs_hook=count_fields)
            result = compare_counts(result, expected)
    except (OSError, ValueError, TypeError, KeyError) as error:
        parser.exit(1, f"{parser.prog}: {error}\n")
    print(json.dumps(result, ensure_ascii=True, indent=2))


if __name__ == "__main__":
    main()
