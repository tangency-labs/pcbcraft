# Library workspace scope (#315)

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
Preview uses the existing draggable/collapsible panel host, including its docking
menu, rather than a separate library-specific panel control.
