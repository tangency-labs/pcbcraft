# Persistent net-class assignment evidence

These are saved development checks and a small CPU comparison for #301, collected
on November 14, 2026. Packaging this directory did not rerun the workloads. The
manifest hashes every stored artifact and each uncompressed log in `checks.zip`.
Captured local paths and unpublished-state fields describe the collection time;
this package does not rewrite them to describe later PR publication.

`performance/registry.pc` and `actions.json` are immutable inputs generated through
native shared commands and production encoding: 64 logical terminals in four
explicitly promoted unnamed nets, with two unassigned class definitions. The
baseline ran on #300 head `2fd9e4d`, whose tree is identical to merge base
`3ba1e41`. The temporary runner sources are retained as `.rs.txt` artifacts;
neither is a compiled project example or shipped instrumentation.

The baseline runner's timed loop opens the frozen project, measures one symbol
move or wire deletion, then checks that four identity records remain and that
Undo restores exact encoded documents. Twenty samples are retained per action.
The final runner adds an untimed setup mode assigning both classes to all four
nets; its timed actions and correctness checks remain the same. `probe.rs.txt`
is the baseline source and `probe-final.rs.txt` is the final runner. Build them
as temporary engine examples in disposable checkouts to reproduce the workload;
use release mode, `run INPUT_DIRECTORY OUTPUT_JSON` for the unassigned case and
`bound INPUT_DIRECTORY OUTPUT_JSON` for the assigned case. Reuse the saved inputs
instead of regenerating UUIDs or editing project documents.

| Capture | Median move (ms) | Median disconnect (ms) |
| --- | ---: | ---: |
| Before assignments implementation | 1.197879 | 1.033920 |
| Initial after, unassigned | 1.189474 | 1.027288 |
| Initial after, assigned | 1.192810 | 1.032007 |
| Final pair, unassigned | 1.196572 | 1.032873 |
| Final pair, assigned | 1.200119 | 1.037182 |

All samples, including maxima, remain in their reports. The shared development
host did not have controlled CPU exclusivity, power or thermal conditions, so
these small differences do not establish a speedup or a regression. Initial
after reports describe an intermediate candidate and are retained for audit;
its exact source tree was not archived. The final pair precedes final formatting
and test/documentation-only edits. `verification.json` identifies implementation
commit `7ac9df2` and hashes its source files; the exact pre-format tree used by
the final binary was not separately archived. These results are scoped
observations, not byte-identical release qualification.

This fixture does not measure a naming command, named-net graph rendering,
large-project or tail behavior, memory/allocation, browser interaction, physical
presentation latency or Wayland endurance. It does not claim zero overhead on
all projects. Assignment resolution is explicit; naming alone assigns no class.
No broad replay or endurance session was performed for this slice.

`verification.json` records the passed workspace, Clippy, formatting, native and
browser build, WASI, dependency and harness checks, along with historical
failures. The final workspace run passed 1,831 tests with 11 ignored, and the
selected WASI run passed 79. Twelve harness tests passed in the sandbox; the one
local-loopback capture fixture blocked by that sandbox passed on a host-loopback
rerun. `checks.zip` retains both that failure and its successful narrow rerun,
as well as the obsolete format-message assertion failure and complete successful
workspace rerun. `failure-review.md` distinguishes exercised cases from code
inspection, including candidate publication, retained references, stale results
and interrupted native saves.
