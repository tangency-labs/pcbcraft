"""Check state IDs and labels under the repository's JSON Schema validator."""

import json
import unittest
from pathlib import Path

from jsonschema import Draft202012Validator

CONTRACTS = Path(__file__).resolve().parent.parent / "docs/schemas/command-contracts.json"


class LifecyclePolicySchemaTests(unittest.TestCase):
    """Adoption policy state text must match through the end of the input."""

    def test_published_policy_refuses_trailing_newline_in_id_or_label(self):
        """Each final newline must be refused independently of other state fields."""
        contracts = json.loads(CONTRACTS.read_text(encoding="utf-8"))
        schema = contracts["commands"]["project.adopt_lifecycle"]["params"]["properties"]["policy"]
        validator = Draft202012Validator(schema)
        state = {"id": "draft", "label": "Released", "disposition": "released"}
        policy = {"scheme": {"library": "pcbcraft", "id": "lifecycle", "revision": 1}, "states": [state]}
        self.assertTrue(validator.is_valid(policy))
        for field in ("id", "label"):
            with self.subTest(field=field):
                changed = {**state, field: state[field] + "\n"}
                self.assertFalse(validator.is_valid({**policy, "states": [changed]}))


if __name__ == "__main__":
    unittest.main()
