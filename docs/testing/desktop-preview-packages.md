# Desktop preview packages

Run **Desktop preview packages** from GitHub Actions once the workflow is on
the default branch. It builds the selected revision on three native runners in
parallel and uploads temporary workflow artifacts. Ordinary PR and main CI
keep their existing jobs. The workflow uses read-only repository permissions.

| Target | Package |
| --- | --- |
| Linux x86-64 | Tarball with `pcbcraft` and `pcbcraft-cli`, built on Ubuntu 22.04 |
| Windows x64 | Portable ZIP with `pcbcraft.exe` and `pcbcraft-cli.exe`, built with MSVC |
| macOS Apple silicon | DMG with `PCBCraft.app` and a separate CLI ZIP |

Each download group includes `SHA256SUMS.txt`. Packages include the Sensor Node
example, the build revision, and the project and dependency notices. The packaged CLI checks its
version and opens the example in a read-only session during the build. macOS
reuses the existing packaging script, which checks the bundle signature. The
preview workflow uses ad-hoc signing on macOS. A broadly distributed macOS
release needs Developer ID signing and notarization. Windows preview binaries
are unsigned.

## Build locally

Run on the target platform with Rust, Cargo, and Python 3.11 or newer. macOS
also needs the Xcode command-line tools. Set the macOS deployment target to
match [the bundle packaging script](../../packaging/macos/package.sh) before
building on macOS.

```sh
cargo build --locked --release -p pcbcraft -p pcbcraft-cli
python3 -B scripts/package-desktop.py linux-x86_64 --bin-dir target/release --output dist/preview
```

Use `windows-x64` or `macos-aarch64` for those native platforms. On Windows,
use `python` if the interpreter is installed under that name. If Cargo uses a
different target directory, point `--bin-dir` at its release directory. Choose
a new output directory for each run.

## Qualify the downloads

On each target, verify the checksum, then extract the ZIP or tarball or mount
the DMG. Launch the editor from the extracted package or installed app and
open the included `sensor-node.pc` offline. On macOS, the example is in the
app bundle under `Contents/Resources/Licenses` and in the CLI ZIP. Save an edit
to a separate path and reopen it. Make another edit and confirm that closing
with unsaved changes prompts before discarding them.

Record the supported platform, build revision and qualification results before
creating a draft GitHub release or linking public downloads. Track preview
acceptance under [#91](https://github.com/tangency-labs/pcbcraft/issues/91).
