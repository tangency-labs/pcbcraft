# Symbol libraries

A symbol library holds **schematic symbols**: what the schematic draws for a part
and where its pins connect. Layout **footprints** are separate definitions in
their own libraries. A symbol file never names a footprint or a pad; a part
links the two later, by assigning a footprint and a pin-to-pad mapping (issue
#28). This document covers symbols only.

## Starter library

PCBcraft ships one library, `libraries/starter`. The engine embeds it at build
time, so the desktop and browser builds offer the same symbols without reading the
file system. Every `*.json` file in `libraries/starter/symbols` is included, in
file-name order. Adding a symbol is adding a file; no code changes. The symbols
are original work under the project's MIT OR Apache-2.0 license.

```text
libraries/starter/
  library.json        manifest: format, id, name, license
  symbols/
    capacitor.json    latest symbol revision
    connector.json
    header-4.json
    led.json
    resistor.json
    *-v1.json         preserved revision 1 definitions
```

The manifest:

```json
{"format": "pcbcraft-library/1", "id": "starter", "name": "PCBcraft starter", "license": "MIT OR Apache-2.0"}
```

New placements use the highest loaded revision of each symbol ID. The browser
lists that revision once; older revisions stay available for projects that pin
them. Revision 2 shortens the default leads while keeping the pins on the grid:
resistor endpoints are 5.08 mm from center, capacitor endpoints 2.54 mm, and LED
and connector endpoints 3.81 mm. The original `*-v1.json` files retain the former
6.35 mm endpoints. Changing a drawing or pin position requires a new revision
so reopening an older project cannot silently move its connections.

## Symbol format `pcbcraft-symbol/1`

Coordinates are integer nanometers from the symbol's center, with y growing
downward, before any rotation. The schematic grid is 1.27 mm (1,270,000 nm).

```json
{
  "format": "pcbcraft-symbol/1",
  "id": "led",
  "revision": 1,
  "name": "LED",
  "reference_prefix": "D",
  "default_value": "LED",
  "terminals": [
    {"id": "1", "name": "Anode", "electrical_type": "passive"},
    {"id": "2", "name": "Cathode", "electrical_type": "passive"}
  ],
  "units": [
    {
      "id": "main",
      "drawing": [
        {"type": "line", "from_nm": [-6350000, 0], "to_nm": [-1651000, 0]},
        {"type": "polyline", "points_nm": [[-1651000, -1905000], [1651000, 0], [-1651000, 1905000], [-1651000, -1905000]]}
      ],
      "pins": [
        {"terminal": "1", "at_nm": [-6350000, 0]},
        {"terminal": "2", "at_nm": [6350000, 0]}
      ]
    }
  ]
}
```

| Field | Meaning |
|---|---|
| `format` | The schema version, which tells PCBcraft how to read the file. A different version is refused with its name. |
| `id` | Stable ID used by `schematic.place_symbol` and saved in projects: 1 to 64 lowercase letters, digits and inner hyphens, unique together with its revision in the library. |
| `revision` | The library's revision of this symbol, 1 or more, tracked separately from `format`. Projects pin it on each component. |
| `name` | Shown in the parts browser, 1 to 64 characters. |
| `reference_prefix` | 1 to 4 capital letters that start automatic references, such as `R`. |
| `default_value` | The value a newly placed symbol shows, 1 to 64 characters. |
| `terminals` | 1 to 128 logical connections, each with a stable `id` (the pin number: 1 to 8 letters, digits, `+`, `-` or `_`, unique in the symbol), an optional `name` up to 32 characters and an `electrical_type`: `passive`, `input`, `output`, `bidirectional`, `power_in`, `power_out` or `unspecified`. Wires and footprints refer to a pin by its `id`. |
| `units` | Drawn sections of the symbol. Each has an `id`, a `drawing` and `pins` that place each terminal. |

Drawing strokes are `line` (`from_nm`, `to_nm`), `polyline` (`points_nm`, 2 to
64 points; repeat the first point to close it), `rect` (`min_nm`, `max_nm`),
`circle` (`center_nm`, `radius_nm`) and `text` (`at_nm`, `text` of 1 to 16
characters, and `align`: `start`, `middle` or `end`). Text labels such as pin
names stay upright when the symbol turns: a half turn swaps `start` and `end` so
a label inside a body edge stays inside, and a quarter turn centers it. Each
drawing has at most 256 strokes, and every point stays within 12.7 mm of the
center.

Unknown fields are refused, so a misspelled field can't be silently ignored.
A file that fails any check is left out of the library with a message naming
the file and the reason; the rest of the library still loads.

## What this build supports

The format leaves room for more than the engine places today:

- **Any number of terminals**, each placed by exactly one pin of the unit. The
  starter two-pad footprint only maps symbols with exactly pins `"1"` and `"2"`;
  other parts have no footprint until footprint libraries exist.
- **One unit.** Multi-unit symbols, such as a dual op-amp, are planned (issue #28).
- **Pins** on the 1.27 mm grid, at most 12.7 mm from the center on each axis,
  each at its own position. Symbol centers stay 12.7 mm inside the sheet edge so
  every pin stays on the sheet.

## Not yet

- **Pinning copies into projects.** Projects save each component's `definition`
  ID and `definition_revision`, and a project opens only when the library has
  that exact revision. Unit placements reference their component and the library-defined
  unit ID; they contain only those links and placement geometry. Saving a full
  copy of each used symbol and its source, so a project reopens without the
  library, comes with libraries outside the app.
- **Other libraries.** Library folders, Git, databases and managed libraries,
  plus import from other formats, are later slices of issue #28.
- **Footprint libraries** follow the same approach, as their own format and
  files.
