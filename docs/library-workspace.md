# Parts Library workspace scope (#315)

The implementation follows the browse and orderable-parts renders supplied by
@travle1 in [#315](https://github.com/tangency-labs/pcbcraft/issues/315). Their
composition is the reference; the running application's pinned Craft controls,
type, colors, spacing and interaction states define the visual treatment.

| Region | Delivery in #315 |
| --- | --- |
| Shell | Fifth available workspace, shared workspace picker and ribbon, Browser, Preview, pane headers and native window actions. It opens without a project and supports mixed splits with definition editors. |
| Sources ribbon | Add library, Refresh, Relocate, Remove and Delete, with availability derived from the selected source and platform. Existing Sources and Manage libraries actions navigate here. The old management modal is removed. |
| Browser | Built-in and Libraries groups; source icon, name, count, read-only mark and exceptional status; four child rows: Symbols, Footprints, Generic parts and Orderable parts. Missing and failed registrations remain visible with diagnostics. |
| Library list | Search and Type choice, selected source name/location/license, count, selectable rows, Type, Category, Packages, Orderable and revision columns. Generic parts expand to show their orderable parts and counts. Category records remain dependency metadata rather than a fifth tree row. |
| Orderable list | Flat view of the same manufacturer records, with Manufacturer, Part number, Generic part, Package, Value, Datasheet and Rev columns. Search includes manufacturer and part number. |
| Preview | Actual native symbol/footprint geometry or a record summary, identity and revision, license and provenance, category/units, packages and pin mappings, orderable parts/specifications, available revisions and exact dependencies. Missing dependencies and quarantined conflicts remain explicit. |
| Compact layout | Shared controls and surfaces, bounded scroll regions, accessible Browser/Items/Preview choices when three usable columns cannot fit. Long names/paths truncate with full text available. |
| Desktop lifecycle | Remember folders, reload at startup, refresh/relocate, validate and install bundles into unused folders, reject fully redundant bundles before writing, forget registrations on Remove, and confirmed system-trash deletion of writable folders with no unpublished editor changes. |
| Browser lifecycle | One-time folder or bundle reads under native limits; remember names without disk handles or contents; reload shows Needs reopening and supplies no items until reopened. Remove forgets the entry; Delete is unavailable. |
| Shared state | Per-pane browsing state and existing view placement/lifecycle. Live MCP can choose and inspect Library. Browsing and source changes preserve project bytes, revision, dirty state and undo history. |

The mockups also illustrate later work. New library, New symbol, New footprint,
New generic part, Edit, Edit copy and the part form belong to #308/#309. Export
belongs to #310; Category/Package/Manufacturer/Value filtering and Compare to
#311; update availability and adoption to #312; Links to #314. Git and remote
source examples belong to #93/#94. Planned controls may remain visible with the shared disabled treatment, as requested by @travle1; they do not execute actions. Invented sample records do not enter the catalog. The Parts Catalog placement picker remains
its existing workflow.

The pinned toolkit already provides tree rows with disclosure/count/status slots,
selectable tables with shared columns, ribbon groups, panel surfaces, search,
choices and confirmations. Adapters belong in `crates/chrome`; the host owns
source identities, filtering, selection, commands and filesystem work. Native
symbol and footprint geometry reuses PCBcraft's qualified preview painters.
Any presentation gap discovered during implementation must be recorded here.

Verification targets the source lifecycle and the new workspace: project-state
invariance, late/stale worker completion, no-clobber installation, redundant
installation refusal, dirty editor protection, browser reopening, source
diagnostics, real selection/filter input, mixed panes and MCP inspection. Visual
QA compares wide dark and compact light renders with the reference composition
and existing app. This work makes no presentation-latency or Wayland endurance
claim and does not require repeating unrelated routing/DRC benchmarks.

The pinned kit has the upright-books `library` master, which the workspace uses.
It has no standalone source-refresh master or folder Add/Relocate/Remove variants.
Those ribbon actions compose its existing folder with original action marks;
the refresh loop is original chrome geometry using the kit's icon weight and
palette. Native drawings remain separate from these generic interface graphics.
Preview uses the existing draggable/collapsible panel host and docking menu.
Its default right dock reserves a full-height column aligned with Browser and
Items; only an explicit Float choice or header drag overlays the table. The
shared panel adapter supports exact dock slots while preserving the existing
canvas-card spacing elsewhere. Runtime drag ownership remains with the pane
between frames. Shared pane headers are clipped separately from workspace
content, and every workspace uses the same native-window resize epilogue.
Add library opens a trigger-anchored Craft menu with full-width action rows;
Git and remote connections remain disabled.


Preview navigation is presentation state owned by each pane. Symbol and footprint
canvases share the footprint picker's pointer-centred wheel zoom and drag-to-pan,
with independent cameras, Fit and zoom buttons; double-click also fits. Selecting
another item or revision resets both cameras. Wheel input over geometry zooms
that canvas, while wheel input over metadata continues scrolling the inspector.

A floating Preview uses the pinned toolkit's accessible width and height sashes,
with arrow-key adjustment after focus. Its position menu provides Expand and
Reset size. Dimensions survive docking and saved local arrangements; older
arrangements without dimensions retain the default size. Escape or focus loss
cancels an active resize. Expanding the Preview changes only its host layout,
never a library record or project document. The pinned kit's workspace card supplies side anchoring and an inner width rail.
Free positioning, independent height and full-host expansion are composed in the
existing shared `Panel` adapter using the kit's accessible sash controls.

An interactive 3D view is a later extension of this Preview host, contingent on
[#86](https://github.com/tangency-labs/pcbcraft/issues/86)'s model records,
package bindings and renderer. That future view should retain the same source
and revision ownership while using its own 3D camera. The current work renders
native 2D symbol and footprint geometry only.


Live clients use `app.library_browse` with `preview: "symbol"` or `"footprint"`
and `fit`, `zoom_percent`, or paired `pan_x`/`pan_y` values in logical canvas
points. Inspection reports both independent cameras. The existing
`view.set_panel_position` accepts `panel: "preview"` in a Library pane, with
paired `width`/`height` for floating placement. Invalid compound requests leave
both browsing and navigation unchanged. These commands share the GUI's camera
and panel state without editing project data.


The picker orders Schematic, PCB Layout and Parts Library before the definition
editors. All five working workspaces use the shared chooser's fixed wide tile
or fixed compact button at one breakpoint. A title's length and extra ribbon
control rows do not change that chooser's dimensions. The display name is Parts
Library; its stable command/layout key remains `library`.

## Part documents (#309)

The New generic part action and Edit/Edit copy for generic and orderable records
open session documents inside this workspace. Browser keeps its source/project
navigation; Items shows the association form, while Preview shows the selected
symbol unit and package footprint. The Browse tab returns to browsing without
discarding the form. Choosing another source in Browser also returns to Browse.
The mapping table and document controls compose the pinned Craft table, choices,
fields and buttons through `crates/chrome`; correspondence highlights reuse
original electronics geometry. This composition needs no new generic control or
external asset. Package model bindings and descriptive fields remain with #86
and #314 respectively. See the [part document workflow](native-library-sources.md#generic-and-orderable-part-documents)
for publication, close protection and shared commands.

Part association fields show their selected item and a Browse action. The chooser
uses catalog search, a library filter, native preview and explicit exact revision
selection, with 50 results per page. It does not expand every installed record
into a field dropdown. Matching metadata is rebuilt only when the query, filter
or source generation changes. The shared table's explicit row-reveal API keeps
keyboard selection visible; narrow hosts use Results/Preview choices inside the
same bounded chooser and fixed Choose/Cancel footer.

Part documents keep their fields on a bounded reading rail when Items is expanded.
Symbol is optional and can be cleared. The Footprints section has an **Add
footprint…** action that opens the searchable chooser; choosing creates one package
and cancellation leaves the document unchanged. A symbol-less part previews its
footprint without a blank symbol column. Pin mapping appears only when both a
symbol and footprint are present.

Named part-document tabs remain visible while browsing. The shared tab strip
shows unsaved dots, keeps the selected document visible through overflow, and
supports ordering and close protection. Selecting a draft returns compact hosts
to Items; Browse and source-tree navigation retain the open session document.
The part form keeps its primary Publish action in a fixed footer outside the
scrolling fields. A single footprint needs no package selector; package naming
and the reserved model slot are in Package details.
