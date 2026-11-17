#!/usr/bin/env python3
"""Check the committed envelope schemas against crates/plugin-rpc/tests/messages.json.

The same table drives the Rust side in crates/plugin-rpc/tests/envelope_parity.rs. This half
matters because a plugin author in another language implements against the schemas and tests
against the Rust types, so the two must accept the same messages. Nothing generates the schemas,
because `uniqueItems` and `pattern` are beyond what the derive can emit, so a schema can drift
from the types in silence unless a check like this one runs.
"""

import json
import sys
from pathlib import Path

try:
    from jsonschema import Draft202012Validator
except ModuleNotFoundError:
    sys.exit("this check needs jsonschema: pip install jsonschema")

ROOT = Path(__file__).resolve().parents[1]
TABLE = ROOT / "crates" / "plugin-rpc" / "tests" / "messages.json"
SCHEMAS = ROOT / "spec" / "schema" / "prpc-1"
ENVELOPES = ("request", "response", "handshake")


def main() -> int:
    table = json.loads(TABLE.read_text())
    validators = {}
    for name in ENVELOPES:
        document = json.loads((SCHEMAS / f"{name}.json").read_text())
        Draft202012Validator.check_schema(document)
        validators[name] = Draft202012Validator(document)

    wrong = []
    for case in table["cases"]:
        got = "accept" if validators[case["schema"]].is_valid(case["message"]) else "reject"
        if got != case["expect"]:
            wrong.append(
                f"  {case['name']}: the schema {got}s it; the mechanism says {case['expect']}"
            )

    print(f"{len(table['cases'])} cases against the committed schemas, {len(wrong)} disagreement(s)")
    for line in wrong:
        print(line)
    return 1 if wrong else 0


if __name__ == "__main__":
    sys.exit(main())
