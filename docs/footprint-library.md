# Starter physical footprints

`libraries/starter/footprints` contains six original SMD definitions and a plated header under
MIT OR Apache-2.0. The engine embeds these definitions for shared command use;
the Parts card browses symbols while the package chooser offers footprints in both editors. `pcb.place_footprint` pins the
selected definition into the project and validates its terminal-to-pad mapping.
A saved project opens from that pinned copy without consulting the built-in
catalog or fetching its source URL.

## Assigning and placing a package

From a schematic symbol's context menu, choose **Assign Footprint…**. In PCB
layout, **Layout → Place → Footprint** opens the same chooser for the
project's components, including components not yet placed on the board. A
schematic-only project can assign packages before creating a PCB. Search
and choose a package, then review the editable pad choices for each logical terminal.
The chooser shows the library, item, revision, license and dimensional source.
**Assign** validates a complete bijection and pins the exact definition and map;
it creates no copper, placement ID or board coordinates. The schematic wiring
and reference stay unchanged.

Assignments survive folder/archive save and offline reopen without a board.
Creating the first board explicitly adopts those exact pinned choices without
placing them; Undo restores the schematic-only choices. In the PCB chooser,
an assigned, unplaced component offers unit-aware X/Y fields and **Place on
board**. Placement uses the pinned package, selects it and centers the PCB view.
The canvas's M/drag and R controls then move and rotate it normally. Choosing a
package and placing it are separate undo steps. A modified chooser must apply
its assignment before it can place; a stale form closes without changing data.

`pcb.assign_footprint` accepts an optional board UUID, a stable `component` ID,
`library`, `definition`, `definition_revision`, explicit `mapping` and
`expected_revision`. `pcb.place_assigned_footprint` takes the board/component,
`x_nm`, `y_nm` and revision, resolving solely from the project. Both use the same
engine command catalog in GUI, live and headless MCP. `inspect_project` reports
`project.footprint_assignments` with `placed` status and a null `board` for
schematic-only choices, and a compact `footprint_catalog` of
available built-in/project-pinned revisions. Legacy physical placements report
their effective assignment without changing saved bytes; marker-only legacy
placements remain explicit compatibility data.
Omit `board` when assigning before board creation. An explicit unknown board is
refused, and placement still requires a physical board. Existing board inspection
also retains its `board.assignments` projection.

A placed package cannot be silently reassigned: after the board has an accepted
state, **Assign** records a proposed package or mapping and **Update PCB from
schematic** reviews and applies it. Existing boards without recorded accepted
state first need explicit **Use current state** adoption in that dialog. Package
choices opened from an update review return to that review before placement;
accepted new parts enter a cursor group in PCB layout and remain unplaced until
its canvas click. Esc/right-click cancels geometry placement while retaining the
assignments. The individual coordinate-based chooser remains available separately.
Compatible terminal/pad numbers are preselected, including named terminals.
Exact revision-qualified library defaults handle known alternative numbering,
such as the starter LED's Anode → pad 2 / Cathode → pad 1. A saved mapping for
the selected package takes precedence. All defaults remain editable, and
incompatible numbering still requires an explicit mapping.
The Preferences-style mapping section separates **Symbol terminal** from
**Footprint pad** in aligned columns. **Match numbers** explicitly replaces the
mapping with equal identifiers, which can differ from a package's library default.
The chooser does not register
folders, import external footprints or create library parts. Those source and
library workflows remain under #92/#28. This slice supports the original starter
packages and physical definitions already pinned into an opened project.

## Packages and sources

Dimensions below are millimeters. Package names use imperial size codes: 0402
has a nominal 1.0 × 0.5 mm body, while 0603 is approximately 1.6 × 0.8 mm.
Copper dimensions describe land bounds, not a 3D model. SMD revision 2 uses
original rounded corners with radius 25% of the shorter land dimension; saved
revision 1 definitions retain their rectangular copper and graphics.

