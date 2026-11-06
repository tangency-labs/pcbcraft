# Current capabilities and MVP scope

This table starts from the baseline at `091adfd` (November 6, 2026), with the
wiring and views rows updated for direct segment editing and native editor windows.
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
| Layout | Engine storage/commands for a generic two-pad footprint and explicit pin-to-pad lookup | Editable board canvas [#54](https://github.com/tangency-labs/pcbcraft/issues/54), reviewed board updates [#55](https://github.com/tangency-labs/pcbcraft/issues/55) and basic track/via editing [#95](https://github.com/tangency-labs/pcbcraft/issues/95) |
| Cross-probing and views | Two schematic views in a split or one detached editor window; independent cameras and selections, shared edits/undo/save, counterpart highlights and optional selection following; child close leaves the primary view in place, Dock restores a split, primary close prompts once. Headless/live MCP can look up the supported pin/pad mapping | General view registry, mixed-workspace splits, multiple detached editors and combined board acceptance [#64](https://github.com/tangency-labs/pcbcraft/issues/64), visual schematic/board highlighting [#65](https://github.com/tangency-labs/pcbcraft/issues/65), live-MCP view/window actions [#61](https://github.com/tangency-labs/pcbcraft/issues/61) |
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
