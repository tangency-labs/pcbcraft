# One closed-board navigation pair

One native dev-profile CPU navigation run per state on the same closed-edge three-resistor demo; eight warmup frames and ten samples per operation. Both pass geometry, input and unchanged-native-byte assertions and after fill is ready. The before is 89c1ac0; after is the uncommitted pour/remove implementation before the final completion-repaint fix, with its source hashes retained. This steady warm navigation trace has no active job handoff and does not exercise that final fix. Shared-host timings/power/thermal state are uncontrolled; this is not evidence of a causal speedup, input/presentation latency or refill throughput. The earlier parallel builds stalled and were reduced; no further broad timing replay was run.

| Workspace / operation | Before p50 / max ms | After p50 / max ms |
| --- | --- | --- |
| Schematic / click.ui | 4.972 / 5.797 | 3.992 / 4.175 |
| Schematic / idle.ui | 4.797 / 13.681 | 4.044 / 11.200 |
| Schematic / zoom.ui | 4.476 / 4.812 | 4.029 / 4.092 |
| Board / click.ui | 6.189 / 7.042 | 4.895 / 5.821 |
| Board / idle.ui | 6.537 / 12.551 | 4.853 / 10.660 |
| Board / zoom.ui | 6.738 / 9.665 | 5.173 / 6.993 |
