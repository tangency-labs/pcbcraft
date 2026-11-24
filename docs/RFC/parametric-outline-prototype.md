# Parametric outline prototype

This worktree exercises the PCB-to-SolidCraft direction in the
[parametric board RFC](parametric-board.md#67-the-board-in-a-solidcraft-assembly).
PCBcraft owns the outline. Width and height are inputs to a prototype command
that evaluates an axis-aligned rectangle against board zero and calls the same
revision-checked drawing edit as the editor. The fixture starts at 80 × 50 mm
with a fixed 12 × 8 mm cutout and a declared 1.6 mm thickness. A resize that
crosses the cutout leaves the saved board alone.

## Run the prototype

Build the example using the workspace's shared target directory or the
worktree's local target directory.

```sh
cargo build --locked --profile ci -p pcbcraft-board-solid --example parametric_outline
outline_tool="${CARGO_TARGET_DIR:-target}/ci/examples/parametric_outline"
demo=/absolute/path/to/a/new/demo-directory
mkdir -p "$demo"
"$outline_tool" init "$demo/board.pc"
```

Start the publisher using an absolute path to a working SolidCraft CLI. It
watches the saved archive and writes the revisioned exchange and the linked
design into the demo directory.

```sh
python3 -B scripts/parametric-outline-bridge.py watch \
  "$demo/board.pc" "$demo/board.craftx" \
  --outline-tool "$outline_tool" --solidcraft-cli /absolute/path/to/solidcraft-cli
```

In SolidCraft, use **Insert Design** to place `board.sc` in an assembly.
From another terminal, resize the fixture from the worktree:

```sh
"$outline_tool" resize "$demo/board.pc" '85 mm' '50 mm'
```

The publisher reports revision 2. Use **Update Links** in SolidCraft to see
the width change on the same assembly component. The project can also be
opened in PCBcraft to inspect its native outline. With `publish` in place of
`watch`, the publisher runs once. Omitting `--solidcraft-cli` writes only the
exchange. Stop a running watcher with Ctrl+C.

## Exchange boundary

The prototype adopts the RFC's `.craftx` ZIP, `craft-exchange/1` manifest,
source UUID, revision, millimetre frame and hashed parts. It uses a provisional
manifest layout and a small `board/1` JSON payload alongside the exact STEP
substrate and width and height in `params.json`. The exchange keeps its source
UUID through resizes, and repeating an unchanged publish leaves the files
byte-identical.

The adapter validates the package hashes and uses the real SolidCraft CLI to
create the linked `.sc` beside the exchange. SolidCraft opens that adapter
file through its existing Insert Design and Update Links commands. The
prototype exercises the live file handoff and component refresh without
requiring a native SolidCraft `.craftx` reader. The exchange, adapter file,
and adapter stamp belong in one dedicated output directory with one publisher.

The prototype is restricted to the PCB-driven, rectangular substrate
workflow. Width and height are command inputs, and the saved project stores
their evaluated outline through the existing board format. Expressions,
persisted driving dimensions, face naming, footprint holds, copper updates
and enclosure-driven imports belong to later slices.
