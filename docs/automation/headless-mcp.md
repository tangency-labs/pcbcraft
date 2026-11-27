# Headless MCP

`nets.promote` and `nets.resolve_ambiguity` use the shared revision-checked
transactions in headless and live sessions. Both take `expected_revision` and
an `anchor`, using an existing terminal/label/port endpoint or a complete typed
anchor from `project.net_registry.nets`. Address `sheet` when multiple sheets
exist; an optional `board` must match the current owner. Resolution also takes
the ambiguous source UUID as `net` and assigns it to the selected current
candidate branch. Promotion of an already persistent network is a no-op.
Bare pins and unnamed one-pin stubs cannot be promoted. Inspection reports saved
IDs, anchors, lineage and decisions only when a registry exists, and never
creates records. See the [storage and command contract](../formats/net-registry-storage.md)
for scope and version details. There are no new editor controls in this slice.

`pcb.add_zone`, `pcb.set_zone` and `pcb.delete_zones` edit
[zone outlines](../pcb/copper-zone-outlines.md#shared-commands) through the same revision-checked
transactions as PCB Layout. A derived typed net target is promoted in the same
undo step as creation; inspection reports authored zone records and current net
identity state. These commands do not create conductive fill.

`pcb.move_items` takes `board`, `expected_revision`, a mixed `items` selection
and integer `dx_nm`/`dy_nm` displacements. It translates the complete selection in
one transaction, preserving identities and carrying unselected track ends on moved
pads. Explicitly selected tracks translate once. Unknown/locked objects, stale
revisions, out-of-range geometry or a new physical short refuse the complete move.

`pcb.delete_items` also accepts zone UUIDs in `items.zones` and deletes mixed
board selections in one transaction. Copy, Cut and Duplicate refuse a selection
containing zones until zone clipboard support is available; they never omit the
zones and operate on a subset.

`pcb.add_drawing`, `pcb.set_drawing` and `pcb.delete_drawing` use the same
transactions as canvas Board Outline editing. Each takes `board` and
`expected_revision`; creation also takes a `layer` UUID, ordered `points`
(`x_nm`, `y_nm` integers) and `closed`. Updating takes `drawing`, `points` and
`closed`; deleting takes `drawing`. Inspection exposes layer metadata and drawing
identities at `project.boards[].layers` and `.drawings`. Paths contain 2–256
vertices (at least three when closed), without a repeated closing vertex. Only
Board Outline layers accept these graphics in this slice. Open paths are allowed,
without a manufacturing or clearance claim.

`pcb.add_tracks` and `pcb.delete_tracks` use the same transactions as the PCB
route tool. Each takes `board` and `expected_revision`. Routing also takes a copper
`layer` UUID from `project.boards[].layers`, optional `width_nm` (10,000 to 10,000,000) and
2–256 ordered `points`; each neighboring pair becomes one straight segment, all in
one undo step. Optional `layer_changes`, a list of `{at, layer}` in increasing
vertex order after the first, places a through via at `points[at]` and continues on
that copper layer; a change at the last vertex ends the route on a via. Omitted
width, via diameter and drill use the board routing preferences, including layer
changes; explicit drills must be smaller than their pads.

`pcb.unroute_connection` takes `board`, `expected_revision`, and `from` and `to`
addresses such as `{"footprint":"footprint-7","pad":"2"}`. Pick `from` on the
arm to remove and `to` on the retained side. `execute` returns `revision`,
`dry_run`, `removed` (sorted `tracks` and `vias` UUID lists), `requested` (the
input pair), and `unrouted` (new ratsnest edges from the change to authored
copper, as pairs of pad addresses), even with project inspection disabled. An
edge may end at a different pad on the retained side than the requested `to`.
Refilled zones can reconnect the pads and clear those edges. Without accepted
connectivity there are no edges to report. Set `dry_run: true` to get the same
report without changing copper, revision or undo. A stale revision returns `stale_revision` with `current_revision` in
`details`; a missing copper endpoint returns `not_found`. See
[Unroute one connection](../pcb/unroute.md#unroute-one-connection) for the
behavior and other refusal cases.

`pcb.check_design` checks the addressed board without editing it; `execute` returns
`design_check` with the `rules` used, `violations` (each with a `kind`, `message`,
representative `at` point and `items`), `counts` per kind and the `total`, listing at
most 1000 findings. Saved board defaults apply unless a request supplies a
supported hard-limit override; authored minimums can only tighten upward and
maximums downward. The report includes per-property `sources`. Kinds are `short`,
`clearance`, `track_width`, `via_diameter`, `via_drill`, `annular_ring`, `unconnected`,
`no_outline`, `ambiguous_outline`, `outside_board`, `edge_clearance` and `courtyard_overlap`.
Closed Board Outline drawings whose boundaries cross, touch or share an edge are grouped into one
union component and reported as `ambiguous_outline` (listing those drawings' UUIDs), while
disjoint closed outlines still nest by even-odd parity so a cut-out away from an overlap remains
a cut-out. An `outside_board` finding for a copper item or
non-plated hole inside a cut-out also lists the cut-out drawing's UUID after that item. A
`courtyard_overlap` lists both footprints and marks a point inside the overlap; touching
courtyards pass. When unsupported courtyard geometry (circles, arcs, open paths) decided
the finding by bounds, its message says the comparison is conservative.

See [fabrication through MCP](../manufacturing/fabrication-mcp.md) for board output.

`project.export_netlist` generates the project's netlist without editing it;
`execute` returns `netlist.files`, one `<project name>-netlist.json` (the project name made portable as for the
fabrication files) with its `name`
and text `contents`, and `netlist.findings` for unplaced units and nets without a
persistent ID. `dry_run: true` returns only the file name and the findings. The
document lists every component with its logical terminals and every net across all
sheets with its persistent UUID and joined terminals; see the
[netlist format](../formats/netlist-format.md). Exporting an unchanged project gives the same bytes.

`pcb.export_netlist` takes `board` and `dry_run` and returns the same shape for
one board: `<project name>-<board name>-netlist.json`, format `pcbcraft-netlist/2` with `basis: accepted`, the
components, packages, terminal pads and nets the board accepted at its last
reviewed update. Findings report nets and components the schematic has changed
since (`pending_net`, `pending_removal`) and a `pending_update` summary matching
`pcb.preview_update` when comparison is available, or `update_unavailable` otherwise.
It fails for a board with footprints but no accepted baseline. `dry_run: true`
returns the filename and findings without rendering the file text. If the full
reply exceeds the live editor's 16 MiB frame limit, save the project and use the
headless command for the full export.

`project.compare_netlist` takes a netlist file's text as `netlist` and compares
it with what the open project exports now, the project netlist or, for a board
file, that board's accepted netlist, without editing anything. `execute` returns
`comparison`: `matches`, `identical`, the counts on both sides, `differences`
such as a terminal on another net (named with its net in the design and the
file's net it is on), and `notes` for renamed nets and other changes that leave connectivity
alone. Terminals are matched by part and terminal, never by net name or
position; see [comparing a file with the design](../formats/netlist-format.md#comparing-a-file-with-the-design).

`pcb.set_copper_locked` requires an explicit `board` UUID, `expected_revision`,
`locked` boolean and at least one of `tracks`/`vias` (unique UUID arrays). It locks
or unlocks the whole selection in one undo step, with no geometry or connectivity
change. `project.boards[].tracks[]` and `.vias[]` always report `locked`, including
false; the typed board snapshot exposes the same flag. Geometry, size and deletion
commands refuse the whole edit when they would modify existing locked copper,
including indirect footprint/segment transforms. No-op operations remain no-ops.
Copy/paste preserve locks on new identities; Cut of protected copper is refused.

`pcb.merge_tracks` takes an explicit `board`, `expected_revision` and `tracks` (unique
UUIDs) and merges the listed unlocked segments that share a layer and width and lie on
one line: a segment that lies wholly within another (a stacked duplicate, or a shorter
segment such as `[2,8]` inside `[0,10]`) is absorbed even beside a junction, via or pad,
and two segments join where they meet at an end that no third track end, via or pad
touches; a partial overlap such as `[0,10]` and `[5,15]` alone stays two tracks. The
lowest UUID survives with the merged ends and keeps its direction, even when it is the
shorter segment, and every other merged UUID is deleted and listed in `removed`; the
copper outline and connectivity are unchanged.
The result is `revision` plus `merge_tracks`: `before` and `after` track records,
`removed` and `reshaped` UUIDs, the `locked` segments left alone, `changed` and
`dry_run`. With `dry_run` true, or when nothing merges, revision and history stay
unchanged; otherwise the merge is one undo step. A report that would encode to more
than 15 MiB is refused with `cap_exceeded` before anything changes, dry run or not, so
the reply always fits the live editor's limit; merge fewer tracks at a time.

`pcb.add_via` places one standalone through via from `at`, with optional `via_diameter_nm` and
`via_drill_nm`, in one undo step, refusing a second via at the same position or a pad
that would join two nets. `pcb.move_tracks` moves `tracks` and/or `vias` by `dx_nm` and
`dy_nm`, keeping sizes and layers. `pcb.reshape_tracks` sets new `start` and `end` points for each listed `track` at once,
keeping layers and widths; a collapsed segment is refused. `pcb.set_track_width` gives
`tracks` one `width_nm`,
and `pcb.set_via_size` gives `vias` one `via_diameter_nm` and `via_drill_nm`. Each is one
undo step, checks shorts for the copper it changes, and is not an edit when nothing
changes. Deleting takes `tracks` and/or `vias`, unique UUIDs from `project.boards[].tracks`
and `.vias`; at least one item is required, and deleting a via keeps its tracks. A route is refused whole when a point is out of range,
neighbors repeat, the layer is not copper in the board setup, or its copper would
join pads on two different nets (the error names both). Only copper the new route
touches is checked, so a short left by an earlier edit does not block unrelated routes.
Copper connects where it overlaps on a shared layer, a via joins the copper it overlaps
on every layer, and the board ratsnest drops airwires between terminals already joined
by tracks. Routing does not check clearance; use `pcb.check_design` for that.

`pcb.plan_route` plans a route without editing: `from`, `to` and a copper `layer`, with
optional `width_nm`, `posture` (`diagonal` or `orthogonal`), `flipped`, `previous`,
`budget` and `expected_revision`. `execute` returns `route_plan` with the resolved
width, `points`, an `outcome` (`reached`, `blocked` with `last_legal`, `incomplete`,
`canceled` or `stale`), any `blocker` and the `probes` charged, also with
`inspect: false`. The points go round existing copper with the effective width,
pair clearances and board-edge clearance. Check that there are at least two points
before passing them to `pcb.add_tracks` with the same `layer` and `width_nm`;
`from` equal to `to` produces a one-point `reached` plan with nothing to add.
For track-only shove use `mode: "shove"`. Use `"shove_vias"` to move through
vias and their attached track ends. Submit the reached plan's `replacements`
and `via_moves` together with the route and its captured revision. For a
connected drag, `pcb.reshape_tracks` accepts the same shove modes and computes
the displacements during preparation. A shove drag accepts up to 256 directly
dragged segments. If the board has an authored zone, `pcb.plan_route` keeps vias
fixed in `shove_vias` mode and `pcb.reshape_tracks` rejects both shove modes. See the
[route planning and drag contract](../pcb/route-planning.md#through-vias-and-connected-segment-drags).

Live `view.set_layer` uses the same Layers handler for `active` and `visible` changes on an addressed view; those
presentation changes do not alter project revision or undo.
`view.set_board_flipped` takes `view` and `flipped` and shows that board view from below
or above through the same handler as **Design → Flip view**, keeping the centered point
and zoom; `inspect_editor` reports `board.flipped` per view. It changes no project data.
`view.set_layer_visibility` takes `view` and `mode` (`all` or `active`) through the
same handler as Show all / Isolate active. Isolation reveals the active layer,
hides the others and cancels unfinished drawing in that view.
`view.set_layer_color` takes `view`, `layer` (a layer UUID) and `color`
(`#RRGGBB`, or null to reset). It uses the swatch form's handler and shares the
personal board/layer palette across all views. `inspect_editor` reports effective
`board.layer_colors` per view, including whether each value is overridden.
The earlier `board.copper_colors` subset remains for compatibility.
`view.set_net_color` takes `view`, an active persistent `net` UUID and `color`
(`#RRGGBB`, or null to use the layer palette). The shared personal color applies
across layers. `view.set_net_colors_visibility` switches the addressed view
between net and layer colors. `inspect_editor` exposes the saved overrides in
`board.net_colors` and the display setting in `board.net_colors_visible`.
Effective net and layer color changes enter the live editor’s Undo and Redo
sequence. They persist locally and leave the project revision and files
unchanged. Physical pad inspection also exposes `pins[].copper_layers`: `resolved` includes the
binding and stack-ordered layer UUIDs; `unconfigured` retains the explicit library
binding without invented layers. Marker-only pins omit physical membership.
`view.navigate_layer` takes `view` and `direction` (`next`, `previous`, `front`,
`back`) through the keyboard's handler. Next/previous wrap in picker order across
technical and hidden layers; front/back resolve physical copper side metadata.
It preserves visibility, project revision and undo, and rejects unknown views or
unavailable copper sides before changing anything.

To start the MCP server from the repository root, run
`cargo run --locked -p pcbcraft-cli -- mcp`. To connect an MCP client, point it
at `pcbcraft-cli mcp`. Pass
`--project path/to/project.json` (or a folder or `.pc`) after `mcp` to start with a saved snapshot. The
headless session is separate from the app window. Send `inspect: false` with
`execute` to omit inspection and return the revision for ordinary edits.
Commands that return their own data, such as exports, clipboard queries and
[schematic annotation preview](../schematic/annotation.md), retain that data.
`schematic.set_symbol_fields` with `dry_run: true` still returns the full
preview, including the changed native documents. Use `annotate.preview` as the
read-only dry run, then `annotate.apply` with its separate session-local token
(not the numeric revision) to commit the reviewed renames. Use `inspect_project` to read
the project, or `cross_probe` to look up a linked terminal or pad. The additive
read-only `resolve_counterparts` tool returns complete pin/pad sets and explicit
unavailable states at a captured revision; see [pin/pad correspondence](../formats/pin-pad-correspondence.md)
for qualified addresses and the versioned response. `cross_probe` retains its
singular compatibility response.

To give a reviewer only command-level read access, start a separate headless process:

```sh
pcbcraft-cli mcp --read-only --project path/to/project.pc
```

The host sets this scope at startup; no MCP request can widen it. `inspect_project`
reports `scope: "read_only"`. `describe_command` and `list_commands` report it
in `structuredContent` and the second text part. The first text part keeps the
unchanged command descriptor or catalog list.
Queries, data exports and transient previews remain available; project edits,
Undo/Redo, Cut and library-source changes return `out_of_scope` without changing
the snapshot or the file. Startup does not recover an interrupted folder save;
if recovery is needed, have the owner open the project normally first. A command
added to the catalog must be explicitly classified as a read before a read-only
session can run it.

This setting confines the command interface of that process, not the client's
filesystem permissions. It does not grant read-only access to a live editor:
`--read-only` cannot be combined with `--connect`.

An MCP client can use `schematic.connect_pins` to join two terminals from
`inspect_project`. Describe the command for its input shape, and send the current
project revision with the two endpoints. Project inspection returns the wires,
connected terminal groups and unconnected pins. `schematic.draw_wire` accepts
terminal references, `{"junction":"junction-N"}` references from inspection, or
new free points as `{"x_nm":0,"y_nm":0}`, plus an ordered `bends` array and
`expected_revision`. Every leg must be horizontal or vertical. One complete route
is one undo step; `schematic.delete_wire` removes an inspected wire ID.
`schematic.place_symbol` also accepts an optional `rotation` of 0, 90, 180 or 270
counterclockwise degrees, defaulting to 0.

`schematic.move_items` and `schematic.delete_items` perform the same atomic group
edits as the editor. Both accept `items: {"symbols": ["symbol-2"], "wires": []}`
and `expected_revision`; moving also takes `dx_nm` and `dy_nm`. Obtain IDs from
inspection. Missing IDs, stale revisions and invalid geometry reject the complete
operation. Selecting a group and the live rectangle remain window state.

Library authoring actions now include atomic `replace_many`, group `transform`,
`duplicate`, typed-primitives `paste` and rectangular `array`, alongside the
existing insert/replace/delete/history operations. They all require the current
library document revision. Array row/column vectors and transform pivot/offset
use integer-nanometre `Point` objects; copies allocate unused terminal/pad
numbers. `app.library_view` can set a distinct `selected_objects` ID array instead
of scalar `selected`; per-pane inspection reports `selection`. These are live
editor contracts; headless project sessions do not host authoring documents.

### Placing an accepted footprint group

`pcb.place_assigned_footprints` places a specified list of assigned, accepted
`components` at `x_nm`/`y_nm`, with optional quarter-turn `rotation`, explicit
`board` and the current `expected_revision`. Packages use pinned definitions and
mappings; initial spacing follows their resolved geometry. Existing placements
stay fixed, and the entire group is one undo step. Duplicate/unknown, already
placed, excluded or pending assignments refuse the whole command. The native
Update PCB cursor handoff commits through this same command. Inspect and describe
the command before supplying IDs or coordinates; spacing is not a DRC result.
