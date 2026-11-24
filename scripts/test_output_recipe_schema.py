"""Regressions for published output-recipe name, file-stem and layer ID schema validation."""

import json
import shutil
import subprocess
import unittest
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parent.parent
CONTRACTS = ROOT / "docs/schemas/command-contracts.json"

RESERVED_PORTABLE_SEGMENTS = [
    "con", "prn", "aux", "nul",
    "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9",
    "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
]

ACCEPTED_NAMES = ["Prototype", "A", "Rev 2 / Proto", "Board 1"]
REJECTED_NAMES_PYTHON = [
    "",
    " ",
    " Prototype ",
    " Prototype",
    "Prototype ",
    "Proto\ntype",
    "Prototype\n",
    "Proto\ttype",
    "Proto\u0000type",
    "Proto\u007ftype",
    "Proto\u0085type",
    "\u00a0Prototype",
]
# ECMA-262 `\s` includes U+FEFF (BOM), whereas Python's `re` `\s` does not.
REJECTED_NAMES_ECMA_ONLY = ["\ufeffPrototype", "Prototype\ufeff"]

ACCEPTED_STEMS = ["prototype", "a", "con-1", "lpt10"]
REJECTED_STEMS = ["", "-proto", "proto-", "Proto", "prototype\n", "../package"]

LAYER = "01234567-89ab-4cde-8fab-0123456789ab"
UPPER_LAYER = LAYER.upper()
REJECTED_LAYERS = [UPPER_LAYER, LAYER.replace("-", ""), "{" + LAYER + "}", LAYER + "\n"]


class OutputRecipeSchemaTests(unittest.TestCase):
    """Evaluate published `pcb.set_output_recipe` name and stem schemas directly."""

    @classmethod
    def setUpClass(cls):
        """Load the published output-recipe content property schemas."""
        contracts = json.loads(CONTRACTS.read_text(encoding="utf-8"))
        content = contracts["commands"]["pcb.set_output_recipe"]["params"]["properties"]["content"]
        properties = content["oneOf"][1]["properties"]
        cls.name_schema = properties["name"]
        cls.stem_schema = properties["file_stem"]
        cls.layers_schema = properties["gerber_layers"]
        cls.content_validator = Draft202012Validator(content)
        cls.name_validator = Draft202012Validator(cls.name_schema)
        cls.stem_validator = Draft202012Validator(cls.stem_schema)
        cls.layers_validator = Draft202012Validator(cls.layers_schema, format_checker=Draft202012Validator.FORMAT_CHECKER)

    def test_published_name_schema_matches_runtime_cases(self):
        """Published name pattern and length limits accept valid names and reject invalid ones."""
        max_len = self.name_schema["maxLength"]
        for name in [*ACCEPTED_NAMES, "a" * max_len]:
            with self.subTest(accepted=name):
                self.assertTrue(self.name_validator.is_valid(name))
        for name in [*REJECTED_NAMES_PYTHON, "a" * (max_len + 1)]:
            with self.subTest(rejected=name):
                self.assertFalse(self.name_validator.is_valid(name))

    def test_published_file_stem_schema_matches_runtime_cases(self):
        """Published stem pattern, length limits and `not.enum` match runtime validation."""
        max_len = self.stem_schema["maxLength"]
        self.assertEqual(self.stem_schema["not"]["enum"], RESERVED_PORTABLE_SEGMENTS)
        for stem in [*ACCEPTED_STEMS, "a" * max_len]:
            with self.subTest(accepted=stem):
                self.assertTrue(self.stem_validator.is_valid(stem))
        for stem in [*RESERVED_PORTABLE_SEGMENTS, *REJECTED_STEMS, "a" * (max_len + 1)]:
            with self.subTest(rejected=stem):
                self.assertFalse(self.stem_validator.is_valid(stem))

    def test_published_layer_schema_refuses_case_only_duplicates(self):
        """One spelling per layer UUID lets `uniqueItems` refuse what runtime decoding merges."""
        self.assertTrue(self.layers_schema["uniqueItems"])
        self.assertTrue(self.layers_validator.is_valid([LAYER]))
        self.assertFalse(self.layers_validator.is_valid([LAYER, LAYER]))
        self.assertFalse(self.layers_validator.is_valid([LAYER, UPPER_LAYER]))
        for layer in REJECTED_LAYERS:
            with self.subTest(rejected=layer):
                self.assertFalse(self.layers_validator.is_valid([layer]))

    def test_published_integer_fields_accept_integer_valued_numbers(self):
        """The origin point and profile revision accept 2.0 like 2 and reject 2.5, as the command does."""
        recipe = {
            "name": "Prototype",
            "board": "01234567-89ab-4cde-8fab-0123456789ab",
            "units": "mm",
            "file_stem": "prototype",
            "gerber_layers": [],
            "drill": True,
            "origin": {"kind": "point", "x_nm": 100, "y_nm": -200},
            "assembly_profile": {"profile": "01234567-89ab-4cde-8fab-0123456789ac", "revision": 2},
        }
        self.assertTrue(self.content_validator.is_valid(recipe))
        for field, path in [("origin.x_nm", ("origin", "x_nm")), ("assembly_profile.revision", ("assembly_profile", "revision"))]:
            for number, expected in [(2.0, True), (2.5, False)]:
                with self.subTest(field=field, number=number):
                    content = json.loads(json.dumps(recipe))
                    content[path[0]][path[1]] = number
                    self.assertEqual(self.content_validator.is_valid(content), expected)

    @unittest.skipUnless(shutil.which("node"), "Node.js is needed for the JavaScript pattern check")
    def test_published_patterns_match_ecmascript_regex(self):
        """Evaluate the published ECMA-262 patterns directly, including U+FEFF edge whitespace."""
        name_pattern = self.name_schema["pattern"]
        stem_pattern = self.stem_schema["pattern"]
        layer_pattern = self.layers_schema["items"]["pattern"]
        cases = (
            [{"pattern": name_pattern, "text": text, "expected": True} for text in ACCEPTED_NAMES]
            + [
                {"pattern": name_pattern, "text": text, "expected": False}
                for text in [*REJECTED_NAMES_PYTHON, *REJECTED_NAMES_ECMA_ONLY]
            ]
            + [{"pattern": stem_pattern, "text": text, "expected": True} for text in [*ACCEPTED_STEMS, *RESERVED_PORTABLE_SEGMENTS]]
            + [{"pattern": stem_pattern, "text": text, "expected": False} for text in REJECTED_STEMS]
            + [{"pattern": layer_pattern, "text": LAYER, "expected": True}]
            + [{"pattern": layer_pattern, "text": text, "expected": False} for text in REJECTED_LAYERS]
        )
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


if __name__ == "__main__":
    unittest.main()
