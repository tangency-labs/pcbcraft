# MCP board and connectivity commands

## Placement origin

`pcb.set_placement_origin` requires an explicit `board`, `expected_revision` and
`origin`: `{"kind":"board_zero"}`, `{"kind":"outline_lower_left"}` (the lower-left
corner of the Board Outline's bounds, following later outline edits), or
`{"kind":"point","x_nm":...,"y_nm":...}` in board coordinates. `null` removes the saved
origin. It is one undo step, keeps the origin's UUID while its anchor changes, and
ignores a repeat of the current value. Board inspection reports `placement_origin`
with the `authored` record and the `effective` origin outputs use, which is null when
an outline-attached origin has no outline yet.

Fabrication outputs shift every coordinate to this origin and write Y up, as seen
from the top; rotations are unchanged. The export result's `fabrication.origin` names
the `anchor`, its `source` (`{"kind":"builtin"}` without a saved origin, otherwise
`{"kind":"board","origin":"<uuid>"}`), the resolved `at` point in board coordinates, and `axes: "x_right_y_up"`. Without a saved origin, outputs
are measured from board zero. Gerber writes exact nanometres; Excellon rounds each
millimetre value to three decimals from integer nanometres with halves going up, so
moving the origin by whole micrometres shifts every written value by exactly that amount.

## Assembly output profiles

`pcb.set_assembly_profile` pins immutable project-owned content. It requires
`expected_revision` and `content`. Omit `profile` to create a UUID, or supply an
existing or portable profile UUID. Content has a positive 32-bit `revision`, a
`name`, and `provenance: {"kind":"authored"}` or
`{"kind":"imported","source":"<human source hint>"}`. The optional `vendor` is
metadata, never a behaviour selector. Names, source hints and vendor metadata
have 1–64 characters without surrounding whitespace or controls; available
profiles have unique names. `bottom_side` defaults to `viewed_from_top`, or is
`viewed_from_bottom`. The schema version (`pcbcraft-assembly-profile/1`) is
independent of content revisions. The command currently requires a project with a
board; it is unavailable in schematic-only projects even though the catalog is
project-owned.

Optional `corrections` has one entry per `library` and footprint `definition`,
with a `rotation` (0–359 whole degrees, counterclockwise from the component side)
and/or a nonzero `centroid` (`x_nm`, `y_nm` in library footprint coordinates, Y
down, each within 100 mm). Each entry must change something. Commands sort entries
and omit zero rotation; an empty array is refused. Integer-valued numbers already
parsed by serde_json are accepted without further rounding. Digits beyond what
the JSON parser keeps are not preserved. Corrections apply across definition revisions; legacy
markers never match. See [placement conventions](../libraries/footprint-schema.md#placement-output-conventions).

An existing ID and content revision is a no-op only for identical content;
different content is a conflict. New content must exceed that identity's revision
watermark. `content: null` retires the named identity without changing existing
adoptions. Reusing an already cleaned revision is refused, even with identical
content; choose a greater revision instead. Undo/Redo also retain issued revision watermarks. An undone pin whose payload is no longer present cannot reuse that revision, even after Save/Open; a higher revision is required. This reservation is local to the saved project lineage, not a global registry across independently edited copies.

`pcb.preview_assembly_profile` requires `board`, `expected_revision`, `profile`
and `revision` (a pinned content revision, or null to clear that board's adoption).
It returns the old/new adoption, changed corrections, bottom convention and actual
affected placement rows before and after, plus an `acceptance` digest. It edits
nothing. `pcb.adopt_assembly_profile` accepts the same parameters and that digest,
recomputing the preview atomically. A stale revision or altered review is refused.
Pinning is not adoption; only explicit adoption changes which profile revision the
board's rows use.
Each edit is one undo step and repeated unchanged adoptions are no-ops. Pinning on
a supported older project adopts the folder, occurrence and allocation metadata
required by the catalog's saved format in that same undo step; existing project,
board and object identities and registered storage roots are preserved. Undo restores profile choices but keeps revision reservations and the storage metadata needed to save them.

Inspection exposes project `assembly_profiles` (identity watermarks and pinned
revisions) and board `assembly_profiles` (explicit revision adoptions), sorted by
identity. Limits are 64 identities including retired and watermark-only ones, 128 retained revisions
and 4096 corrections across retained revisions. Cleanup on edits releases only
unadopted superseded or retired payloads; adopted revisions survive retirement,
offline save/reopen and Undo/Redo. Undo shares immutable payloads. Profiles change
no board geometry or fabrication output. No placement exporter, import/export
wrapper or profile-management UI is provided here.

## Saved board defaults

`pcb.set_rules` requires an explicit `board`, `expected_revision` and complete
`rules` values, or `rules: null` to restore built-in defaults. Board inspection
reports authored records and effective values with stable rule IDs and per-field
provenance. DRC resolves saved limits; request overrides can tighten authored
minimums/maximums but cannot weaken them or persist a change. It also reports via
diameter and drill bounds. Omitted dimensions on track/via creation use the same
preferred sizes as the GUI. See [board-rules.md](../pcb/board-rules.md) for the full
property, precedence, compatibility and undo contract.

## Intentional no-connect targets

`schematic.set_no_connect` uses `target: {"part":"component-N","terminal":"1"}`
for a logical pin, or `target: {"junction":"junction-N"}` for a physical open
wire end, plus `marked` and `expected_revision`. Both forms use the same edits
as Q and the Capture tool. `schematic.check_no_connects` returns a read-only,
revision-qualified report even with `inspect:false`; its `markers`/`findings`
arrays describe pins and `wire_markers`/`wire_findings` describe wire ends.
Totals and truncation cover both. These markers record intent without changing
the real netlist or physical DRC. See the [contract](../schematic/no-connect-markers.md).

## Electrical rules check

`schematic.run_erc` runs every rule in the ERC registry, in a fixed order, on
the open project and keeps the result as the session's ERC report. It takes no
parameters, changes no project data, revision or undo history, and works with
`inspect:false`. The reply is `{"revision","status":"current","erc_report"}`.
`erc_report` holds `project`, the `revision` it ran on, `total_findings` and a
`rules` array; each entry has the rule's stable `rule` id, `title`,
`total_findings`, `truncated` and a `result` keyed by the result kind. The only
rule so far is `no_connect`; its result is `{"no_connect_check": ...}`, exactly
the object `schematic.check_no_connects` returns under that key, with the same
finding keys. Two runs on one
revision return identical bytes.

`schematic.erc_report` reads the kept report without running anything. Before
any run it returns `status: "not_run"`. After any project edit, Undo or Redo the report
is `status: "stale"`: the reply gives `report_revision` and no findings, and
`rerun` names the command to run again. Opening another project discards it.
The schematic ERC dialog shows this same report and asks you to run it again
when it is stale. `schematic.check_no_connects` keeps its original response.

Schematic selections may include an optional `no_connects` array of the same
pin/wire target objects. `view.select` exposes marker selection and
`view.set_selection_filters` accepts `no_connects`. Passing these targets to
`schematic.delete_items` removes only the selected markers, or combines them
with ordinary owner deletion in one undoable transaction. Target validation is
sheet-scoped and revision-checked. Copy/group-move selections must include each selected marker's owner; standalone
marker geometry is not persisted. A direct wire-end X drag uses the existing
`schematic.move_junction` command and preserves marker identity.

A placed no-connect mark blocks new wire attachments at that point. Shared
wire drawing, pin connection and endpoint attachment commands reject the edit
atomically with “Remove the no-connect marker before connecting a wire.” This
also covers coordinate and wire/at aliases. Remove the mark through the same
commands or Delete before connecting; saved conflicts from older projects are
preserved and remain reportable.


## Explicit net-class rules

`net_classes.create`, `net_classes.update` and `net_classes.remove` author the
project catalog with `expected_revision`, without creating a physical board.
`net_classes.inspect` reads that catalog, while `net_classes.resolve` combines
explicit `classes` and optional `other_classes` UUID sets with built-ins or an
explicitly addressed `board`. Both queries return their data with `inspect: false`.
See the [property, conflict and storage contract](../formats/net-classes.md) for examples
and precedence. `nets.inspect_classes` discovers current typed targets without
promotion; with a target it returns bindings, origins and conflicts.
`nets.set_classes` replaces the full class set with revision checking and promotes
an eligible derived target in the same history step. The assignment query retains
its payload with `inspect: false`. See the [assignment contract](../formats/net-class-assignments.md)
for target shapes and protected removal/rebind. `pcb.route_rules` inspects effective
sizes, clearance and conflicts at an addressed copper contact without mutation,
including with `inspect: false`. Omitted routing sizes use that contact's class
preferences; explicit changed copper must satisfy applicable hard dimensions.
Queries and DRC retain per-object class IDs, revisions and assignment origins.
See the [consumer contract](../formats/net-class-consumers.md) for parameters, provenance,
stale authority and operation-specific conflict behavior.

Standalone via net intent uses `pcb.add_via` with optional `target`, or
`pcb.set_via_net` with `vias` and either `target` or `automatic: true`. Both share
revision-checked GUI transactions. See [via net assignments](../pcb/via-net-assignments.md)
for target qualification, inspection and unresolved-state behavior.

## Route planning and connected drags

`pcb.plan_route` reads the current board and returns points, a planning outcome,
and any proposed copper displacements. In `highlight` mode it tests the preferred
path and returns blocked geometry for display. `shove` can move eligible tracks,
and `shove_vias` can move eligible through vias with their attached track ends.
Submit a reached plan's `replacements` and `via_moves` with the route and its
captured revision.

`pcb.reshape_tracks` accepts the same explicit modes and `expected_revision` for
up to 256 directly dragged segments. Highlight validates the requested geometry,
Walkaround can add detour legs, and Shove can also move eligible neighboring copper.
When a board has an authored zone, `pcb.plan_route` keeps vias fixed and
`pcb.reshape_tracks` refuses policy-driven drags. See the
[route planning and drag contract](../pcb/route-planning.md#through-vias-and-connected-segment-drags).
