# Native project archive corpus

These are byte-for-byte copies of original PCBCraft fixtures and examples from
repository history (MIT OR Apache-2.0). They cover representative pre-release
formats on the supported side of the [native Open compatibility boundary].
They are not a claim of coverage for every schema revision or a shipped release.

| Fixture | Source revision and path | Format in the saved bytes |
| --- | --- | --- |
| `two-resistors-v2.pc.zip` | `94053f214c641098c20c03d057be06b9126cf46d:crates/app/tests/fixtures/split/two-resistors.pc.zip` | Container 2, `project.pc`, project/5, sheet/1, flat board/1 |
| `two-resistors-previous-no-manifest.pc.zip` | `94053f214c641098c20c03d057be06b9126cf46d:crates/app/tests/fixtures/split/two-resistors-no-manifest.pc.zip` | Manifest-less ZIP with a wrapping directory and `project.pc`; project/5, sheet/1, flat board/1 |
| `dual-led-editing-demo-v3.pc` | `21631d1bba751fb5d47709e1f1a0e9ffa3c7dcc3:examples/dual-led.pc` | Container 3, `project.json`, project/6, sheet/1, flat board/5 with footprints |
| `dual-led-flasher-v3.pc` | `716be63e2e45f37861775509ab41cf3ce6b8b9f2:examples/dual-led.pc` | Container 3, `project.json`, project/17, sheet/6, instances/2, folder board/15 with footprints, tracks and vias |

The resistor archives come from the split layout's `storage_fixtures` example.
The two LED demos were built with engine commands by the `indicator_demo`
example. The [Open test] pins the SHA-256 of each archive and checks the
project returned by the native reader against expectations taken from those
bytes. Do not refresh these inputs with a later writer. For each release that adds a format, append one or more small,
original command-built projects saved by that release, collectively covering the
new forms the writer emits. Record the release revision, manifest and document
versions, hashes and expected content. Standalone JSON and container 1 remain
outside normal Open and are tested separately as diagnostic inputs.

`dual-led-editing-demo-connectivity.json` is the expected logical projection of
`dual-led-editing-demo-v3.pc`, not a saved project. It records the terminal
membership from `examples/README.md` at the editing demo's source revision
above, alongside saved part and pin fields, for the [independent reader comparison].
`dual-led-editing-demo-source-counts.json` supplies the synthetic source-side
counts for the same demo: six two-pin parts and one three-pin transistor, with
four connected groups. The transistor's three unwired pins count as pins but
not as nets. This fixture is not a decoded third-party board.

[independent reader comparison]: ../../../../../docs/formats/reading-native-projects.md#worked-example-one-flat-sheet

The [compatibility tests] also check object counts and the exported terminal
inventory, net membership, authored names and unconnected terminals. They
ignore export order and generated names for unnamed nets. Both previous split
archives are also explicitly converted and the resulting copies checked
against the same source reference. Counts come from the frozen schematic and
board records and their pinned footprint definitions, not from a re-save.
The resistor circuit's single wire gives `R1.2`–`R2.1`; the two LED examples'
net references come from `examples/README.md` and `examples/flasher.md` at their
source revisions above, with the flasher's power-symbol terminals from the
contemporaneous `crates/engine/tests/indicator_demo.rs` circuit test. These are
synthetic checks; no imported production board is in this corpus.

[native Open compatibility boundary]: ../../../../../docs/formats/native-project-storage.md#compatibility-and-fixtures
[Open test]: ../../native_open_corpus.rs
[compatibility tests]: ../../native_open_corpus.rs
