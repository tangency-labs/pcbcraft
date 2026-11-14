# PR #349 review fixes

[PR #349](https://github.com/tangency-labs/pcbcraft/pull/349) received three
CodeRabbit findings at `1f30e82`: explicit ribbon sizes were replaced at pickup,
a partial explicit via pair could not repair otherwise unambiguous preferences,
and a benchmark assertion could accept a matching pre-existing track.

Ribbon selection now retains independent explicit width and via operands. Class
and subsequent unclassified pickup preserve those choices; board/default-rule
replacement resets them, while hard-invalid choices are refused without changing
the project. Real X/click and standalone-via tests exercise all combinations of
width/via overrides, later choices after class pickup, hard rejection and reset.
The shared resolver exposes unambiguous pre-relationship preferences for a single
explicit via counterpart; final bounds/ring validation still applies, and genuine
equal-precedence ambiguity still refuses. Native command/codec regressions cover
both repaired operands and unchanged rejection authority.

The benchmark now verifies the added identity, count, geometry, layer and width.
Its regression includes a matching old track with an incorrect, misplaced or
absent new track. All three regressions fail under their counterfactual old
behavior and pass with the fixes. Raw failed and corrected logs are in `checks.zip`.

The final workspace run passed 1,846 tests, with 11 ignored. Workspace and feature
Clippy, formatting, 15 harness tests, WASI board/resolver tests, browser compile
and native builds passed. One existing harness capture case initially lacked
local-loopback permission; its authorized affected rerun passed. The initial
test-only Clippy correction and restricted-run failure remain in the logs.

A single debug correctness replay of the exact retained assigned artifact passes
the stronger oracle. Its diagnostic samples are not used for timing comparisons.
Earlier release measurements and GPU images retain their recorded source/scope;
this review fix adds no new performance, presentation or endurance qualification.
The UI controls and rendering implementation are unchanged. Source fingerprints,
check commands and resolution metadata are retained beside this record.
