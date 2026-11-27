#!/usr/bin/env python3
"""Package a flat native library for the browser's Upload bundle action.

This reads library data only; it never edits project documents. The engine remains
the authority for native schema, geometry and identity validation on upload.
"""
import argparse
import json
from pathlib import Path

# Flat native directories, including publisher lifecycle assertions, in the source loader's order.
DIRECTORIES = ("symbols", "footprints", "categories", "parts", "manufacturer-parts", "lifecycle-assertions")


def is_document_name(name: str) -> bool:
    """Match the flat JSON leaf-name rules in the engine's is_document_path."""
    return (
        name.endswith(".json")
        and len(name.encode("utf-8")) <= 128
        and not name.startswith(".")
        and "/" not in name
        and "\\" not in name
        and not any(ord(char) <= 0x1F or 0x7F <= ord(char) <= 0x9F for char in name)
    )


def bundle(root: Path) -> bytes:
    """Bound regular input files and reject unsupported paths before bundling."""
    if root.is_symlink() or not root.is_dir():
        raise ValueError("The library must be a regular directory")
    paths = [root / "library.json"]
    for directory in (root / name for name in DIRECTORIES):
        if directory.is_symlink():
            raise ValueError(f"{directory}: symlinks are unsupported")
        if directory.exists():
            paths.extend(sorted(directory.iterdir()))
    if len(paths) > 256:
        raise ValueError("A library has at most 256 files")
    documents = {}
    total = 0
    for path in paths:
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"{path}: expected a regular JSON file")
        if not is_document_name(path.name):
            raise ValueError(
                f"Unsupported library path {path.relative_to(root).as_posix()!r}: "
                "expected a flat JSON filename of at most 128 UTF-8 bytes without a leading dot, separators or control characters"
            )
        if path.stat().st_size > 1024 * 1024:
            raise ValueError(f"{path}: exceeds 1 MiB")
        with path.open("rb") as source:
            data = source.read(1024 * 1024 + 1)
        if len(data) > 1024 * 1024:
            raise ValueError(f"{path}: exceeds 1 MiB")
        total += len(data)
        if total > 8 * 1024 * 1024:
            raise ValueError("Library content exceeds 8 MiB")
        documents[path.relative_to(root).as_posix()] = data.decode("utf-8")
    result = json.dumps({"format": "pcbcraft-library-bundle/1", "documents": documents}, ensure_ascii=False, indent=2).encode("utf-8") + b"\n"
    if len(result) > 16 * 1024 * 1024:
        raise ValueError("Encoded bundle exceeds 16 MiB")
    return result


def main() -> None:
    """Write a new bundle, preserving any existing destination file."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("library", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        data = bundle(args.library)
        with args.output.open("xb") as output:
            output.write(data)
    except (OSError, UnicodeError, ValueError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
