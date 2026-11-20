# shellcheck shell=bash
# Shared setup for the packaging scripts. Source it: `. "$(dirname "$0")/../env.sh"`.
# Adapted from MailCraft and FieldCraft packaging (MIT OR Apache-2.0).
#
# Exports:
#   ROOT                    workspace root
#   VERSION                 [workspace.package] version from Cargo.toml (override: PCBCRAFT_VERSION)
#   DIST                    output directory for release artifacts (default: $ROOT/dist/release)
#   PCBCRAFT_BUILD_SHA      git commit recorded in the macOS Info.plist
#   PCBCRAFT_BUILD_DATE     UTC build date, YYYY-MM-DD
#   CARGO_TARGET_DIR        cargo's target dir (default: $ROOT/target)
#   APP_ID                  reverse-DNS macOS bundle identifier

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT

# Read [workspace.package] version from the root Cargo.toml.
workspace_version() {
  awk '
    /^\[/ { in_pkg = ($0 == "[workspace.package]") ; next }
    in_pkg && $1 == "version" { gsub(/[" ]/, "", $3); print $3; exit }
  ' "$ROOT/Cargo.toml"
}

VERSION="${PCBCRAFT_VERSION:-$(workspace_version)}"
if [ -z "$VERSION" ]; then
  echo "error: could not read [workspace.package] version from $ROOT/Cargo.toml" >&2
  exit 1
fi
export VERSION

DIST="${DIST:-$ROOT/dist/release}"
mkdir -p "$DIST"
DIST="$(cd "$DIST" && pwd)"
export DIST

PCBCRAFT_BUILD_SHA="${PCBCRAFT_BUILD_SHA:-$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo unknown)}"
export PCBCRAFT_BUILD_SHA
PCBCRAFT_BUILD_DATE="${PCBCRAFT_BUILD_DATE:-$(date -u +%Y-%m-%d)}"
export PCBCRAFT_BUILD_DATE
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
APP_ID="io.tangencylabs.pcbcraft"
export APP_ID

# Emit a GitHub Actions warning (plain stderr outside Actions).
warn() {
  if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::warning::$*"; else echo "warning: $*" >&2; fi
}

# Copy repository licence and readme files into a package directory.
copy_docs() {
  local dest="$1" f
  for f in README.md LICENSE LICENSE-MIT LICENSE-APACHE NOTICE ATTRIBUTION.md COPYRIGHT THIRD-PARTY-LICENSES.txt; do
    if [ -f "$ROOT/$f" ]; then cp "$ROOT/$f" "$dest/"; fi
  done
  copy_font_licences "$dest"
}

# Copy bundled font licences (including optional CRAFT_FONTS_DIR embeds) into dest.
copy_font_licences() {
  local dest="$1" f
  if [ -f "$ROOT/crates/visuals/fonts/OFL.txt" ]; then
    cp "$ROOT/crates/visuals/fonts/OFL.txt" "$dest/OFL-Inter.txt"
  fi
  if [ -d "$ROOT/licenses/egui-default-fonts" ]; then
    mkdir -p "$dest/egui-default-fonts"
    for f in "$ROOT"/licenses/egui-default-fonts/*; do
      if [ -f "$f" ]; then cp "$f" "$dest/egui-default-fonts/"; fi
    done
  fi
  [ -n "${CRAFT_FONTS_DIR:-}" ] || return 0
  for f in "$CRAFT_FONTS_DIR"/fonts/*/OFL.txt; do
    if [ -f "$f" ]; then cp "$f" "$dest/OFL-$(basename "$(dirname "$f")").txt"; fi
  done
}

# Portable SHA-256 checksum helper.
sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}
