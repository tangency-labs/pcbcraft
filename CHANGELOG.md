# Release notes

## 0.1.0 (unreleased)

DRC measures copper and board-edge clearance against the same rounded pad
contours as connectivity, avoiding false findings at empty bounding corners.
Exact clearance equality still passes, including half-nanometer radii.

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
