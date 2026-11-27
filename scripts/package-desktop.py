#!/usr/bin/env python3
"""Package native PCBcraft release binaries for an explicitly supported preview host."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "linux-x86_64": "x86_64-unknown-linux-gnu",
    "windows-x64": "x86_64-pc-windows-msvc",
    "macos-aarch64": "aarch64-apple-darwin",
}
PROGRAMS = ("pcbcraft", "pcbcraft-cli")
NOTICE_PREFIXES = ("LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE", "COPYRIGHT", "OFL")
REPO_NOTICES = (
    "LICENSE-APACHE",
    "LICENSE-MIT",
    "ATTRIBUTION.md",
    "crates/geometry/SOLIDCORE-NOTICE",
    "crates/chrome/assets/shared-ui/SOLIDCRAFT-UI-LICENSE",
    "crates/chrome/assets/ribbon/SOLIDCRAFT-NOTICE",
)


def command(*args, capture=False, env=None):
    return subprocess.run(args, cwd=ROOT, check=True, text=True, capture_output=capture, env=env).stdout


def native_dependencies(target):
    """Select the desktop and CLI dependency closure from the locked platform metadata."""
    metadata = json.loads(command("cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", target, capture=True))
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    workspace = set(metadata["workspace_members"])
    pending = [key for key in workspace if packages[key]["name"] in PROGRAMS]
    seen = set()
    while pending:
        key = pending.pop()
        if key in seen:
            continue
        seen.add(key)
        pending.extend(dep["pkg"] for dep in nodes[key]["deps"] if any(kind["kind"] is None for kind in dep["dep_kinds"]))
    return sorted((packages[key] for key in seen - workspace), key=lambda p: (p["name"], p["version"], p["id"]))


def dependency_notices(target):
    """Collect license texts and notices as shipped by the resolved dependency packages."""
    inventory = []
    notices = {}
    missing = []
    for package in native_dependencies(target):
        name = f"{package['name']} {package['version']}"
        declared = package.get("license")
        inventory.append(f"{name} ({declared or 'license file'})")
        directory = Path(package["manifest_path"]).parent
        paths = {p for p in directory.iterdir() if p.is_file() and p.name.upper().startswith(NOTICE_PREFIXES)}
        if package.get("license_file"):
            paths.add(directory / package["license_file"])
        # Some crates bundle fonts or retain license texts in an immediate LICENSES directory.
        licenses = directory / "LICENSES"
        if licenses.is_dir():
            paths.update(p for p in licenses.iterdir() if p.is_file())
        if package["name"] == "epaint_default_fonts":
            paths.update((directory / "fonts").glob("*.txt"))
        if not paths:
            if not declared:
                raise ValueError(f"Missing license declaration and notice files for {name}")
            missing.append(f"{name}: {declared}")
        for path in sorted(paths):
            content = path.read_text(encoding="utf-8-sig").strip()
            if content:
                notices.setdefault(content, set()).add(f"{name} / {path.name}")
    sections = ["PCBcraft desktop and CLI dependency notices", *inventory]
    for content, names in sorted(notices.items(), key=lambda entry: sorted(entry[1])):
        sections.append("\n" + "\n".join(sorted(names)) + "\n\n" + content)
    if missing:
        sections.extend(["\nPackages declaring a license without a separate license file:", *missing])
    return "\n".join(sections) + "\n"


def stage_resources(destination, target):
    """Stage the build metadata and notices that accompany the native preview."""
    destination.mkdir(parents=True)
    for path in REPO_NOTICES:
        shutil.copy2(ROOT / path, destination)
    for path in sorted(ROOT.glob("crates/chrome/assets/fonts/*/OFL.txt")):
        shutil.copy2(path, destination / f"{path.parent.name}-OFL.txt")
    (destination / "THIRD-PARTY-LICENSES.txt").write_text(dependency_notices(target), encoding="utf-8")
    revision = command("git", "rev-parse", "HEAD", capture=True).strip()
    compiler = command("rustc", "--version", capture=True).strip()
    (destination / "BUILD.txt").write_text(f"Revision: {revision}\nTarget: {target}\nCompiler: {compiler}\n", encoding="utf-8")


def package_macos(work, artifacts, resources, desktop, cli):
    """Use the existing signed bundle and DMG pipeline with the generated preview resources."""
    target_dir = work / "macos"
    binaries = target_dir / TARGETS["macos-aarch64"] / "release"
    binaries.mkdir(parents=True)
    for binary in (desktop, cli):
        (binaries / binary.name).symlink_to(binary)
    env = dict(os.environ, CARGO_TARGET_DIR=str(target_dir), DIST=str(artifacts), PCBCRAFT_PACKAGE_RESOURCES=str(resources))
    command("bash", "packaging/macos/package.sh", "--arch", "aarch64", "--skip-build", env=env)


def package_linux(artifacts, resources, portable, version):
    """Package the same Linux binaries as a DEB and RPM with runtime dependencies."""
    glibc_versions = []
    for program in PROGRAMS:
        requirements = command("readelf", "--version-info", portable / program, capture=True)
        glibc_versions.extend(re.findall(r"\bGLIBC_(\d+(?:\.\d+)+)\b", requirements))
    if not glibc_versions:
        raise ValueError("Could not determine the Linux binaries' glibc requirement")
    glibc_minimum = max(glibc_versions, key=lambda value: tuple(map(int, value.split("."))))
    env = dict(
        os.environ,
        PCBCRAFT_GLIBC_MIN=glibc_minimum,
        PCBCRAFT_PACKAGE_VERSION=version,
        PCBCRAFT_PACKAGE_RELEASE=os.environ.get("PCBCRAFT_PACKAGE_RELEASE", "1"),
        PCBCRAFT_PACKAGE_BIN=str(portable),
        PCBCRAFT_PACKAGE_RESOURCES=str(resources),
    )
    for packager in ("deb", "rpm"):
        destination = artifacts / f"{portable.name}.{packager}"
        command("nfpm", "package", "--config", "packaging/linux/nfpm.yaml",
                "--packager", packager, "--target", destination, env=env)


def package(platform, bin_dir, output):
    """Publish complete artifacts and checksums only after packaging and CLI smoke checks."""
    target = TARGETS[platform]
    host = next(line.removeprefix("host: ") for line in command("rustc", "-vV", capture=True).splitlines() if line.startswith("host: "))
    if host != target:
        raise ValueError(f"{platform} preview requires a native {target} runner, got {host}")
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    suffix = ".exe" if platform.startswith("windows-") else ""
    desktop, cli = (bin_dir / (program + suffix) for program in PROGRAMS)
    stem = f"pcbcraft-{version}-{platform}"
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        raise ValueError(f"Choose a new output directory: {output}")

    with tempfile.TemporaryDirectory(prefix=".pcbcraft-package-", dir=output.parent) as scratch:
        work = Path(scratch)
        artifacts = work / "artifacts"
        artifacts.mkdir()
        resources = work / "resources"
        stage_resources(resources, target)
        portable = work / stem
        shutil.copytree(resources, portable)
        staged_cli = portable / cli.name
        shutil.copy2(cli, staged_cli)
        actual = command(staged_cli, "--version", capture=True).strip()
        if actual != f"pcbcraft-cli {version}":
            raise ValueError(f"Expected CLI version {version}, got {actual}")
        subprocess.run(
            [staged_cli, "mcp", "--read-only", "--project", ROOT / "examples/sensor-node.pc"],
            cwd=work,
            input=b"",
            timeout=30,
            check=True,
        )

        if platform.startswith("macos-"):
            package_macos(work, artifacts, resources, desktop, cli)
        else:
            shutil.copy2(desktop, portable / desktop.name)
            archive_format = "zip" if suffix else "gztar"
            shutil.make_archive(str(artifacts / stem), archive_format, work, stem)
            if platform == "linux-x86_64":
                package_linux(artifacts, resources, portable, version)

        checksums = []
        for path in sorted(artifacts.iterdir()):
            with path.open("rb") as stream:
                digest = hashlib.file_digest(stream, "sha256").hexdigest()
            checksums.append(f"{digest}  {path.name}")
        (artifacts / "SHA256SUMS.txt").write_text("\n".join(checksums) + "\n", encoding="utf-8", newline="\n")
        artifacts.rename(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform", choices=TARGETS)
    parser.add_argument("--bin-dir", type=Path, required=True, help="directory containing both native release binaries")
    parser.add_argument("--output", type=Path, required=True, help="new directory for the completed packages")
    args = parser.parse_args()
    try:
        package(args.platform, args.bin_dir.resolve(), args.output.resolve())
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Desktop packaging failed: {error}\n")


if __name__ == "__main__":
    main()
