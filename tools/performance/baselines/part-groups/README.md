# Part-group membership history evidence

This bounded Linux engine comparison addresses the retained-Undo memory finding
in [PR #378](https://github.com/tangency-labs/pcbcraft/pull/378). It compares
`703f4ace7e52f807f493f8317045d9d55c2e3493`, immediately before per-instance
sharing, with `7d4f23fe144076c7f500fd0a9df21abf030763fa`, which contains that
fix. The subsequent hierarchy-Cut correction does not change the membership-edit
path measured here.

## Question and stopping condition

Does the shared representation reduce retained process memory for ordinary
membership edits while preserving native state and Undo/Redo? Use one exact
command-generated fixture with 256 part instances, 64 groups and every instance
initially in every group. In one process per revision, alternate one instance
between 63 and 64 groups for 1,000 edits, retaining normal Undo history. Then
verify all 1,000 Undo and Redo steps and canonical native round trips.

Run this before/after pair once. This is a bounded diagnostic, not maximum-capacity
qualification or an independent statistical latency baseline. No larger replay,
allocation profiler, UI capture or endurance run is needed to answer this question.

## Observations

Both runs passed the changed-revision, canonical round-trip, exact Undo and exact
Redo assertions.

| Observation | Before sharing | With sharing |
| --- | ---: | ---: |
| Observed process memory high water | 456.33 MiB | 24.49 MiB |
| Observed peak RSS | 456.33 MiB | 24.34 MiB |
| Total time inside 1,000 membership edits | 458.29 ms | 23.61 ms |
| Edit p50 / p95 | 0.465 / 0.603 ms | 0.023 / 0.026 ms |
| Complete child-process elapsed time | 546.33 ms | 81.20 ms |
| Resource samples, 10 ms polling | 50 | 8 |

These observations support the sharing fix on this workload. They do not establish
a maximum memory bound or a portable speedup. RSS includes startup, fixture
decoding, retained history, codec checks and Undo/Redo; it is not an isolated heap
or membership allocation total. Sampling reports observed peaks and can miss a
short-lived peak. The 1,000 edit samples share one process and fixture, rather
than representing independent process replays.

Both binaries used the same Rust toolchain, release profile (`opt-level="s"`,
thin LTO, 16 codegen units), disabled incremental compilation and eight build jobs
on the same host. Builds and functional test compilation finished before resource
collection. The workspace lockfiles differ, but the engine's normal/build/dev
dependency/version trees are identical; [the normalized tree](engine-dependencies.txt)
records that boundary. This comparison includes the intervening PR revisions and
does not isolate every contribution to the timing difference.

## Reproduction and retained artifacts

[Source metadata](source-metadata.json) records the revisions and source,
fixture, executable and lockfile hashes. [Environment](environment.json) records
the toolchain and build settings. [Comparison](comparison.json) contains exact
summary values. The four compressed before/after resource and timing reports
retain every collected sample and correctness result. Absolute command paths and
the machine-specific platform string were replaced with portable labels; no
sample was removed. Local paths, usernames, host identifiers and hardware details
are not included.

Decompress `fixture.pc.gz` to `fixture.pc` and reuse those exact bytes on both
revisions. It was created with the `generate` mode of
[membership-memory.rs](membership-memory.rs), using shared commands and the native
packed encoder. Regeneration allocates new UUIDs, so it will produce a different
artifact hash; do not mix regenerated inputs within a comparison.

In each isolated checkout of the recorded revisions, temporarily copy the same
runner into `crates/engine/examples/pr378_membership_memory.rs` and build:

```sh
CARGO_BUILD_JOBS=8 CARGO_INCREMENTAL=0 cargo build --offline --locked --release -p pcbcraft-engine --example pr378_membership_memory
```

Retain each resulting executable separately, then remove that temporary example.
The runner is evidence tooling, not a production Cargo target or a new CI test.
The generation command is `pr378_membership_memory generate fixture.pc`; the
retained fixture already exists, so generation is unnecessary for reproduction.

Use the existing resource collector with each executable and an output directory
that does not already exist:

```sh
python3 scripts/collect-performance-resources.py /path/to/before-runner fixture.pc environment.json before --iterations 1000 --timeout 120
python3 scripts/collect-performance-resources.py /path/to/after-runner fixture.pc environment.json after --iterations 1000 --timeout 120
```

For this runner, the collector's iteration argument means retained membership
edits. Timing samples cover the commands, while resource collection covers the
whole process. The identical fixture hash and runner source hash must match
`source-metadata.json`; `SHA256SUMS` covers the retained evidence files.

The supported 4,096-part/256-group maximum, allocator totals, interactive latency,
native Windows/macOS, browser execution and Wayland endurance remain unmeasured
by this comparison. Existing capacity and sharing regressions provide separate
correctness evidence; the 98 MiB history figure in the PR remains a structural
estimate rather than a process-memory measurement.
