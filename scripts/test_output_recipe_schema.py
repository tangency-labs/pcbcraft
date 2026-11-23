"""Regressions for published output-recipe name and file-stem schema validation."""

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
        cls.name_validator = Draft202012Validator(cls.name_schema)
        cls.stem_validator = Draft202012Validator(cls.stem_schema)

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

    @unittest.skipUnless(shutil.which("node"), "Node.js is needed for the JavaScript pattern check")
    def test_published_patterns_match_ecmascript_regex(self):
        """Evaluate the published ECMA-262 patterns directly, including U+FEFF edge whitespace."""
        name_pattern = self.name_schema["pattern"]
        stem_pattern = self.stem_schema["pattern"]
        cases = (
            [{"pattern": name_pattern, "text": text, "expected": True} for text in ACCEPTED_NAMES]
            + [
                {"pattern": name_pattern, "text": text, "expected": False}
                for text in [*REJECTED_NAMES_PYTHON, *REJECTED_NAMES_ECMA_ONLY]
            ]
            + [{"pattern": stem_pattern, "text": text, "expected": True} for text in [*ACCEPTED_STEMS, *RESERVED_PORTABLE_SEGMENTS]]
            + [{"pattern": stem_pattern, "text": text, "expected": False} for text in REJECTED_STEMS]
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
