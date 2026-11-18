# Release notes

## 0.1.0 (unreleased)

- Resolve complete pin/pad counterpart sets through the engine and the read-only `resolve_counterparts` MCP tool, including shared symbol-unit appearances, multiple lands per terminal and explicit unavailable states. Existing singular `cross_probe` responses are unchanged; exact cross-view pin/pad highlighting remains a follow-up.

Pins and Pads can now be selected for inspection through their per-view filters,
canvas and Browser rows, or the live `view.select_inspection` command. Inspection
has its own highlight and read-only Properties label without selecting an owner
for editing. These filters start off to preserve existing picking defaults;
project edits and reopened documents do not restore stale inspection selections.

`list_commands` adds the command API version, a catalog revision and the
optional protocol features the build offers, so an agent can tell when the
commands it was written for may no longer work and when its cached discovery
data is out of date. Existing replies keep their fields; see
[command API version](docs/mcp.md#command-api-version).

Agents can plan a route before adding it: `pcb.plan_route` returns H/V/45° points
on one copper layer that go round existing pads, tracks and vias, locked or not,
keeping the effective track width, net-class clearances and board-edge clearance.
A route that cannot get through stops at its last legal point and names what
blocked it. Planning never edits the board; the points go to `pcb.add_tracks` as
one undo step. The route tool now uses the same planner with bounded search,
magnetic target hysteresis and full-path validation before fixing a clicked
vertex or placing a through via (#246).

Placing a via at the current fixed route head now validates that exact location,
even when an earlier, longer route preview remains visible, so invalid vias are
rejected before changing layers.

Preferences → Appearance → **Panel style** switches the workspace shell
between **Floating** cards and a **Compact** layout, following SolidCraft. In
Compact the ribbon, workspace tile, Browser, Net Inspector, docked panels and
the Parts Library columns lose their gaps, shadows and outlines and join the
window edge and each other with single dividers and tighter padding; cards
docked in the same column stack directly beneath each other. The Symbol and
Footprint editors no longer show a dark strip between the ribbon and the
canvas. The choice is remembered with your settings and available
through `app.set_preferences`; each view keeps its saved dock side and floating
positions, and project data and history are unchanged.

Part instances can be collected into named part groups shared by the schematic
and the board. `part_groups.create`, `update`, `remove` and `inspect`
manage the catalog, and `part_instances.set_groups` and
`part_instances.inspect_groups` set and read a part instance's groups by its
stable ID, so annotation does not change membership. A group that still has
members cannot be removed. Projects with part groups save as
`pcbcraft-project/24` (board instances `pcbcraft-instances/5` when memberships
exist). Dense memberships use `pcbcraft-project/39` and board instances
`pcbcraft-instances/7` through a compact UUID dictionary when expanded records
exceed existing size limits; no supported membership is dropped. Earlier files
keep the expanded representation while it fits, and compact files keep their
encoding after reopening. Unchanged saves preserve canonical encoder bytes;
accepted noncanonical input may normalize. Copy/Cut leave part groups in their
owning project, so duplicated parts start ungrouped; projects without groups
keep their format and canonical encoder bytes.

Nets share readable names across schematic browsing, Rules and PCB inspection.
Unnamed signals use net-owned numbers such as `Net-0001`, which survive component
reference edits and deletion on a surviving net. Split/merge reconciliation and
Undo/Redo retain the corresponding identity decisions. PCB pads, vias and wide
tracks show fitted labels through the view-local **Layers → Net names** toggle.
Accepted PCB names remain at their reviewed baseline until Update PCB accepts
schematic changes; native baseline format 4 preserves that presentation.

PCB views now remember their own snap spacing and presets separately from
schematic views. Choosing a board grid no longer replaces the schematic startup
spacing, and switching workspaces restores each editor's choice. Schematic views
start at 50 mil and PCB views at a finer 10 mil (0.254 mm); previously saved
schematic spacings still apply to schematic views, and saved board choices,
including an explicit 50 mil, are kept.

Intentional no-connect crosses can be placed on schematic pins with Q or Capture
and removed through pin context menus. A focused check reports unmarked pins and
connected, missing or incompatible marker targets. Marker-bearing projects use
project/19; projects without markers retain their existing low-level encoding.

DRC runs against an immutable snapshot while the editor remains interactive, with
stage progress, cancellation and replacement by the latest rerun. Native workers
and browser batches use the same checker. Edits, rule changes and project/view
transitions discard unfinished reports; live MCP checks wait without blocking
editor input. Completed findings retain their source revision.

- Pin immutable assembly output profile revisions with `pcb.set_assembly_profile`, then review changed corrections and actual placement values with `pcb.preview_assembly_profile` before explicitly accepting them with `pcb.adopt_assembly_profile`. Profiles hold bottom-side conventions and per-definition rotation and centroid corrections; vendor identity is optional metadata, not a behaviour selector. Pinning a revision does not change existing board adoptions, and retirement preserves adopted content and Undo/Redo. Issued profile revisions remain reserved after Undo and Save/Open, so a branching edit cannot replace their content. Project format 36 pins the revisions; adoptions require at least board format 22, stack fields 23, explicit via assignments 24, material reports 25, arcs 26 and footprint locks 34 (35 when both arcs and locks are present). Generic rows and other profiles are unchanged. No placement file is written yet.
- Measure fabrication outputs from a named placement origin with `pcb.set_placement_origin`: board zero, the Board Outline's lower-left corner or a fixed point. Gerber and Excellon use the resolved placement-origin frame, Y up; Excellon coordinates are rounded to 0.001 mm, and the export result names the origin used. Arc-bearing boards do not yet support fabrication output. Boards with an origin require at least board format 21; profile adoptions require 22, stack fields 23, explicit via assignments 24, material reports 25, arcs 26 and footprint locks 34 (35 when both arcs and locks are present). Clearing the origin selects the format required by the remaining board data. Excellon millimetres are now rounded from integer nanometres with halves going up, so a value ending in exactly half a micrometre may be written 0.001 mm higher than before, and values just below zero are written `0.000` instead of `-0.000`; all other output bytes are unchanged.
- Save board dimensional limits and routing preferences through Analyze → Rules and `pcb.set_rules`; routing, DRC and captured clearance queries share effective values and provenance. Authored rules require at least board format 19; a placement origin, profile adoptions, stack fields, via intent, material reports, arcs or footprint locks select the required format without dropping rules. Boards without rules keep the format required by their other content.

Board setup stores an optional role and base copper for each copper layer, a
tolerance and basis for its finished thickness, and the declared finished board
thickness, through `pcb.set_board_setup`. Embedded setup with these fields requires
at least board format 23. Standalone setup profiles use version 2 for these
extensions without material reports, or version 3 with reports. Older builds
that do not support these formats refuse them. Explicit via assignments require
at least board format 24, material reports at least 25, and arcs or footprint locks
select 26 or 34, respectively (35 when both are present).

Materials take sourced thermal conductivity, thermal expansion, dielectric
strength and CTI reports, each with its unit in the field name, its test
conditions and a required source; a CTI given with a material group must lie in
that group's IEC 60664-1 band. Reports require at least board format 25, and
formats 26, 34 and 35 can retain them. Standalone setup profiles with reports
use version 3. Removing reports leaves the format required by the remaining data.

Overlapping schematic symbol drawings show a placement warning and compact amber
count badges, with affected references on hover. Placement remains allowed.
Right-click and use Select symbol
to choose a hidden part; a subsequent drag moves the chosen symbol. Warnings
follow the current sheet and stay outside project files and undo history.

Schematic symbols can now be mirrored horizontally (**X**) or vertically (**Y**)
about their center from the canvas, context menu, Capture ribbon, or the shared
`schematic.mirror_symbol` command. Reflections compose with the stored quarter
turn, keep terminal, field and pin-to-pad identities, carry attached wires to the
transformed pins and leave reference/value text upright. Applying the same axis
twice restores the original pose; undo, clipboard, multi-unit sheets and
folder/archive reopen preserve the transform. Mirrored sheets use sheet/6 with an
optional `mirrored` flag and return to the earlier encoding when the last mirror
is removed; older readers reject sheet/6 rather than dropping the reflection.
Connecting legs drawn to retain an offset named contact record their wire
identity, endpoints and bends in sheet/7, so a later mirror removes only that
generated leg after reopen and never an authored route. Edited legs become
authored routes again, and removing the last live record returns the sheet to
the earlier encoding.

Default and Power library sources are marked Built-in and cannot be removed or
replaced through the Sources dialog or shared library commands. External library
registrations remain removable without changing pinned project definitions.

Tracks and vias can be locked together in Properties or through
`pcb.set_copper_locked`. Locked copper stays selectable and refuses geometry,
size and deletion changes, including indirect changes from footprint transforms
and segment dragging. Mixed edits fail atomically. Clipboard copies retain locks;
Cut refuses protected copper. Locks persist with board format 18 or a later
supported format, and board-kind clipboard format 3 (4 with explicit via intent),
which older builds refuse. Without active copper locks, the remaining authored
fields determine the board format. Undo/redo preserves geometry and copper lock
intent.

Placed-footprint lock metadata survives native split-document and packed-archive
round trips and is visible in inspection and typed board views. Active footprint
locks use board format 34 without arcs, or format 35 when both are present. Both
formats can carry reports and compatible earlier extensions. Removing locks
restores the format required by the remaining content. Fresh canonical encoding
omits false; Native Save can retain
an unchanged board document verbatim.
This is storage only: no footprint lock command, GUI toggle or edit protection yet.
Clipboard and diagnostic single-file codecs explicitly refuse active footprint locks.

DRC measures copper and board-edge clearance against the same rounded pad
contours as connectivity, avoiding false findings at empty bounding corners.
Exact clearance equality still passes, including half-nanometer radii. A reusable
engine clearance query now captures board/snapshot/revision authority and separates
conservative obstacle candidates from contour decisions; conflicting intended
memberships remain visible even at zero clearance.

Projects now use a fixed `project.json` entry point inside their working folder,
and packed projects use `<project-name>.pc`, a ZIP archive of the same documents.
The archive retains `manifest.json` with document sizes and SHA-256 hashes, using
`pcbcraft-container/3`. Working folders remain the form to commit to Git; project
locations and the first-save location workflow are unchanged.

Existing `project.pc` split folders and container-v2 `.pc.zip` archives open
without rewriting their files; their first Save upgrades them automatically.
Recent-project previews also accept these layouts. The explicit `pcbcraft-cli convert SOURCE DESTINATION` command optionally converts
previous split folders and container-v2 archives to a new folder or `.pc` package,
leaving the source unchanged. Renaming an old archive alone does not convert it.
Historical power, power-bar and ground annotations migrate to pinned native power
symbols, retaining names, positions, rotations and wire connections. Those sheets
and project metadata are updated on Save; unrelated payloads retain their bytes.
Standalone JSON/container-v1 projects remain unsupported.

New and converted projects use project/16 with a registered board folder, including
single-board projects. Supported flat `project.json` projects open unchanged and
migrate on first Save without requiring design edits. The journal moves the board
and updates its registry together, preserving board identity and physical bytes.
Successful layout adoption remains across Undo/Redo; subsequent unchanged saves
produce no diff. Foreign physical-data directories select an unused persisted
namespace rather than being claimed by the project.


Native libraries now support reusable part and category records, canonical typed
quantities and manufacturer/MPN records alongside symbols and footprints. Parts
Catalog placement pins the complete part dependency set, with optional package
and manufacturer selection. The part form edits typed requirements and reports
when a retained MPN no longer meets them. Symbol-only instances can also choose
external native footprints through the assignment form. Sources remain separate
from pinned project data, including after relocation or removal.

Multi-unit symbols share one logical part, reference and physical package across
sheets. Project format 15 carries native part selections, board format 17
preserves multi-pad/explicitly unconnected-pad mappings, and accepted baseline
format 2 retains those maps. Older builds reject these new formats. Existing
single-unit/native records keep their low-level encoding when unchanged; the
ordinary first-save board-folder migration remains in effect. These additions
remain within the unreleased 0.1.0 version. High-pin-count devices, KiCad import,
vendor APIs and general pad stacks remain separate work.

Each sheet now has a stable occurrence UUID, with one shared circuit use for the
existing hierarchy. Board projects store these records in `instances.json` beside
`board.json`; schematic-only projects keep them in `project.json`. Opening an
older project leaves it unchanged, and first Save persists deterministic
compatibility IDs without changing component references, values, package choices
or accepted board state. Creating the first board moves the same records in one
undoable action. Repeated sheet instances remain future work.

Named and explicitly promoted nets now have independent persistent UUIDs in a
sparse registry. Ordinary continuations retain their IDs, splits and merges
record lineage, and unresolved splits can be resolved through the shared
revision-checked `nets.promote` and `nets.resolve_ambiguity` commands in headless
or live MCP. Undo, reopen and first-board creation preserve the decisions. The
first record selects project/17 and board instances/2 in the existing folder/ZIP
layout; reading an older project does not promote its nets. There is no new UI,
and net classes and zones remain separate work.
