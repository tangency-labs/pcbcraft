"""Regressions for schema string validation and checker setup failures."""

import copy
import json
import shutil
import subprocess
import sys
import unittest
from pathlib import Path

from jsonschema import Draft202012Validator
from referencing import Registry, Resource

ROOT = Path(__file__).resolve().parent.parent
CHECKER = ROOT / "scripts/check-footprint-schemas.py"


class FootprintSchemaTests(unittest.TestCase):
    """Keep canonical string constraints consistent across schema consumers."""

    @classmethod
    def setUpClass(cls):
        """Load the published schema and an existing native library definition."""
        cls.schema = json.loads((ROOT / "docs/schemas/footprint-v1.schema.json").read_text())
        cls.validator = Draft202012Validator(cls.schema)
        cls.definition = json.loads((ROOT / "libraries/basic-components/footprints/resistor-0402.json").read_text())

    def test_circular_non_plated_holes_require_new_format_and_no_terminal(self):
        """Accept only bounded mechanical records and leave the legacy schema closed."""
        schema = json.loads((ROOT / "docs/schemas/footprint-v4.schema.json").read_text())
        registry = Registry().with_resource(self.schema["$id"], Resource.from_contents(self.schema))
        validator = Draft202012Validator(schema, registry=registry)
        hole = {"center": {"x_nm": 0, "y_nm": 0}, "diameter_nm": 3200000}
        value = dict(self.definition, format="pcbcraft-footprint/4", non_plated_holes=[hole, hole])
        validator.validate(value)
        validator.validate(dict(value, pads=[]))
        self.assertFalse(self.validator.is_valid(value))
        self.assertFalse(validator.is_valid(self.definition))
        for invalid in ([], None, [dict(hole, number="1")], [dict(hole, diameter_nm=0)], [hole] * 129):
            self.assertFalse(validator.is_valid(dict(value, non_plated_holes=invalid)))

    def test_reference_metadata_and_pad_identifiers_reject_control_characters(self):
        """Catch newline suffixes that Python's dollar anchor previously accepted."""
        paths = [
            ("library",), ("id",), ("name",), ("license",),
            ("source", "kind"), ("source", "reference"), ("source", "locator"),
            ("pads", 0, "number"),
        ]
        self.validator.validate(self.definition)
        for path in paths:
            for suffix in ["\n", "\r\n", "\x00", "\x85"]:
                with self.subTest(path=path, suffix=repr(suffix)):
                    value = copy.deepcopy(self.definition)
                    parent = value
                    for key in path[:-1]:
                        parent = parent[key]
                    parent[path[-1]] += suffix
                    self.assertFalse(self.validator.is_valid(value))

    def test_artwork_text_rejects_control_characters(self):
        """Exercise the text pattern through a complete footprint definition."""
        value = copy.deepcopy(self.definition)
        value["artwork"] = [{"purpose": "fabrication", "graphic": {
            "geometry": {"type": "text", "at": {"x_nm": 0, "y_nm": 0},
                         "text": "REF", "height_nm": 1000000, "rotation": 0, "align": "middle"},
            "stroke_nm": 100000, "fill": "none",
        }}]
        self.validator.validate(value)
        for suffix in ["\n", "\r\n", "\x00", "\x85"]:
            with self.subTest(suffix=repr(suffix)):
                value["artwork"][0]["graphic"]["geometry"]["text"] = "REF" + suffix
                self.assertFalse(self.validator.is_valid(value))

    @unittest.skipUnless(shutil.which("node"), "Node.js is needed for the JavaScript pattern check")
    def test_patterns_agree_in_python_and_javascript(self):
        """Check valid strings and forbidden suffixes using both regex dialects."""
        cases = []
        for pattern, valid in [
            (self.schema["$defs"]["library_id"]["pattern"], "resistor-0402"),
            (self.schema["$defs"]["terminal_id"]["pattern"], "A1"),
            (self.schema["properties"]["name"]["pattern"], "Reference label"),
        ]:
            for suffix in ["", "\n", "\r\n", "\x00", "\x85"]:
                text = valid + suffix
                expected = suffix == ""
                validator = Draft202012Validator({"type": "string", "pattern": pattern})
                self.assertEqual(validator.is_valid(text), expected)
                cases.append({"pattern": pattern, "text": text, "expected": expected})
        program = """
const fs = require('fs');
const cases = JSON.parse(fs.readFileSync(0, 'utf8'));
for (const item of cases) {
    if (new RegExp(item.pattern).test(item.text) !== item.expected) {
        throw new Error(JSON.stringify(item));
    }
}
"""
        subprocess.run(["node", "-e", program], input=json.dumps(cases), text=True, check=True, capture_output=True)

    def test_local_print_overrides_are_sparse_and_typed(self):
        """Saved placement print records agree with the runtime's sparse constraints."""
        schema = json.loads((ROOT / "docs/schemas/footprint-placement-v15.schema.json").read_text())
        validator = Draft202012Validator(schema["properties"]["reference_text"])
        for value in [{"text": "Power input"}, {"visible": False}, {"rotation": 0},
                      {"size": {"height_nm": 1500000, "stroke_width_nm": 150000}}]:
            with self.subTest(valid=value):
                validator.validate(value)
        for value in [{}, None, {"text": ""}, {"text": "R1\n"}, {"text": "R" * 129},
                      {"text": None}, {"visible": 1}, {"font": "unknown"},
                      {"rotation": 360}, {"align": "left"}, {"unknown": True},
                      {"size": {"height_nm": 1500000}},
                      {"size": {"height_nm": 0, "stroke_width_nm": 1}}]:
            with self.subTest(invalid=value):
                self.assertFalse(validator.is_valid(value))

    def test_orientation_statement_selects_format_7(self):
        """Format 7 is format 4 plus a required, closed orientation object; holes stay optional."""
        npth = json.loads((ROOT / "docs/schemas/footprint-v4.schema.json").read_text())
        schema = json.loads((ROOT / "docs/schemas/footprint-v7.schema.json").read_text())
        registry = Registry().with_resources(
            (item["$id"], Resource.from_contents(item)) for item in (self.schema, npth)
        )
        validator = Draft202012Validator(schema, registry=registry)
        value = copy.deepcopy(self.definition)
        value["format"] = "pcbcraft-footprint/7"
        value["orientation"] = {"polarized": True, "pin_1": "1", "zero": "ipc-7351"}
        validator.validate(value)
        self.assertFalse(self.validator.is_valid(value), "format 1 has no orientation")
        hole = {"center": {"x_nm": 0, "y_nm": 2000000}, "diameter_nm": 500000}
        validator.validate(dict(value, non_plated_holes=[hole]))
        validator.validate(dict(value, pads=[], non_plated_holes=[hole]))
        self.assertFalse(validator.is_valid(dict(value, pads=[])), "a package needs a pad or a hole")
        self.assertFalse(validator.is_valid(dict(value, non_plated_holes=[])))
        self.assertFalse(Draft202012Validator(npth, registry=registry).is_valid(dict(value, format="pcbcraft-footprint/4", non_plated_holes=[hole])))
        for orientation in [
            None, {}, [True, "1", "ipc-7351"], {"polarized": None},
            {"polarized": True, "zero": "ipc-7350"}, {"polarized": True, "zero": {"ipc-7351": None}},
            {"polarized": True, "pin_1": None}, {"polarized": True, "side": "top"},
        ]:
            with self.subTest(orientation=orientation):
                value["orientation"] = orientation
                self.assertFalse(validator.is_valid(value))
        del value["orientation"]
        self.assertFalse(validator.is_valid(value), "format 7 needs the statement")

    def test_checker_selects_the_schema_by_format(self):
        """A supplied format-7 definition passes; the same object as format 1, 4 or 2 fails."""
        import tempfile
        value = copy.deepcopy(self.definition)
        value["format"] = "pcbcraft-footprint/7"
        value["orientation"] = {"polarized": True}
        value["non_plated_holes"] = [{"center": {"x_nm": 0, "y_nm": 2000000}, "diameter_nm": 500000}]
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "diode.json"
            path.write_text(json.dumps(value))
            result = subprocess.run([sys.executable, str(CHECKER), str(path)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            for other in ["pcbcraft-footprint/1", "pcbcraft-footprint/4", "pcbcraft-footprint/2"]:
                with self.subTest(format=other):
                    value["format"] = other
                    path.write_text(json.dumps(value))
                    result = subprocess.run([sys.executable, str(CHECKER), str(path)], capture_output=True, text=True)
                    self.assertEqual(result.returncode, 1)

    def test_missing_dependency_has_a_setup_message(self):
        """A Python environment without site packages receives an installation hint."""
        result = subprocess.run([sys.executable, "-S", str(CHECKER)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn('pip install "jsonschema>=4.18"', result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    def test_older_dependency_has_a_version_message(self):
        """Reject a reported pre-registry version before validating any records."""
        program = """
import runpy
import sys
from unittest.mock import patch
with patch('importlib.metadata.version', return_value='4.17.3'):
    runpy.run_path(sys.argv[1], run_name='__main__')
"""
        result = subprocess.run([sys.executable, "-c", program, str(CHECKER)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn("requires jsonschema 4.18 or later", result.stderr)
        self.assertNotIn("Traceback", result.stderr)


if __name__ == "__main__":
    unittest.main()
