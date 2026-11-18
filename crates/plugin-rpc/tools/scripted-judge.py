#!/usr/bin/env python3
"""The scripted judge.

The semantic stage asks an external judge three questions of the prose: `s1` whether the prose left
a claim unmentioned or an assertion unstated, `s2` whether two documents say the same thing twice,
and `s3` whether the prose contradicts a claim that passed. This script answers each task with the
empty verdict, so a check runs with no model call at all.

That is sound only while the prose is written so that every answer really is empty. A claim added
without a sentence that names it, or a paragraph that contradicts one that passed, is then not
caught here: it is caught by the bind pass, which refuses a declared claim no section names. Point
`.veridoc/config.json` at a real judge to have the semantic stage check the rest.
"""

import json
import sys

FIXTURES = {
    "s1": {"unmentionedClaims": [], "unstatedAssertions": []},
    "s2": {"pairs": []},
    "s3": {"pairs": []},
}


def main() -> int:
    request = json.loads(sys.stdin.read())
    task = request.get("task")
    verdict = FIXTURES.get(task)
    if verdict is None:
        print(f"scripted judge: no fixture for task {task!r}", file=sys.stderr)
        return 4
    print(json.dumps(verdict))
    return 0


if __name__ == "__main__":
    sys.exit(main())
