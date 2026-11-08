# PCBCraft: rules for agents

Read [VISION.md](VISION.md) first. PRs are written by agents; keep them small, and say what the user sees.

## Opening issues

Check for an existing issue and read the repository's current label names and
descriptions before opening one. Include at least one classification label
(`bug`, `enhancement`, `documentation` or `question`) and the relevant existing
`area: ...` labels. Documentation-only issues and trackers spanning several areas
can omit area labels when no specific area owns the work. Use `future` only for explicitly
deferred work; `good first issue` and `help wanted` need a deliberate assessment.

Pass labels in the creation request when the tool supports them. Otherwise add
them immediately afterward, then read the issue back to verify they were applied
before reporting creation complete. Preserve existing labels when adding missing
ones. If permissions prevent labeling, report the remaining triage work instead
of claiming the issue is fully classified. This is an agent workflow requirement,
not a GitHub restriction on contributors who cannot apply labels.

## License and sources: the rule that can't be undone

The project is MIT OR Apache-2.0, so code can move freely between the Craft apps.

- Never read, copy or port KiCad source, headers, patches or annotated-source docs, or those of any other GPL EDA tool. This covers `tangency-labs/pcbcraft-gpl` too: its rules allowed reading KiCad source.
- Allowed: public file format specs, and running KiCad as a black box to study how it behaves.
- Symbol, footprint and 3D libraries keep their own licenses. Record the source, revision and license in the PR, and don't embed them by default.
- No GPL, LGPL or AGPL dependencies.
- If you're unsure where something came from, leave it out and ask.

## How it's built

- One command set: anything a person can do, MCP can do through the same internal commands.
- No file surgery: agents never hand-edit project files to get things done.
- Same controls as KiCad. New ideas go on top of that base, not in place of it. Don't copy KiCad's icons or artwork.
- Project files stay friendly to Git: a save with no changes gives zero diff, and moving one part changes only that part's lines.
- Keep the app's chrome easy to move onto the shared Craft UI crate: colours, sizes, menus and dialogs come from `crates/chrome`, not from styling in the app.
- Copying from SolidCraft and the other MIT/Apache Craft apps is fine and encouraged. Record it in `ATTRIBUTION.md`.
- Add or update Rust `///` doc comments for new or changed public APIs and for new or changed functions with branching logic, including private helpers. Describe behavior callers rely on, such as input constraints, errors, or state changes. Review the diff for missing or stale doc comments before opening a PR. CodeRabbit’s [Docstring Coverage check](https://docs.coderabbit.ai/pr-reviews/pre-merge-checks) targets 80% of eligible functions touched by the diff.

## Extension and cross-application boundaries

Build useful features in bounded slices while keeping their model, commands,
providers and presentation separate. Use stable identities and typed, versioned
contracts at extension boundaries; do not make display names, paths, array
positions or the current starter parts into permanent domain assumptions.
Prefer an interface exercised by a current feature over speculative frameworks.
Schematic and PCB editors share the project session and application shell, but
keep rendering, tools and contextual panels in separate modules. The shell owns
host placement and input routing; electrical behavior belongs in the engine.
Neither editor may require the other editor's widgets or camera to exist. A
separate executable is not a substitute for an explicit module/API boundary.
Materials, manufacturing profiles, calculators and library providers must be
able to grow without becoming prerequisites for a basic board workspace.

Address physical boards explicitly within a project (#122), even while the UI
exposes only one. Board-local placements, stackups/materials, rules, fabrication
settings and results belong to that board; view targets and correspondence carry
its ID. Do not infer identity from a name, path, stackup or the first list entry.
Keep component-to-board assignment distinct from sheet organization, and keep
panelization and assembly variants separate from multiple physical boards.
Legacy defaults must be explicit compatibility behavior; reject unknown IDs and
require a target when future projects contain multiple boards. Discuss the
persisted board-list migration before changing the native format.

Read [the multiboard and sheet instance design](docs/multiboard-and-sheet-instances.md)
and its linked instance, storage, assembly/output and net-identity contracts
before work in those areas. They distinguish intended behavior from current
capabilities and define the migration and acceptance cases for each slice.

Treat first-class plugins as an architectural requirement (#100). Contributions
should use explicit host interfaces for commands, tools/panels and providers,
with capability discovery and compatibility checks when exposed to packages.
Plugins inspect snapshots and submit validated, revision-checked transactions
through the same command path as GUI and MCP; they do not directly mutate UI
state or project files. Keep execution/lifecycle policy outside domain models,
and preserve ordinary native results when a plugin is absent. Do not call a
collection of internal modules a shipped plugin API or choose a runtime without
the scoped source, dependency and desktop/browser review.

Keep future SolidCraft links (#70) explicit and independent of either app's
window layout: project/document/object IDs, pinned revisions, units, coordinate
transforms and reviewed updates belong in an adapter contract. PCBcraft owns its
electronics session, native files, history and save lifecycle and must run
without SolidCraft's executable or mechanical application orchestration. Qualified
shared libraries remain reuse candidates under the source rules above. New
integration work must not silently synchronize, replace or transfer ownership
of either application's documents.

## Review failure paths before opening a PR

For changes that introduce or alter blocking I/O, concurrency, or shared state, do a separate review pass after the usual checks. Follow the changed code into its callers and callees, including across crates, and verify the relevant cases:

- Identify what can wait, what else waits behind it, and how timeouts, cancellation, and shutdown release workers or other resources.
- Trace errors back to the owner. Check whether a failed request affects just that request or can stop a shared service or leave state partially changed.
- Walk through a stalled or disconnected peer, a full queue, or two overlapping operations where applicable. Add a focused regression test when existing tests miss a concrete failure path.

In the PR description, name the failure paths reviewed and distinguish the ones exercised by tests from the ones checked by reading the code.
