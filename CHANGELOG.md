# Release notes

## 0.1.0 (unreleased)

- Edit references, values, component fields and footprint assignments on several components in one undo step through the shared `part.set_fields` command (#83). Rows are physical components, so a multi-unit part is one row; the whole edit is validated before anything changes, fields a row does not name are kept, and `dry_run` lists every field the edit would change. See [part commands](docs/automation/part-commands.md).
- Rotating or mirroring a schematic symbol in place no longer moves a net label onto another of its pins. Labels stay where they are, so a label on one pin could end up on another and silently join that pin's net; the edit is now refused with a message naming the label and the pin.
- Moving, rotating or mirroring schematic symbols no longer joins two nets when a wire that follows a moved pin would run through a net label or power symbol of another net. The edit is refused with a message naming the label, so the wire can be rerouted first.
- A multi-item schematic selection (symbols, wires and net labels) now mirrors with **X**, **Y** or the Capture ribbon's Mirror buttons as one undo step, through the shared `schematic.mirror_items` command. The group is reflected about its center, snapped so pins and wire corners stay on the grid; symbols compose the mirror with their rotation as single-symbol mirroring does, net labels flip their text direction, and wires to unselected parts stay on their pins. Mirroring twice on the same axis restores the saved project. A single selected symbol keeps using `schematic.mirror_symbol`.
- Check for polarized packages with no readable orientation using `pcb.check_assembly`. The read-only report warns once per package revision when symmetrical lands, silkscreen and fabrication artwork leave its orientation ambiguous and no zero orientation is declared. Read-only headless sessions can run it. New footprint exports and publications use `pcbcraft-footprint/7`, which is `pcbcraft-footprint/6` plus an optional orientation statement; earlier footprint formats stay readable, refuse the statement and keep their bytes when pinned, and assembly output profiles are unchanged.
- `pcbcraft-cli --help` reports the [oldest readable split project entry-point format](docs/formats/native-project-storage.md#compatibility-and-fixtures), `pcbcraft-project/5`.
- Opening a project saved by a newer PCBCraft names the newer format and the newest one this version reads, for every project document and packed manifest, instead of reporting an unrelated field. See [native project storage](docs/formats/native-project-storage.md#compatibility-and-fixtures).
- **Analyze → Rules → Working voltage** lets designers declare and inspect a working-voltage requirement (`DC` or `Peak AC`, referenced to `Earth` or a named net) on a saved net or net class, backed by the shared `electrical_intent.set` and `electrical_intent.inspect` MCP commands. Reports state the authored source, mark the requirement unverified (no ERC, DRC or solver consumes it yet), and flag `needs_review` with the reason when an owning or reference net is split, merged, left ambiguous, or deleted. Native saves use `pcbcraft-project/68` and, for registered boards, `pcbcraft-instances/12` whether or not a declaration is present. Files using the previous headers still open and migrate on save. Declarations are optional `authored_intent` fields, and the project envelope versions those fields inside the unchanged `pcbcraft-net-classes/1` catalog.
- Add shared exact circular arc contact geometry (`board::arc::geometry::ArcGeometry`) and wire circular arcs into board copper connectivity (`copper`, `copper_nets`, `copper_with_nets`), ratsnest, via inspection and the engine `copper_shorts` primitive. Clearance queries, DRC, copper preparation, transforms, clipboard, fabrication and PCB editing keep refusing arc boards until follow-up PRs account for arcs.

A self-contained KiCad 6–10 schematic can be imported as a new project, through
`project.import_kicad_schematic` in headless MCP or `pcbcraft-cli import-kicad`.
Symbols, units, references, values, fields, fitting and exclusion flags, power
symbols, wires, labels and no-connects carry over; the import refuses, with
reasons, anything that would change which pins connect, such as KiCad 10 jumper
pin groups, and lists what it left out. See [KiCad schematic import](docs/schematic/kicad-import.md) (#60).

Pane headers support explicit up/down and left/right splits, with previews and
saved orientation. Dragging between panes moves the existing view and collapses
its old slot. Splitting a pane itself opens another view. Separate Split window
up/down targets move the existing pane across the full window, and live commands
expose both placement scopes.

- The Net Inspector's **Across sheets** section and the read-only `nets.sheet_overview` command list every net that reaches more than one sheet, with each sheet's page, name and how the net arrives: global label, power symbol, hierarchical label, sheet pin or a shared part terminal. Drawing this flow on the root sheet remains a follow-up.
- The built-in Assistant requests a compact MCP command catalog with `list_commands` and `detail: false`, so it can read the full list of command IDs and short summaries within its tool output limit. `list_commands` still returns full descriptors by default, and `describe_command` returns the schema for an individual command (#636).
- Live MCP UI input (`ui.click`, `ui.drag`, `ui.key`, `ui.text`, `ui.move`, `ui.scroll`) now works in detached editor windows as well as the main window. Input needs the addressed window's focus and current layout, and is cancelled on focus loss, physical input, a resize, redocking, closing the window or a revoked connection. See [UI automation](docs/automation/ui-automation.md).
- Live MCP can size a native window for a recording: `ui.resize` requests a client size in logical points and `ui.window` reports the measured size, scale and maximized/fullscreen state, with the request shown as applied, pending or constrained. See [UI automation](docs/automation/ui-automation.md#size-a-window-for-recording).
- Copper zones can remove islands that touch no pad, via or track of their net (**Remove unconnected islands** in zone Properties, `remove_unconnected_islands` in zone options). Fill inspection now lists each removed island with its reason, area and bounds (#232). Normal board saves and packed exports use `pcbcraft-board/59`, even without zones. Each saved zone records the option explicitly; boards in format 43 or earlier supported formats load with it off.
- Resolve complete pin/pad counterpart sets through the engine and the read-only `resolve_counterparts` MCP tool, including shared symbol-unit appearances, multiple lands per terminal and explicit unavailable states. Existing singular `cross_probe` responses are unchanged; exact cross-view pin/pad highlighting remains a follow-up.

Pins and Pads can now be selected for inspection through their per-view filters,
canvas and Browser rows, or the live `view.select_inspection` command. Inspection
has its own highlight and read-only Properties label without selecting an owner
for editing. These filters start off to preserve existing picking defaults;
project edits and reopened documents do not restore stale inspection selections.

`pcb.set_test_point` marks individual placed pads and vias for assembly or
bare-board fabrication testing. The two marks are independent, name the board
face, and survive Undo/Redo and native save/reopen. Board format 43 introduced
optional marks alongside the format 42 records, and format 59 retains those
marks. Format 42 boards still open but reject stored marks. Releases predating
format 43 cannot read it. Clearing the last mark omits the list without
changing the current save format. This change does not add reports.

`list_commands` adds the command API version, a catalog revision and the
optional protocol features the build offers, so an agent can tell when the
commands it was written for may no longer work and when its cached discovery
data is out of date. Existing replies keep their fields; see
[command API version](docs/automation/command-api.md#command-api-version). MCP `execute` now
accepts an optional pin to that API version, refusing a different version with
`version_mismatch`. Discovery can mark deprecated commands and parameters;
successful calls that use them add warnings without changing the ordinary result.
See [stating a version](docs/automation/command-api.md#stating-a-version).

`pcb.merge_tracks` tidies routed copper: it merges the listed collinear segments
on the same layer and width that meet at a free end, and absorbs stacked
duplicates, keeping the lowest UUID with the merged ends. Ends at a junction,
via or pad and locked segments stay as they are, so connectivity does not change.
The whole merge is one undo step, and a dry run returns the same diff without
changing the project. In the PCB editor, **Layout → Modify → Merge Tracks** runs
it on the selected tracks.

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

Failed MCP calls and live editor replies now carry a stable error code, such
as `stale_revision`, `not_found` or `cap_exceeded`, with machine-readable
details, so an agent can act on a failure without parsing its message. The
codes are published in [error codes](docs/automation/command-api.md#error-codes) and are never
renamed or reused. Messages are unchanged, except that `ui.` commands now
refuse unknown keys, missing required keys and non-null scalar or array
parameter lists with the same wording as window commands. Omitted or null
parameters retain their empty-map behavior. A stale `expected_revision` now
precedes other edit parameter problems. Read-only route planning keeps its
`stale` plan outcome.

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
members cannot be removed. Projects with part groups use project/24 (expanded
memberships) or project/39 (compact) unless profiles, pairs or a designator lock
select project/36, /45 or /51. Board instance documents use instances/5 or /7
for the two representations, or instances/8 when pairs are present. The compact
UUID dictionary keeps dense memberships within the existing size limits without
dropping supported membership. Earlier files
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
- `pcb.export_fabrication` returns a drill chart beside the files: one row per tool of each drill file, with its plating, tool number, size as written and hole count, built from the same holes as the drill files. See [fabrication through MCP](docs/manufacturing/fabrication-mcp.md#generate-fabrication-files).
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

On desktop, `pcbcraft <path>` opens a project folder, its `project.json` or a
`.pc` archive as the app starts, without a picker; the opened project enters the
workspace and joins recent projects. The path may be a symlink to a project
folder or archive; recent projects record its target. A startup open that fails reports the error
in the status bar, keeps Home usable and writes the same line once to stderr
when available. Windows GUI release builds need stderr redirection to capture
diagnostics. The launcher takes no options and at most one path, so a
usage error prints one line and exits before any window opens.

Equivalent loose units of a multi-unit part can now be fitted into open slots of
one package with `schematic.complete_packages` through headless or live MCP. It
changes nothing unless `scope` is `sheet` or `project`, and `dry_run` reports the
same transfers without editing. Connectivity, saved net and sheet identities and
authored wire routes are kept. Parts with a locked designator, no-connect intent
or a footprint that board review cannot yet retire are skipped. Applying is one
undo step. Read-only sessions refuse the command. Placement does not complete
packages, and automatic annotation remains in #45.