| Definition | Pad width × height | Pad centers | Dimensional reference |
|---|---|---|---|
| `resistor-0402` | 0.60 × 0.60 | X ±0.525 | [Vishay D/CRCW e3](https://www.vishay.com/docs/20035/dcrcwe3.pdf), revision 14-Apr-2026, p. 11, reflow land pattern |
| `resistor-0603` | 0.75 × 1.00 | X ±0.750 | Same Vishay reference, 0603 reflow land pattern |
| `capacitor-0402` | 0.40 × 0.50 | X ±0.400 | Original choice within [Kyocera's mounting land ranges](https://ele.kyocera.com/en/product/capacitor/ceramic/mlcc-handling/mounting-01/) |
| `capacitor-0603` | 0.80 × 0.80 | X ±0.800 | Same Kyocera reference, 1608 metric range |
| `led-0402` | 0.70 × 0.50 | X ±0.450 | [Kingbright APG1005CGC-T](https://www.kingbrightusa.com/images/catalog/spec/apg1005cgc-t.pdf), DSAM9856 V11B, 12-Mar-2025, p. 1 |
| `header-1x02-254` | 1.70 × 1.70, 1.00 drill | X ±1.270 | Original demo header; square pad 1 and circular pad 2 |
| `sot23` | 0.60 × 0.70 | 1: (−0.95, 1.00), 2: (0.95, 1.00), 3: (0, −1.00) | Original demo lands; body and 1.9 mm same-side pin pitch from [Nexperia SOT23](https://assets.nexperia.com/documents/outline-drawing/SOT23.pdf), 12-Oct-2022 |

These JSON definitions and vector graphics are original work from published
dimensional facts. No manufacturer artwork, PDF, third-party footprint library
or KiCad implementation is bundled. Courtyards, silkscreen and SOT-23 land sizes
are demo choices, without a manufacturing qualification claim.

The LED definition places pad 1 (cathode) on the left and pad 2 (anode) on the
right, with a cathode silkscreen mark. The starter LED symbol uses terminal 1
for anode and 2 for cathode, so the demo explicitly maps `{"1":"2","2":"1"}`.
The starter NPN symbol uses 1 Base, 2 Emitter and 3 Collector, matching
[Nexperia PMBT3904](https://assets.nexperia.com/documents/data-sheet/PMBT3904.pdf),
16-Feb-2024. Q1 in the demo is an unwired package example.

## Definition and placement contract

The [footprint format specification](footprint-schema.md) defines field ownership,
reference defaults and overrides, coordinate transforms and compatibility, with
machine-readable schemas for canonical definitions and physical placements.

A `pcbcraft-footprint/1` document has `library`, `id`, positive `revision`, `name`,
`license`, `source`, `pads` and `graphics`. `source` records `kind: "original"`,
`reference` and `locator`; it describes provenance and is never fetched on load.
Identity is the library/item/revision tuple, independently of the file's location.
A conflicting definition claiming an already-pinned identity is refused.

Pads have a unique `number` of 1–8 letters, digits, `+`, `-` or `_`, an
integer-nanometer `center`, positive `width_nm` and `height_nm`, whole-degree
counterclockwise `rotation` in 0…359 and a `shape` of `rectangle`,
`rounded_rectangle`, `circle` or `oval`. Rounded rectangles carry a positive
`corner_radius_nm` at most half the shorter dimension. Other shapes omit that
field (zero); circles require equal width and height. Surface lands use
`copper: "component_side"` without a drill. Plated through-hole lands use
`copper: "all_copper"` and a positive `drill_nm` smaller than both land dimensions.
Slots and arbitrary pad stacks remain unsupported. Definitions allow 1…128 distinct pads and up to 64
graphics. Graphics use the same rectangular geometry and a `purpose` of
`fabrication`, `silkscreen` or `courtyard`. Coordinates and dimensions are bounded
by 100,000,000 nm; unknown fields and invalid geometry fail validation.

Optional `artwork` adds up to 128 parameterized drawing objects. Each carries a
`purpose` (`fabrication`, `silkscreen`, `courtyard`) and `graphic` with editable
`geometry`, physical `stroke_nm` and `fill: "none"`. The shared geometry supports
lines, rectangles, circles, arcs, polylines and stroke text as described in the
[symbol format](symbol-library.md). Silkscreen paths are clipped against pad
clearance by the board renderer. Absent artwork is omitted on save, preserving
legacy definitions. These additions retain `pcbcraft-footprint/1`; old readers
will reject unknown fields/shapes rather than silently dropping them.

Placement supplies the built-in definition ID and an explicit `mapping` object
from symbol terminal ID to pad-number string. Mapping validation, placement,
board views and cross-probing preserve the complete identifier spelling, including
alphanumeric names and numeric-looking names such as `01`. Maps support up to 128
pads. Every terminal and every pad must appear exactly once. Two-pad callers may instead supply the legacy `pin_1_pad`
shortcut; it cannot be combined with `mapping`. Invalid assignments change
neither the project nor its undo history.

Board instances carry the pinned identity, mapping, origin and optional rotation;
geometry remains in the definition. Rotation transforms pad centers and every
rectangle corner together. Coordinates use board axes with positive Y down.
A 90-degree counterclockwise footprint rotation maps local `(x, y)` to `(y, -x)`
after side mirroring. Quarter turns retain exact integer coordinates, and other
whole-degree angles round to the nearest nanometer. Fit and hit bounds include
the physical graphics. Surface copper renders on the front/component
side, independently of the active layer; hiding front copper hides these SMD
pads. The engine projects each physical pad's copper membership as explicit layer
UUIDs in physical stack order. Plated lands span all copper layers, remain visible
while any occupied layer is visible, and retain their cut-out drills. They use the
active copper layer's color when it is visible, otherwise the first visible layer
in physical stack order. This does not move a surface pad onto another layer.
Front copper defaults to red and back to blue; internal layers alternate purple
and green defaults. Every layer swatch opens a shared color form with Apply,
Cancel and Reset to default. Overrides follow board/layer UUIDs through renaming,
are shared by views of the same board, and persist in local editor settings.
Engineering files, package definitions and document undo never contain those colors.
Unconfigured physical pads retain explicit component-side/all-copper compatibility
bindings, using front red/plated gold until a real stackup is authored; projection
never invents copper identities. Marker-only legacy pads still have no metal.
Technical graphics follow their corresponding visible
layers; new views hide fabrication and courtyard graphics by default. Existing
view arrangements preserve their saved visibility choices, and Layers can show
the detail graphics. Pad numbers use light or dark ink according to copper brightness,
sized and oriented to fit the land. Connected pads also show snapshot-local Net Inspector labels when both
lines fit; these labels are derived presentation and never persist net identity. With multiple
sheets, the annotation includes its owning sheet name and is independent of the
active schematic view.
Side flipping, mask/paste apertures, arbitrary shapes and general
library source registration remain future work.

The marker-only `generic-through-hole-2` compatibility definition is unchanged.
It does not acquire physical copper or drill geometry from its name. Existing
projects retain their original definitions and encoding until explicitly edited;
the updated [Dual LED demo](../examples/README.md) is a new physical example.
The [native storage guide](native-project-storage.md) specifies board versions 5–7
and pinned footprint paths.

## Silkscreen settings

In PCB **Properties → Silkscreen**, choose **Edit silkscreen settings** to set
text height, stroke width and pad clearance. Fields accept the current display
unit or an explicit suffix such as `1 mm` or `6 mil`. Apply makes one shared,
undoable board edit; Cancel discards the draft. The settings travel with the
project and apply across its PCB views, independently of application theme and
zoom. An edit prepared against an older revision is refused.

The initial style is 1.0 mm text height, 0.15 mm stroke width and 0.25 mm clearance
from copper. These are chosen defaults informed by
[JLCPCB's ordering instructions](https://jlcpcb.com/help/article/instructions-for-ordering)
(1 mm text and 0.15 mm lines) and its
[character design guidance](https://jlcpcb.com/blog/character-design-specifications)
(0.25 mm recommended pad clearance), consulted 7-Nov-2026. Settings permit other
positive text/stroke dimensions for other processes; they are design parameters,
not a substitute for manufacturing rules or DRC.

Starter SMD revision 4 and header revision 3 use `silkscreen` centerline paths,
with `points` in local nanometers and a `closed` flag. The `reference` field names
`pcbcraft-technical-1`, a smooth, proportional stroke font derived from the
OFL-1.1 [Relief SingleLine font](https://github.com/isdat-type/Relief-SingleLine).
Its [data, notices and conversion recipe](../crates/engine/assets/fonts/pcbcraft-technical/README.md)
ship with the engine. It includes real lowercase forms and uses a question mark
for unsupported characters. Reference text is derived from the component
reference and placed using its complete ink bounds, including Q's descender,
so pad-clearance clipping does not erase meaningful letter strokes. Text height
specifies nominal capital height; curves retain the font's optical overshoot.
Stroke width and minimum ink spacing remain physical quantities. The earlier
`pcbcraft-stroke-1` geometry and legacy rectangle graphics remain available for
older pinned definitions without changing their saved data.

The optional `reference_offset` adds a local X/Y displacement to the automatic
anchor. In the Footprint Editor, select **Definition**, enable **Generate
silkscreen reference**, and set the offsets in Properties. The canvas previews
the template as `REF`. A board placement can add its own offset, and its
component still supplies the printed reference text.

Selecting reference lettering opens **Silkscreen text** in PCB Properties. The
component reference remains visible alongside **Printed text**, font, size,
stroke width, local rotation, alignment, visibility and offsets. Clear **Use
component reference** to print local words such as `Power input`. These edits
belong to the placed footprint and its Gerber ink. **Rename… → Rename part**
explicitly changes the shared component reference through its validated command,
including the schematic. A local text literal retains its wording across a
component rename. **Reset silkscreen** restores inherited wording and appearance.
Body selection offers **Edit silkscreen text** to recover hidden lettering.
A dashed view-only leader connects selected or moving text to its footprint,
independently of unrouted electrical airwires. Selection lightens the copper and
technical-layer colors, including the generated reference and fixed silkscreen.
The Properties fields use aligned label/value columns; position offsets expand
under **Position**, and Apply/Reset remain below the scrolling fields in short
windows. Editing a dimension creates its local size override automatically.
Fixed footprint silkscreen paths and artwork still belong to the pinned library
definition; these reference controls do not provide individual board-instance
editing of those objects.

Passive footprints use two short silk lines outside the pads, LEDs a cathode
bar, and SOT-23 a clipped body outline and an external triangular pad-1 marker.
The earlier 0.08 mm hollow polarity boxes are absent from these revisions.
Before projection, the engine clips centerlines against copper exclusion bounds
expanded by clearance plus half the ink width. Curved and rotated lands use
conservative enclosing bounds. This maintains the requested copper clearance;
solder-mask apertures and manufacturing export remain separate work. Pad numbers
and net labels are editor annotations, independent of the printed silk settings.

The command `pcb.set_silkscreen` takes an optional board UUID, `expected_revision`
and a `settings` object with integer `text_height_nm`, `stroke_width_nm` and
`pad_clearance_nm`. Text height is positive and at most 10 mm, stroke width is
positive and at most half the height, and clearance is 0…10 mm. Invalid settings
change neither geometry nor history. The default style is a compatibility
constant; authoring a different style stores it under board format 6.
