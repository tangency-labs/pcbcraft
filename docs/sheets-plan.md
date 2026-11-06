# Sheet editing plan

Status: planned, following the floating Parts library work.
The user confirmed these decisions on November 5, 2026:

- Extend PR #39's split-document storage model.
- Include reuse under several parents in the first complete hierarchy version.
- Use KiCad's terminology and controls: hierarchical sheets and labels, **S**
  for a hierarchical sheet, double-click to enter and **Alt+Backspace** to leave.
  Child hierarchical labels maintain parent sheet pins automatically.

## Base and sources

[PR #39](https://github.com/tangency-labs/pcbcraft/pull/39), inspected at
`f26efd04addb50f17e9c28e1615ef73ca4d12448`, was merged as
`20c2cca41f3c9b06feb207e59936d711d440db66` and integrated into this branch. Its format 5 entry
point already contains an ordered list of sheet IDs, names and relative files,
with project-level components. Each `pcbcraft-sheet/1` document owns placements,
wires and junctions. The reader currently rejects multiple sheets explicitly.
The newer PR revision changes recovery and archive handling rather than that
ownership model. The final revision also probes filesystem permissions in tests.

Keep the Browser/Parts branch independently reviewable. Create the sheets stack
from main with PR #39 and the Browser/Parts changes, retaining their attribution.
Preserve the storage branch. Publication of the Browser/Parts PR is authorized;
the sheets implementation remains a separate task.

Behavior and presentation references are PCBcraft Next's
`docs/sheet-workflows.md`, `src/desktop/sheets.rs` and `src/desktop/panels.rs`
at `568cd495397a91563e563e7163efb226d84fa83a`, MIT OR Apache-2.0.
Reimplement the workflows through upstream's engine and chrome; do not transplant
the prototype Session or project model. The KiCad-source exposure recorded in
Next's `docs/sources.md` remains separate from this work. Published manuals may
clarify controls; excluded implementation sources and derived fixtures remain
prohibited.

## Commit sequence

Each row is a local commit or a small stack on its own topic branch. The first
complete hierarchy milestone includes steps 3–5, including reuse.

| Step | User-visible result | Implementation boundary |
|---|---|---|
| 1. Flat-sheet commands and storage | MCP creates, renames and deletes sheets, and places or wires parts on a specified sheet. | Extract sheet-owned collections from `Project`, retain project components and one history, extend PR #39's codec to all sheets, and add shared commands and inspection. |
| 2. Flat-sheet navigation | Browser lists all sheets; the Sheets ribbon group creates one; context menus rename or review deletion. Breadcrumb and `n / N` follow the selection. | Add chrome-backed dialogs and per-sheet camera/selection state. Navigation cancels transient gestures. Test real UI commands and inspect rendered full and compact layouts. |
| 3. Reusable hierarchy commands | MCP places a new child drawing or another occurrence of an existing drawing, and inspects distinct physical parts in each occurrence. | Add sheet-symbol links, persistent occurrence IDs and component bindings. Reject cycles and excessive expansion atomically. Save each shared drawing once. |
| 4. Hierarchy in the editor | S starts sheet placement; the dialog offers New drawing or Reuse existing. Browser nests occurrences, double-click enters and Alt+Backspace leaves. | Add previews, placement, selection, movement and reviewed deletion of sheet symbols. Navigate by occurrence and exercise nested reuse, undo, stale gestures and reopening. |
| 5. Hierarchical labels and wiring | Child labels automatically expose parent sheet pins; wiring those pins connects the parent and child circuits. | Add stable interface identities and label/pin endpoint kinds. Reconcile every use of an interface in one command and compile occurrence-qualified connectivity. |

## Flat-sheet contract

`Project` owns ordered sheets, components, pinned definitions and the board.
Each sheet owns symbols, wires, junctions and derived local connectivity.
The engine has no globally active GUI sheet: view selection stays in `crates/app`.
Mutation commands carry a sheet ID and `expected_revision`. Existing callers
may omit the sheet only when there is exactly one possible target; ambiguous
requests fail rather than silently editing Main. Apply this rule to movement,
rotation, deletion and field editing as well as placement and wiring. Inspection
publishes the IDs clients pass back.

Commands validate ownership before adoption. An endpoint from another sheet is
not a local wiring target, even at matching coordinates. Preserve global
object-ID uniqueness, project-wide component references and pin-to-pad links.
Deleting a sheet removes its geometry and components without remaining placements,
including their footprint associations. The dialog describes that impact, and
one Undo restores it. Keep at least one sheet; an unchanged rename is a no-op.

Flat sheets can continue using `pcbcraft-project/5` and `pcbcraft-sheet/1`, whose
structures already represent the data. Preserve every formats 1–4/container-v1
migration and PR #39's folder/archive behavior. Write the ordered sheet list as
independently mergeable records rather than one serialized line. Renaming a sheet
changes metadata rather than its UUID or filename. Editing one sheet leaves
unaffected documents byte-for-byte unchanged, including valid custom formatting.

Navigation, selection and cameras never advance the project revision or enter
undo history. A scope switch cancels placement, wire drafts, symbol drags, part
drag payloads and pending field edits. Gestures capture their target sheet or
occurrence as well as the revision. Undo and deletion reconcile the active view
against the current snapshot without recreating deleted objects from UI caches.

## Reuse and physical identity

Distinguish a sheet definition from an occurrence of its drawing. Sheet symbols
link to child definition IDs, and those links determine the hierarchy. Ordered
roots provide entry points. Occurrence records preserve IDs for each root and
each `(parent occurrence, sheet-symbol ID)` pair; validation requires the records
to agree exactly with the hierarchy derived from links. Names, references,
coordinates and breadcrumb paths never serve as identity. Repeated parents
generate distinct nested occurrences as well as distinct children.

The proposed binding model keeps physical components at project level and maps
each occurrence's local component slot to a physical component ID. Two uses of
one drawing therefore have distinct references and footprint associations.
Moving shared geometry changes the drawing once; editing a physical component's
reference or value resolves through its occurrence binding. Reuse initializes
new components from the selected source occurrence and allocates unused references
without renumbering existing parts. Existing component, placement and footprint
IDs survive migration. Deletion preserves definitions still used elsewhere and
removes bindings and components that are no longer reachable.

Occurrence/binding and hierarchy fields need explicit schema versions. Retain
PR #39's folder layout and publication protocol, but do not silently reinterpret
a format-5 component link. Define the next project/sheet schema and migration
from format 5 in step 3, keeping all older input readers. Persist occurrence and
binding records in stable order, without derived connectivity or net hashes.
A shared geometry edit should still change only its owning sheet document.

## Hierarchical connectivity

A logical child interface has a permanent ID, name and electrical direction.
Hierarchical-label placements reference it; corresponding parent pins resolve
it through a sheet symbol. Renaming an interface preserves IDs and updates all
displayed parent pins. Moving a label changes its local anchor rather than the
parent pin identity. Repeated labels for one interface share that identity;
contradictory directions or ambiguous definitions must fail explicitly.

Local endpoints distinguish symbol terminals, junctions, hierarchical labels
and sheet pins. Whole-project connectivity qualifies local endpoints by
occurrence, then joins explicit matching parent-pin/child-interface pairs.
Equal names, references or coordinates in separate occurrences do not connect
them. An interface edit reconciles every use in one candidate before a single
history commit. Removing its last label/pin also removes incident wires, with
a reviewable impact and reversible Undo.

Bound definition count, hierarchy depth and expanded occurrences before
constructing the candidate. Validate indirect cycles through reused ancestors,
not just the newly placed edge. Geometry edits preserve IDs and component
bindings of unaffected occurrences while invalidating derived connectivity.

## Verification

Before each commit, run formatting, workspace Clippy with warnings denied,
workspace tests and the WebAssembly check. Check licenses whenever dependencies
change. `cargo-deny` was unavailable for the Parts library work, whose dependencies
were unchanged. Preserve Co-Authored-By trailers and record adaptation sources
in ATTRIBUTION.md.

Use independent fixtures for two isolated flat sheets, two uses of one child,
nested reuse and an indirect cycle. Cover failed/stale commands, undo/redo,
exact save/reopen identities, no-op saves, unchanged-document preservation,
current and legacy archives, cross-sheet endpoint rejection and distinct pad
mappings. Connectivity tests prove intended bridges and absence of accidental
bridges. Review changed storage paths through failure/recovery callers before
relying on PR #39's tests for a new document shape.

For visual commits, render the real egui application at one pixel per point,
compare with Next at the same scale and inspect the output. Cover empty and
populated sheets, selected/expanded Browser rows, repeated child occurrences,
creation/deletion dialogs and compact layouts. Restore temporary renderer
changes and retain proof images outside tracked source.
