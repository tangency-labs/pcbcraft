# Composition-guidance rebase

The branch was rebased onto `d564701`, including the project-wide composition
rules from [PR #354](https://github.com/tangency-labs/pcbcraft/pull/354). All ten
commits replayed unchanged with author and co-author credits retained; the old
history is preserved by the backup ref in [source.json](source.json).

The follow-up narrows selected-net resolution and draft capture to explicit
immutable model, selection, source and display inputs. The application shell
assembles those inputs and coordinates commands. Both board and class fields
compose the same bounded dimensional-input adapter. The existing engine resolver,
shared commands, source authority, saved data and UI treatment are unchanged.

All 14 affected Rules/navigation tests pass, along with affected app/chrome
Clippy, formatting, WASM check/package and native app/CLI builds. Earlier full
workspace and functional visual evidence retains its original scope; this
behavior-preserving extraction adds no new full-suite, visual or performance
qualification. [Verification](verification.json) records check boundaries, with
complete logs under `setup/issue303/composition-rebase-review`. No push, PR or
native app relaunch was performed.
