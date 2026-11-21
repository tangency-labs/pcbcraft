# Exact circular copper records

Part of [#259](https://github.com/tangency-labs/pcbcraft/issues/259). Native
storage, validation and shared circular arc contact geometry for copper
connectivity, ratsnest, via inspection and short detection are available;
clearance queries, DRC, curved routing and native Gerber arcs remain follow-up
work.

`pcb.set_arc_tracks` replaces the complete arc list of the addressed board.
Supply `board`, `expected_revision` and `arcs`; `dry_run: true` validates and
returns the full `before` and `after` lists without changing the project,
revision or undo history. Apply is one undo step, and an equal list is a no-op.
Caller-supplied UUIDs make replay and external parametric updates explicit;
inspect the current revision before retrying a stale request. An empty list
removes all arcs. The engine command is shared by live and headless MCP; there
is no interactive curved-route tool yet.

Each record has these fields:

- `id`: non-nil UUID, unique across arcs, straight tracks and vias on this board.
- `layer`: an existing copper-layer UUID in this board's setup.
- `width_nm`: integer nanometres, from 10,000 through 10,000,000 inclusive.
- `center`, `start`, `end`: bounded integer-nanometre points, each with `x_nm`
  and `y_nm`, using the existing board coordinate range.
- `direction`: `clockwise` or `counterclockwise`, from start to end in board
  coordinates (X right, Y down). The direction chooses between complementary
  sweeps; opposite endpoints describe a semicircle in either direction.

All nanometre fields require JSON integer tokens. Fractional, exponent and
negative-zero spellings (for example `250000.0`, `2.5e5` or `-0`) are refused by
commands and native storage, even if their mathematical value is integral.
This preserves the existing point parser; JSON Schema's `type: integer` alone
cannot express that lexical restriction and is not complete numeric equivalence.

The circle is defined by center and start. Its radius is the exact square root
of their squared integer distance, not an independently editable saved value.
The end's radial distance may differ by at most 1 nm, inclusive, to accommodate
integer endpoints. Validation compares squared integer expressions, not rounded
radii or trigonometry; accepted coordinates are never snapped or rewritten.
Equal endpoints, either endpoint at the center, and distinct endpoints on the
same positive radial ray are rejected. Full circles are unsupported, rather
than interpreting equal endpoints as either zero or a complete turn.

A board holds at most 4,096 arcs, in UUID order. Unknown fields, explicit null,
invalid directions, duplicate identities, foreign/non-copper layers and invalid
dimensions fail before any project or history change. This is geometric record
validation, not a manufacturing tolerance or a claim that a route is safe.

## Supported and unsupported consumers

Board copper connectivity (`copper`, `copper_nets`, `copper_with_nets`),
ratsnest (`ratsnest`), via inspection (`inspect_vias`) and the engine short
detection primitive (`copper_shorts`) evaluate exact circular arc contact
geometry without chord substitution. Via inspection resolves derived `net` and
`connected_to_pad` across arcs while retaining authored `net_assignment`,
`assignment_mode` and registry `net_state`.

While any arc exists, board rendering/picking and PCB editing still explain
that circular copper is unsupported. Clearance queries, route-rule contact
queries, route planning, copper preparation, group transforms, clipboard
capture/paste and fabrication refuse instead of omitting arcs or treating their
endpoints as a straight segment. Session DRC refuses; the direct board-view
DRC report contains an `unsupported_geometry` finding and no partial checks.
Inspection exposes the exact `arcs` list and `geometry_supported: false`.
Logical schematic inspection remains independent of physical geometry.

Saving, reopening, undo/redo and `pcb.set_arc_tracks` remain available. Native
folder/archive saves preserve exact records. The diagnostic single-document
writer refuses arcs. Clearing the last arc restores the format required by the
remaining board data. Arc-bearing boards use format 26 unless they also have
an active footprint lock, in which case they use format 35. Both formats retain
the released profile-adoption (22),
extended-stack (23), explicit via-intent (24), thermal/insulation material-report
(25) and earlier board fields without changing those records. Boards without arcs keep the format and bytes
required by their remaining authored data.

Clearance queries, full DRC, hit testing, snapping, bounded render
approximations, transforms, clipboard and native Gerber arcs remain follow-up
work. There is no flattening or polyline conversion in this contract.

## External updates

These are ordinary board-owned records under the existing immutable project
snapshot and shared revision-checked command path, not a second geometry store.
An external caller can retain UUIDs, inspect and dry-run an exact replacement,
and submit only against the revision it read. SolidCraft/FieldCraft exchange
must use the same board identity and snapshot authority. A future externally
driven outline update remains an outline edit, not an implicit change to these
arc records. No plugin protocol or simulation bridge is introduced here.
