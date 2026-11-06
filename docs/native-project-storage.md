# Native project storage

PCBcraft stores a project as independently editable documents. The editor and MCP
still share one project, command set and undo history; storage boundaries do not
create separate schematic and board sessions.

## File encodings and save behavior

The working form is a project folder with a plain UTF-8 JSON `project.pc` entry
point. Desktop New Project defaults to this form. A project's first Save asks
for a new folder, suggesting the project's name; one still named Untitled takes
the chosen folder or archive name as part of that save, with no undo step.
Cancelling or failing that save leaves it Untitled and unsaved. Open accepts the
folder, its entry point, a `.pc.zip` archive or a legacy project. Save keeps the opened form;
Save As offers a project folder and a packed project. The folder destination is
a new or empty project directory, while archive Save As requires an unused file.
Unrelated root files such as `README.md` and `.git` remain untouched.
Regular `.DS_Store`, `Thumbs.db` and `desktop.ini` files are also left alone inside
project subdirectories; they are not project documents and are never published
or removed by a save. Other unknown files in managed subdirectories are refused.

```text
my-board/
  project.pc
  sheets/<sheet-uuid>.json
  board.json
  library/symbols/<library>/<symbol>.r<revision>.json
  assets/                       # reserved for future asset references
```

Opening `project.pc` opens the entire project. Copy the folder or export its
`.pc.zip` to share it; the entry point alone does not contain the other documents.
A project rename changes the name in `project.pc`, without renaming its files.

The browser uploads packed projects and legacy single-file projects and downloads
`.pc.zip`. It cannot reconstruct a folder from an uploaded entry point alone.
Download encoding runs in batches of 256 records and archive hashing/copying in
256 KiB chunks, allowing the editor to yield between batches. The board-header
phase also writes its bounded setup of at most 64 material revisions, 128 layers,
127 strata and 256 nested prepreg plies. A requested download
does not confirm a durable write and does not clear the editor's dirty state.

## Document ownership

| File | Format | Contents |
|---|---|---|
| `project.pc` | `pcbcraft-project/5` | Project UUID, name, ordered sheet list and shared components |
| `sheets/<sheet-uuid>.json` | `pcbcraft-sheet/1` | Sheet UUID, unit placements, junctions and wires |
| `board.json` | `pcbcraft-board/1`, `/2` or `/3` | Footprints; version 2 adds layer/stackup setup, version 3 adds material reports and construction |
| `library/symbols/<library>/<symbol>.r<revision>.json` | `pcbcraft-symbol/1` | One full pinned symbol copy, including its library ID |

A sheet-list entry contains `id`, `name` and `file`. The file and the document's
UUID must agree. The current engine supports one editable sheet and explicitly
refuses multiple sheets, so it cannot silently discard them. The list and file
layout leave room for later multi-sheet editing.

Components own their stable ID, library/definition/revision, reference number,
displayed reference and value. Unit placements own their ID, component link,
unit, position in integer nanometers and rotation in degrees (0, 90, 180, 270).
Wires retain terminal or junction endpoints and optional authored bends; legacy
auto-routed pin links may omit bends. The sheet always has a junction array.
Footprints retain their component link, definition/revision, pin-to-pad mapping
and position. Empty projects still have sheet and board documents.

An unconfigured board retains version 1 and its existing encoding. Initializing
setup uses version 2, whose required `setup` contains a `layers` catalog and a
`stackup` profile. Layers have UUIDs, names, typed purposes and optional surface
sides; the profile has a UUID and front-to-back `strata`. Copper strata reference
catalog layer UUIDs, while dielectric strata own UUIDs and optional material
names. Profile, layer and dielectric IDs must be non-nil and mutually unique.
Catalog order is canonical UUID order; physical order belongs only to strata.
The legacy material name is descriptive text only. Material-aware setup uses
version 3, which adds a project-local `materials` catalog, exact ID/revision
references, sourced Tg and Dk/Df reports, core/prepreg construction, ordered plies
and optional symmetric finished-gap tolerance. The [material contract](board-materials.md)
defines field meanings, units and bounds. Unmodified version-2 projects retain
their bytes and unknown construction; material-aware fields require version 3
even when a field's collection is empty in the input.

The current uniform rigid profile alternates copper and dielectric, with front
and back copper at its ends and every catalog copper layer appearing once.
There are at most 128 catalog layers and 64 copper layers. Names are unique
case-insensitively, unpadded, nonblank, control-free and at most 80 characters;
material names use the same text bounds but need not be unique. Optional
`thickness_nm` is an integer from 1 through 100,000,000; the sum of specified
thicknesses cannot exceed 100,000,000 nm. Omission means unknown; explicit null
is rejected. The derived laminate thickness remains unknown until all strata
are specified, and is never stored as a second independently editable value.

