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

Symbol editor and Footprint editor are workspace types. A native window hosts
a workspace; it is not a second implementation of the editor. The proposed
desktop default is a separate editor window so the project remains visible,
with an in-window presentation for browser use and deliberate docking.

Each definition document owns its history and dirty state. A canvas owns only
selection, camera and tool drafts. Closing or moving a view must not publish a
definition, discard unsaved work without a decision or transfer project ownership.
The existing project remains the authority for applying a reviewed definition
change to a placed component. Project-local variants leave source libraries alone.

These notes do not close #50, #78 or #79 by themselves. Electrical-rule coverage,
multi-unit devices, source registration, manufacturing layers, arbitrary shapes
and reviewed library-to-project updates need their own supported contracts and
acceptance evidence.
