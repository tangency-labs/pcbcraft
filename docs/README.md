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
| Connect an AI agent | [Assistant and MCP](mcp.md); [UI automation](ui-automation.md) for native clicks, drags, keys and text |
| Contribute code or documentation | [Contributing](../CONTRIBUTING.md), [agent instructions](../AGENTS.md) and [Vision](../VISION.md) |
| Change project data or library formats | [Native project storage](native-project-storage.md), then the applicable [format reference](#reference-and-formats) |
| Measure a changed interaction or engine path | [Performance testing](performance-testing.md), [coverage](performance-coverage.md) and the applicable saved workload |

Agents working on the repository should read [AGENTS.md](../AGENTS.md),
[VISION.md](../VISION.md), [README.md](../README.md),
[ATTRIBUTION.md](../ATTRIBUTION.md) and [deny.toml](../deny.toml) before selecting
the relevant area contract. [CONTRIBUTING.md](../CONTRIBUTING.md) describes code
ownership and change-appropriate checks. Inspect current code and linked issues
before treating an older plan as implemented behavior.

## Editing and libraries

| Topic | Guides |
| --- | --- |
| Tools, shortcuts, views and project files | [Editor guide](editor-guide.md) · [Schematic keyboard shortcuts](keyboard-shortcuts.md) |
| Net names, labels and power symbols | [Named nets and power symbols](net-labels.md) |
| Sheet hierarchy and interfaces | [Hierarchical sheets](hierarchical-sheets.md) |
| Selection exchange and reuse | [Copy and paste](clipboard.md) |
| Intentional unused connections | [No-connect markers](no-connect-markers.md) |
| Schematic-to-board review and airwires | [Single-board updates](single-board-update.md) · [Accepted board connections](board-ratsnest.md) |
| Explicit via nets | [Placement, Properties and storage](via-net-assignments.md) |
| Copper-zone outline authoring | [Zone drawing, editing and storage](copper-zone-outlines.md) |
| Board rules and materials | [Board rules](board-rules.md) · [Materials and dielectric construction](board-materials.md) |
| Create classes, assign nets and inspect effective rules | [Shared net-class editor](net-class-editor.md) |
| Parts, sources and definition editing | [Native library sources](native-library-sources.md) · [Parts Library workspace](library-workspace.md) · [Properties and library editors](properties-and-library-editors.md) |
| Included and example definitions | [Bundled symbol libraries](builtin-symbol-libraries.md) · [Symbol libraries](symbol-library.md) · [Starter footprints](footprint-library.md) · [Native-parts example](../examples/native-parts/README.md) |

## Reference and formats

These pages describe contracts and implemented slices. Check each document's
supported versions and deferred work before extending a format or consumer.

| Area | References |
| --- | --- |
| Native files and compatibility | [Project storage](native-project-storage.md) · [Optional single-board foundation](single-board-storage.md) · [Single-use sheet occurrences](sheet-occurrence-storage.md) |
| Persistent net records | [Sparse net registry](net-registry-storage.md) |
| Netlist export | [JSON netlist format](netlist-format.md) · [Schema](schemas/netlist-v1.schema.json) |
| Object identities | [Identity inventory](object-identity.md) |
| Dimensional net classes | [Catalog and resolver](net-classes.md) · [Assignments and topology reconciliation](net-class-assignments.md) · [Routing and checking consumers](net-class-consumers.md) |
| Part groups | [Catalog, membership and storage](part-groups.md) |
| Library records | [Symbol schema](symbol-schema.md) · [Footprint and placement schema](footprint-schema.md) · [Parts, categories and manufacturer records](part-schema.md) · [Machine-readable schemas](schemas/) |
| Typed quantities | [Quantities and units](quantity-units.md) |
| Cross-sheet clipboard | [Hierarchical clipboard contract](hierarchical-clipboard-contract.md) |
| Shared geometry | [Snapshot copper queries](copper-clearance-query.md) · [Walkaround route planning](route-planning.md) |
| Solid bodies and STEP | [Geometry kernel and STEP library](geometry-kernel.md) |
| Shared interface toolkit | [Craft UI migration and current pin](shared-ui-migration.md) |
| Local transport | [Craft control protocol](../crates/craft-control/README.md) |

## Architecture and plans

These documents explain design decisions and staged work. Several combine
delivered foundations with proposed extensions; they do not establish feature
availability by themselves.

| Topic | Documents and scope |
| --- | --- |
| PCB hosting and physical representation | [Board workspace plan](board-workspace-plan.md) · [PCB representation and shared rules](pcb-representation.md) |
| Library/provider architecture | [Library sources model](library-sources.md); native source behavior is documented in the guide above |
| Net identity policy | [Persistent net identity](net-identity.md), including broader multiboard requirements |
| Earlier net-registry rollout | [Path to the registry](net-registry-path.md), retained as a planning record; implemented slices are linked under Reference and formats |
| Sheet paper, title blocks and parameters | [Sheet frames, parameters and templates](sheet-frames-and-parameters.md), a proposal for [#46](https://github.com/tangency-labs/pcbcraft/issues/46) |
| Future multiboard and reusable circuits | [Overview](multiboard-and-sheet-instances.md) · [Circuit definitions and instances](circuit-instances.md) · [Storage and migration](multiboard-storage.md) · [Assembly and outputs](multiboard-assembly-and-outputs.md) |

## Verification and history

| Topic | Documents and scope |
| --- | --- |
| Schematic-to-board regression | [Single-board forward-update acceptance](single-board-forward-acceptance.md); automated coverage, separate from owner-manual acceptance |
| Measurement method and limits | [Performance testing](performance-testing.md) · [Coverage record](performance-coverage.md) |
| Recorded baseline | [Initial performance baseline](performance-baseline.md), with its original environment and revision |
| Releases and attribution | [Changelog](../CHANGELOG.md) · [Third-party notices](../ATTRIBUTION.md) |
| Source-boundary history | [November 7 layer-shortcut search](source-boundaries/2026-11-07-layer-shortcut-search.md); apply the current [source rules](../AGENTS.md#license-and-sources-the-rule-that-cant-be-undone) and task-specific resolution records |

The [MVP acceptance issue](https://github.com/tangency-labs/pcbcraft/issues/96)
and [milestone](https://github.com/tangency-labs/pcbcraft/milestone/1) track the
shareable preview. The [workflow index](https://github.com/tangency-labs/pcbcraft/issues/89)
also covers later work; implementation tasks and their dependencies live in GitHub.