Version 1 refuses setup, and versions 2/3 refuse missing setup. Unknown fields,
purposes, references and unsupported versions fail before project adoption.
Setup decoding preserves original JSON for typed validation, so duplicate keys
are refused rather than overwritten while checking the version's vocabulary.
Legacy single-document encoders refuse configured boards instead of losing
their setup; ordinary folder/archive saves use the current split-document
codecs. The [representation plan](pcb-representation.md) explains future regions,
footprint bindings, shared rules and the unit contract with #75.

Saved symbol copies include all terminals, strokes and pins. A project opens
without its source library. Loading never upgrades a pinned definition to the
latest revision. The last component using a copy determines its lifetime:
deleting that component removes the unused copy on the next save. Libraries and
symbols are identified by their IDs and revisions; paths locate those identities.

All documents reject unknown fields. Required lists and optional wire geometry
reject explicit null. Before adoption, validation checks the entire project:
component ownership and references, unit membership, wire endpoints, junction
usage, footprint mappings, exact definition revisions and unused copies. Object
IDs are unique across placements, junctions, wires and footprints; component IDs
have their own namespace. The session derives its next ID from all object types.
A component without a placed unit is still refused.

## Git behavior

The planned [named arrangements](board-workspace-plan.md#named-arrangements-and-reopening-a-project)
restore document views, panes and windows from per-project user-local state.
They do not add layout or camera records to these electrical file formats.
Automatic editor-layout restoration is not implemented yet; an explicit portable
arrangement export would be a separate format and feature.

Commit the working folder. Each editable record has a stable ordering and its
own line; separator commas occupy separate lines so appending a record does not
rewrite the preceding record. Saved symbol files have one line per terminal,
stroke and pin. Timestamps, generated hashes, caches and thumbnails stay out of
the working documents.

Moving or rotating a symbol changes only its sheet record. Editing a reference
or value changes only its component record in `project.pc`. Board geometry stays
in `board.json`. Placing a new definition changes the project and sheet documents
and adds its saved symbol file. A no-op save rewrites no project documents.
Even after another document changes, unaffected documents retain their original
bytes, including valid custom formatting.

Independent edits to existing records merge with Git's normal text driver, as
exercised by a real `git merge-file` regression test. Concurrent additions can
still conflict at the end of a list and can allocate colliding IDs or references.
Resolve those conflicts explicitly, preserving distinct identities and updating
references to any reassigned IDs, then reopen for validation. Blindly retaining
both records is insufficient when their IDs collide. An ID-aware merge driver is
outside this change.

```gitattributes
*.pc text eol=lf
*.json text eol=lf
*.pc.zip binary
```

Ignore `.pcbcraft-save/`, `.pcbcraft-stage-*/` and `.*.pcbcraft-lock`. ZIP exports
are binary; optional text conversion can make local diffs readable but does not
provide normal text merging. The folder is the collaboration format.

## Packed projects and manifests

A `.pc.zip` contains the same relative document paths and a root `manifest.json`:

```json
{
  "format": "pcbcraft-container/2",
  "parts": [
    {"path": "board.json", "size_bytes": 66, "sha256": "<SHA-256 of the exact bytes>"}
  ],
  "assets": []
}
```

The example is abbreviated: `parts` lists every document except the manifest,
sorted by path, with its exact size and SHA-256 digest. Assets are reserved;
nonempty assets and asset files are refused until the engine supports their
references. The working folder carries no manifest or cross-file hashes, which
would otherwise change alongside every edit.

A present manifest is authoritative. Missing parts, unlisted files, duplicate
claims, malformed manifests, unknown versions and size/digest mismatches fail
the open. A broken manifest never falls back to manifest-less loading.

A hand-zipped project **without a manifest** is accepted when it contains exactly
a valid project layout. An optional single top-level folder, such as
`my-board/project.pc`, is stripped; multiple roots and unrelated files are
refused. Ordinary directory entries are accepted. In archives without a manifest,
`__MACOSX/` sidecar trees and regular `.DS_Store`, `Thumbs.db` and `desktop.ini`
files are discarded before locating the project root. They still undergo ZIP
path, type, CRC and size checks. A present manifest disables these exceptions.
All document schema and cross-document checks still apply. The open notice says
that the next save or download will add a manifest. PCBcraft always writes one.

Writers use Store, fixed timestamps, fixed permissions and a deterministic entry
order (manifest first, then paths in lexical order). Readers accept Store and
Deflate. Traversal, absolute/drive paths, duplicate or overlapping entries,
symlinks, special files, encryption, ZIP64 and split archives are refused. Reads
are bounded by both declared and actual expanded size. A no-op save of a current
manifested archive preserves its original bytes, including compression and
comments. A first save of a manifest-less archive intentionally adds the manifest.

| Bound | Limit |
|---|---|
| Legacy single JSON document | 4 MiB |
| Each split JSON document | 16 MiB |
| All split documents combined | 48 MiB |
| Project documents | 4,096 |
| Container v2 manifest | 1 MiB |
| Packed file | 64 MiB |

These bounds supplement existing symbol, wire, geometry and library-definition
limits; separating files does not remove those model constraints.

## Folder publication and recovery

Desktop file work runs off the editor thread against an immutable snapshot.
The shared `pcbcraft-storage` crate also gives the headless CLI the same folder
reader and recovery behavior. A nonblocking sibling advisory lock serializes
cooperating opens and saves, and saved baseline bytes detect stale writers.
A busy lock produces an error instead of leaving another operation waiting.
If filesystem permissions prevent opening the lock file for writing, an open
uses an existing readable lock in shared mode, or reads without a lock if none
exists. Both fallbacks require no recovery journal before and after reading;
saves and recovery always require the writer lock. This permits opening a
read-only project and using Save As to place edits in a writable location.

A save stages only changed or removed documents. It writes their new bytes and
old backups into a temporary directory, flushes them, and writes a journal with
relative paths, sizes and hashes. After rechecking the baseline, it atomically
renames the staging directory to `.pcbcraft-save`. This is the commit point:
interruptions before it retain the old project; after it, the next open completes
the new project before reading the project into the editor.

Recovery checks every journal path and old/new payload, verifies that affected
files still contain old or new bytes, and validates the proposed complete project
before applying remaining replacements and deletions. Unexpected external edits
or corrupt recovery data stop recovery without overwriting them. New files are
flushed before replacement, and directories are synced on Unix. Recovery scratch
files stay inside the transaction directory, so interrupted writes cannot leave
unrecognized files in `sheets/` or `library/`.

Once all replacements are complete, recovery renames the journal directory out
of its active location before cleanup. A crash during cleanup therefore cannot
leave an active journal with missing staging files. Leftover staging directories
are ignored and may be removed when no save is running. Interrupted initial saves
can be recovered by opening their folder or entry-point path.

Tests simulate interruption after staging writes, journal publication, each
replacement/deletion and completion. They verify that reopening yields the full
old or new state. These are process-interruption tests on Linux, not hardware
power-loss tests. Directory fsync is currently Unix-specific; other platforms
retain the journal recovery protocol without the same directory-durability
claim. Programs that ignore the advisory lock can still race with publication.

Failed opens leave the active project, revision and dirty state unchanged. Edits
made during a background save stay unsaved when its earlier snapshot completes.
Archive publication retains the existing single-file staging and baseline check.

## Legacy migration and fixtures

Plain JSON formats 1–4 and container v1 remain readable. Formats 1–3 resolve the
exact retained starter revisions; format 4 supplies its full symbol copies.
All migrate in memory to the same split documents without changing IDs, geometry,
wire connectivity or footprint mappings. Save asks for a new folder destination
by default; Save As can instead select `.pc.zip`. It never silently replaces a
legacy text `.pc` with an archive or a folder entry point requiring missing files.

The original `two-resistors-format-1.pc`, `two-resistors-format-2.pc` and
`two-resistors.pc` (format 4) fixtures remain legacy inputs. Canonical outputs live
under `crates/app/tests/fixtures/split/`, including the folder, a manifested archive
and a hand-zipped archive without a manifest. Generate a fresh comparison set with:

```sh
cargo run --locked -p pcbcraft-engine --example storage_fixtures -- /tmp/new-fixtures
```

The output directory must not already exist. The generator uses the production
encoder; regression tests compare migrations, document boundaries, library-copy
lifetimes and archive round trips. Legacy writers remain available to fixture and
benchmark code, while normal editor saves use the split format.

## Design references

The validation and round-trip requirements draw on independently authored
PCBcraft Next material at revision `568cd495397a91563e563e7163efb226d84fa83a`
under MIT OR Apache-2.0. The proposal uses Tangency's existing session and
terminal model to specify a new fixture.
[Persistence design](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/docs/specs/persistence.md),
[codec reference](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/src/persistence.rs),
[schematic test contracts](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/tests/schematic.rs),
[project test contracts](https://github.com/travle1/pcbcraft-next/blob/568cd495397a91563e563e7163efb226d84fa83a/tests/sheets.rs).
