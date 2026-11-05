# PCBCraft: rules for agents

Read [VISION.md](VISION.md) first. PRs are written by agents; keep them small, and say what the user sees.

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

## Review failure paths before opening a PR

For changes that introduce or alter blocking I/O, concurrency, or shared state, do a separate review pass after the usual checks. Follow the changed code into its callers and callees, including across crates, and verify the relevant cases:

- Identify what can wait, what else waits behind it, and how timeouts, cancellation, and shutdown release workers or other resources.
- Trace errors back to the owner. Check whether a failed request affects just that request or can stop a shared service or leave state partially changed.
- Walk through a stalled or disconnected peer, a full queue, or two overlapping operations where applicable. Add a focused regression test when existing tests miss a concrete failure path.

In the PR description, name the failure paths reviewed and distinguish the ones exercised by tests from the ones checked by reading the code.
