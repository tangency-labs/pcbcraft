# Current capabilities and MVP scope

This table starts from the baseline at `091adfd` (November 6, 2026), with the
wiring row updated for direct segment editing. Update it alongside feature changes.
The [quickstart](quickstart.md) exercises the working schematic workflow;
[MVP acceptance](https://github.com/tangency-labs/pcbcraft/issues/96) defines what
still needs to work before the first collaboration-ready preview.

| Workflow | Available in this baseline | Remaining scope |
| --- | --- | --- |
| Projects | Create/name, open, save/save-as folders or `.pc.zip`; shared undo/redo; older native formats migrate | Untitled-first creation and recovery: [#69](https://github.com/tangency-labs/pcbcraft/issues/69) |
| Schematic editing | One sheet; place, rotate, edit fields, move/delete symbols; directional boxes, modifier selection and group move/delete | Multiple sheets/hierarchy: [#40](https://github.com/tangency-labs/pcbcraft/issues/40)/#41; clipboard, mirror and G-drag: [#44](https://github.com/tangency-labs/pcbcraft/issues/44) |
| Wiring | Orthogonal routes; direct segment selection, drag/G and Delete; corner/free-end/shared-junction handles; Slice, Break and whole-wire actions; shared undo and headless MCP commands | Automatic endpoint attachment and further reshape work [#67](https://github.com/tangency-labs/pcbcraft/issues/67), automatic healing [#68](https://github.com/tangency-labs/pcbcraft/issues/68); use Slice before branching from a wire's interior |
| Libraries | Five starter symbol types; pinned symbol revisions survive offline reopen; one placed unit per component | Native local libraries and general footprint assignment [#92](https://github.com/tangency-labs/pcbcraft/issues/92); authoring [#78](https://github.com/tangency-labs/pcbcraft/issues/78)/#79; Git/catalog sources [#93](https://github.com/tangency-labs/pcbcraft/issues/93)/#94 |
| Navigation | Zoom, pan, Fit, floating Browser/Parts, Net Inspector, theme/scale/navigation preferences | Configurable grid spacing [#73](https://github.com/tangency-labs/pcbcraft/issues/73), opacity [#74](https://github.com/tangency-labs/pcbcraft/issues/74) and mil/mm input/display [#75](https://github.com/tangency-labs/pcbcraft/issues/75); current snap spacing is fixed at 1.27 mm |
| Layout | Engine storage/commands for a generic two-pad footprint and explicit pin-to-pad lookup | Editable board canvas [#54](https://github.com/tangency-labs/pcbcraft/issues/54), reviewed board updates [#55](https://github.com/tangency-labs/pcbcraft/issues/55) and basic track/via editing [#95](https://github.com/tangency-labs/pcbcraft/issues/95) |
| Cross-probing and views | Headless/live MCP can look up the supported pin/pad mapping | Visual schematic/board highlighting [#65](https://github.com/tangency-labs/pcbcraft/issues/65) and split/detached editor views [#64](https://github.com/tangency-labs/pcbcraft/issues/64); the Assistant's pop-out is not a detached editor |
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
because the browser cannot confirm a durable write. The desktop Assistant and
local MCP listener are not available in the browser build.

The board walkthrough, polished demo and fresh-user onboarding acceptance in [#90](https://github.com/tangency-labs/pcbcraft/issues/90)
remain pending their dependencies. Documentation command checks and automated UI
tests do not substitute for a new person's end-to-end acceptance run.
