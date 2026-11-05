# Native project storage specification

[PCBCraft's vision](../VISION.md#how-our-files-should-work) sets two observable
requirements for native projects under version control. Saving an unchanged
project produces zero diff, and moving one part changes only that part's lines.
The native `.pc` extension identifies a PCBCraft project. This specification
defines plain JSON for working files and a ZIP container for sharing. Both encodings carry
the same project schema. The desktop, browser and MCP CLI can open either one.

## File encodings and save behavior

| Encoding | Intended use | Contents |
| --- | --- | --- |
| Plain JSON `.pc` | Default for new projects and Git-tracked work | One UTF-8 project document tagged `pcbcraft-project/2` |
| ZIP `.pc` | Portable copy for sharing or bundling assets | A manifest, the same project document, and any bundled asset bytes |

Open determines the encoding from the file contents. A ZIP signature selects
the archive reader, which must validate the PCBCraft manifest before accepting
the project. Other inputs go through the JSON reader, which validates the
project format tag. A corrupt ZIP or an unrelated ZIP produces a format error
and leaves the active project intact.

On desktop, **Save** preserves the encoding of the open file. **Save As**
offers **Plain JSON** and **ZIP archive**, with the same `.pc` extension for
both. New projects default to plain JSON. The browser offers the same encoding
choices when downloading and reports a download request. A successful
download request cannot confirm a durable write.

Converting between encodings preserves project, sheet, component, symbol, wire,
and footprint identities, along with pinned definitions and connectivity.
If a project requires assets carried by an archive, conversion to plain JSON
must keep those assets available under a supported project asset layout or
fail with a list of the affected dependencies. The first archive reader and
writer can ship with an empty asset list because schema version 1 has no
asset references.

## ZIP container version 1

The root entries are `manifest.json` and `project.json`. The manifest is
UTF-8 JSON. It identifies the container independently of the project schema
and records the byte length and SHA-256 of each payload. `project.json`
contains the same versioned JSON the plain reader accepts.

| Manifest field | Meaning |
| --- | --- |
| `format` | Exactly `pcbcraft-container/1` |
| `project` | Object with `size_bytes` and `sha256` for `project.json` |
| `assets` | Array of objects with `size_bytes` and `sha256`, empty when there are no bundled assets |

A bundled asset is stored at `assets/<sha256>`, using the lowercase
64-character digest of the exact uncompressed bytes. Write each digest once.
The project schema owns the asset's role, provenance and license information.
When that schema supports assets, each bundled asset must match a project
reference and all required bundled references must resolve before adoption.
The container does not need to extract anything into the working directory.

The v1 reader accepts regular, unencrypted ZIP entries using Store or
Deflate. ZIP64 and split archives are unsupported. The current project schema
permits exactly two entries and requires an empty asset list. It rejects duplicate names, unexpected entries, absolute or
traversal paths, symlinks, and special files. Enforce declared and actual
uncompressed byte counts while reading, and verify every payload digest
before using it. Apply these initial limits to the container and the decoded
project together.

| Limit | Maximum |
| --- | --- |
| Archive file | 64 MiB |
| Total uncompressed content | 256 MiB |
| Entries | 1,024 |
| Manifest | 256 KiB |
| Project JSON | 4 MiB |
| Individual asset | 128 MiB |

An unchanged save preserves the original JSON or archive bytes. For a changed
archive, write manifest and project first, followed by assets in digest
order. Use fixed archive timestamps and consistent entry metadata. The plain
encoding remains the Git merge surface, and a ZIP created for sharing can
be regenerated from the same snapshot.

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

The plain JSON gate runs in the workspace test suite with the checked-in
[two-resistor fixture](../crates/app/tests/fixtures/two-resistors.pc) and
[native storage acceptance tests](../crates/app/src/project_files/native/tests.rs).
The Git merge case uses the installed `git merge-file` command. The same native
suite covers ZIP conversion, rejection before adoption, and publication. The
[ZIP codec tests](../crates/engine/src/persistence/archive.rs) exercise Deflate,
manifest size and digest validation, and prohibited entries.

## ZIP acceptance and later layout decisions

1. Convert the original resistor fixture from plain JSON to ZIP and back.
   Confirm that both use `.pc`, load to the same project, and preserve
   connectivity and footprint mappings. Confirm that Save retains each
   encoding and that a no-op leaves the original bytes intact.
2. Reject a foreign ZIP, duplicate entries, unsafe entry names, a mismatched
   digest, and payloads that exceed declared or allowed bounds. Confirm that
   each failure leaves the active project and its dirty state intact.
3. Repeat the interrupted save, competing writer and late edit checks with an
   archive. Measure packaging and unpacking separately from editor frame time.
   Keep compression and file IO off the desktop UI thread and bound browser
   work so the editor can continue responding.

Splitting large projects into a Git-friendly directory, expanding the
definition capture policy, and setting up migrations for later versions
remain separate design decisions.

## Current implementation

The current project document is plain UTF-8 JSON tagged
`pcbcraft-project/2`. The files contain a project ID, name, one sheet with a
stable ID, and one line per placement and wire. Each symbol line records its
`rotation` in degrees counterclockwise (0, 90, 180 or 270). It records
`reference` and `value` only when a person edited them, so a symbol keeps one
line and an untouched symbol never gains those fields. An edited reference has
no spaces and is unique among the shown references; both texts are 1 to 32 or
64 characters on one line.

### Format 1 migration

Opening a `pcbcraft-project/1` file runs an explicit migration: each symbol
becomes unrotated with its automatic reference and its definition's value, and
the editor says the next save writes the current format. That save changes
the format line and adds `"rotation":0` to every symbol line, once. A format 1
file that carries a format 2 field is refused. The migration is tested against
the prior-version fixture `crates/app/tests/fixtures/two-resistors-format-1.pc`,
which must save as exactly `two-resistors.pc`. Builds that only read format 1
refuse format 2 files with the unsupported-format error. Blank lines between records
let neighboring edits merge. A wire saves the two symbol IDs and pin numbers.
A symbol pins its built-in definition revision. An optional board section
stores footprint placements and the explicit pin-to-pad mapping. The
generic two-pad footprint can be assigned through `pcb.place_footprint`.
The engine builds indexes for pin and pad lookup, which MCP exposes through
`cross_probe`. Board tracks, vias, layout editing, and reviewed schematic-to-board
updates need a later schema and editor slice.

Object IDs preserve the current command addresses and are scoped to the
project. Two Git branches can move different placements independently.
Concurrent additions from the same ancestor can allocate the same numeric ID
or reference. The loader rejects a resulting collision so it can be resolved
explicitly. A future allocator can avoid the ID collision without changing
how terminals refer to placed symbols.

The decoder rejects unknown fields, invalid connections, unknown definition
revisions, and files over 4 MiB. Unchanged saves and browser re-downloads
retain the bytes originally opened. Edited snapshots are written in ID order.
The desktop uses a sibling advisory `.*.pcbcraft-lock` file, which belongs
outside Git tracking. Cooperating desktop writers check their expected bytes
under that lock before replacing the file. Save As requires an unused name.
A separate program that ignores the lock can still race with publication.
The browser reads the chosen file asynchronously and validates the bounded
payload before adoption. Downloads yield between batches of JSON encoding and
256 KiB steps of ZIP hashing or copying. The ZIP writer uses Store, so it
does not compress the snapshot. The browser owns the final write.

The canvas renders typed project snapshots. A symbol drag previews geometry
and wire endpoints before committing a single shared move command on release.
Geometry-only moves retain the cached connectivity index. Desktop saves hold
an immutable snapshot while edits continue. The release benchmark measures
this supported model, including indexed pin/pad lookup, independently of GPU
frame timing.

```sh
cargo run --release --locked -p pcbcraft-engine --example project_perf
```

Pass an output path after `--` to open the same synthetic fixture in the editor.
The fixture has one sheet with 2,048 symbols, 2,047 wires, and 2,048 linked
footprints. In one Linux release run on a Ryzen 9 7950X, the p95 was 0.037 ms
for a move with cached connectivity, 1.23 ms to encode, and 2.77 ms to decode
and validate. Full MCP inspection took 60.3 ms at p95. A browser smoke test
preserved the original file bytes and exported an edited snapshot while
panning. Software-rendered Chromium reported about 33 ms at p95 for animation
frame intervals, both during export and during pan alone. The 16.7 ms target
still needs qualification on desktop and hardware-accelerated web. These
measurements cover the synthetic single-sheet fixture. For the same fixture, ZIP
packaging of the already encoded JSON took 1.37 ms at p95, and opening the
ZIP, including JSON validation, took 3.82 ms at p95 in a Linux release run.
These are engine timings, excluding disk I/O and editor frame time.

## Design references

The validation and round-trip requirements draw on independently authored
PCBcraft Next material at revision `568cd495397a91563e563e7163efb226d84fa83a`
under MIT OR Apache-2.0. The proposal uses Tangency's existing session and
terminal model to specify a new fixture.
[Persistence design](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/docs/specs/persistence.md),
[codec reference](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/src/persistence.rs),
[schematic test contracts](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/tests/schematic.rs),
[project test contracts](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/tests/sheets.rs).
