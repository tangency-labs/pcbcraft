# Via displacement and connected segment dragging (#248)

Across three native iterations per revision, the four existing shove cases
kept their outcomes and probe counts on the saved board. On a second fixture, a through via moved 325,002 nm
with its two attached tracks, and a connected drag pushed a neighboring rail.
The harness checked the published copper, exact Undo/Redo and native reopen.

## Track-only comparison

Both binaries replayed the same [saved track fixture](tracks-fixture.json.gz).
Median service times below are in milliseconds over three iterations per case.
The first case in each iteration includes preparing a fresh session's planning
geometry. Validation and publication apply to reached routes.

| Case | Outcome | Probes | Plan before → after | Validate before → after | Publish before → after |
| --- | --- | ---: | ---: | ---: | ---: |
| Single | Reached, one track pushed | 65 | 14.227 → 14.959 | 15.842 → 16.676 | 0.280 → 0.280 |
| Cascade | Reached, two tracks pushed | 140 | 0.685 → 0.767 | 15.569 → 16.664 | 0.244 → 0.262 |
| Branch | Reached, one track pushed | 67 | 0.512 → 0.543 | 15.156 → 16.245 | 0.253 → 0.271 |
| Lock | Blocked, zero tracks pushed | 89 | 0.489 → 0.581 | — | — |

The validation medians increased by 0.83–1.10 ms. Single-case planning
ranged from 14.22 to 16.91 ms before and 13.53 to 20.55 ms after. The shared
host and three samples limit attribution of these differences. Both revisions
passed synchronous/batched plan equality, deterministic replay, changed-copper
clearance, exact history and reopen, and the queue cancellation checks.
[Raw before](tracks-before.json) · [Raw after](tracks-after.json)

## Via and connected drag

The [joint fixture](joints-fixture.json.gz) adds a through via with a track
ending on each side, and a connected pair beside a wider rail. Each operation
opens a fresh session. The via case reaches its target on a straight path in
seven probes. Both followers keep their IDs, layers, widths and fixed ends and
meet the new via center. The drag case checks the selected pair's shared
endpoint and matches the full previewed rail path against committed copper,
including both original rail endpoints.

| Candidate operation | Sample 1 (ms) | Sample 2 (ms) | Sample 3 (ms) | Median (ms) |
| --- | ---: | ---: | ---: | ---: |
| Via plan | 17.570 | 14.559 | 14.799 | 14.799 |
| Via prepare | 4.076 | 3.853 | 4.281 | 4.076 |
| Via publish | 0.275 | 0.287 | 0.264 | 0.275 |
| Drag prepare | 29.580 | 28.497 | 28.550 | 28.550 |
| Drag publish | 0.281 | 0.269 | 0.282 | 0.281 |

Planning and preparation request batches of 256 work units. Drag preparation
includes finding and validating the displaced rail. Publication consumes an
already validated edit. Exhaustive clearance, exact Undo/Redo and native
reopen run outside these timings. Each gesture published one revision.
[Raw joint report and via plan](joints-after.json)

## Reproduction

Both binaries used the locked dependencies, Rust 1.100.0 and the workspace
`ci` profile on an Apple M5 Max with 128 GiB running macOS 27.2.1.
Power, thermal state and concurrent host activity were uncontrolled.

| Build | Commit | SHA-256 of native `board_perf` |
| --- | --- | --- |
| Upstream main | `b76694d757e9d3b2df7c838219bab02186e50814` | `1bd6549e39182737e0e909da7845c78c0f243af79e3731bca192afe7aeb5696d` |
| Candidate | `d017f16914363bf6efde66b0b679f7d475c45d77` | `fd7b1d7a764babff4174a0f091dfea6cc441496e5edd4d6a1d7bd7a5741b09f9` |

| Fixture | Uncompressed artifact SHA-256 | Embedded native project SHA-256 |
| --- | --- | --- |
| Track | `83608fa0220b318af2b890a75add337a979a35cd66cf35854fd1c77354a2fedc` | `1b0cb64e2f1897aab0aac24361f1b3fd7dcdf62627aea2d239953c7740d0878b` |
| Joint | `a11a4d5689872b4840871fbcb0f120c336302e8d4204f60f7ecc557f4a9d1283` | `a502c54a2db3efae1976fab80a33b50beb8edf185a4083bfb963624e5ccc2c07` |

Build the `board_perf` example of `pcbcraft-app` at each commit with
`--locked --profile ci --features performance-harness`. Set `before_perf`
and `after_perf` to the resulting binary paths and run in this directory.
The commands use recorded environment metadata for the original host. Update
the fields to match the machine, compiler and profile used for a new measurement.

```sh
out=$(mktemp -d)
gzip -dc tracks-fixture.json.gz > "$out/tracks.json"
gzip -dc joints-fixture.json.gz > "$out/joints.json"
jq .environment tracks-before.json > "$out/before-env.json"
jq .environment tracks-after.json > "$out/after-env.json"
jq .environment joints-after.json > "$out/joints-env.json"
"$before_perf" shove "$out/tracks.json" "$out/before-env.json" 3 "$out/before.json"
"$after_perf" shove "$out/tracks.json" "$out/after-env.json" 3 "$out/after.json"
"$after_perf" shove "$out/joints.json" "$out/joints-env.json" 3 "$out/joints-after.json"
```

The `shove_fixture` command generated the track fixture from the saved
`channels-v1` input. The candidate's `shove_joint_fixture` command extended
the same board using `pcb.add_via` and `pcb.add_tracks`. These commands
allocate fresh IDs, so replay the archived bytes for matched comparisons.

## Unmeasured boundaries

The three samples describe native CPU service costs. Physical pointer-to-visible
latency, rendering, allocation and endurance remain unmeasured. Browser replay
also remains unmeasured. This checkout lacked a runnable browser diagnostic
package and a wasm-bindgen CLI matching the pinned crate.
