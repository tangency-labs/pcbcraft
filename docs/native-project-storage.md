# Native project storage proposal

[PCBCraft's vision](../VISION.md#how-our-files-should-work) sets two observable
requirements for native projects under version control. Saving an unchanged
project produces zero diff, and moving one part changes only that part's lines.
This proposal defines a first schematic round trip that can test both properties.
The working format, sharing format and file layout remain open for review.

## What a project preserves

Give the project, sheet, component, placed symbol and wire persistent identities
that survive reopening, renaming and reordering. A terminal reference identifies
a symbol and a pin within its resolved definition. A wire between two pins stores
those references, so moving a symbol keeps the wire attached without rewriting
the wire record. Derived pin coordinates, displayed routes and the computed
netlist can be rebuilt when the project opens.

Use a deterministic order for records, keys and unordered endpoint pairs.
Preserve authored ordering where it carries meaning. Keep placements in
independently diffable records and store geometry as exact integer coordinates
in the engine's units. Reopening and saving supported files without an edit
preserves the tracked bytes, including files changed by a clean merge. Keep
timestamps, previews, caches, preferences, selections, undo history, session
revisions, assistant state and credentials outside the tracked project.

Pin each used definition by persistent ID and revision. The first profile can
resolve an approved, versioned built-in symbol offline. If the project must
carry a copy of a used definition, record it once with its origin, revision and
license or notice, subject to redistribution approval. An unavailable required
definition produces a dependency error, since substituting a newer pin layout
could change connectivity. Large assets, if supported, can live separately
and be verified by content hash.

## Opening and saving safely

Version the project schema independently of the application. Set limits for
input bytes, decompressed bytes if packaged, nesting, strings and record
counts, and enforce them before allocation or during decoding as appropriate.
Validate IDs, geometry, wire endpoints and required definitions before replacing
the open project. A newer schema, unsupported authoritative field, duplicate
ID or dangling endpoint should produce an actionable error and leave the open
project untouched. A migration is an explicit operation with prior-version
fixtures. If the format permits asset paths, confine resolution to approved
project locations and reject traversal, escaping links and special files.

Save a snapshot of one session revision. Validate and stage it before
publication, and compare the destination against the baseline from open or the
last successful save as part of publication. Two supported writers starting
from the same baseline should have at most one successful publication. An
external change detected before publication should produce a conflict, keeping
both the user's edits and the external version available. For a multi-file
layout, choose a commit and recovery mechanism that lets readers see a whole
old or new snapshot. A sequence of independent file replacements can expose
a mixed project. For each supported platform, test the publication boundary
and document its guarantees for other processes writing to the destination.

A canceled or failed save before publication leaves the previous snapshot
readable and the current edits unsaved. If another edit lands while a save is
in flight, the save marks only the staged revision as persisted. Desktop,
browser and MCP should use the same engine snapshot and validation contracts,
with storage adapters that report their own publication results.

## First fixture and acceptance

Create an original one-sheet fixture with two approved built-in resistors.
Place `R1` at `(0, 0)` and `R2` at `(25_400_000, 0)` in nanometers. Give each a
fixed component and symbol ID, and connect pin `2` of `R1` to pin `1` of `R2`
with one fixed wire ID. Both symbols use the same pinned definition.

1. Save, reopen and compare the identities, exact positions, definition revision
   and terminal group. Pins `R1.1` and `R2.2` remain isolated. Save again and
   compare every tracked byte.
2. Move `R1` to `(0, 1_270_000)` using the shared editing command. Save and
   check that the diff is confined to `R1`'s placement record. Reopen and verify
   the same wire ID and terminal membership. Separately move `R2` in a copy
   from the common baseline and check that the two edits can merge and reopen.
3. Try to open variants with an unsupported version, an oversized input and a
   nonexistent terminal. Check that each fails before it replaces the open
   project. Simulate a failure before publication and race two writers from
   the same baseline. Check the destination and dirty state in both cases.
4. Stage a save after moving `R1`, then move `R2` before publication completes.
   Reopening the saved snapshot should show only the `R1` move, with the
   `R2` move still unsaved in the active session.

## Decisions before implementation

- Choose the text encoding and tracked layout, including record boundaries
  that allow independent edits to merge. Decide whether a working directory
  and a ZIP for sharing are both supported and how they relate.
- Choose the initial schema, limits and migration policy. Agree on how an
  unchanged supported file retains its bytes and when an explicit rewrite is
  appropriate.
- Agree on the definition capture policy and the publication, concurrency
  and recovery guarantees supported by each platform and layout.

## Design references

The validation and round-trip requirements draw on independently authored
PCBcraft Next material at revision `568cd495397a91563e563e7163efb226d84fa83a`
under MIT OR Apache-2.0. The proposal uses Tangency's existing session and
terminal model to specify a new fixture.
[Persistence design](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/docs/specs/persistence.md),
[codec reference](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/src/persistence.rs),
[schematic test contracts](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/tests/schematic.rs),
[project test contracts](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/tests/sheets.rs).
