# Continuous integration

PRs use staged validation so development pushes do not repeat the complete suite.
Drafts skip CI jobs. Ready code PRs run repository checks, formatting and a quick
native workspace compile. Adding `approved-for-ci` enables Clippy and native tests
for the affected packages and their dependents. A passing selected stage reports
`checks` as successful; missing approval is not an error.

## What runs when

| Stage/event | Work |
| --- | --- |
| Draft PR | No CI jobs |
| Documentation-only PR | Markdown, schemas and Python/Node repository checks |
| Code PR without `approved-for-ci` | Repository checks, formatting and quick native compile |
| Code PR with `approved-for-ci` | Repository checks, formatting, affected-package Clippy and unit/integration tests, plus command-contract comparison |
| PR changing Cargo manifests, Cargo.lock or deny.toml | Dependency license/source/ban checks in addition to its selected stage |
| Main code push, merge group, nightly or manual workflow | Full native workspace suite, doctests and opt-in harness; Windows and macOS tests, browser packaging, WASI and dependency checks |
| Unrelated label change on an approved PR | Repository checks and reuse of an actual approved native pass for the identical head and merge commit; no new native build |

Windows and macOS tests, browser packaging, WASI, doctests and the opt-in performance harness do
not run on ordinary PR events, even after approval. Removing `approved-for-ci`
returns later pushes to quick feedback. Adding it enables affected native tests;
it does not start the broad platform suite. Use Actions → CI → Run workflow on a
branch when a full run is needed before merge.

A green PR result means its selected checks passed, not that every platform was
qualified. PCBcraft currently has no merge queue or branch protection. Full
coverage therefore runs after code lands on main and nightly, unless requested
manually beforehand. If a merge queue is enabled later, the existing merge-group
trigger runs the full suite before its group lands. This policy reduces PR work;
it does not establish a faster full cold build or enforce a GitHub merge rule.

## Package selection and failure handling

Selection reads NUL-delimited Git paths with renames disabled, checking both sides
of a rename. Approved native checks use Cargo metadata to find each source file's
workspace package and transitively include packages that depend on it. Normal,
build, dev, optional and target-specific dependencies are included conservatively;
renamed dependencies retain their original package identity. An app-only source
change selects `pcbcraft-app`, `pcbcraft` and `pcbcraft-web`.

Root/configuration changes, manifests, fixtures, scripts, removed owners or
missing Git history select the entire native workspace. They cannot select an
empty suite. PR tests use Cargo's [`--tests` target selection](https://doc.rust-lang.org/cargo/commands/cargo-test.html#target-selection)
for unit and integration tests; full events retain default target selection,
including doctests, and also run the opt-in performance harness. No Rust test is
deleted or newly ignored.

The aggregate rejects failed, cancelled, missing or unexpectedly skipped jobs and
malformed stage outputs. Unrelated labels cannot turn a quick pass into approved
native coverage. A read-only waiter accepts only an existing successful run with
all approved native jobs successful, the identical head SHA and the identical
checked-out merge SHA recorded in the run title. It waits at most an hour for an
existing run, never starts a build, and fails closed on API errors, revoked
approval, stale revisions or missing results. Approval-label events and new
pushes supersede prior PR work; unrelated labels cannot cancel a real build.

## Full builds, desktop platforms and caches

Pages builds and deploys run inside CI after the aggregate `checks` job succeeds
in full-validation mode. Only public-repository main pushes and manual CI runs
on main can deploy; PR, nightly and documentation-only runs do not publish the
site. The build checks out the exact tested commit, and deployment checks main
again before publishing so a superseded revision is skipped. An API failure
prevents deployment. The separate push-triggered Pages workflow has been removed,
so manual deployment also goes through the full CI workflow.

Both Pages jobs use explicit status conditions so the quick-compile job's
intentional skip during full CI does not suppress publication. Cancellation
still stops publication, and deployment requires both the aggregate checks and
the Pages build to succeed.

Full runs retain the complete Windows workspace suite, both Windows Clippy
passes, Python/Node regressions and the opt-in harness. Windows checkouts keep
`core.autocrlf=true`; Git attributes protect recorded evidence bytes. The build
job stages the existing native-window lifecycle test from Cargo's JSON output.
A separate GitHub Windows desktop runs that normally opt-in test with DX12/FXC
and a 180-second deadline. Zero tests or a nonzero exit fail, and only the owned
process is killed on timeout. This checks frame delivery, window lifetimes and
close cancellation, not endurance or physical input latency.

The macOS job runs the Rust workspace suite, including the desktop launcher and
app title-bar tests. Visual and pointer-interaction checks on a macOS desktop
still require a separate run.

The Cargo `ci` profile retains assertions and overflow checks, disables incremental
builds, and uses reduced debug metadata with optimized dependencies. Shipping
release and ordinary development profiles are unchanged. Platforms have separate
cache keys; only main saves caches and PRs restore them. New keys initially build
cold. The defaults are `ubuntu-latest`, `windows-latest` and `macos-latest`.
`CI_RUNNER`, `CI_RUNNER_WINDOWS` and `CI_RUNNER_MACOS` select provisioned runners.
Custom Windows builds must be x64 MSVC executables compatible with the presenting
`windows-latest` desktop. `CI_BUILD_JOBS` defaults to four on Linux, and
`CI_BUILD_JOBS_WINDOWS` and `CI_BUILD_JOBS_MACOS` default to two.

## Evidence and scope

The previous [baseline run](https://github.com/tangency-labs/pcbcraft/actions/runs/44153702095)
took 35.4 minutes on Windows and 20.2 minutes for Linux checks. The initial staged
[quick compile job](https://github.com/tangency-labs/pcbcraft/actions/runs/44183500650)
passed in 3m28s without a restored cache, while expensive jobs were skipped. Its
old approval-required error is removed by this revision. These observations are
not a controlled full-build comparison; Windows full-build runtime remains an
unresolved cost for the broad suite.

This work is separate from the investigation/container work in
[#399](https://github.com/tangency-labs/pcbcraft/issues/399). Policy tests cover
stages, package consumers, label changes, unknown inputs, strict completion and
exact-revision reuse. Actual full Windows desktop qualification requires a full
hosted run; local Linux results do not establish it.
