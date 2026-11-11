# Path to the net registry

November 10, 2026 · **Proposal for discussion.** It proposes how to reach the
[persistent net identity](net-identity.md) design from today's storage, and
asks the format-migration questions AGENTS.md wants discussed before the native
format changes. Related: [#122](https://github.com/tangency-labs/pcbcraft/issues/122)
(multiboard storage), [#77](https://github.com/tangency-labs/pcbcraft/issues/77)
(net classes and rules), [#57](https://github.com/tangency-labs/pcbcraft/issues/57)
(copper zones).

## Why now

Copper zones and net classes both need a saved, stable net target. Nets are
derived today: a pour or a class rule has nothing durable to point at. The
decision for zones is to wait for the registry rather than store an interim
binding, such as a net name or a single pad, that would need migrating later.
The registry is therefore the next step for both #57 and #77.

## What exists

- **Project identity.** Every project has a saved project UUID.
- **Board registry.** `pcbcraft-project/9` and its successors (12, 13) hold a
  required `boards` array with zero or one registration: a board UUID and name.
  Project/14 adds `layout_root` and its portable board-folder segment in place of
  the transitional root `board.json` (see [single-board-storage.md](single-board-storage.md)).
  Supported split schemas with implicit board ownership keep a deterministic
  compatibility board ID, derived from the
  project UUID with a domain-separated SHA-256 and UUID v8.
- **Numeric watermark.** `numeric_id_high_water` keeps numeric design-object IDs
  from being reused within the saved lineage.
- **Accepted baseline.** `board.json` keeps the accepted net/terminal baseline
  used by reviewed board updates and the ratsnest.
- **Stable objects.** Components have numeric IDs, and terminals are identified
  by their pin text. Net labels and hierarchical labels have numeric IDs too.
  Sheets have UUIDs, and each has exactly one use: repeated sheet instances are
  refused today (see [hierarchical-sheets.md](hierarchical-sheets.md)).
- **Atomic folder saves.** A save journals every changed and removed document and
  recovers to the complete old or new project after an interruption (see
  [folder publication and recovery](native-project-storage.md#folder-publication-and-recovery)).
  Document paths that Windows reads as devices are already refused.

Not built yet:
- `instances.json` and `schematic_instances`;
- circuit uses and sheet occurrence identities;
- any persistent net ID.

## What the registry needs

The [anchor encoding](net-identity.md#canonical-anchor-key-encoding) defines four
anchor kinds:

| Anchor kind | Key fields | Missing today |
| --- | --- | --- |
| Component terminal | Project UUID, scope (board UUID, or the project UUID for schematic-only), zero occurrence, component ID, terminal text | Nothing |
| Named label | Project UUID, scope, occurrence UUID, local object ID | The occurrence UUID |
| Hierarchical port | Same as a label | The occurrence UUID |
| Authored junction | Same as a label | The occurrence UUID |

Power symbols are ordinary components with declared power terminals, so a net
named GND by power symbols is anchored by component terminals. It needs no
separate kind, and a ground pour's net can be promoted as soon as slice 3 lands.

Its storage home is a board's `instances.json`, or `project.json` under
`schematic_instances` for schematic-only records. So the registry needs
occurrence identities and those two homes.

## Proposed order

Follow [multiboard-storage.md](multiboard-storage.md#migration-and-implementation-order),
taking from each step only what the registry needs. Each slice is its own
reviewed format change with its own migration tests, and each lands before the
next starts. These migrations start from a `project.json` folder or a
container-v3 `.pc` package, including the supported older split document schemas
within that layout. They do not restore support for `project.pc` folders,
container-v1/v2 archives or standalone project files; see the
[compatibility boundary](native-project-storage.md#compatibility-and-fixtures).

### Slice 1: board folders ([#228](https://github.com/tangency-labs/pcbcraft/issues/228))

- **Layout.** `project.json` gains `layout_root` and each board registration gains
  its `folder`. The board's `board.json` moves to
  `join(layout_root, "boards", folder, "board.json")`.
- **Folder names** follow the
  [portable folder rules](multiboard-storage.md#portable-board-folder-names):
  a slug plus 12 hexadecimal characters of the board UUID. They are checked
  against Windows reserved names (with the existing document-path check),
  case-only collisions and non-ASCII names.
- **Containers.** The packed `.pc` (`pcbcraft-container/3`) stores the split
  documents with their paths, so it carries the new layout. Single-document
  encodings (including `pcbcraft-container/1` and `pcbcraft-project/10`) have no
  folders and remain diagnostic-only inputs; normal app/CLI opens refuse them.
- **Where the code changes.** The split-document codec in the engine, the folder
  reader and recovery in `pcbcraft-storage`, and the app's native and web
  open/save paths. Board commands already resolve the board by its ID, so they
  don't change.
- **Migration.**
  - Opening a supported flat `project.json`/container-v3 project leaves its files
    untouched. Its first Save migrates the layout even without design edits.
  - One journaled save writes the registered board folder's `board.json`, updates
    the project entry point and removes the root board together. The second save
    produces no diff. New and explicitly converted projects use board folders
    from their first save, including single-board projects.
  - Successful adoption persists across Undo/Redo without an electrical edit;
    failed/cancelled saves do not adopt metadata.
  - Existing `boards/`, `builds/`, `documents/` or `outputs/` directories that
    the project doesn't own are never claimed. A
    `pcbcraft-data-<project-id-prefix>/` root is chosen instead and
    saved once.
- **Out of scope:** relocating a board folder, retired-board areas, variants,
  builds, outputs and associated documents. None exists yet, and each keeps the
  storage design's rules when it arrives.

### Slice 2: occurrence identities ([#229](https://github.com/tangency-labs/pcbcraft/issues/229))

- **Records.**
  - Each existing sheet gets exactly one sheet occurrence.
  - All flat roots and ordinary subsheets share one legacy circuit use, so a
    multi-unit component stays one package.
  - Occurrence and circuit-use IDs use the storage design's legacy derivation
    (domain-separated SHA-256/UUID v8 from the project UUID, plus the sheet UUID
    for occurrences). They are persisted on migration.
- **Homes.** On a board project, these records go in `<board_root>/instances.json`
  with its board-ID header. On a schematic-only project, they go in `project.json`
  under `schematic_instances`. Creating the first board moves them in the same
  transaction.
- **Components** keep their numeric IDs, which become the instance IDs. No
  component, reference, value, package choice or terminal mapping changes.
- **Behavior.** The editor behaves exactly as before. No repeated uses, no
  definition editing scope and no visible parent sheet are added.

### Slice 3: the net registry ([#230](https://github.com/tangency-labs/pcbcraft/issues/230))

- **IDs.** Net IDs are UUIDs, allocated independently and never reused. They
  are never derived from names or members.
- **Storage.** Records live in the slice 2 homes.
- **Which nets are saved.** Only intentionally named nets, nets an authored
  record targets and explicitly promoted nets get records. The first such record
  is a zone. An ordinary unnamed net stays derived.
- **Records.** Each has one primary anchor and at most three fallbacks, chosen
  by the [fixed priority](net-identity.md#choosing-and-retaining-the-primary-anchor)
  and stored once. It has no membership list.
- **Edits.**
  - Ordinary edits, disconnecting a bare pin, and a one-to-one continuation
    keep the ID.
  - Losing the primary anchor adopts a surviving fallback, or a canonical
    replacement from the unique continuation.
  - A split keeps the ID on the branch with the primary anchor and gives the
    other branches fresh IDs with lineage.
  - A split with no unique anchored continuation is recorded as an unresolved
    ambiguity, not guessed.
  - A merge gets a fresh ID with lineage to both sources.
- **Commands.** `nets.promote` gives a derived net a persistent ID.
  `nets.resolve_ambiguity` assigns an ambiguous ID to one branch. Both are
  revision-checked and undoable, and GUI and MCP share them. Inspection reports
  persistent IDs, anchors, lineage and open ambiguities. Reading never promotes
  anything or marks the project dirty.
- **Scope.** This slice is engine only, with no new UI beyond inspection.
- **Deferred to #77:** class bindings and other properties. A net record carries
  identity and lineage only, so this slice has no class inheritance to settle.

### Then zones

A zone stores its net's registry ID. Creating a zone on a net that has no record
promotes it in the same transaction. The zone steps are
[#231](https://github.com/tangency-labs/pcbcraft/issues/231) (drawing and editing),
[#232](https://github.com/tangency-labs/pcbcraft/issues/232) (fill) and
[#233](https://github.com/tangency-labs/pcbcraft/issues/233) (Gerber output). Zone authoring, the fill (computed with
i_overlay, pending its dependency review) and contour output then follow the
zones plan proposed in #214.

## Alternatives considered

- **A transitional registry.** This would put a root `instances.json` holding only
  the registry next to today's root `board.json`. Zones would arrive two format
  steps sooner. The cost is a second transitional format, plus a file that has to
  move later and whose schema grows twice. It's not proposed, because nothing
  points away from the storage design.
- **The registry inside `board.json`.** The design keeps the logical registry out
  of the physical document on purpose, and the registry would have to move later.

## Decisions to discuss

Each has a proposed answer. Review can change any of them before slice 1 starts.

1. **Slice 2 scope.** Proposed: only occurrences and one legacy circuit use, with
   components keeping their IDs. The full component-definition/instance split from
   [circuit-instances.md](circuit-instances.md#stable-identities-and-addresses)
   is needed for repeated circuits, not for the registry. It lands with
   repeated-instance editing.
2. **Where components are stored after slice 2.** Proposed: they stay in
   `project.json` until the repeated-instance slice, which moves them into
   `instances.json` together with the definition/instance split. Moving them
   earlier would change their record shape twice. Their later arrival adds a
   section to `instances.json` without reshaping what slice 2 writes.
3. **Junction anchors.** Proposed: left out of slice 3. These are saved wire points
   that ordinary wire edits create and prune, so they make weak anchors. They are
   only needed for a net with no terminal, label or port that an authored record
   targets.
4. **Format numbers.** Assigned against the implementation base when each slice
   lands, not fixed here.

## Acceptance

**Slice 1**
- An implicit-board project in a supported split schema and one at each of
  `project/9`, 12 and 13 open without rewriting in a `project.json` folder or
  container-v3 `.pc` package; first Save migrates even without design edits.
- The first migrating save moves `board.json` into its folder, keeps the board ID,
  name, placements, setup, outline, copper and baseline, and the second save has
  no diff.
- An interruption at each step of that save reopens as the complete old or the
  complete new project, never a mix.
- A foreign `boards/` directory selects the `pcbcraft-data-…` root and is left
  untouched.
- Folder names are checked for `con`, `aux`, case-only collisions and non-ASCII
  names.
- A container-v3 `.pc` round-trips through the new layout. Historical
  single-document fixtures stay unchanged in diagnostic codec tests and remain
  refused by normal app/CLI opens.
- A project folder opened by an older build after migration is refused with a
  format message, not partly read.

**Slice 2**
- Derived occurrence and circuit-use IDs are stable across opens before the
  first migrated save, and are persisted by it.
- A multi-unit component on two flat sheets is one instance.
- References, values, package choices and terminal mappings are unchanged.
- A schematic-only project stores its records in `project.json`. Creating the first
  board moves them to `instances.json` in one transaction.

**Slice 3:** the cases from
[the design's acceptance list](net-identity.md#acceptance-before-implementation)
that apply without classes.
- Disconnect and reconnect an isolated pin, and a pin with a dangling unnamed
  wire stub. The net keeps its ID and nothing is reported as unresolved.
- Delete the primary anchor. The ID survives on a fallback or a canonical
  replacement, through undo and reopen.
- A named singleton keeps its record.
- A two-branch split keeps the ID on the anchored branch and gives the other
  branch a fresh ID with lineage.
- A lost-anchor ambiguous split is recorded, then resolved by command, and the
  resolution survives reopen.
- A compatible merge gets a fresh ID with lineage, and retired IDs are never
  reused.
- **Fallback anchors:** a net with primary A and fallback B later gains C and D,
  and the stored anchors don't change. Delete B, then disconnect A. The net keeps
  its ID and picks a new primary from C and D, through undo and reopen.
- The four-anchor cap holds. A non-anchor wire edit changes only its sheet and
  doesn't rewrite the registry or promote baseline nets.
- Codec vectors for the anchor encoding: 9 versus 10, UUID ordering, empty and
  filled tokens, every kind and scope tag, and one component's terminals on
  different sheets.
- A project with no registry records saves byte-identically. Registry records
  appear only when the first one is created.

The conflicting class merge, ownership transfer between boards and system scope
stay with #77 and #122.
