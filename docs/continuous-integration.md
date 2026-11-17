# Continuous integration

CI keeps the full native correctness suite on Linux for code changes and selects
Windows integration work by changed paths. Main code pushes, merge groups, the
daily 06:23 UTC schedule and manual runs execute the full validation set. The
same workflow serves draft and ready PRs. The existing `checks` result remains
the final status: it fails when a selected job fails, is cancelled, is missing or
is unexpectedly skipped. A failure in selection or a cheap repository check
also fails that result.

## Job selection

| Change/event | Validation |
| --- | --- |
| Known root Markdown/license files, Markdown under `docs/`, issue templates | Markdown, schemas, Python/Node script tests and CI policy tests; compiler jobs are skipped |
| Known application/shared Rust packages | Full native Linux Clippy/tests and browser build; engine/geometry/units also select WASI |
| Native app, storage, clipboard, library, transport or engine persistence entry points | The above and targeted Windows tests plus the real-window smoke |
| Cargo manifests/lockfile, toolchain, workflows, scripts, vendored code, unknown paths or unavailable Git comparison | All jobs, including the complete Windows suite |
| Main code push, merge group, nightly or manual workflow | All jobs, including complete Windows tests and the opt-in performance harness |

Selection reads NUL-delimited Git paths with renames disabled so both old and
new paths are checked. Invalid/missing revisions select all validation. Unknown
packages, schemas, fixtures and configuration are deliberately conservative.
The policy tests also scan current Windows-conditional Rust sources so adding a
new platform-specific area requires selecting it. These tests run in the cheap
repository job, before any selected target needs to complete.

## Windows coverage

Ordinary relevant PRs run the complete storage, MCP, control, agent, units,
geometry, CLI and chrome package tests. They also run engine persistence,
clipboard and library families, plus app native-file, recent-project, clipboard,
library/part editor, save/close, detached-view, native-keyboard, live MCP and
Assistant process-launch cases. This retains the path alias, hidden lock-file,
verbatim path, checkout line-ending and evidence regressions introduced by
[#392](https://github.com/tangency-labs/pcbcraft/pull/392).

General editor/model cases and feature-enabled performance-harness tests remain
in the full Linux PR suite and the full Windows main/nightly/manual runs. Full
Windows runs also retain both Windows Clippy passes. No Rust test is deleted or
newly ignored. Windows checkouts still use `core.autocrlf=true`; the existing Git
attributes continue to protect recorded evidence bytes. Python and diagnostic
page tests also execute on Windows whenever its job is selected.

The build job stages the existing `native_window_lifecycle` integration-test
executable from Cargo's JSON artifact output. A separate `windows-latest` job
downloads it and runs its one normally opt-in test against real native windows
and the DX12/FXC backend. The smoke checks frame delivery, child/root lifetimes
and cancellation/close behavior, with an external 180-second deadline. A zero-test
run or nonzero exit fails. Only the owned process is terminated on timeout.
This adds an OS-window boundary to the previous headless suite; it is not an
endurance, physical input-latency or full product-startup qualification.

## Build profile and caches

`--profile ci` inherits development behavior, keeps debug assertions and overflow
checks, disables incremental compilation, uses optimization level 1 for workspace
code and level 2 for dependencies, and emits only line-table debug data for the
workspace. Dependencies omit debug data. There is no CI LTO; the shipping release
profile remains separate. Developers can still use the ordinary profiles for a
full debugger session or the release profile for performance measurements.

Linux, Windows, browser and WASI use separate Rust cache keys with a CI-profile
prefix and workspace artifacts included. Only runs on main save those caches;
PRs restore without uploading their own build outputs. New PR pushes cancel
superseded PR work. Main, scheduled and manual runs use separate concurrency
groups, and running jobs are allowed to finish so they can populate the caches. Reruns use their own group
and cannot cancel a newer PR run.

Default runners remain `ubuntu-latest` and `windows-latest`. Repository variables
`CI_RUNNER` and `CI_RUNNER_WINDOWS` can select provisioned build runners, including
Blacksmith labels. Custom Windows build runners must produce x64 MSVC executables
compatible with `windows-latest`. The desktop smoke always uses GitHub's Windows runner because
it needs a presenting desktop session. `CI_BUILD_JOBS` defaults to four and
`CI_BUILD_JOBS_WINDOWS` to two; these can be adjusted for provisioned resources.
No provider credentials, runner subscriptions or branch-protection settings are
changed by this workflow.

## Evidence and scope

The completed [baseline run 44153702095](https://github.com/tangency-labs/pcbcraft/actions/runs/44153702095)
used the previous workflow on `3fb2207`. Its Windows job took 35.4 minutes: 27.1
minutes in the workspace test step and 3.5 minutes saving the cache. App test
execution was 723.43 seconds and engine execution 571.53 seconds; compilation
was part of the broader test step. Linux's combined check job took 20.2 minutes.
These are observations of one hosted run, not controlled cold/warm comparisons
or a predicted saving for the new workflow.

This implementation is separate from the investigation and container/baseline
work in [#399](https://github.com/tangency-labs/pcbcraft/issues/399). It changes
profile settings, selection and scheduling; the first run of the new cache keys
is expected to compile without a populated main cache. Hosted durations and
Windows desktop execution must be assessed from the new workflow's actual run.
Local Linux tests cannot certify the Windows runner. Workflow syntax is checked
with actionlint, while policy tests exercise missing history, renames, unknown
paths, cancelled/skipped work, native-smoke timeout and zero-test refusals.
