#!/usr/bin/env python3
"""Check the committed profile-spec meta-schema against crates/plugin-rpc/tests/profile_cases.json.

The same table drives the Rust half in tests/profile_spec.rs. This half matters because a profile
author validates their profile spec with `schema/prpc-1/profile.json` and loads it with a host's
reader, so the two must accept the same files. Nothing generates the meta-schema, so it can drift
from the reference loader in silence unless a check like this one runs.
"""

import json
import sys
from pathlib import Path

try:
    from jsonschema import Draft202012Validator
except ModuleNotFoundError:
    sys.exit("this check needs jsonschema: pip install jsonschema")

ROOT = Path(__file__).resolve().parents[1]
TABLE = ROOT / "crates" / "plugin-rpc" / "tests" / "profile_cases.json"
META = ROOT / "spec" / "schema" / "prpc-1" / "profile.json"


def main() -> int:
    document = json.loads(META.read_text())
    Draft202012Validator.check_schema(document)
    validator = Draft202012Validator(document)
    table = json.loads(TABLE.read_text())

    wrong = []
    for case in table["cases"]:
        got = "accept" if validator.is_valid(case["document"]) else "reject"
        if got != case["expect"]:
            first = next(iter(sorted(validator.iter_errors(case["document"]), key=lambda e: e.json_path)), None)
            detail = f": {first.message}" if first and got == "reject" else ""
            wrong.append(f"  {case['name']}: the meta-schema {got}s it; the table says {case['expect']}{detail}")

    print(f"{len(table['cases'])} cases against the committed meta-schema, {len(wrong)} disagreement(s)")
    for line in wrong:
        print(line)
    return 1 if wrong else 0


if __name__ == "__main__":
    sys.exit(main())
