# Documentation

Start with a task below, then use the references for the area you are changing.
The [capability inventory](capabilities.md) records implemented support and limits;
design pages also contain planned behavior. Their scope and version notes matter
when deciding what the current application supports.

## Start here

| Task | Reading path |
| --- | --- |
| Try the application | [Quickstart](quickstart.md) → [Dual LED editing example](../examples/README.md) → [Editor guide](editor-guide.md) |
| Edit a PCB | [PCB layout workflow](pcb-layout-workflow.md) and [Editor guide](editor-guide.md), with current limits in [Capabilities](capabilities.md) |
| Connect an AI agent | [Assistant and MCP](mcp.md) · [Headless MCP](automation/headless-mcp.md) · [UI automation](automation/ui-automation.md) |
| Review or export manufacturing files | [Fabrication drill files](manufacturing/drill-files.md) · [Generate files or review a ZIP through MCP](manufacturing/fabrication-mcp.md) |
| Contribute code or documentation | [Contributing](../CONTRIBUTING.md), [agent instructions](../AGENTS.md) and [Vision](../VISION.md) |
| Change project data or library formats | [Native project storage](formats/native-project-storage.md), then the applicable [format reference](#reference-and-formats) |
| Measure a changed interaction or engine path | [Performance testing](testing/performance-testing.md), [coverage](testing/performance-coverage.md) and the applicable saved workload |

Agents working on the repository should read [AGENTS.md](../AGENTS.md),
[VISION.md](../VISION.md), [README.md](../README.md),
[ATTRIBUTION.md](../ATTRIBUTION.md) and [deny.toml](../deny.toml) before selecting
the relevant area contract. [CONTRIBUTING.md](../CONTRIBUTING.md) describes code
ownership and change-appropriate checks. Inspect current code and linked issues
before treating an older plan as implemented behavior.

## Browse by topic

- [Schematic](schematic/) and [PCB layout](pcb/)
- [Libraries](libraries/) and [manufacturing](manufacturing/)
- [Automation and MCP](automation/)
- [File formats and data contracts](formats/) and [JSON schemas](schemas/)
- [Architecture and design plans](design/), [RFCs](RFC/) and [testing](testing/)

## Editing and libraries

| Topic | Guides |
| --- | --- |
| Tools, shortcuts, views and project files | [Editor guide](editor-guide.md) · [Schematic keyboard shortcuts](schematic/keyboard-shortcuts.md) · [Board keyboard shortcuts](pcb/board-keyboard-shortcuts.md) · [Library editor keyboard shortcuts](libraries/library-editor-keyboard-shortcuts.md) |
| Net names, labels and power symbols | [Named nets and power symbols](schematic/net-labels.md) |
| Sheet hierarchy and interfaces | [Hierarchical sheets](schematic/hierarchical-sheets.md) |
| Review reference designators | [Schematic annotation preview](schematic/annotation.md) |
| Selection exchange and reuse | [Copy and paste](schematic/clipboard.md) |
| Intentional unused connections | [No-connect markers](schematic/no-connect-markers.md) |
| Schematic-to-board review and airwires | [Single-board updates](pcb/single-board-update.md) · [Accepted board connections](pcb/board-ratsnest.md) |
| Inspect individual pins and pads | [Per-view inspection selection](schematic/pin-pad-selection.md) |
| Explicit via nets | [Placement, Properties and storage](pcb/via-net-assignments.md) |
| Remove one routed arm | [Unroute one connection](pcb/unroute.md#unroute-one-connection) |
| Space footprints or protect placed copper | [Set-pitch distribution](pcb/set-pitch-distribution.md) · [PCB locks](pcb/locks.md) |
| Copper-zone outline authoring | [Zone drawing, editing and storage](pcb/copper-zone-outlines.md) |
| Board rules and materials | [Board rules](pcb/board-rules.md) · [Materials and dielectric construction](pcb/board-materials.md) |
| Create classes, assign nets and inspect effective rules | [Shared net-class editor](pcb/net-class-editor.md) |
| Parts, sources and definition editing | [Native library sources](libraries/native-library-sources.md) · [Parts Library workspace](libraries/library-workspace.md) · [Properties and library editors](libraries/properties-and-library-editors.md) |
| Included and example definitions | [Bundled symbol libraries](libraries/builtin-symbol-libraries.md) · [Symbol libraries](libraries/symbol-library.md) · [Starter footprints](libraries/footprint-library.md) · [Native-parts example](../examples/native-parts/README.md) |

## Reference and formats

These pages describe contracts and implemented slices. Check each document's
supported versions and deferred work before extending a format or consumer.

| Area | References |
| --- | --- |
| Native files and compatibility | [Project storage](formats/native-project-storage.md) · [Reading a saved project](formats/reading-native-projects.md) · [Optional single-board foundation](formats/single-board-storage.md) · [Single-use sheet occurrences](formats/sheet-occurrence-storage.md) |
| Persistent net records and differential pairs | [Sparse net registry](formats/net-registry-storage.md) |
| Netlist export | [JSON netlist format](formats/netlist-format.md) · [Schema, format 1](schemas/netlist-v1.schema.json) · [Schema, format 2 (board)](schemas/netlist-v2.schema.json) |
| Schematic document output | [Schematic PDF writer](manufacturing/schematic-pdf.md) |
| Object identities | [Identity inventory](formats/object-identity.md) |
| Object-query parsing | [Syntax, trees and byte-offset diagnostics](design/object-queries.md), without evaluation or UI |
| Pin/pad correspondence | [Complete counterparts and unavailable states](formats/pin-pad-correspondence.md) |
| Dimensional net classes | [Catalog and resolver](formats/net-classes.md) · [Assignments and topology reconciliation](formats/net-class-assignments.md) · [Routing and checking consumers](formats/net-class-consumers.md) |
| Part groups | [Catalog, membership and storage](formats/part-groups.md) |
| Library records | [Symbol schema](libraries/symbol-schema.md) · [Footprint and placement schema](libraries/footprint-schema.md) · [Parts, categories and manufacturer records](libraries/part-schema.md), with [format-5 descriptive metadata](libraries/part-schema.md#descriptive-metadata-and-search) · [Machine-readable schemas](schemas/) |
| Typed quantities | [Quantities and units](formats/quantity-units.md) |
| Cross-sheet clipboard | [Hierarchical clipboard contract](formats/hierarchical-clipboard-contract.md) |
| Shared geometry | [Snapshot copper queries](pcb/copper-clearance-query.md) · [Exact circular copper records](pcb/circular-copper-records.md) · [Straight via teardrop records](pcb/teardrop-records.md) · [Walkaround route planning](pcb/route-planning.md) |
| Planar regions, solid bodies and STEP | [Geometry kernel and STEP library](design/geometry-kernel.md) |
| Shared interface toolkit | [Craft UI migration and current pin](design/shared-ui-migration.md) |
| Automation contracts | [MCP command API and errors](automation/command-api.md) · [Part commands](automation/part-commands.md) · [Board and connectivity commands](automation/board-commands.md) · [Craft control protocol](../crates/craft-control/README.md) |

## Architecture and plans

[Continuous integration](testing/continuous-integration.md) describes quick PR feedback,
approval and affected native checks, deferred platform coverage and build caches.

These documents explain design decisions and staged work. Several combine
delivered foundations with proposed extensions; they do not establish feature
availability by themselves.

| Topic | Documents and scope |
| --- | --- |
| PCB hosting and physical representation | [Board workspace plan](design/board-workspace-plan.md) · [PCB representation and shared rules](design/pcb-representation.md) · [PCB layout design plan](design/pcb-layout-gaps.md) |
| Library/provider architecture | [Library sources model](design/library-sources.md); native source behavior is documented in the guide above |
| Net identity policy | [Persistent net identity](design/net-identity.md), including broader multiboard requirements |
| Earlier net-registry rollout | [Path to the registry](design/net-registry-path.md), retained as a planning record; implemented slices are linked under Reference and formats |
| Sheet paper, title blocks and parameters | [Sheet frames, parameters and templates](design/sheet-frames-and-parameters.md), a proposal for [#46](https://github.com/tangency-labs/pcbcraft/issues/46) |
| Parametric board (RFC, proposal) | [Parametric board RFC](RFC/parametric-board.md): first the board published to SolidCraft and refreshed in place in an assembly, then outlines linked over the Craft bridge, board parameters, parts held to edges and the update that carries their copper, for [#70](https://github.com/tangency-labs/pcbcraft/issues/70), [#86](https://github.com/tangency-labs/pcbcraft/issues/86) and [#153](https://github.com/tangency-labs/pcbcraft/issues/153) |
| Future multiboard and reusable circuits | [Overview](design/multiboard-and-sheet-instances.md) · [Circuit definitions and instances](design/circuit-instances.md) · [Storage and migration](design/multiboard-storage.md) · [Assembly and outputs](design/multiboard-assembly-and-outputs.md) |

## Verification and history

| Topic | Documents and scope |
| --- | --- |
| Schematic-to-board regression | [Single-board forward-update acceptance](testing/single-board-forward-acceptance.md); automated coverage, separate from owner-manual acceptance |
| Measurement method and limits | [Performance testing](testing/performance-testing.md) · [Coverage record](testing/performance-coverage.md) |
| Recorded baseline | [Initial performance baseline](testing/performance-baseline.md), with its original environment and revision |
| Via displacement and connected drag measurements | [Saved #248 fixtures, matched native replay and results](../tools/performance/baselines/issue248/README.md) |
| Releases and attribution | [Changelog](../CHANGELOG.md) · [Third-party notices](../ATTRIBUTION.md) |
| Source-boundary history | [November 7 layer-shortcut search](source-boundaries/2026-11-07-layer-shortcut-search.md); apply the current [source rules](../AGENTS.md#license-and-sources-the-rule-that-cant-be-undone) and task-specific resolution records |

The [MVP acceptance issue](https://github.com/tangency-labs/pcbcraft/issues/96)
and [milestone](https://github.com/tangency-labs/pcbcraft/milestone/1) track the
shareable preview. The [workflow index](https://github.com/tangency-labs/pcbcraft/issues/89)
also covers later work; implementation tasks and their dependencies live in GitHub.
