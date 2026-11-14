# Sensor Node navigation

## Initial navigation work

@travle1 reported that zooming, clicking and other interactions were slow in a
project with 77 components, 81 placed schematic units across four sheets and
39 board footprints. The running desktop executable used the debug profile.
The supplied project and its libraries remain local; the reports identify its
production-encoded bytes without redistributing design or library documents.

The release comparison replays the same snapshot, viewport and input trace with
eight warmup frames and ten samples for each action in the first schematic
sheet and the PCB editor. Zoom and click handling are checked, and the authored
archive must remain byte-identical. These are targeted CPU service-cost samples,
not tail-latency, compositor or input-to-photon measurements.

| UI boundary, median ms | Before | Candidate |
| --- | ---: | ---: |
| Schematic click | 4.990 | 0.787 |
| Schematic zoom | 4.997 | 0.818 |
| PCB click | 10.948 | 1.986 |
| PCB zoom | 11.310 | 2.265 |

The initial Callgrind investigation found repeated technical-geometry generation
in editor synchronization that needed only footprint IDs. Counting placements
and filtering an empty selection also constructed geometry. The implementation
provides identity-only enumeration and direct placement lookup, reuses technical
paths for the same immutable board/component inputs, and resolves layer colors
once per canvas pass instead of for each path. No geometry algorithms, authored
commands, file formats or shared Craft drawing primitives change.

The separate native reference replay uses one iteration on an isolated Xvnc
display with Vulkan lavapipe, providing 44 GPU readbacks and one real MCP edit
on each side. The comparator accepted the fixture, renderer, viewport and
window-state sequences; final screenshot hashes match. This is rendering and
integration evidence on the canonical fixture, not native latency qualification
of the supplied project. The Xvnc attempt to disable DRI3 was rejected, which is
recorded in the environment; the software Vulkan driver was explicitly selected.

Whole-process maximum RSS was 36,456 KiB before and 35,756 KiB after in one CPU
run each. Those observations are resource context and do not establish a memory
or allocation improvement. Concurrent development, power and thermal conditions
were uncontrolled. Browser runtime, physical presentation and Wayland endurance
remain unmeasured; the WebAssembly build is a compile check.

After the measured candidate, concurrency review changed cold-cache reads to
fall back on contention and generate paths outside slot initialization, avoiding
a wait on another worker's path generation. Warm-hit results and path output
are unchanged. `source-identity.json` identifies the measured binaries and the
later desktop test build rather than claiming they are identical executables.

The graphics cache holds slots for one current board/component allocation pair,
shared across history rather than retained separately per revision. Weak keys
do not retain old model allocations. Replaced slots remain independent for any
in-flight reader. Tests cover direct lookup, edits, physical settings, deletion,
Undo/Redo, unpublished candidates, stale slots, poisoned locks and contended
readers. Palette tests compare the existing resolver with per-pass colors under
both themes, personal/default colors, active/hidden layers and copper bindings.

The local evidence directory also retains exploratory debug reports, Callgrind
data, build/check logs, source fingerprints and immutable baseline/test binaries.
The short measurement pair answered the repeated-navigation-cost question;
no repeated statistical or endurance qualification was performed.

## Continuous zoom follow-up

Manual testing found that the initial change improved ordinary interaction but
left zoom sluggish on the three populated child sheets and PCB layout. The
first navigation trace covered the overview sheet and alternated two zoom sizes,
which failed to expose the repeated cold font work. The additional `--zoom-all`
trace therefore covers every populated sheet, including the overview, at
2560 × 1600, with thirty new zoom-in scales and thirty zoom-out scales.

Debug UI medians before the text change were about 34 ms on Power, 94 ms on
Microcontroller, 72 ms on Sensors and 92 ms on PCB. Stationary frames generated
almost no texture deltas, while zoom repeatedly uploaded fresh glyph data.
Reusing the installed fonts at stable raster sizes and continuously transforming
their text meshes removed most of that repeated work. The final debug replay
medians were about 5, 7, 6 and 10 ms respectively. Some cold glyph/raster changes
remain; the largest observed final debug UI sample was about 32 ms on
Microcontroller. This does not establish a strict frame-latency budget.

The shared canvas adapter retains continuous text size and visual bounds, rather
than snapping displayed labels to the raster size. It applies to schematic
fields, native pin/body lettering, net labels and PCB pad annotations; interface
text is unchanged. Tests exercise warmed glyph reuse at 1×/2× DPI, multiline and
rotated bounds, and physical text sizing. Eight actual GPU captures of Sensors
and PCB layout at Fit and a closer zoom in both themes were inspected locally.
The user project and these project-specific images are not included here.

The initial native reference pair above predates the canvas text change and its
identical final-pixel result applies only to the initial navigation work. The
later text adapter changes raster sampling, so its validation uses continuous
size/bounds regressions and new GPU captures rather than claiming identical
pixel output to the old font rasters.
