# Release notes

## 0.1.0 (unreleased)

Tracks and vias can be locked together in Properties or through
`pcb.set_copper_locked`. Locked copper stays selectable and refuses geometry,
size and deletion changes, including indirect changes from footprint transforms
and segment dragging. Mixed edits fail atomically. Clipboard copies retain locks;
Cut refuses protected copper. Locks persist with board format 18 and board-kind
clipboard format 3, which older builds refuse. Unlocked files keep their earlier
encodings; undo/redo preserves both geometry and lock intent.

DRC measures copper and board-edge clearance against the same rounded pad
contours as connectivity, avoiding false findings at empty bounding corners.
Exact clearance equality still passes, including half-nanometer radii. A reusable
engine clearance query now captures board/snapshot/revision authority and separates
conservative obstacle candidates from contour decisions; conflicting intended
memberships remain visible even at zero clearance.

Projects now use a fixed `project.json` entry point inside their working folder,
and packed projects use `<project-name>.pc`, a ZIP archive of the same documents.
The archive retains `manifest.json` with document sizes and SHA-256 hashes, using
`pcbcraft-container/3`. Working folders remain the form to commit to Git; project
locations and the first-save location workflow are unchanged.

Existing `project.pc` split folders and container-v2 `.pc.zip` archives open
without rewriting their files; their first Save upgrades them automatically.
Recent-project previews also accept these layouts. The explicit `pcbcraft-cli convert SOURCE DESTINATION` command optionally converts
previous split folders and container-v2 archives to a new folder or `.pc` package,
leaving the source unchanged. Renaming an old archive alone does not convert it.
Historical power, power-bar and ground annotations migrate to pinned native power
symbols, retaining names, positions, rotations and wire connections. Those sheets
and project metadata are updated on Save; unrelated payloads retain their bytes.
Standalone JSON/container-v1 projects remain unsupported.

New and converted projects use project/16 with a registered board folder, including
single-board projects. Supported flat `project.json` projects open unchanged and
migrate on first Save without requiring design edits. The journal moves the board
and updates its registry together, preserving board identity and physical bytes.
Successful layout adoption remains across Undo/Redo; subsequent unchanged saves
produce no diff. Foreign physical-data directories select an unused persisted
namespace rather than being claimed by the project.


Native libraries now support reusable part and category records, canonical typed
quantities and manufacturer/MPN records alongside symbols and footprints. Parts
Catalog placement pins the complete part dependency set, with optional package
and manufacturer selection. The part form edits typed requirements and reports
when a retained MPN no longer meets them. Symbol-only instances can also choose
external native footprints through the assignment form. Sources remain separate
from pinned project data, including after relocation or removal.

Multi-unit symbols share one logical part, reference and physical package across
sheets. Project format 15 carries native part selections, board format 17
preserves multi-pad/explicitly unconnected-pad mappings, and accepted baseline
format 2 retains those maps. Older builds reject these new formats. Existing
single-unit/native records keep their low-level encoding when unchanged; the
ordinary first-save board-folder migration remains in effect. These additions
remain within the unreleased 0.1.0 version. High-pin-count devices, KiCad import,
vendor APIs and general pad stacks remain separate work.

Each sheet now has a stable occurrence UUID, with one shared circuit use for the
existing hierarchy. Board projects store these records in `instances.json` beside
`board.json`; schematic-only projects keep them in `project.json`. Opening an
older project leaves it unchanged, and first Save persists deterministic
compatibility IDs without changing component references, values, package choices
or accepted board state. Creating the first board moves the same records in one
undoable action. Repeated sheet instances remain future work.

Named and explicitly promoted nets now have independent persistent UUIDs in a
sparse registry. Ordinary continuations retain their IDs, splits and merges
record lineage, and unresolved splits can be resolved through the shared
revision-checked `nets.promote` and `nets.resolve_ambiguity` commands in headless
or live MCP. Undo, reopen and first-board creation preserve the decisions. The
first record selects project/17 and board instances/2 in the existing folder/ZIP
layout; reading an older project does not promote its nets. There is no new UI,
and net classes and zones remain separate work.
