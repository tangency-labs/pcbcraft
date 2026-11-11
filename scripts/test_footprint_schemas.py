"""Regressions for schema string validation and checker setup failures."""

import copy
import json
import shutil
import subprocess
import sys
import unittest
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parent.parent
CHECKER = ROOT / "scripts/check-footprint-schemas.py"


class FootprintSchemaTests(unittest.TestCase):
    """Keep canonical string constraints consistent across schema consumers."""

    @classmethod
    def setUpClass(cls):
        """Load the published schema and an existing native library definition."""
        cls.schema = json.loads((ROOT / "docs/schemas/footprint-v1.schema.json").read_text())
        cls.validator = Draft202012Validator(cls.schema)
        cls.definition = json.loads((ROOT / "libraries/starter/footprints/resistor-0402.json").read_text())

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
