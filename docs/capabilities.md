# Current capabilities and MVP scope

For a first session, use the [quickstart](quickstart.md) or
[editor guide](editor-guide.md). The [documentation map](README.md) links topic
guides, format references, design plans and verification records.

## Capability inventory

| Workflow | Implemented slices | Remaining scope |
| --- | --- | --- |
| Projects | Compact Home cards start a New Schematic or New PCB in the chosen workspace; New Project opens Untitled at once and a desktop first save names it after its folder/archive; rename, open, save/save-as folders or `.pc`; desktop Home remembers 12 recent projects with compact rendered schematic cards with previews sized to the card width and reopen/remove/clear actions across restarts; Save/Don't Save/Cancel before closing, New or Open replaces unsaved work; shared undo/redo; supported split document schemas remain readable under the [current project layout](native-project-storage.md#compatibility-and-fixtures); explicit CLI conversion copies the previous split layout to a new folder or package without changing the source; [stable single-use sheet occurrences](sheet-occurrence-storage.md) survive hierarchy edits, first-board creation, undo and save/reopen | Local recovery of unsaved work: [#69](https://github.com/tangency-labs/pcbcraft/issues/69) |
| Schematic editing | Up to 64 flat or hierarchical sheets with create/rename/delete and A4–A0 landscape/portrait paper (Browser **Paper** menu or `sheet.set_page`), drawn on a uniform theme-following canvas by default with a frame (Browser **Frame** menu or `sheet.set_template`): the Default border, zone markers and parameter-filled title block, or a plain border; Appearance preferences offer optional paper shading and per-theme sheet/canvas colours, while the canvas around it stays editable and keeps its grid unless **Show grid outside paper** is off; Fit and Home cards frame the paper, and older sheets get paper around their drawing on open; project and sheet parameters (`@project.Revision`, `@sheet.Title`) edited in Properties with nothing selected or through `parameter.set`/`parameter.list`/`parameter.resolve`, with sheet values defaulting to the project's, and read-only built-ins (`@instance.Name`, `@instance.Page`, `@instance.Pages`), S placement, body/title movement and four-corner resizing with attached-wire previews, double-click entry/Alt+Backspace return, synchronized child labels/parent pins and explicit inter-sheet connectivity, explicit command targets, independent pane navigation and shared undo/save/reopen; compact/expanded selection properties with atomic reference/value/orientation and unit-aware X/Y edits for selected symbols, attached-wire movement and read-only terminal/library details; place, rotate, mirror horizontally or vertically about the symbol center with attached wires, edit standard/custom component fields and population/exclusion attributes, move/delete symbols; independently drag reference/value/custom text with per-unit visibility, name display, physical size/alignment/rotation, offsets and automatic-layout reset; local/global net labels and global power/ground anchors with named inspection, shared GUI/MCP commands and undo/save/reopen; M/Disconnect and Move picks a selected or hovered symbol and preserves old wiring for replacement; M moves a selected whole wire or mixed group while disconnecting contacts to unselected objects and preserving internal connections/no-connect markers; G and group dragging retain external connections; drawing-bound overlap warnings with compact amber counts, reference tooltips and a right-click chooser for hidden symbols; directional boxes, modifier selection and group move/delete for components, wires and named-net anchors; system clipboard copy/cut/paste and duplicate across panes/windows/instances, pinned native definitions and package intent, rotation, reference/net-name options and one-step undo; explicit plain-text conversion to local/global net-label groups; complete hierarchical subtree/interface clipboard, independent duplication or explicitly selected cut relocation, stable cached previews and pan/zoom | Repeated child instances remain deferred; remaining group G-drag: [#44](https://github.com/tangency-labs/pcbcraft/issues/44); template editing, logos and frames in PDF output: [#46](https://github.com/tangency-labs/pcbcraft/issues/46) |
| Wiring | Orthogonal routes with a held-Ctrl grid override; direct starts at unconnected pins/free ends, W at the pointer and cached magnetic targets; segment drag/G and Delete; corner/free-end/shared-junction handles; pin/endpoint/segment attachment; exact part/wire drop contacts and two-pin insertion into straight runs; overlap/return healing and new-route loop replacement, enabled by default with a saved Auto-heal wires toggle in Preferences → Schematic and the Modify menu; Slice, Heal and Delete in the ribbon; contextual Break releases one or both attached ends, with free movement of detached straight pieces, guarded departure and reconnection on return; shared undo and headless MCP commands | Obstacle-aware routing [#30](https://github.com/tangency-labs/pcbcraft/issues/30); ambiguous contacts and edits beyond healing's complexity limits are refused; loops with intermediate pins or branches are preserved |
| Libraries | Bundled Basic components and Power native libraries with ordinary component instances, explicit global-terminal roles and editable instance defaults; 30 Basic components and 11 Power symbol types; 18 original SMD and plated-header footprint definitions; saved built-in library switches and configurable typed placement filters; native generic part/package/MPN selection, typed exact/min/max/range/one-of requirements with canonical units and compatibility findings, and complete pinned dependencies survive offline reopen; up to 32 independently placed schematic units per part with shared terminal identity, fields and package assignment across sheets; independent symbol/footprint authoring workspaces available alongside schematic/PCB in the shared workspace picker, with Draw/Library/View ribbons, mixed splits, detach/dock/native windows, per-pane navigation/selection and primitive properties, integrated pin leads/labels/styles, oval and alphanumeric pads, editable curves/text and technical layers, generated footprint references with editable default offsets, group transforms, numbered duplication/arrays, internal clipboard, native validation, separate undo, draft-save/open/export, desktop native-folder publication with stable item identities and immutable revisions; generic/orderable part documents inside Parts Library with exact symbol/package references, explicit complete pad maps, typed fixed values/specifications, separate Undo/Redo, session-only drafts, publication and dependent-part reference review; live MCP | Further library/source workflows [#92](https://github.com/tangency-labs/pcbcraft/issues/92); advanced authoring, project application and persistent definition layouts [#78](https://github.com/tangency-labs/pcbcraft/issues/78)/#79; Git/catalog sources [#93](https://github.com/tangency-labs/pcbcraft/issues/93)/#94 |
| Navigation | Dockable Layers, Properties and Selection filters with header dragging, edge stacking, per-view local arrangements and live MCP positioning, and a Floating/Compact panel style that joins the ribbon, Browser and docked cards to the window edge in Compact; per-view selection filters for symbols (including power/ground), wires, labels/sheets, footprints and board graphics, with All items/Reset, collapse and disabled unsupported categories; Zoom (one shared manual range up to 1024× across all four editors), middle-button drag pan or drag up/down zoom, optional right-button drag pan, Fit, floating Browser/Net Inspector cards, Parts Catalog with cross-library/MPN search, a dense expanded list, independent symbol/package zoom previews, remembered window/divider sizing and a compact dock/float presentation, once/repeat part and power placement, theme/scale/navigation preferences; per-view schematic and PCB grid spacing with separate remembered preferences and presets, and shared opacity; PCB grid hidden by default, with Off/Dots/Lines in Preferences; mil/mm display units with a pointer coordinate readout and unit-aware spacing entry | Remaining footprint/wire geometry property fields with the inspector [#50](https://github.com/tangency-labs/pcbcraft/issues/50) under [#75](https://github.com/tangency-labs/pcbcraft/issues/75) |
| Board rules | Saved board-scoped hard dimensional limits and preferred routing sizes, with dimension illustrations beside the shared Rules fields, through Board Setup → Design rules or Analyze → Rules → Board defaults and shared GUI/live/headless MCP; stable rule identity, effective values/provenance, diagnostic overrides that cannot weaken authored limits, DRC/query integration and undo/save/reopen | Project-level [class definitions and dimensional resolution](net-classes.md), plus [persistent assignments](net-class-assignments.md), are available through shared commands/MCP; [class-aware routing, queries and DRC](net-class-consumers.md) use effective dimensions with per-object provenance. The [shared editor form](net-class-editor.md) creates/edits classes, assigns supported current nets and inspects effective values and conflicts without requiring a board; advanced electrical/manufacturing analyses remain separate under #77 |
| Layout | [Single-layer zone outlines](copper-zone-outlines.md) with net selection/promotion, polygon/rectangle creation, edge/vertex editing, boundary hatching, modifier/box/mixed selection and atomic Delete, Properties, layer/filter visibility, shared commands and undo/save/reopen; no fill yet; Closer initial empty-board drawing scale with independent saved navigation; schematic-only projects and explicit creation of the first board, with optional board targets and compatible migration of supported implicit-board schemas; Core-resolved physical pad-layer membership with per-layer copper rendering, editable copper/technical layer swatches, ribbon color patches, keyboard layer navigation and a local palette shared across views; layer-addressed Board Outline lines and rectangles with canvas creation, selection, vertex/whole-graphic movement and deletion, shared GUI/live/headless MCP transactions, undo, stable folder/archive persistence, layer visibility and Fit; copper track routing on any setup copper layer with X, an explicit width, 45° posture, a route context menu, layer-colored previews that remain stable during background preparation, with a resolved-clearance halo, a pointer-following crosshair, proximity-based grid snapping with held-Ctrl override, and pad, track-end and via snapping, through vias with V and explicit diameter/drill that switch the routing layer, [explicit via net assignment](via-net-assignments.md) through searchable placement and Properties choosers with bounded result pages, automatic contact inheritance, unresolved-target diagnostics and same-board clipboard preservation; standalone vias with Place › Via (Ctrl+Shift+X; macOS: Command+Shift+X; native non-Latin physical-X fallback), box/U/click selection of tracks and vias with lighter copper colors matching pads, M and drag moves, D segment drags that keep neighbors attached (`pcb.reshape_tracks`), footprint moves, rotations, flips and alignments that carry attached track ends and refuse new shorts, width and via size editing in Properties, a design rules check (Inspect › DRC and `pcb.check_design`) for conflicting intended nets within a copper island (including direct pad and via-mediated contact), copper clearance against resolved circular, oval, rounded-rectangle and rectangular pad contours, track width, annular ring, unrouted connections, board outline/edge and courtyard overlap with a results card, canvas markers and status; cancellable native/background and browser/incremental DRC with stage progress, coalesced reruns, stale-completion rejection and deferred live MCP checks; refusal of net-shorting copper, layer-colored net highlighting with Backtick and automatic copper emphasis while routing from a trace, independently selectable and movable footprint reference text with owner highlighting and airwires, placement-local printed words/font/size/rotation/alignment/visibility in Properties, explicit shared component renaming, and matching silkscreen output, exact footprint X/Y/rotation/side entry in Properties, align and distribute (Modify menu and `pcb.align_footprints`), bottom-side footprints (F, Modify › Flip and `side` on `pcb.set_footprint_transform`) mirrored onto the back copper and technical layers with board format 14 persistence, a view-only ruler (Inspect › Measure, Ctrl+Shift+M) snapping to pads, track ends and vias, a mirrored bottom view (Design › Flip view and live `view.set_board_flipped`), fabrication outputs (Design › Outputs and `pcb.export_fabrication`) as Gerber X2 copper/mask/paste/silkscreen/outline layers and an Excellon drill file in one deterministic archive with unique portable entry names, measured from a named placement origin (`pcb.set_placement_origin`: board zero, outline lower-left or a point), immutable project-pinned assembly output profiles with bottom-side conventions and per-definition rotation/centroid corrections (`pcb.set_assembly_profile`, project format 36), read-only correction/placement previews and explicit digest-checked board adoption (`pcb.preview_assembly_profile`, `pcb.adopt_assembly_profile`, board format 22, or 23 with stack fields); at most 64 identities, 128 retained revisions and 4096 corrections per project; adoption-safe retirement and Undo/Redo, with no placement file exporter or profile-management UI yet, click selection and deletion, airwires cleared by routed copper, one undo step per route and per-segment board format 12 (tracks) and 13 (vias) persistence; PCB workspace with the shared project Browser, grouped Layers, placement Properties and Board setup with board-local physical silkscreen text/stroke/clearance settings, and physical SMD pad/body/courtyard rendering alongside legacy pad-center markers; separate package assignment and coordinate-based initial placement through a shared chooser in either workspace, with explicit terminal mappings and pinned offline packages; directional footprint box selection with modifier combinations, single-footprint move/quarter-turn rotation and selected-footprint deletion through canvas, ribbon and shared GUI/live/headless MCP commands, with snapping, cancellation, undo and offline persistence; engine commands for starter SMD packages, a legacy two-pad marker and explicit terminal-to-pad lookup; one complete Craft Board setup form on first and subsequent opens, with two-layer FR-4/1 oz/1.6 mm starting settings, generic material families, a physical silkscreen preview and shareable `.pcsetup.json` setup snapshots, for every even copper count from 2 through 64, explicit compatible populated-board resolution, layer names/optional membership, cross-section aligned to the table and a laminate total, exact optional thickness/tolerance and locked-gap fitting, core/prepreg construction and sourced Tg/Dk/Df material revisions; copper-layer roles, base copper and finished copper tolerance/basis, and declared finished board thickness with tolerance and minimum through `pcb.set_board_setup` (board format 23); validated layer catalog and uniform rigid stackup, custom materials, sourced Tg/Dk/Df reports, core/prepreg plies, undo and offline persistence through headless commands; system clipboard copy/cut/paste and duplicate across panes/windows/instances for mixed footprints, outlines, tracks and vias, explicit schematic/layer choices, cut relocation, rotation and atomic undo; explicit CSV/TSV coordinate conversion to Board Outline polylines with units, closure and Y inversion | Manufacturing-profile editors, impedance calculations and general layer bindings [#76](https://github.com/tangency-labs/pcbcraft/issues/76), shared nets/rules [#77](https://github.com/tangency-labs/pcbcraft/issues/77), editable board canvas [#54](https://github.com/tangency-labs/pcbcraft/issues/54) and the rest of basic track/via editing (push-and-shove, clearance checks) [#95](https://github.com/tangency-labs/pcbcraft/issues/95) |
| Cross-probing and views | Saved layouts under File → Layouts, per-layout activation and management, naming dialogs, confirmation before deletion and user-local save/reopen restoration, one Browser tree for Sheets/Boards and their symbols, nets and footprints, navigation and selection within the focused pane; ordinary Browser clicks retain the current workspace, with explicit context actions for cross-workspace opening; shared tree-row metrics and disclosure controls in Symbol/Footprint editors, and a closable, draggable header per split or detached pane, with no document tab strip in the main single view; drag-out creates a window in the same running instance, with same-window pane reordering that retains dividers/cameras/cached documents, source shading, animated destination previews, a visible Dock button on detached headers and window drop targets anchored beside the grabbed header on every platform; drag-in docks beside a destination pane, with direct cross-window drops when positions are available; schematic and PCB inspection targets in per-window split layouts; multiple native editor windows with Linux title-bar dragging/maximize/restore and edge/corner resizing, including while project dialogs block editing; splitting inside them, tab placement/close actions, independent cameras and selections, shared edits/undo/save, bidirectional component counterpart highlights and optional directed following. The workspace picker opens a target here, beside the current view or directly in another window. Any window can close while others remain; a survivor takes over project controls. Last-window Close or explicit Quit prompts once. Live MCP drives the same view, workspace, arrangement, window and panel actions by ID, including footprint selection; headless/live MCP can look up the supported pin/pad mapping | Editable-board acceptance [#54](https://github.com/tangency-labs/pcbcraft/issues/54), individual pin/pad selection and missing-mapping feedback [#65](https://github.com/tangency-labs/pcbcraft/issues/65), multiboard storage/assignment [#122](https://github.com/tangency-labs/pcbcraft/issues/122), portable arrangement export, full multi-monitor restoration, outside-client drag previews, provider view payloads |
| Automation | Headless command catalog; the desktop live connection runs every project command with shared undo, plus window commands for schematic and PCB views, workspaces, footprint selection, named arrangements under File → Layouts, windows, panels, preferences, Home and in-place Save, and `inspect_editor`; opt-in Assistant chat with installed ACP agents (Claude Code, Codex and others) or OpenAI-/Anthropic-compatible model endpoints, running the same commands with SolidCraft's permission modes and approvals; endpoint keys in the system keychain; read-only `project.export_netlist` writes the schematic's components, logical terminals and nets across all sheets as a deterministic [JSON netlist](netlist-format.md) | Headless sessions are separate from the open GUI |
| Sharing | Native folder/archive storage; a routed ATtiny402/TMP102 sensor-node fixture and a secondary transistor flasher, with command-built generators and offline dependencies; source builds for desktop/web | Polished demo and preview artifacts [#91](https://github.com/tangency-labs/pcbcraft/issues/91); user/contributor onboarding [#90](https://github.com/tangency-labs/pcbcraft/issues/90) |
| Copper locking | Shared GUI/MCP track/via lock command, explicit inspection, atomic protection for movement/reshape/size/delete and indirect copper transforms; lock-preserving clipboard copies, undo/redo and native reopen | Footprint/zone locks and locked-object selection filters remain in #81; walkaround treats all existing copper as fixed, and shove solvers remain in #247–#248 |
| Native library sources | Read-only Built-in Basic components/Power registrations; Parts Library workspace with Project/Built-in/Sources groups, exact project-pinned revisions, usage and source status, offline previews, native source previews and nested/flat orderable records; desktop remembered folders, no-clobber bundle installation and confirmed system-trash deletion; whole-library bundle export (desktop file, browser download); browser folder/bundle reads with names retained for reopening; symbol/footprint/part/category/manufacturer search with file/conflict diagnostics, exact part resolution and canonical quantity normalization; native part placement and typed requirements; external footprint assignment; complete project pinning and offline reopen after source relocation/removal; [source guide](native-library-sources.md) | Git sources [#93](https://github.com/tangency-labs/pcbcraft/issues/93), provider catalogs [#94](https://github.com/tangency-labs/pcbcraft/issues/94), browser/remote publishing and reviewed project updates |
| Later professional work | Planned, not implemented as a complete workflow | Full ERC/DRC [#47](https://github.com/tangency-labs/pcbcraft/issues/47)/#58, copper-zone fill/output [#57](https://github.com/tangency-labs/pcbcraft/issues/57), fabrication [#59](https://github.com/tangency-labs/pcbcraft/issues/59), advanced routing [#85](https://github.com/tangency-labs/pcbcraft/issues/85) and 3D/STEP [#86](https://github.com/tangency-labs/pcbcraft/issues/86) are outside the first MVP |

## Implementation notes

[Intentional no-connect markers](no-connect-markers.md) attach to logical part
terminals and open wire ends through Q, Capture and context actions. They follow
placed units and endpoints, support ordinary click/marquee selection and
Delete/Backspace without deleting their owners. A wire-end X can stretch its
attached wire, and Q/Capture clears previous selection/highlights. Marks survive duplication/clipboard and native save/reopen, and have a focused,
read-only check with navigable missing, incompatible and connected-target
findings, including unmarked dangling wire ends. Marker edits preserve the actual
netlist and physical DRC findings. Marked targets block new wires and endpoint
attachments through both GUI and MCP until the mark is removed; automatic wire
hover/start is suppressed throughout their magnetic target area. Full type-conflict/power-driver ERC and incremental scheduling remain
with #47/#245/#243.

Typed quantities accept and display engineering prefixes from atto (`a`) through
exa (`E`), including femto (`f`) and peta (`P`), with exact canonical values and
round-trip editing. See the [quantity contract](part-schema.md).

This table starts from the baseline at `091adfd` (November 6, 2026), with the
wiring and views rows updated for endpoint attachment, local healing, PCB inspection
and saved arrangements. Update it alongside feature changes.
The [quickstart](quickstart.md) exercises the working schematic workflow;
[MVP acceptance](https://github.com/tangency-labs/pcbcraft/issues/96) defines what
still needs to work before the first collaboration-ready preview.

DRC and the engine's [snapshot copper query](copper-clearance-query.md) share
supported pad/track/via contours, contact and clearance predicates. Queries retain
board, snapshot, revision and effective-clearance authority, with conservative
candidates and an exhaustive reference. Native circular arcs remain separate work.
[Walkaround route planning](route-planning.md) (`pcb.plan_route`) uses the same query
to plan single-layer H/V/45° routes round existing copper with class-aware widths,
clearances and board-edge clearance, without editing; the route tool's interactive
use of it is later #246 work. Captured local queries
now reuse a layer-aware packed index and support disposable geometry overlays;
see the [query contract](copper-clearance-query.md) and
[measurement limits](../tools/performance/baselines/issue244/README.md).

[Shared readable net names](net-labels.md#generated-names-and-copper-labels)
cover schematic browsing, Rules, PCB inspection and fitted pad/via/track overlays.
Accepted boards retain their reviewed names until Update PCB; the overlay toggle
is view-local. Repeated-instance and multiboard naming remains within the current
single-use/single-board scope.

The engine [persistent net registry](net-registry-storage.md) preserves independent
UUIDs and neutral numbers for meaningful nets, with bounded anchors, split/merge
lineage and explicit ambiguity resolution through shared headless/live MCP
commands. It survives undo, save/reopen and first-board creation without storing
a second connectivity graph. [Net-class assignments](net-class-assignments.md)
can be authored through shared commands, with retained origins, split inheritance,
merge conflicts and protected class removal. [Class-aware routing, clearance
queries and DRC](net-class-consumers.md) consume those bindings with effective
dimensions and per-object provenance. [Editor controls](net-class-editor.md) use the same commands and scoped identities. Zones,
repeated instances and system registries remain separate work.
[Part groups](part-groups.md) collect part instances by stable part-instance
ID through shared headless/live MCP commands, with a project-wide catalog,
explicit membership, protected removal, undo and save/reopen. Dense membership
uses compact native storage without reducing the supported set of 4,096 part
instances in 256 groups. Copy/Cut leave groups project-owned; pasted independent
parts start ungrouped. Rules that consume groups are later work.

The boardless PCB workspace explains setup, footprint updates and routing, with
explicit Create board and Open schematic actions. Create board opens a disposable
setup draft; Apply creates the board and settings together, while Cancel leaves
the project unchanged. Board defaults and net classes are available after creation.

The engine [single-board update workflow](single-board-update.md) previews and
atomically accepts footprint/package and connectivity changes through shared
commands and headless/live MCP. Desired assignments and accepted geometry remain
distinct across offline reopen; package choices can be assigned and saved before
any board exists, then adopted by explicit first-board creation. Deleted schematic parts retain their footprints
until acceptance. The [PCB ratsnest](board-ratsnest.md) displays only accepted
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

Symbol Grab/Move previews support R rotation, with movement and orientation committed
together as one undo step and discarded together on cancellation.
Power/ground G-drag retains touching wire contacts regardless of placement order,
with connected previews and shared undo; fixed pins stay in place. New placements
are ordinary native library instances; the old power-marker styles are unsupported.
Schematic part/power placement repeats by default for new settings, including catalog
drag/drop with custom power-net names, and completed wires leave the tool ready for another route. Canvas context menus provide the
supported drawing, selection and view actions, with spacing choices in a Grid submenu.
The [editor guide](editor-guide.md) covers one-shot placement and cancellation.

Issue numbers above refer to the
[repository backlog](https://github.com/tangency-labs/pcbcraft/issues). The
[MVP milestone](https://github.com/tangency-labs/pcbcraft/milestone/1) selects
bounded editing, cross-probing, library and demo slices; the
[professional workflow index](https://github.com/tangency-labs/pcbcraft/issues/89)
also includes later work. Completing the MVP is not a fabrication-readiness claim.

Desktop saves write to the chosen folder/archive. App version 0.1.0 uses
`project.json` entry points and container-v3 `.pc` ZIP packages, intentionally
breaking older project layouts (see [release notes](../CHANGELOG.md)).
Browser uploads use current archives; downloads produce `.pc` and do not clear the dirty state
because the browser cannot confirm a durable write, so the page's leave warning
stays on until the project is replaced. The desktop Assistant and
local MCP listener are not available in the browser build.

Native live MCP also provides [named UI targets and queued input](ui-automation.md)
for demonstrations and interaction automation. Common chrome adapters and project
canvases publish targets; specialized widgets may still require coordinates.
Pointer, key, text and scroll operations use normal UI handling and report
completion or cancellation through a bounded ticket. Input supports the original
native window and its split panes; detached windows currently support lookup
only. This does not record video.

The schematic ribbon's **Ground** action starts a GND placement directly. **P**
opens the built-in Power collection, which includes both arrow-shaped **VCC** and
flat-bar **VCC (bar)** choices with the same editable default net name.

The board walkthrough, polished demo and fresh-user onboarding acceptance in [#90](https://github.com/tangency-labs/pcbcraft/issues/90)
remain pending their dependencies. Documentation command checks and automated UI
tests do not substitute for a new person's end-to-end acceptance run.

PCB Draw tools operate on the active visible layer: copper linework uses the
validated conductive-segment path, while non-copper graphics preserve physical
strokes through saves and clipboard operations. Silkscreen, mask and paste
artwork is included in the matching fabrication layer; documentation-layer
graphics remain native board content. Persistent layer/width/via controls stay
visible across PCB tabs, with routing in Layout and alignment in its Modify menu.

Typed primary quantities captured in accepted board history remain SI-formatted
in PCB Properties and Browser after schematic deletion, until reviewed footprint
removal. Older and untyped history retains its stored text; see the
[native storage contract](native-project-storage.md).

The **Parts Library** workspace browses native libraries with search/Type, nested and
flat orderable parts, native previews, revisions and dependencies. Desktop source
actions include remembered folders, refresh/relocate, bundle installation into
unused folders, removal and confirmed system-trash deletion of whole libraries, plus individual
generic/orderable identities on Unix desktop with dependency and open-document
guards. Browser folder/bundle
reads remember names that need reopening after reload. See
[native library sources](native-library-sources.md) for platform limits. Share ›
Export writes the selected library to one bundle file. Disabled planned controls
do not yet provide comparison or connections.
Desktop New library, New symbol/footprint, Edit and Edit copy use the existing
authoring workspaces; Publish writes immutable native revisions into writable
registered folders. Browser publishing remains unavailable.
Schematic, PCB layout, symbol and footprint editors share manual zoom up to
102400%, with pointer-anchored wheel/pinch navigation and content-specific Fit.

Generic-part authoring supports independently optional symbols and footprints in
`pcbcraft-part/2`, including footprint-only and metadata-only definitions. Retained
format-1 parts remain readable. Parts Library can publish, search and preview
these definitions and their manufacturer records. Schematic placement requires a
symbol; direct board-only part placement is not yet implemented.
