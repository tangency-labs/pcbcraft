# Release notes

## 0.1.0 (unreleased)

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

New and converted projects use project/14 with a registered board folder, including
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
