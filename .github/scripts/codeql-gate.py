#!/usr/bin/env python3
"""Fail on CodeQL results when GitHub cannot store them.

A private repository without GitHub Code Security cannot receive code
scanning uploads, so codeql.yml analyzes without uploading and runs this
gate on the SARIF output instead. It applies GitHub's default code scanning
check policy: fail on security alerts of high or critical severity, and on
other alerts at error level. Every result is printed as an annotation.

    python3 .github/scripts/codeql-gate.py <sarif-dir>
"""

import json
import pathlib
import sys

HIGH = 7.0  # GitHub's threshold for a "high" security severity


def rules_by_id(run: dict) -> dict:
    rules = {}
    tool = run.get("tool", {})
    for component in [tool.get("driver", {}), *tool.get("extensions", [])]:
        for rule in component.get("rules", []):
            rules[rule.get("id")] = rule
    return rules


def location(result: dict) -> tuple[str, int]:
    for loc in result.get("locations", []):
        phys = loc.get("physicalLocation", {})
        uri = phys.get("artifactLocation", {}).get("uri", "?")
        line = phys.get("region", {}).get("startLine", 1)
        return uri, line
    return "?", 1


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    files = sorted(pathlib.Path(sys.argv[1]).glob("*.sarif"))
    if not files:
        print("::error::no SARIF output to gate on")
        return 1
    total, blocking = 0, 0
    for path in files:
        for run in json.loads(path.read_text()).get("runs", []):
            rules = rules_by_id(run)
            for result in run.get("results", []):
                if result.get("suppressions"):
                    continue
                total += 1
                rule_id = result.get("ruleId", "?")
                rule = rules.get(rule_id, {})
                props = rule.get("properties", {})
                level = result.get("level") or rule.get("defaultConfiguration", {}).get(
                    "level", "warning"
                )
                try:
                    severity = float(props.get("security-severity", ""))
                except ValueError:
                    severity = None
                blocks = severity >= HIGH if severity is not None else level == "error"
                blocking += blocks
                uri, line = location(result)
                text = result.get("message", {}).get("text", "").replace("\n", " ")
                kind = "error" if blocks else "warning"
                sev = f" severity {severity}" if severity is not None else ""
                print(f"::{kind} file={uri},line={line}::{rule_id} ({level}{sev}): {text}")
    print(f"{total} CodeQL result(s), {blocking} blocking")
    return 1 if blocking else 0


if __name__ == "__main__":
    sys.exit(main())
