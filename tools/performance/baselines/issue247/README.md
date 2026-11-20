# Shove solver evidence (#247)

The implementation is `83bdf997`, based on `52601864`. It adds the engine shove solver,
the read-only `pcb.plan_shove` query and the `pcb.shove_tracks` edit described in
[shove](../../../../docs/route-shove.md). The route tool is unchanged, so no rendered
replay or presentation measurement applies to this slice.

## Inputs and environment

The saved command-generated fixtures were reused unchanged:

| Fixture | Uncompressed artifact SHA-256 |
| --- | --- |
| `channels-v1` | `2bcba0154913f8e396b975f470040edc649342e4ed40bbe77af107c5ae2f0efa` |
| `dense-v1` | `df1731d2e45075cf29c595d9e8c997bb284bbcc51e144ae99d49b0a25eec55da` |

`board_perf` was built with rustc 1.100.0 in the release profile on an Apple M5 Max
(macOS 27.2.1, AC power) while other builds ran on the host; see the
[environment](environment.json). Its SHA-256 is
`ca56f22fbc5236cdb0c1b116010517c56facb486d2efc29f6daadc7a70315516`.

```sh
cargo build --locked --release -p pcbcraft-app --features performance-harness --example board_perf
board_perf shove FIXTURE environment.json 100 shove.json
```

## Existing boundaries

No existing measured path changed. The clearance overlay gained a membership probe
that only the shove uses, the walkaround's width selection moved into a shared helper
on the command path, and the hull gained functions the walkaround does not call. No
before/after replay was recorded for this reason.

## Shove diagnostic

The new `shove` diagnostic has no before counterpart. Over 100 iterations it plans a
straight two-point shove along each walkaround case on one captured query, requires
identical plans on every iteration and, on the first iteration, commits each reached
plan through `pcb.shove_tracks` against the exhaustive pair reference. Raw reports:
[channels-v1](shove-channels-v1.json.gz), [dense-v1](shove-dense-v1.json.gz).

| Fixture | Reached | Blocked (prefix by bisection) | Most probes | All plans p50 / p95 |
| --- | ---: | ---: | ---: | ---: |
| channels-v1 | 5 | 3 | 39 | 0.762 / 26.501 ms |
| dense-v1 | 6 | 2 | 39 | 1.282 / 36.442 ms |

No case in these fixtures pushes copper: every straight line either runs clear or
meets a pad or a track that cannot move (it ends inside the pusher's hull or is joined
in mid-span), so the reports measure clear shoves and blocked shoves with their prefix
bisection, not propagation. Pushing and propagation are exercised by the engine's
command-built fixtures, which are not timed. The p95 values were taken under other
build load on the host and support no latency claim; all-plan p50 values are below 1.3 ms.

These are solver service costs on one host. Route-tool preview and commit latency,
the browser runtime, allocations and presentation remain **unmeasured**.
