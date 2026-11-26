# UI kit fixes awaiting review upstream

This is `crates/ui-kit` (`solidcraft-ui-kit` 0.49.1) from
[SolidCraft](https://github.com/tangency-labs/solidcraft) at
`e2b7beba91e590270217b99c1b5f89f81bc3a476`, the revision the workspace pins for
every other SolidCraft crate, with three fixes. Each is under review upstream as
its own pull request. When all three are merged, move the pin to a revision that
contains them and delete this directory, its `[patch]` entry and its `exclude`
entry in the workspace `Cargo.toml`.

| Fix | Upstream | PCBcraft issue |
| --- | --- | --- |
| A tree row's trailing badge (`lock`, `pin`) sits between a cut-off name and its count instead of on the count (`src/tree.rs`) | [#22363](https://github.com/tangency-labs/solidcraft/pull/22363), issue [#22361](https://github.com/tangency-labs/solidcraft/issues/22361) | [#622](https://github.com/tangency-labs/pcbcraft/issues/622) |
| A table whose columns share its width fits its scroll area at display scales where 8 points is not a whole pixel, such as 0.9375, so no stray horizontal scrollbar shows (`src/table.rs`) | [#22383](https://github.com/tangency-labs/solidcraft/pull/22383), issue [#22382](https://github.com/tangency-labs/solidcraft/issues/22382) | [#624](https://github.com/tangency-labs/pcbcraft/issues/624) |
| A `Button::Ghost` button has no frame until it is hovered, pressed or focused: its explicit fill and stroke no longer override `frame(false)` (`src/actions.rs`) | [#22396](https://github.com/tangency-labs/solidcraft/pull/22396), issue [#22395](https://github.com/tangency-labs/solidcraft/issues/22395) | [#625](https://github.com/tangency-labs/pcbcraft/issues/625) |

Each fix comes with its upstream regression test (`src/browser_tests.rs`,
`src/table_tests.rs`, `src/actions_tests.rs`). [`fixes.patch`](fixes.patch) is the
exact diff against the revision above; every other file is byte-identical to it.

## Manifest

`Cargo.toml` is standalone: it writes out the version, edition, `rust-version`,
`unsafe_code` lint and dependency versions that the original takes from
SolidCraft's workspace, and takes its SolidCraft dependencies from the same git
revision so the build keeps one copy of each. It drops SolidCraft's Clippy
workspace lints, which do not apply to a dependency.

## Licence

The UI kit is Brand Material under the SolidCraft Brand and Interface Licence
(`LICENSE`, byte-identical to SolidCraft's `LICENSE-BRAND` and to
`crates/chrome/assets/shared-ui/SOLIDCRAFT-UI-LICENSE`), not PCBcraft's MIT OR
Apache-2.0. Carrying this modified copy is covered by the Tangency Labs
authorization recorded in `ATTRIBUTION.md`; the fixes themselves are contributed
back through the SolidCraft repository.

## Re-creating it

```sh
git -C <solidcraft> archive e2b7beba91e590270217b99c1b5f89f81bc3a476 crates/ui-kit | tar -x
cp -r crates/ui-kit/. vendor/solidcraft-ui-kit/
git apply --directory=vendor/solidcraft-ui-kit vendor/solidcraft-ui-kit/fixes.patch
```

Then restore this file and the standalone `Cargo.toml`.
