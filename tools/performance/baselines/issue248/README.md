# Via displacement and connected segment dragging (#248)

The four track-only cases run at both revisions on the same saved board. The
second fixture adds a through via and a connected drag to exercise the two new
operations. Each replay starts from its saved project and takes three samples.

## Reproduction

Build the native `board_perf` example in the workspace's `ci` profile with the
`performance-harness` feature and the locked dependencies. Both builds used
rustc 1.100.0 on an Apple M5 Max running macOS 27.2.1. The raw reports include
the compiler commit, target, profile, host and timing samples in milliseconds.

From this directory, set `before_perf` and `after_perf` to the absolute paths
of the binaries built at the two revisions.

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

The original `shove_fixture` command derived the track-only board from the
checked-in channels-v1 fixture. The candidate's `shove_joint_fixture` command
extended that saved board using `pcb.add_via` and `pcb.add_tracks`. Generation
assigns new object IDs. Replay the archived fixtures to reproduce these samples.

## Track-only replay

The single, cascade and branch routes must reach their endpoints and push one,
two and one tracks respectively. The locked obstacle blocks its route. For each
reached case the harness compares the bounded and synchronous plans, validates
changed copper against exhaustive clearance, publishes once and checks exact
Undo/Redo and native reopen. It also checks that canceled planning work retires.

[Raw before samples](tracks-before.json) · [Raw after samples](tracks-after.json)

## New operations

The via case sends a straight route past a through via with a track ending on
each copper side. It checks the planned via identity and verifies the published
center and each follower's layer, width, fixed end and new contact. The drag
case moves a connected pair into a wider rail, checks the pair's shared joint
and compares every previewed rail segment against committed copper, including
the rail's original endpoints. Both edits publish once and pass the exhaustive
changed-copper, native reopen and exact Undo/Redo checks.

Planning and preparation advance the same bounded jobs in batches of 256.
Preparation for the drag includes finding and validating its displaced rail.
Publication consumes an already validated edit. History and exhaustive checks
run outside the timed boundaries. [Raw joint samples and via plan](joints-after.json)

## Scope

The before and after runs shared a development host with other work. Three
samples describe these particular native service calls. The first case in each
iteration also pays for constructing the fresh session's planning geometry.
The runs do not establish a latency budget or a tail distribution. Browser,
window rendering, pointer-to-visible latency, allocation and endurance were
unmeasured. This checkout lacked a runnable browser diagnostic package and a
wasm-bindgen CLI matching its pinned crate version.
