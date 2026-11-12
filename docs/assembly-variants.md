# Assembly variants and outputs

Assembly variants describe population choices for the project's registered
physical board. They do not create another board, copy a schematic component,
or change footprint placement, tracks, vias, nets or connectivity. Base is the
implicit default assembly; named variants are sparse overrides of Base.

## Population rules

The component's **Do not populate** attribute sets Base population. Each named
variant can leave a component unchanged (inherit), mark it fitted, or mark it
DNP. An unset override inherits the current Base attribute, so later Base edits
remain visible in variants that have not overridden that component. Component
IDs, rather than references or names, own overrides; renaming a reference does
not detach it. Multi-unit symbols are one logical component and appear once in
the BOM.

The three exclusion attributes have separate jobs:

| Attribute | BOM | Placement | Board state |
| --- | --- | --- | --- |
| DNP / not fitted | Omit | Omit | Preserve component, footprint and routing |
| Exclude from BOM | Omit | Include if otherwise fitted | Preserve |
| Exclude from pick-and-place | Include if otherwise fitted | Omit | Preserve |
| Exclude from board | Omit | Omit | Existing board-update behavior applies |

An authored variant whose stable component ID is missing is an error during
load or export; the exporter does not silently discard the stale choice. Normal
component deletion retires its sparse overrides in the same undoable project
change. Undo restores the component and those overrides together.

## GUI and commands

Open **Analyze → Reports → Variants** to create, rename or delete variants, edit each
component's population and explicitly export Base or a named variant. The
selection in this dialog is temporary view state. It does not activate a global
variant, dirty the project, or change another window's choice.

GUI and MCP use the same revision-checked engine commands:

| Command | Parameters and effect |
| --- | --- |
| `pcb.create_assembly_variant` | `board`, `expected_revision`, `name` |
| `pcb.set_assembly_variant` | `board`, `expected_revision`, `variant`, and either `name` or `component` plus `fitted` (`true`, `false`, or `null` to inherit) |
| `pcb.delete_assembly_variant` | `board`, `expected_revision`, `variant` |
| `pcb.export_assembly` | `board`, and required `variant` (`null` explicitly means Base) |

The read-only export command returns both files from one project snapshot and
does not advance the project revision. The GUI packages the same files into a
ZIP using the existing save/download flow.

## CSV contract

`BOM.csv` has one row per fitted logical component and carries reference, value,
footprint definition, stable component ID and quantity. A component can appear
in the BOM without a board placement; its footprint column is then empty.
`Placement.csv` contains fitted placed components that are not excluded from
position files. It carries reference, value, footprint definition, X/Y in mm,
rotation in degrees, side and stable component ID. Both files are UTF-8 CSV with
quoted cells and CRLF row endings. Distinct output exclusions apply after the
variant's fitted state is resolved.

Coordinates are in the board frame with Y converted to Y-up, matching the
fabrication exports. The top-side angle is the persisted board rotation. A
bottom-side placement adds 180 degrees because its local X axis is mirrored
before rotation; the side column remains explicit. The values describe the
placement as projected into the exported board frame and do not imply a
part-number substitution or alternate-footprint approval.

## Persistence

Base population remains in component metadata. Once a named variant exists,
project format 18 records its stable UUID and owning board in `project.json`;
the variant's name and sparse component-ID overrides live in
`boards/<board-folder>/variants/<variant-uuid>.json` (or beside a legacy flat
board). Save/reopen preserves identities and choices. Projects without named
variants retain their existing project format. See the
[native storage notes](native-project-storage.md#document-ownership).

This slice supports one registered physical board, matching current project
capabilities. Alternate part values, approved substitutions, multi-board builds
and export-run provenance are separate work; an assembly variant only changes
population.
