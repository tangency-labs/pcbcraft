# Optional single board storage

This bounded implementation supports schematic-only projects and projects with
one physical board. It supplies explicit ownership and compatibility migration
before the broader [multiboard storage](multiboard-storage.md) and circuit
instancing workflows are implemented.

## Entry point and ownership

Split project format `pcbcraft-project/9` extends the numeric-watermark format
with a required non-null `boards` array. It contains zero or one registration;
additional entries are refused. The existing project UUID, ordered sheets,
components and required `numeric_id_high_water` retain their validation rules.
Project/12 combines that same registry with the component metadata extension from
legacy-ownership project/11. Project/9 rejects metadata, project/11 rejects a board
registry, and project/12 requires both the registry and allocation watermark;
none silently adopts fields from the other vocabulary.

| Registration field | Rule |
| --- | --- |
| `id` | Stable non-nil UUID, distinct from the project and sheet UUIDs |
| `name` | Trimmed, nonblank, control-free text of at most 200 characters |
| `file` | Exactly `board.json` in this transitional single-board encoding |

An empty array means the project is schematic-only. It has no physical target,
setup or board document, and a stray `board.json` is rejected as an unexpected
managed document. A registration requires its referenced physical document.
Physical data remains in the existing board formats, with pinned footprint
definitions under the existing library paths. Board commands resolve the
registration's ID; omitted targets default only when a board actually exists.

Version 9 binds the project's ordinary schematic contents as one whole capture
scope to its explicitly created board. It cannot assign only selected flat roots,
split a circuit use or represent mixed physical owners. Sheet hierarchy and
browser presentation do not establish a different owner. Later occurrence and
board-assignment migrations must preserve this whole-scope association explicitly.

Project/14 replaces the transitional `file` field with a portable `folder`
segment and adds `layout_root`, while retaining zero-or-one ownership. New and
converted projects retain this layout in project/16, which adds
[single-use occurrence records](sheet-occurrence-storage.md). Supported flat layouts open
without rewriting; their first Save moves the board even without design edits,
then subsequent unchanged saves have no diff. The native storage contract's
[board-folder migration](native-project-storage.md#board-folders-and-first-save-migration)
defines collision handling, foreign namespaces and journaled publication.
Neither layout discovers boards by scanning directories.

## Creation and legacy migration

`project.new` starts schematic-only. `project.new` with `create_board: true`
creates the project and its board in one undoable action, used by New PCB on Home.
`pcb.create_board` adds the first board to an existing project, optionally taking
its name and requiring the current revision. A second creation is refused.
Opening the PCB workspace alone does not create physical state.

Legacy formats retain their sole deterministic compatibility board identity and
their old encoding on Open and in the low-level codec until Save migrates it. The first authored physical change
persists that same ID as a registration named `Board`, upgrading the entry point
in the same transaction. Schematic-only edits do not independently register a
legacy board. Undo restores the earlier ownership representation while allocation
metadata continues to obey its monotonic contract. Once Save adopts the current
format, Undo/Redo retain the storage layout, selected root and occurrence IDs
across design history.

An older project may legally have a sheet UUID equal to its deterministic board
UUID, because those were separate typed addresses. That case keeps its legacy
ownership encoding through physical edits instead of renaming either identity.
Project/14 migration preserves this exact deterministic compatibility ID and
`Board` name even when a sheet shares its UUID; new arbitrary colliding board
registrations remain invalid. The typed board and sheet addresses stay distinct.

New board IDs use independent UUID allocation. Rename, save, reopen and view
creation cannot derive another identity from a name or a path. GUI and MCP use
the same creation and target-validation commands.

## Other encodings and views

Packed projects use the same split documents and existing bounded container
validation. The diagnostic single-document encoder uses
`pcbcraft-project/10` for explicit optional ownership, with the same required
watermark and registrations. It retains its existing restrictions on multiple
sheets, hierarchy, component metadata, field appearance and physical board
features that need split documents. Adopted occurrence identities also require
split documents or a packed archive; diagnostic export refuses to discard them.
Older formats reject registration fields rather than silently discarding them.

Schematic-only footprint choices use the bounded
[project/13 extension](native-project-storage.md#schematic-only-package-choices).
They belong to the project until explicit first-board creation adopts them, with
no change to board registration or per-board folder layout. Assigning a package
does not create a board, and physical placement remains unavailable until one exists.

An empty PCB workspace is user-local presentation state addressed by project ID.
It has no BoardTarget and grants no physical editing authority. A view of a
specific board that becomes unavailable does not silently bind another document.
Neither empty workspace arrangement state nor cameras appear in project files.
