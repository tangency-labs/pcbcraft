# One matched saved-project navigation pair

The unchanged original project hash and geometry match. Eight warmup frames and ten samples per measured operation; native dev-profile CPU layout/tessellation only. This old zone/origin fixture has no closed Board Outline, so the after state correctly reports unavailable fill. This pair covers navigation with unavailable fill, not rendering/material or fill-job latency. Shared-host timing/power conditions are unmeasured; smaller observations do not establish a causal speedup. Stop after this pair.

| Workspace / operation | Before p50 / max ms | After p50 / max ms |
| --- | --- | --- |
| Schematic / click.ui | 5.008 / 5.293 | 3.908 / 3.968 |
| Schematic / idle.ui | 5.229 / 13.735 | 3.922 / 11.692 |
| Schematic / zoom.ui | 5.297 / 5.947 | 3.901 / 3.934 |
| Board / click.ui | 5.592 / 6.100 | 4.751 / 4.846 |
| Board / idle.ui | 5.632 / 12.257 | 4.729 / 9.898 |
| Board / zoom.ui | 6.227 / 7.546 | 4.715 / 4.767 |
