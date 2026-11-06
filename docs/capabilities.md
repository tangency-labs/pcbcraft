# Current capabilities and MVP scope

This table starts from the baseline at `091adfd` (November 6, 2026), with the
wiring and views rows updated for segment editing, PCB inspection and saved arrangements.
Update it alongside feature changes.
The [quickstart](quickstart.md) exercises the working schematic workflow;
[MVP acceptance](https://github.com/tangency-labs/pcbcraft/issues/96) defines what
still needs to work before the first collaboration-ready preview.

| Workflow | Available in this baseline | Remaining scope |
| --- | --- | --- |
| Projects | New Project opens Untitled at once and a desktop first save names it after its folder/archive; rename, open, save/save-as folders or `.pc.zip`; Save/Don't Save/Cancel before closing, New or Open replaces unsaved work; shared undo/redo; older native formats migrate | Local recovery of unsaved work: [#69](https://github.com/tangency-labs/pcbcraft/issues/69) |
| Schematic editing | One sheet; place, rotate, edit fields, move/delete symbols; directional boxes, modifier selection and group move/delete | Multiple sheets/hierarchy: [#40](https://github.com/tangency-labs/pcbcraft/issues/40)/#41; clipboard, mirror and G-drag: [#44](https://github.com/tangency-labs/pcbcraft/issues/44) |
| Wiring | Orthogonal routes; direct segment selection, drag/G and Delete; corner/free-end/shared-junction handles; Slice, Break and whole-wire actions; shared undo and headless MCP commands | Automatic endpoint attachment and further reshape work [#67](https://github.com/tangency-labs/pcbcraft/issues/67), automatic healing [#68](https://github.com/tangency-labs/pcbcraft/issues/68); use Slice before branching from a wire's interior |
| Libraries | Five starter symbol types; pinned symbol revisions survive offline reopen; one placed unit per component | Native local libraries and general footprint assignment [#92](https://github.com/tangency-labs/pcbcraft/issues/92); authoring [#78](https://github.com/tangency-labs/pcbcraft/issues/78)/#79; Git/catalog sources [#93](https://github.com/tangency-labs/pcbcraft/issues/93)/#94 |
| Navigation | Zoom, pan, Fit, floating Browser/Parts, Net Inspector, theme/scale/navigation preferences; per-view schematic grid spacing and opacity; mil/mm display units with a pointer coordinate readout and unit-aware spacing entry | Board grids [#73](https://github.com/tangency-labs/pcbcraft/issues/73)/[#74](https://github.com/tangency-labs/pcbcraft/issues/74); unit-aware property fields with the inspector [#50](https://github.com/tangency-labs/pcbcraft/issues/50) and units through live MCP [#61](https://github.com/tangency-labs/pcbcraft/issues/61) under [#75](https://github.com/tangency-labs/pcbcraft/issues/75) |
| Layout | Read-only PCB workspace with footprint Browser, grouped Layers, placement Properties and pad-center rendering; engine storage/commands for a generic two-pad footprint and explicit pin-to-pad lookup; validated layer catalog and uniform rigid stackup, two/four-layer presets, exact optional thicknesses, custom materials, sourced Tg/Dk/Df reports, core/prepreg plies, undo and offline persistence through headless commands | Stackup/impedance UI, calculations and general layer bindings [#76](https://github.com/tangency-labs/pcbcraft/issues/76), shared nets/rules [#77](https://github.com/tangency-labs/pcbcraft/issues/77), editable board canvas [#54](https://github.com/tangency-labs/pcbcraft/issues/54), reviewed board updates [#55](https://github.com/tangency-labs/pcbcraft/issues/55) and basic track/via editing [#95](https://github.com/tangency-labs/pcbcraft/issues/95) |
| Cross-probing and views | Named arrangements with explicit creation/naming dialogs, confirmation before removal and user-local save/reopen restoration, per-pane document tab groups and Browser sheet navigation; schematic and PCB inspection targets in per-window split layouts; multiple native editor windows, splitting inside them, tab placement/close actions, independent cameras and selections, shared edits/undo/save, bidirectional component counterpart highlights and optional directed following. The workspace picker opens a target here, beside the current view or directly in another window. Any window can close while others remain; a survivor takes over project controls. Last-window Close or explicit Quit prompts once. Headless/live MCP can look up the supported pin/pad mapping | Editable-board acceptance [#64](https://github.com/tangency-labs/pcbcraft/issues/64), individual pin/pad selection and missing-mapping feedback [#65](https://github.com/tangency-labs/pcbcraft/issues/65), multiboard storage/assignment [#122](https://github.com/tangency-labs/pcbcraft/issues/122), portable arrangement export, full multi-monitor restoration, provider view payloads, live-MCP view/window actions [#61](https://github.com/tangency-labs/pcbcraft/issues/61) |
| Automation | Headless command catalog; desktop live inspection and revision-checked `schematic.move_symbol`; opt-in Assistant proposals | Full live GUI/MCP command coverage [#61](https://github.com/tangency-labs/pcbcraft/issues/61); headless sessions are separate from the open GUI |
| Sharing | Native folder/archive storage, original development fixtures, source builds for desktop/web | Polished demo and preview artifacts [#91](https://github.com/tangency-labs/pcbcraft/issues/91); user/contributor onboarding [#90](https://github.com/tangency-labs/pcbcraft/issues/90) |
| Later professional work | Planned, not implemented as a complete workflow | Full ERC/DRC [#47](https://github.com/tangency-labs/pcbcraft/issues/47)/#58, copper zones [#57](https://github.com/tangency-labs/pcbcraft/issues/57), fabrication [#59](https://github.com/tangency-labs/pcbcraft/issues/59), advanced routing [#85](https://github.com/tangency-labs/pcbcraft/issues/85) and 3D/STEP [#86](https://github.com/tangency-labs/pcbcraft/issues/86) are outside the first MVP |

Issue numbers above refer to the
[repository backlog](https://github.com/tangency-labs/pcbcraft/issues). The
[MVP milestone](https://github.com/tangency-labs/pcbcraft/milestone/1) selects
bounded editing, cross-probing, library and demo slices; the
[professional workflow index](https://github.com/tangency-labs/pcbcraft/issues/89)
also includes later work. Completing the MVP is not a fabrication-readiness claim.

Desktop saves write to the chosen folder/archive. Browser uploads use archives or
legacy single files; downloads produce `.pc.zip` and do not clear the dirty state
because the browser cannot confirm a durable write, so the page's leave warning
stays on until the project is replaced. The desktop Assistant and
local MCP listener are not available in the browser build.

The board walkthrough, polished demo and fresh-user onboarding acceptance in [#90](https://github.com/tangency-labs/pcbcraft/issues/90)
remain pending their dependencies. Documentation command checks and automated UI
tests do not substitute for a new person's end-to-end acceptance run.
