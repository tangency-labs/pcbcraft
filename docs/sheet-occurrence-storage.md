# Single-use sheet occurrence storage

This implements slice 2 of [the net registry plan](net-registry-path.md), #229,
on the current `project.json` working-folder and container-v3 `.pc` archive
layout. Board folders, layout roots and the existing component records remain
in place. It adds identities for the existing one-use hierarchy; repeated
instances, component-definition splitting and persistent net IDs remain separate.

## Records and ownership

Project format 16 extends project/15, retaining native part selections and multi-pad package maps. For a schematic-only project it requires
`schematic_instances` in `project.json`. For a board project that field is
absent and the board registration owns an `instances.json` beside `board.json`.
The instance document's format is `pcbcraft-instances/1`, and its `board` header
must match the registered board UUID. A second storage home is rejected.
Container, sheet, board and pinned-definition versions do not change.

The instance payload contains `circuit_uses` and `sheet_occurrences`. There is
exactly one circuit-use record, containing its non-nil UUID `id`. Every sheet
has one occurrence, with these fields:

| Field | Meaning |
| --- | --- |
| `id` | Stable, non-nil occurrence UUID |
| `definition` | Existing sheet UUID |
| `circuit_use` | Reference to the sole circuit-use record |
| `parent` | Parent occurrence UUID, or null for a flat root |
| `parent_slot` | Stable numeric subsheet-symbol ID in that parent, or null for a root |

Records sort by UUID, with one occurrence per line. Parent and slot describe the
existing hierarchy and must agree with it exactly. Reparenting changes those two
relationships and preserves the occurrence ID. Renaming a sheet changes no IDs.
Deleting a sheet removes only its occurrence and promotes surviving children to
roots, following the existing hierarchy commands. New sheets and duplicated
subtrees receive independent occurrence UUIDs; Cut relocation preserves them.
All ordinary sheets share one use, so adding these records cannot split a
component into packages by sheet. Components remain numeric project-owned
records with unchanged references, values, package choices and terminal maps.

Readers reject missing or duplicate sheet occurrences, repeated definitions,
nil/colliding identities, foreign uses, incorrect parents/slots, extra ownership
homes, foreign board headers, unsupported versions and unknown fields. Existing
sheet, document, archive and geometry limits remain in force.

## Compatibility identities

Older supported split projects open without rewriting their source. Their
in-memory inspection derives compatibility IDs, which the first successful Save
persists. SHA-256 receives the role bytes including their terminal zero byte,
then raw RFC-order UUID bytes, without textual formatting or a length prefix:

| Role bytes | UUID input |
| --- | --- |
| `pcbcraft:legacy-circuit-use:1\0` | Project UUID |
| `pcbcraft:legacy-sheet-occurrence:1\0` | Project UUID followed by sheet UUID |

Take the first 16 digest bytes, set byte 6's high nibble to 8, and byte 8's high
bits to binary 10. Role strings differ from each other and from the existing
legacy-board role. A collision is rejected rather than assigned another ID.

For project `00112233-4455-6677-8899-aabbccddeeff` and sheet
`10213243-5465-7687-98a9-bacbdcedfe0f`, the codec vectors are:

- Circuit use: `3a62a9eb-47a7-876f-ac20-c8a4fc611074`.
- Occurrence: `41ef7bfe-b5ae-8afe-beed-f319542cf74d`.

New projects and sheets created after adoption use independently allocated UUIDs.
No ID derives from a sheet name, hierarchy path, array position or filename.
Persistent net anchors are not added by this slice.

## Save and lifecycle behavior

The low-level codec preserves a supported older encoding until migration is
explicitly prepared. Native Save and Save As prepare the current occurrence
format even for an otherwise unchanged project/14 or /15 snapshot. The journal publishes
the entry point and new instance document together, preserving exact unaffected
sheet, physical-board and pinned-definition bytes. Later unchanged saves produce
no diff. Older readers reject the new project version.

Creating the first board moves the same use/occurrence records from
`schematic_instances` to the derived board instance document in one revision-
checked transaction. Undo restores the schematic-only home and the same IDs.
Successful Save adopts occurrence metadata across history without an electrical
edit; failed/cancelled saves do not adopt it. Edits made during a background save
remain unsaved and retain their own sheet identities. Browser Download writes a
migrated copy without claiming durable publication or changing active metadata.

An existing user-owned `instances.json` in a registered board folder must not be
overwritten during first migration. Preparation/publication refuses that collision
and preserves the prior project and user file. Journal interruption tests cover
the complete entry-point/instance-document transition and first-board movement.
Recovery retains the existing old-before-commit/new-after-commit contract and
platform durability limits.

Project/15 remains readable with or without native part selections and its prior
flat or board-folder layout. Migration to project/16 preserves its pinned parts,
manufacturer records, typed requirements and physical maps. Multi-unit drawings
across flat sheets retain one component/reference/package in the shared circuit
use; sheet occurrences do not introduce independent part instances.
