# PCB layout and rules capabilities

## Board rules

### Available

- Saved board-scoped hard dimensional limits and preferred routing sizes, with dimension
  illustrations beside the shared Rules fields, through Board Setup → Design rules or Analyze →
  Rules → Board defaults and shared GUI/live/headless MCP.
- Stable rule identity, effective values/provenance, diagnostic overrides that cannot weaken
  authored limits, DRC/query integration and undo/save/reopen.

- Project-level [class definitions and dimensional resolution](../formats/net-classes.md), plus [persistent
  assignments](../formats/net-class-assignments.md), are available through shared commands/MCP.
- [class-aware routing, queries and DRC](../formats/net-class-consumers.md) use effective dimensions with
  per-object provenance. The [shared editor form](../pcb/net-class-editor.md) creates/edits classes,
  assigns supported current nets and inspects effective values and conflicts without requiring a
  board.
### Remaining scope

- Advanced electrical and manufacturing analyses remain under #77.

## Layout

### Available

- [Single-layer zone outlines](../pcb/copper-zone-outlines.md) with net selection/promotion,
  polygon/rectangle creation, edge resizing, vertex insertion/removal, axis alignment, boundary
  hatching, modifier/box/mixed selection and atomic Delete, Properties, layer/filter visibility,
  shared commands and undo/save/reopen.
- Cancellable derived copper pours and personal net colors shared by zones, connected tracks and SMD
  pads across layers.
- Closer initial empty-board drawing scale with independent saved navigation.
- Schematic-only projects and explicit creation of the first board, with optional board targets and
  compatible migration of supported implicit-board schemas.
- Core-resolved physical pad-layer membership with per-layer copper rendering, editable
  copper/technical layer swatches, ribbon color patches, keyboard layer navigation and a local
  palette shared across views.
- Layer-addressed Board Outline lines and rectangles with canvas creation, selection,
  vertex/whole-graphic movement and deletion, shared GUI/live/headless MCP transactions, undo,
  stable folder/archive persistence, layer visibility and Fit.
- X starts routing on any configured copper layer, with explicit width, 45° posture, a route
  context menu and support for up to 65,536 segments and 16,384 vias within the board document limit.
