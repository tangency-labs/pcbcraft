# Desktop builds and releases

Run **Desktop preview packages** from GitHub Actions once the workflow is on
the default branch. It builds the selected revision on three native runners in
parallel and uploads temporary workflow artifacts. The nightly and release
jobs reuse this build.

| Target | Package |
| --- | --- |
| Linux x86-64 | Tarball with `pcbcraft` and `pcbcraft-cli`, built on Ubuntu 22.04 |
| Windows x64 | Portable ZIP with `pcbcraft.exe` and `pcbcraft-cli.exe`, built with MSVC |
| macOS Apple silicon | DMG with `PCBCraft.app` and its bundled CLI |

Each download group includes `SHA256SUMS.txt`. Packages include the build
revision and the project and dependency notices. The packaging check runs the
CLI and opens the checkout's Sensor Node fixture in a read-only session. macOS
reuses the existing packaging script, which checks the bundle signature. The
preview workflow uses ad-hoc signing on macOS. A broadly distributed macOS
release needs Developer ID signing and notarization. Windows preview binaries
are unsigned.

After moving `PCBCraft.app` to Applications, you can run the macOS CLI directly
from the app bundle.

```sh
/Applications/PCBCraft.app/Contents/MacOS/pcbcraft-cli --version
```

## Nightlies

After full CI passes on `main`, the same CI run builds desktop packages from
the commit that passed. Main pushes, the scheduled CI run and manual full
runs on `main` can all produce these nightlies. Download the
`pcbcraft-nightly-<commit>-<platform>` artifacts from the CI run within 14 days.
The app bar shows the version and Git's abbreviated commit hash, for example
`v0.1.0 nightly (a1b2c3d)`. Manual preview artifacts are also available for
selected revisions.

When all three platforms finish and the commit is still at the tip of
`main`, CI updates the rolling [Nightly prerelease](https://github.com/tangency-labs/pcbcraft/releases/tag/nightly).
Its notes link to the tested commit's CI run, the three packages and their
combined checksums. The page stays at the same address as the `nightly` tag
moves forward. Nightly is marked as a prerelease so GitHub's Latest release
continues to point to a stable version.

## Prepare a release

1. Set `[workspace.package].version` in the root `Cargo.toml` if the release
   needs a new version. Run `cargo update --workspace` and merge the manifest
   and lockfile changes into `main`.
2. Confirm that full CI passed on the current `main` commit. If needed, run
   **CI** manually on `main` and wait for it to finish.
3. In GitHub, open **Actions**, select **Prepare release**, and click
   **Run workflow** with `main` selected. Compare the commit in the run title
   with the one that passed CI. The workflow builds Linux x86-64, Windows x64,
   and macOS Apple silicon from that commit in parallel.
4. After the workflow succeeds, open the draft under **Releases**. Verify and
   [qualify the downloads](#qualify-the-downloads), review the release notes,
   and publish the draft when ready.

The workflow reads the version from the selected commit. After it verifies the
packages, it creates the corresponding tag, such as `v0.1.0`, on that commit
and attaches all three packages and `SHA256SUMS.txt` to the draft. A prerelease
version such as `0.1.0-rc.1` creates a prerelease draft. You can rerun a failed
attempt from its Actions run. An existing tag must point to the same commit,
and only a draft's assets can be refreshed.

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
open a local project offline. You can use `examples/sensor-node.pc` from the
source checkout. Save an edit
to a separate path and reopen it. Make another edit and confirm that closing
with unsaved changes prompts before discarding them.

Record the supported platform, build revision and qualification results before
publishing a draft GitHub release or linking public downloads. Track preview
acceptance under [#91](https://github.com/tangency-labs/pcbcraft/issues/91).
