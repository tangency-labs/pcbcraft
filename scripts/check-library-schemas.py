#!/usr/bin/env python3
"""Validate native symbol, footprint, category and part JSON against the published schemas.

Uses Python 3.10+ and jsonschema 4.18+. References resolve from this repository
only. Cross-record semantic validation belongs to the engine's library resolver.
"""
import argparse
import json
import sys
from importlib.metadata import version
from pathlib import Path

if sys.version_info < (3, 10):
    raise SystemExit("Use Python 3.10 or later to run this checker.")
try:
    from jsonschema import Draft202012Validator
    from jsonschema.exceptions import SchemaError, ValidationError
    from referencing import Registry, Resource
except ImportError:
    raise SystemExit('Install the checker dependency with python3 -m pip install "jsonschema>=4.18".') from None
if tuple(int(part) for part in version("jsonschema").split(".")[:2]) < (4, 18):
    raise SystemExit('This checker requires jsonschema 4.18 or later.')

ROOT = Path(__file__).resolve().parent.parent
FORMATS = {
    "pcbcraft-symbol/1": "urn:pcbcraft:schema:symbol:1",
    "pcbcraft-symbol/2": "urn:pcbcraft:schema:symbol:2",
    "pcbcraft-symbol/4": "urn:pcbcraft:schema:symbol:4",
    "pcbcraft-symbol/3": "urn:pcbcraft:schema:symbol:3",
    "pcbcraft-footprint/1": "urn:pcbcraft:schema:footprint:1",
    "pcbcraft-category/1": "urn:pcbcraft:schema:category:1",
    "pcbcraft-part/1": "urn:pcbcraft:schema:part:1",
}


def distinct_object(pairs):
    """Reject duplicate JSON keys before a dictionary can discard their ambiguity."""
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"Duplicate JSON key: {key}")
        result[key] = value
    return result


def read_json(path):
    """Bound regular native records to the source loader's per-file limit."""
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"{path}: expected a regular file")
    with path.open("rb") as source:
        data = source.read(1024 * 1024 + 1)
    if len(data) > 1024 * 1024:
        raise ValueError(f"{path}: native record exceeds 1 MiB")
    return json.loads(data.decode("utf-8"), object_pairs_hook=distinct_object)


def validators():
    """Check every published schema and register all references without network retrieval."""
    schemas = [read_json(path) for path in sorted((ROOT / "docs/schemas").glob("*.schema.json"))]
    for schema in schemas:
        Draft202012Validator.check_schema(schema)
    registry = Registry().with_resources((schema["$id"], Resource.from_contents(schema)) for schema in schemas)
    return {schema["$id"]: Draft202012Validator(schema, registry=registry) for schema in schemas}


def records(root):
    """Discover flat native item directories; manifests are validated by the source loader."""
    return [path for name in ("symbols", "footprints", "categories", "parts") for path in sorted((root / name).glob("*.json"))]


def validate(paths, sources):
    """Validate bundled/example records and additional native items using their explicit formats."""
    checks = validators()
    bundled = [path for library in sorted((ROOT / "libraries").iterdir()) if library.is_dir() for path in records(library)]
    examples = records(ROOT / "examples/native-library") + records(ROOT / "examples/native-parts")
    inputs = bundled + examples + paths + [path for source in sources for path in records(source)]
    for path in inputs:
        try:
            value = read_json(path)
            if not isinstance(value, dict) or value.get("format") not in FORMATS:
                raise ValueError("Unsupported native record format")
            checks[FORMATS[value["format"]]].validate(value)
        except (OSError, ValueError, ValidationError) as error:
            raise ValueError(f"{path}: {error}") from error
    print(f"Validated {len(checks)} schemas and {len(inputs)} native library records offline.")


def main():
    """Expose structural validation without modifying library or project data."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("records", nargs="*", type=Path)
    parser.add_argument("--source", action="append", default=[], type=Path, help="additional flat native library folder")
    args = parser.parse_args()
    try:
        validate(args.records, args.source)
    except (OSError, ValueError, SchemaError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