- Routing supports bounded clearance-aware walkaround and opt-in
  [track shove](../pcb/route-planning.md#track-shove). Shove keeps pads and locked copper fixed,
  protects branch and free-end contacts, and can move ordinary bends in uniform traces.
  It can displace through vias, preserves spacing by default and can merge same-net copper.
- A route retains its starting net through vertices and vias and refuses foreign-net adoption
  from unconnected pads. Layer-colored previews remain stable during background preparation and
  show a clearance halo and pointer-following crosshair. Held Ctrl overrides proximity-based grid
  snapping, which also recognizes pads, track ends and vias.
- V places a through via with an explicit diameter and drill and switches the routing layer.
  Place › Via (Ctrl+Shift+X, or Command+Shift+X on macOS) places a standalone via, with a native
  physical-X fallback for non-Latin keyboards.
- [Explicit via net assignments](../pcb/via-net-assignments.md) use searchable placement and
  Properties choosers with bounded result pages, automatic contact inheritance, unresolved-target
  diagnostics and same-board clipboard preservation.
- Box, U and click select tracks and vias in lighter copper colors matching pads. Picking favors
  active-layer tracks, and Shift+Space cycles through overlapping visible objects.
- M and group drags move the selected copper rigidly. The
  [dockable Router Settings](../pcb/route-planning.md#track-shove) apply Walkaround, Shove and
  Highlight collisions to new routes, D and held drags on a single track with its neighbors
  attached (`pcb.reshape_tracks`). Properties edits track width and via size. Footprint moves,
  rotations, flips and alignments carry attached track ends and refuse new shorts.
- Modify › Merge Tracks and `pcb.merge_tracks` merge collinear and stacked duplicate segments with
  free shared ends in one undo step.
- Inspect › DRC and `pcb.check_design` check shorts (including direct pad and via-mediated contact),
  clearance to resolved pad contours, width, annular ring, unrouted connections, board edges and
  courtyard overlap. Findings appear in the results card, canvas and status.
- Closed outlines use even-odd nesting for cutouts. Meeting outlines form one union component and
  report `ambiguous_outline`, while disjoint cutouts stay cutouts.
- Native background and browser incremental DRC support cancellation, stage progress, coalesced
  reruns, stale-completion rejection and deferred live MCP checks.
- Backtick highlights nets in the layer color. Starting a route from a trace emphasizes its copper
  automatically. Copper edits that would short nets are refused.
- Footprint reference text can be selected and moved with owner highlighting and airwires.
  Properties can set board-local text appearance or rename the shared designator. The changes
  appear in silkscreen output.
- Properties accepts exact footprint X, Y, rotation and side. The Modify menu and
  `pcb.align_footprints` align or distribute footprints, with
  [exact origin pitch](../pcb/set-pitch-distribution.md) available through MCP.
- F, Modify › Flip or `pcb.set_footprint_transform` flips footprints onto back copper and technical
  layers. Board format 14 stores the side.
- The view-only ruler (Inspect › Measure, Ctrl+Shift+M) snaps to pads, track ends and vias.
  Design › Flip view and `view.set_board_flipped` give a mirrored bottom view.
- Copper supports click selection and deletion. Completing a connection clears its airwire.
  A route is one Undo step. Board formats 12 and 13 added track and via persistence.
- PCB workspace with the shared project Browser, grouped Layers, placement Properties and Board
  setup with board-local physical silkscreen text/stroke/clearance settings, and physical SMD
  pad/body/courtyard rendering alongside legacy pad-center markers.
- Separate package assignment and coordinate-based initial placement through a shared chooser in
  either workspace, with explicit terminal mappings and pinned offline packages.
- Directional footprint box selection with modifier combinations, single-footprint move/quarter-turn
  rotation and selected-footprint deletion through canvas, ribbon and shared GUI/live/headless MCP
  commands, with snapping, cancellation, undo and offline persistence.
- Engine commands for starter SMD packages, a legacy two-pad marker and explicit terminal-to-pad
  lookup.
- One complete Craft Board setup form on first and subsequent opens, with two-layer FR-4/1 oz/1.6 mm
  starting settings, generic material families, a physical silkscreen preview and shareable
  `.pcsetup.json` setup snapshots, for every even copper count from 2 through 64, explicit
  compatible populated-board resolution, layer names/optional membership, cross-section aligned to
  the table and a laminate total, exact optional thickness/tolerance and locked-gap fitting,
  core/prepreg construction and sourced Tg/Dk/Df material revisions.
- Copper-layer roles, base copper and finished copper tolerance/basis, and declared finished board
  thickness with tolerance and minimum through `pcb.set_board_setup` (at least board format 23).
- Sourced thermal conductivity, thermal expansion, dielectric strength and CTI material reports with
  read-only technical details and at least board format 25/setup profile 3 persistence.
- Validated layer catalog and uniform rigid stackup, custom materials, sourced Tg/Dk/Df reports,
  core/prepreg plies, undo and offline persistence through headless commands.
- System clipboard copy/cut/paste and duplicate across panes/windows/instances for mixed footprints,
  outlines, tracks and vias, excluding selections with active footprint locks, explicit
  schematic/layer choices, cut relocation, rotation and atomic undo.
- Explicit CSV/TSV coordinate conversion to Board Outline polylines with units, closure and Y
  inversion.

### Remaining scope

- Manufacturing-profile editors, impedance calculations and general layer bindings
  [#76](https://github.com/tangency-labs/pcbcraft/issues/76), shared nets/rules
  [#77](https://github.com/tangency-labs/pcbcraft/issues/77), editable board canvas
  [#54](https://github.com/tangency-labs/pcbcraft/issues/54) and the rest of basic track/via editing
  (push-and-shove, clearance checks) [#95](https://github.com/tangency-labs/pcbcraft/issues/95).

## Unroute a connection

### Available

- [Pad-to-pad unroute](../pcb/unroute.md#unroute-one-connection) via shared headless/live MCP
  commands: preview or remove one routed arm without dropping connections between the other pads.
- Lock protection and one-step Undo.

### Remaining scope

- No canvas tool, net-wide or part-wide removal.

## Pin/pad correspondence

### Available

- [Complete read-only counterpart resolver](../formats/pin-pad-correspondence.md) in the engine and
  headless/live MCP, including shared symbol units, multiple pads per terminal, accepted package
  mappings and explicit unavailable/stale states.
- Legacy `cross_probe` remains compatible. [Per-view inspection selection](../schematic/pin-pad-selection.md)
  through Pins/Pads filters, canvas, Browser and live commands.

### Remaining scope

- Exact cross-view highlighting/Reveal #398.

## Footprint locks

### Available

- [Native storage, inspection and direct placement
  protection](../pcb/locks.md#placed-footprint-locks).
- move, rotate, flip, align/distribute and delete refuse locked targets atomically.
- Active locks are supported in board formats 34, 35 (arcs), and 37 (teardrops). Each PCB view can
  exclude locked footprints, tracks and vias from selection without hiding them.

### Remaining scope

- No footprint lock/unlock command, GUI lock-authoring toggle or routing-obstacle policy yet.
- Clipboard and diagnostic single-file codecs refuse active footprint locks.

## Copper locking

### Available

- Shared GUI/MCP track/via lock command, explicit inspection, atomic protection for
  movement/reshape/size/delete and indirect copper transforms.
- Lock-preserving clipboard copies, undo/redo and native reopen.

### Remaining scope

- Zones and board graphics do not have lock flags (zone locks remain in #81).
- Walkaround treats existing copper as fixed, track shove honors copper locks, and [via displacement
  and connected drag](../pcb/route-planning.md#through-vias-and-connected-segment-drags) preserve locked
  neighbors across layers.
- Further propagation and zone integration remain in #248.

## Implementation notes

The physical stack also exposes [derived and named layer groups](../pcb/board-materials.md#layer-groups).
Physical stackup stages copper roles and named memberships; `pcb.inspect_layer_groups`
reads the same IDs/names headlessly. Rule-scoping consumers and live simulation links
remain future work.

Setup edits can be reviewed through `pcb.preview_board_setup` before the shared
Apply command: layer removals/renumbering, affected objects and consent/refusal
are returned with a disposable session-local token. Preview does not dirty the
project or add history. See [setup review](../pcb/board-materials.md#review-a-setup-edit).
The detailed review is available to native command clients and live/headless MCP;
the Board setup form does not yet present it.

DRC and the engine's [snapshot copper query](../pcb/copper-clearance-query.md) share
supported pad/track/via contours, contact and clearance predicates. Queries retain
board, snapshot, revision and effective-clearance authority, with conservative
candidates and an exhaustive reference. Circular clearance and DRC contours remain separate
work. [Exact circular copper records](../pcb/circular-copper-records.md) support shared
revision-checked replacement, dry-run diffs, native save/reopen and exact
arc contact geometry for copper connectivity, ratsnest, via inspection and
engine short detection; clearance queries, DRC, copper preparation,
PCB rendering/editing, transforms, clipboard and fabrication still explicitly
refuse boards containing arcs rather than returning incomplete straight-only
results.
[Straight via teardrop intents](../pcb/teardrop-records.md) use a shared
revision-checked add/remove command with dry-run, per-site rule reports, one
undo step and native save/reopen. Geometry consumers refuse boards with these
records until teardrop support is integrated; there is no drawing tool or
fabrication output for them yet.
[Walkaround route planning](../pcb/route-planning.md) (`pcb.plan_route`) uses the same query
on arc-free boards to plan single-layer H/V/45° routes round existing copper with
class-aware widths, clearances and board-edge clearance without editing. The route
tool uses resumable searches, magnetic target hysteresis and full-path validation
before adopting clicked vertices or vias; final publication still uses one shared
copper transaction. Full preview/commit latency qualification remains under #246.
Captured local queries
now reuse a layer-aware packed index and support disposable geometry overlays;
see the [query contract](../pcb/copper-clearance-query.md) and
[measurement limits](../../tools/performance/baselines/issue244/README.md).

The engine [persistent net registry](../formats/net-registry-storage.md) preserves independent
UUIDs and neutral numbers for meaningful nets, with bounded anchors, split/merge
lineage and explicit ambiguity resolution through shared headless/live MCP
commands. It survives undo, save/reopen and first-board creation without storing
a second connectivity graph. [Differential pair records](../formats/net-registry-storage.md#differential-pair-records)
retain explicit positive and negative net UUIDs and stored review intent. This
is storage and inspection only, not pair authoring or routing.
[Net-class assignments](../formats/net-class-assignments.md)
can be authored through shared commands, with retained origins, split inheritance,
merge conflicts and protected class removal. [Class-aware routing, clearance
queries and DRC](../formats/net-class-consumers.md) consume those bindings with effective
dimensions and per-object provenance. A [working-voltage requirement](../formats/electrical-intent.md)
can be declared on persistent nets or classes in Rules or through shared commands. It is reported as
unverified and does not affect routing or checks. [Editor controls](../pcb/net-class-editor.md) use the same commands and scoped identities. Zones,
repeated instances and system registries remain separate work.
[Part groups](../formats/part-groups.md) collect part instances by stable part-instance
ID through shared headless/live MCP commands, with a project-wide catalog,
explicit membership, protected removal, undo and save/reopen. Dense membership
uses compact native storage without reducing the supported set of 4,096 part
instances in 256 groups. Copy/Cut leave groups project-owned; pasted independent
parts start ungrouped. Rules that consume groups are later work.

The boardless PCB workspace explains setup, footprint updates and routing, with
explicit Create board and Open schematic actions. Create board opens a disposable
setup draft; Apply creates the board and settings together, while Cancel leaves
the project unchanged. Board defaults and net classes are available after creation.

The engine [single-board update workflow](../pcb/single-board-update.md) previews and
atomically accepts footprint/package and connectivity changes through shared
commands and headless/live MCP. Desired assignments and accepted geometry remain
distinct across offline reopen; package choices can be assigned and saved before
any board exists, then adopted by explicit first-board creation. Deleted schematic parts retain their footprints
until acceptance. The [PCB ratsnest](../pcb/board-ratsnest.md) displays only accepted
logical connections, with per-view visibility, live MCP control and explicit
unavailable endpoints. The Update PCB dialog reviews those typed changes from
either workspace in a scrollable change report, with category filters, unfiltered
warning/error totals and clipboard export. It resolves missing choices through the
package chooser and, by default, hands
accepted unplaced parts to a spaced cursor group for one-click placement, with
quarter-turn rotation and cancellation. Package assignment refreshes the review
directly; compatible pad numbers are preselected for named and unnamed terminals, with
saved choices taking precedence over revision-qualified library defaults. Full change
records remain available in the report, and a placement option can leave the
updated report open. Atomic group placement shares the engine and
live/headless MCP command, while the individual X/Y chooser remains available. Stale or replaced previews require
re-review; legacy current-state adoption and board creation remain separate
explicit edits.

Typed primary quantities captured in accepted board history remain SI-formatted
in PCB Properties and Browser after schematic deletion, until reviewed footprint
removal. Older and untyped history retains its stored text; see the
[native storage contract](../formats/native-project-storage.md).
