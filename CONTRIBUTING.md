# Contributing to PCBCraft

Start with [VISION.md](VISION.md), [AGENTS.md](AGENTS.md),
[ATTRIBUTION.md](ATTRIBUTION.md) and [deny.toml](deny.toml). They define the
product, source boundaries, architecture and dependency policy. The
[MVP milestone](https://github.com/tangency-labs/pcbcraft/milestone/1) and
[acceptance checklist](https://github.com/tangency-labs/pcbcraft/issues/96)
identify the first shareable release; the
[broader workflow index](https://github.com/tangency-labs/pcbcraft/issues/89)
includes later features that are not MVP gates.

## Build and find your way around

Use Rust 1.95 or later with Cargo, rustfmt and Clippy, a platform linker and a
graphical session for the desktop app. These commands run from the repository
root unless noted:

```sh
rustup component add rustfmt clippy
cargo build --locked -p pcbcraft -p pcbcraft-cli
cargo run --locked -p pcbcraft
```

For the browser, install the WebAssembly target and Trunk. CI currently uses
Trunk 0.21.14; keep its version consistent with
[the workflow](.github/workflows/ci.yml) when updating these instructions.

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked trunk --version 0.21.14
cd apps/pcbcraft-web
trunk serve --locked
```

Visit `http://127.0.0.1:8781`. `trunk build --release` from the same directory
produces `dist/web` at the repository root. Return to the repository root for
the remaining commands. A successful Rust WebAssembly check
alone does not verify the packaged web assets or browser interaction. Use the
[quickstart](docs/quickstart.md) to exercise the real app with an existing fixture.

If Trunk reports `invalid value '1' for '--no-color'`, the shell has supplied
`NO_COLOR=1` where Trunk 0.21.14 expects a boolean. Launch it with
`NO_COLOR=true trunk serve --locked` in that environment.

| Area | Responsibility |
| --- | --- |
| `crates/engine` | Project model, validation, commands, immutable snapshots and undo/redo; no UI |
| `crates/app` | Desktop/browser composition, canvas tools, preferences and project-file lifecycle |
| `crates/chrome` | Shared colors, sizes, icons, menus, cards and other presentation primitives |
| `crates/mcp` | Tool discovery and dispatch over the engine; a restricted live adapter and a separate headless session |
| `crates/craft-control` | Bounded framing and authenticated local transport |
| `crates/storage` | Native folder/archive I/O and publication; encoding/decoding is in the engine |
| `apps/` | Thin desktop, CLI and web launchers |
| `libraries/starter` | Original, revisioned symbol definitions embedded in both builds |

An authored edit belongs in the engine and is called by the GUI through the same
command path MCP uses. Adding a catalog command exposes it to headless discovery;
the live adapter has an explicit allowlist, so a new command is not automatically
available in an open window. View-only actions must leave electrical revision,
dirty state, saved bytes and document undo unchanged. The
[board workspace plan](docs/board-workspace-plan.md) describes how these ownership
boundaries extend to multiple views.

Use the [native storage contract](docs/native-project-storage.md) before changing
saved data and the [symbol format guide](docs/symbol-library.md) before changing
library definitions. Preserve stable IDs/order, no-op saves and migrations from
every supported old format. Generate fixtures through shared commands and the
production encoder instead of hand-editing project documents. To generate an
original fixture set, use a destination that does not already exist:

```sh
cargo run --locked -p pcbcraft-engine --example storage_fixtures -- /tmp/pcbcraft-fixtures-new
```

## Choose a bounded change

Check the issue discussion, [open PRs](https://github.com/tangency-labs/pcbcraft/pulls)
and any shared local worktrees before starting. A branch name or uncommitted work
may reveal an active task even when no PR exists. Use your own topic branch or
worktree from current `origin/main`; preserve other people's working files. Keep
a broad parent issue open when delivering only one of its slices.

When opening an issue, follow the [label policy](AGENTS.md#opening-issues): use
an existing classification label and relevant area labels, then verify the result.
With the CLI, pass labels through `gh issue create --label`; inspect the current
catalog with `gh label list`. Contributors without label permissions can describe
the affected area in the issue so a maintainer can classify it.

Production Rust must not use `unsafe`, `unwrap`, `expect`, `panic!` or placeholder
branches. Add `///` documentation for public APIs and changed functions with
branching logic, explaining input constraints, errors and state changes. Put
styling in chrome and keep filesystem/transport work away from the editor loop.
For shared state, I/O or concurrency changes, follow AGENTS.md's separate
failure-path review after ordinary checks, including cancellation, stale work,
shutdown and partial failures.

The project is MIT OR Apache-2.0. Do not read or adapt KiCad or other GPL EDA
implementation code, headers, patches, tests, source-derived fixtures or
annotated-source pages, including forks and mirrors. Published manuals/format
specifications and black-box observation are permitted. Qualify libraries and
assets separately, retain notices, and record permitted adaptations with source,
revision, license and changes in ATTRIBUTION.md. Unknown provenance stays out
pending clarification; this contribution guide does not lift existing source
exposure pauses.

## Verify and submit

The [required CodeRabbit review gate](docs/review-gate.md) documents review
enforcement, retries after a rate limit or failed review, and explicit maintainer
overrides with a recorded reason. Its regression tests run in CI's `checks` job:

```sh
python3 -B -m unittest discover -s scripts -p 'test_coderabbit_review_gate.py'
```

Run the workspace checks before a commit, including for documentation changes:

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo check --locked -p pcbcraft-web --target wasm32-unknown-unknown
```

The test suite includes loopback transport tests, so a sandbox that forbids local
sockets cannot run the complete suite. Report that limitation rather than
silently skipping those tests. Match CI's dependency check when cargo-deny is
available:

```sh
cargo install --locked cargo-deny --version 0.20.2
cargo deny --locked check licenses sources bans
```

If the tool is unavailable, say so in the PR and state whether dependencies
changed; CI still runs its license/source/ban checks. The current allowed licenses
are defined in deny.toml, not by an assumption that a crate is generally reusable.

UI tests in `crates/app/src/tests.rs` drive the real app with egui_kittest clicks
and keys through its accessibility tree. Add a focused regression when it covers
a meaningful interaction or failure path. For a visual change, also render and
inspect the actual app at relevant sizes/themes. When following PCBcraft Next,
read the qualified reference code and compare screenshots at the same scale.
Temporary rendering modules, dependency features and generated screenshots must
not accidentally enter the implementation diff. Native window behavior needs
native-window verification in addition to headless UI tests.

Update the quickstart and [capability table](docs/capabilities.md) when user-visible
behavior changes. Verify documentation commands and local links, and distinguish
an automated smoke test from an independent person's onboarding run. Describe
the concrete behavior, issue scope, tests, visual evidence and remaining limits
in the PR. For the failure-path review, separate cases exercised by tests from
those checked by reading the code. Keep the PR small enough to assess on its own;
the repository's rules do not grant an agent permission to publish or merge
without the task owner's authorization.
