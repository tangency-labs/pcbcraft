# Symbol libraries

A symbol library holds **schematic symbols**: what the schematic draws for a part
and where its pins connect. Layout **footprints** are separate definitions in
their own libraries. A symbol file never names a footprint or a pad; a part
links the two later, by assigning a footprint and a pin-to-pad mapping (issue
#28). This document covers symbols only.

## Bundled libraries

PCBcraft ships **Default** (`libraries/starter`, retaining the stable `starter` ID)
and **Power** (`libraries/power`). Both use the native source loader, catalog and
ordinary symbol instances. Build-time embedding supplies their native documents
to desktop and browser; folder and portable-bundle loading use the same parser.
The definitions are original work under MIT OR Apache-2.0.

```text
libraries/starter/
  library.json        manifest: format, id, name, license
  symbols/
    capacitor.json    latest symbol revision
    connector.json
    header-4.json
    led.json
    npn.json
    resistor.json
    *-v1.json         preserved revision 1 definitions
```

The manifest:

```json
{"format": "pcbcraft-library/1", "id": "starter", "name": "Default", "license": "MIT OR Apache-2.0"}
```

New placements use the highest loaded revision of each symbol ID. The browser
lists that revision once; older revisions stay available for projects that pin
them. Revision 2 shortens the default leads while keeping the pins on the grid:
resistor endpoints are 5.08 mm from center, capacitor endpoints 2.54 mm, and LED
and connector endpoints 3.81 mm. The original `*-v1.json` files retain the former
6.35 mm endpoints. Changing a drawing or pin position requires a new revision
so reopening an older project cannot silently move its connections. Resistor and
capacitor revision 3 retain revision 2 geometry but use neutral `R` and `C` values
for new generic placements. The `*-v2.json` copies preserve the former `1k` and
`100nF` defaults for pinned revisions and migration.

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
| `terminals` | 1 to 128 logical connections, each with a stable `id` (the pin number: 1 to 8 letters, digits, `+`, `-` or `_`, unique in the symbol), an optional `name` up to 32 characters and an `electrical_type`: `passive`, `input`, `output`, `bidirectional`, `power_in`, `power_out`, `tri_state`, `open_collector`, `open_emitter`, `no_connect` or `unspecified`. Wires and footprints refer to a pin by its `id`. |
| `units` | Drawn sections of the symbol. Each has an `id`, a `drawing` and `pins` that place each terminal. |

Drawing strokes are `line` (`from_nm`, `to_nm`), `polyline` (`points_nm`, 2 to
64 points; repeat the first point to close it), `rect` (`min_nm`, `max_nm`),
`circle` (`center_nm`, `radius_nm`) and `text` (`at_nm`, `text` of 1 to 16
characters, and `align`: `start`, `middle` or `end`). Text labels such as pin
names stay upright when the symbol turns: a half turn swaps `start` and `end` so
a label inside a body edge stays inside, and a quarter turn centers it. Each
drawing has at most 256 strokes, and every point stays within 100 mm of the
center.

Pins may add optional `appearance`: `orientation` (`right`, `down`, `left`, `up`)
points from attachment toward the body; `length_nm`, `style` (`line`, `inverted`,
`clock`, `inverted_clock`), `show_name`, `show_number`, `name_height_nm` and
`number_height_nm` control their integrated rendering. Omission preserves legacy
separately drawn leads and labels without changing saved bytes.

A `graphic` drawing entry wraps a shared `graphic` object with `geometry`,
`stroke_nm` and `fill` (`none`, `solid`, `background`). Geometry is a tagged `type`
of `line`, `rectangle`, `circle`, `arc`, `polyline` or `text`; coordinates use
`Point` objects (`x_nm`, `y_nm`). Arcs store center/radius/start/sweep; polylines
store points and `closed`; text stores anchor/content/height/rotation/alignment.
Parameterized text permits 128 characters, closed fills require convex geometry,
and physical stroke widths must be positive and at most 2 mm. These optional
additions use `pcbcraft-symbol/1`; old entries retain their original serialization.

Unknown fields are refused, so a misspelled field can't be silently ignored.
A file that fails any check is left out of the library with a message naming
the file and the reason; the rest of the library still loads.

## What this build supports

The native parser and placement commands support:

- **1 to 128 logical terminals**, each appearing in at least one unit. The
  legacy two-pad footprint only maps symbols with exactly pins `"1"` and `"2"`.
  [Physical footprints](footprint-library.md) validate complete terminal and pad coverage
  with exclusive pad ownership. Multiple pads may map to one terminal, and other
  pads can be explicitly unconnected;
  the starter NPN uses three SOT-23 pads.
- **1 to 32 units in format 4**, including shared supply-terminal appearances.
  Each unit is placed once per part, and units on different sheets retain one
  reference, Value and physical package. Formats 1–3 retain one unit. See the
  [multi-unit contract](symbol-schema.md#multiple-schematic-units).
- **Pins** on the 1.27 mm grid, at most 12.7 mm from the center on each axis,
  each at its own position. Symbol centers stay 12.7 mm inside the sheet edge so
  every pin stays on the sheet.

## Native sources

**Libraries → Sources** now loads native folders on desktop and portable bundles
on desktop/browser. The [source guide](native-library-sources.md) covers
registration, search, revision conflicts and offline pinning. External symbols
retain their manifest library name and license as portable attribution with
the saved definition.

## Not yet

- **Other libraries.** Git, databases and managed libraries,
  plus import from other formats, are later slices of issue #28.
  [library-sources.md](library-sources.md) proposes the model for them.
- **Footprint libraries** follow the same approach, as their own format and
  files.

## Electrical behavior and defaults (`pcbcraft-symbol/2`)

Version 2 accepts the version 1 drawing/terminal vocabulary and optionally adds
`behavior: {"type":"power","terminal":"1"}` and `defaults` containing ordinary
component `metadata` and unit `field_layout`. The declared terminal must exist;
its global electrical name comes from the placed component's Value. A custom
library can use this role regardless of its names or drawings. Defaults can be
supplied by any definition and remain editable on each instance. Version 1
rejects these fields rather than silently ignoring them.

The [bundled-library contract](builtin-symbol-libraries.md) describes source
identity, instance behavior and removal of the old power-marker representation.

## Published schemas and explicit categories (`pcbcraft-symbol/3`)

[The symbol schema contract](symbol-schema.md) publishes machine-readable schemas
for versions 1–4. Version 3 adds an explicit default category and optional primary
quantity-field binding, retained by native loading, authoring export and pinned
symbol save/reopen. Versions 1–2 reject that field and preserve their existing
encodings. Category lookup uses exact native identities; names/drawings never
determine a symbol's units. See [part-schema.md](part-schema.md) for reusable
part definitions, typed category fields and canonical quantities.
