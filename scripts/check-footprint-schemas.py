#!/usr/bin/env python3
"""Validate footprint schemas and canonical records without opening project files.

Requires Python 3.10 or later and jsonschema 4.18 or later, with its referencing
support. The engine performs the additional semantic checks documented in
docs/footprint-schema.md.
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

installed_version = version("jsonschema").split(".")
if (int(installed_version[0]), int(installed_version[1])) < (4, 18):
    raise SystemExit('This checker requires jsonschema 4.18 or later. Run python3 -m pip install "jsonschema>=4.18".')


def validate(definitions: list[Path], placements: list[Path], placement_version: int = 15) -> None:
    """Check the selected vocabulary offline, then validate bundled and supplied records."""
    root = Path(__file__).resolve().parent.parent
    schemas = [
        json.loads((root / "docs/schemas" / name).read_text(encoding="utf-8"))
        for name in ("footprint-v1.schema.json", f"footprint-placement-v{placement_version}.schema.json", "library-common-v1.schema.json")
    ]
    for schema in schemas:
        Draft202012Validator.check_schema(schema)
    registry = Registry().with_resources(
        (schema["$id"], Resource.from_contents(schema)) for schema in schemas
    )
    validators = [Draft202012Validator(schema, registry=registry) for schema in schemas]
    bundled = sorted((root / "libraries/starter/footprints").glob("*.json"))
    if not bundled:
        raise ValueError("No bundled footprint definitions found.")
    for validator, paths in zip(validators[:2], [bundled + definitions, placements], strict=True):
        for path in paths:
            try:
                validator.validate(json.loads(path.read_text(encoding="utf-8")))
            except (OSError, ValueError, ValidationError) as error:
                raise ValueError(f"{path}: {error}") from error
    print(
        f"Validated {len(schemas)} schemas, {len(bundled) + len(definitions)} "
        f"footprint definitions and {len(placements)} physical placement records."
    )


def main() -> None:
    """Accept additional standalone records and report invalid input as a CLI error."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("definitions", nargs="*", type=Path, help="additional footprint definition JSON files")
    parser.add_argument("--placement", action="append", default=[], type=Path, help="one physical placement JSON record")
    parser.add_argument("--placement-version", type=int, choices=(15, 17), default=15, help="board vocabulary for physical placement records")
    arguments = parser.parse_args()
    try:
        validate(arguments.definitions, arguments.placement, arguments.placement_version)
    except (OSError, ValueError, SchemaError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
