# Continuous integration

PRs use staged validation so development pushes do not repeat the complete test
suite. Drafts skip CI jobs. A ready PR runs repository checks and, for code
changes, `cargo check --workspace` and formatting. It skips test execution,
Clippy, Windows, browser packaging and WASI at this stage. Adding the
`approved-for-ci` label starts full qualification; later pushes retain that
approval and run full qualification again. Remove the label to return to quick
feedback while revising a change.

## Qualification and merge readiness

| Stage/event | Work |
| --- | --- |
| Draft PR | No CI jobs |
| Ready PR, documentation only | Markdown, schemas and Python/Node repository checks |
| Ready code PR without `approved-for-ci` | Repository checks, formatting and quick native compile |
| Code PR with `approved-for-ci` | Full Linux and Windows Clippy/tests, performance harness, browser packaging, WASI, dependencies and Windows desktop smoke |
| Main code push, merge group, nightly or manual workflow | Full qualification |
| Unrelated label change on an approved PR | Repository checks and reuse of an actual full pass for the identical PR head and merge commit; no new build |

The quick compile reports its own result. The existing `checks` result represents
merge readiness and remains unsuccessful for an unapproved code PR, with an
explanation to add `approved-for-ci` after review. A quick green compile is not a
full test pass. Documentation-only changes need no approval. Missing Git history,
unknown files, fixtures and configuration require full qualification once approved;
they receive a workspace compile in the earlier feedback stage.

PCBcraft currently has no configured merge queue or branch-protection rules.
Unlike SolidCraft's affected-package approval stage followed by its full merge
queue, approval here runs every platform before merge. The workflow gate does not
create GitHub branch protection: requiring `checks` in a branch rule is a separate
repository setting. Until then, contributors must wait for full `checks` before
merging code; GitHub does not prevent an owner from merging around it.

Selection reads NUL-delimited Git paths with renames disabled, checking both sides
of a rename. The aggregate rejects failed, cancelled, missing or unexpectedly
skipped jobs and malformed stage outputs. Unrelated labels cannot turn skipped
jobs into a qualifying pass. Their read-only waiter accepts only an existing
successful run with every full job successful, the identical head SHA and the
identical checked-out merge SHA recorded in the run title. It waits for an existing
run for at most an hour, never starts another build, and fails closed on API errors,
revoked approval, stale revisions or missing results. An approval-label event or
a new push supersedes prior PR work; other labels cannot cancel a real build.

## Windows and build settings

Full qualification retains the entire Windows workspace suite, both Clippy
passes and the opt-in performance harness. No Rust test is deleted or newly
ignored. Windows checkouts keep `core.autocrlf=true` and existing Git attributes
protect evidence bytes. Python and diagnostic-page regressions run on Windows too.

The build job stages the existing `native_window_lifecycle` test from Cargo's JSON
artifact output. A separate `windows-latest` job runs that normally opt-in test
against native windows with DX12/FXC and a 180-second deadline. Zero tests and
nonzero exits fail. Only the owned process is killed on timeout. This tests frame
delivery, child/root lifetimes and close cancellation; it does not qualify
endurance, physical input latency or full product startup.

`--profile ci` retains assertions and overflow checks, disables incremental builds,
uses optimization level 1 and line-table debug data for workspace code, and level
2 without debug data for dependencies. Shipping release and ordinary development
profiles are unchanged. Linux, Windows, browser and WASI have separate cache keys;
only main saves caches, while PRs restore them. New keys initially compile cold.

Default runners remain `ubuntu-latest` and `windows-latest`. `CI_RUNNER` and
`CI_RUNNER_WINDOWS` can select provisioned build runners; custom Windows builds
must produce x64 MSVC executables compatible with `windows-latest`. Desktop smoke
always uses GitHub's presenting Windows desktop. `CI_BUILD_JOBS` defaults to four,
and `CI_BUILD_JOBS_WINDOWS` to two. Runner subscriptions and credentials are not
changed here.

## Evidence and scope

The previous [baseline run](https://github.com/tangency-labs/pcbcraft/actions/runs/44153702095)
on `3fb2207` took 35.4 minutes on Windows, including 27.1 minutes in workspace tests
and 3.5 minutes saving the cache. Its Linux job took 20.2 minutes. These are single
hosted observations, not a controlled comparison or a promised improvement.

This PR reduces how often expensive validation runs; a faster full cold build has
not been established. It is separate from the investigation/container work in
[#399](https://github.com/tangency-labs/pcbcraft/issues/399). Policy tests exercise
stages, label transitions, missing history, renames, unknown fixtures, strict
completion and qualification reuse. Local Linux tests cannot certify Windows
execution; that boundary requires an actual full hosted run.
