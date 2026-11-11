# Release notes

## 0.1.0 (unreleased)

Projects now use a fixed `project.json` entry point inside their working folder,
and packed projects use `<project-name>.pc`, a ZIP archive of the same documents.
The archive retains `manifest.json` with document sizes and SHA-256 hashes, using
`pcbcraft-container/3`. Working folders remain the form to commit to Git; project
locations and the first-save location workflow are unchanged.

This breaks compatibility with earlier `project.pc` folders, container-v2
`.pc.zip` archives and legacy single-document JSON/container-v1 projects. Normal
desktop, browser and CLI opens refuse those inputs without overwriting them. The explicit `pcbcraft-cli convert SOURCE DESTINATION` command converts
previous split folders and container-v2 archives to a new folder or `.pc` package,
leaving the source unchanged. Renaming an old archive alone does not convert it.
Standalone projects and retired document features remain unsupported. Sheet, board and library payload schemas are unchanged by this layout conversion.

New and converted projects use project/14 with a registered board folder, including
single-board projects. Supported flat `project.json` projects open unchanged and
migrate on first Save without requiring design edits. The journal moves the board
and updates its registry together, preserving board identity and physical bytes.
Successful layout adoption remains across Undo/Redo; subsequent unchanged saves
produce no diff. Foreign physical-data directories select an unused persisted
namespace rather than being claimed by the project.
