#!/usr/bin/env python3
"""Validate footprint schemas and canonical records without opening project files.

Requires the Python jsonschema package. The engine performs the additional
semantic checks documented in docs/footprint-schema.md.
"""

import argparse
import json
from pathlib import Path

from jsonschema import Draft202012Validator
from jsonschema.exceptions import SchemaError, ValidationError
from referencing import Registry, Resource


def validate(definitions: list[Path], placements: list[Path]) -> None:
    """Check both schemas offline, then validate bundled and supplied records."""
    root = Path(__file__).resolve().parent.parent
    schemas = [
        json.loads((root / "docs/schemas" / name).read_text(encoding="utf-8"))
        for name in ("footprint-v1.schema.json", "footprint-placement-v15.schema.json")
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
    for validator, paths in zip(validators, [bundled + definitions, placements], strict=True):
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
    arguments = parser.parse_args()
    try:
        validate(arguments.definitions, arguments.placement)
    except (OSError, ValueError, SchemaError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
