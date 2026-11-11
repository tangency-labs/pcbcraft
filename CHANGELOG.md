# Release notes

## 0.1.0 (unreleased)

Projects now use a fixed `project.json` entry point inside their working folder,
and packed projects use `<project-name>.pc`, a ZIP archive of the same documents.
The archive retains `manifest.json` with document sizes and SHA-256 hashes, using
`pcbcraft-container/3`. Working folders remain the form to commit to Git; project
locations and the first-save location workflow are unchanged.

This breaks compatibility with earlier `project.pc` folders, container-v2
`.pc.zip` archives and legacy single-document JSON/container-v1 projects. Those
inputs are refused by desktop, browser and CLI opens without converting or
overwriting them. Renaming an old archive does not convert it, and this release
does not include a migration command. Keep an earlier build for existing work.
Existing sheet, board and library schemas are unchanged.
