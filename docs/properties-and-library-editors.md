# Properties and library editors

The properties inspector and the symbol/footprint editors share Craft controls,
but edit different things. Properties changes a selected project's component or
placement. A library editor authors a definition with its own identity, draft,
validation, undo and save lifecycle. Opening a definition must not silently change
its placed copies.

The user requested compact and expanded properties following PCBcraft Next, plus
symbol and footprint editor workspaces on November 7, 2026. The presentation
reference is Next's `src/desktop/inspector.rs` at
`568cd495397a91563e563e7163efb226d84fa83a`, MIT OR Apache-2.0, and the user's
read-only `02-schematic-properties.png` FigCraft export. Neither design files nor
the prototype are modified by this work.

## Product notes from published manuals

These observations describe documented behavior, not independent usability tests.
Only publisher user documentation was consulted. No KiCad/Altium implementation,
icons, artwork, library assets or example project files informed the implementation.

| Reference | Documented feature | PCBcraft direction |
| --- | --- | --- |
| [KiCad 10 Schematic Editor](https://docs.kicad.org/10.0/en/eeschema/eeschema.html), Editing symbol properties / Properties Manager | A selection inspector complements a detailed properties dialog; compatible properties can be edited across a selection. | Keep the compact inspector and expanded form on the same command path. Add bulk editing only for explicitly supported common properties. |
| Same manual, Creating and editing symbols | The symbol editor separates graphics, terminals and fields; library edits and schematic-copy edits have different update scopes. Pin tables support larger devices. | Preserve stable terminal identities. Make source versus project-copy editing explicit, and review updates to existing components. Treat multi-unit support and pin-table import as later bounded work. |
| [KiCad 10 PCB Editor](https://docs.kicad.org/10.0/en/pcbnew/pcbnew.html), Footprint editor / Footprint pads | Footprint authoring has its own canvas; pads have explicit numbers, geometry and technology. Repeated placement, pad tables, arrays and defaults assist larger packages. | Start with original SMD and plated-hole geometry supported by the native schema. Keep pad numbers distinct from positions and preserve mappings. Add arrays and bulk pad editing after basic authoring is reliable. |
| Same manual, Checking footprints | Checks cover courtyard problems, holes/annuli, incompatible layers and copper or silkscreen collisions. | Report the checks actually implemented. A definition passing schema validation must not be presented as manufacturing-qualified or as passing full DRC. |
| [Altium: Creating a Schematic Symbol](https://www.altium.com/documentation/altium-designer/components-libraries/creating-schematic-symbol) | Graphics and electrically active pin ends are separate; symbol-level and primitive-level properties are contextual. | Keep the selected primitive's inspector separate from definition metadata. Show connection points clearly while editing. |
| [Altium: Creating a PCB Footprint](https://www.altium.com/documentation/altium-designer/components-libraries/creating-pcb-footprint) | Origin/grid setup, numbered pads, overlays and mechanical graphics support footprint construction; dimensions should be checked against component specifications. | Make the placement origin visible, support exact unit-aware dimensions and separate copper, silkscreen, fabrication and courtyard purposes. Mask/paste behavior and 3D need explicit subsequent contracts. |
| [Altium: Elements of the Environment](https://www.altium.com/documentation/altium-designer/design-environment-elements) | Properties changes context with the active document/selection and can use collapsible regions; document editors and host panels have separate roles. | Retain PCBcraft's ribbon, Browser, floating cards, themes and view ownership. Use contextual sections rather than copying Altium's visual chrome. |

The manuals were consulted on November 7, 2026; the Altium symbol and footprint
pages identify October 15 and October 14, 2026 updates respectively. These links
are behavioral references, not permission to inspect implementation pages or to
reuse separately licensed content.

## Properties behavior

The compact inspector belongs to the focused editor's selection. Counterpart
highlights do not grant edit authority. Reference, value and orientation are
applied atomically; typing in a draft does not change the project. Expanding the
form does not create another draft or undo history. Invalid input stays editable,
Cancel leaves the document unchanged and stale view/project revisions are refused.

Footprint selection includes a physical preview, explicit terminal-to-pad map
and an expandable source/license section. Assignment pins a definition without
placing copper. Initial board placement stays a separate command. A placed
package cannot be silently exchanged by this chooser.

## Workspace and window ownership

Symbol Editor and Footprint Editor appear alongside Schematic and PCB layout
in the shared workspace picker. Each has Draw, Library and View ribbon tabs,
a canvas, Browser and compact/expanded Properties. The picker can open them
here, beside the current pane or in a new native window. Their View ribbon
also provides Split, Detach/Dock and New window, using the same host/layout
controller as project views. Browser builds support splits within one tab.

Up to four definition documents can remain open, each with one session and undo
history shared by its views. Selection, camera, panel visibility, ribbon tab and
pending fields belong to individual panes. Closing a pane or non-last window
retains the document and its unapplied fields; Review edits can recover fields
from a closed pane. The last window and explicit Quit protect unsaved definitions
before the ordinary project-close decision. Closing the original native window
preserves the remaining window's identity and services.

Definition pane placements currently last for the application session. While
library panes are present, the last saved project arrangement is retained rather
than writing runtime definition IDs into project arrangement storage.

Each definition document owns its history and dirty state. A canvas owns only
selection, camera and tool drafts. Closing or moving a view must not publish a
definition, discard unsaved work without a decision or transfer project ownership.
The existing project remains the authority for applying a reviewed definition
change to a placed component. Project-local variants leave source libraries alone.

These notes do not close #50, #78 or #79 by themselves. Electrical-rule coverage,
multi-unit devices, further source integration, manufacturing layers, arbitrary shapes
and reviewed library-to-project updates need their own supported contracts and
acceptance evidence.

## Initial authoring slice

Choose **Symbol Editor** or **Footprint Editor** from the workspace picker.
The picker remains accessible in compact headers, and each editor's **Library → New**
creates another definition of its kind. **Library → Open** loads a saved item.
Switching through the picker retains document-local selection, camera, ribbon,
panel visibility and unfinished property fields. It cancels an unfinished pointer
gesture without committing it.

The project **Libraries** ribbon groups source management, saved-item Open,
Resume and **Edit copy**. Edit copy forks the focused view's selected symbol or
physical footprint under a new identity, entering its matching editor workspace
while leaving the source and placed instances unchanged. File has no workspace
launchers; inside an editor it holds document operations.
The browser tree selects definition metadata or a stable primitive ID.
Compact and expanded properties share one revision-bound draft, including invalid
typed lengths; Apply commits once, and Reset discards only those field edits.

Symbols support numbered terminals, names, electrical types, line/rectangle/circle
graphics and text. Existing polylines can be edited in expanded properties.
Footprints support numbered rectangular, rounded or circular SMD/plated-hole pads,
technical rectangles on fabrication/silkscreen/courtyard and silkscreen paths.
Fields accept explicit mm, mil or inch lengths and store integer nanometres.
Drawing and movement snap to the editor grid; symbol pins must satisfy the native
1.27 mm terminal grid. The local origin is fixed at zero and visible on the canvas.
M previews a move and clicks commit; R quarter-turns physical pads/rectangles.
Escape cancels unfinished gestures. Schema validation checks supported geometry
and electrical identities, including unique numbers and valid drills/annuli;
it does not perform manufacturing DRC.

The editable `pcbcraft-library-item/1` JSON envelope records definition identity,
revision, provenance (including a forked footprint’s original source record)
and stable primitive IDs separately from project storage.
Its object order is deterministic, with a 1 MiB encoded size limit, 512 primitives
and 64 undo snapshots per document. The current footprint schema further limits
a definition to 64 pads, 64 technical rectangles and 64 silkscreen paths.
Object/content allocation counters survive
Undo and save/reopen, so deleted IDs and observed content revisions are not reused.
Undo can therefore leave allocation bookkeeping unsaved even when geometry returns
to its saved state. Runtime document IDs and edit revisions are not saved. Save Library Copy publishes a new `.pcitem.json` file atomically on
desktop and refuses existing destinations, including source files. An exact
captured snapshot becomes saved only after publication and sync succeed; newer
edits remain unsaved. Open validates an editor file into a new document and
refuses replacing an already open identity. A cancelled picker or failed write
leaves documents intact. Browser downloads and native-definition exports do not
confirm a durable editor save, so they retain the unsaved state.

Export Native Definition writes the existing standalone symbol/footprint schema
to a new file. Keep the editor item as the provenance-bearing authoring master;
the existing standalone symbol format does not carry its license or stable
editor primitive IDs. Unknown symbol provenance requires recording its license
before saving or exporting. Native source registration/search is available through
**Libraries → Sources**, as described in the [source guide](native-library-sources.md).
Deliberate application of an authored definition to a project remains follow-up
work; these editors do not yet add their documents to Parts or the package catalog.

Live MCP switches the same four workspace types through `app.set_workspace`
and discovers `app.library_new`, `app.library_fork_selected`,
`app.library_edit`, `app.library_view`, `app.library_focus`, `app.library_open`, `app.library_save`
and `app.library_close`. Forking addresses an explicit source view ID.
`inspect_editor.workspace` identifies the visible workspace, while
`inspect_editor.library_editors` reports document IDs,
primitive records, dirty fields and each document's revision. `app.library_view`
accepts an explicit pane ID and controls its ribbon, Browser, Properties and
navigation. `app.set_workspace` accepts source pane and placement; ordinary
`view.split`, `view.open_beside`, `view.detach`, `view.dock`, `view.open_window` and
`view.close` use the same layout path for definition and project views. Edits require
that library revision and use the same engine authoring transactions as the GUI.
Headless project sessions do not yet host these editor documents. Multi-unit
symbols, arrays, bulk tables, arbitrary pad shapes, mask/paste, 3D, source publishing
and reviewed updates to placed components remain follow-up scope under #78/#79.
