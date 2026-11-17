#!/usr/bin/env python3
"""Example Dmux hook: check a control-total threshold.

Dmux hooks are opt-in.  When you run `dmux run --hooks <dir>`, every *.py file
in that directory is run (or `dmux hook <results.json> <script|dir>` for a
saved results file).  A hook reads the run's results as JSON on stdin and writes
one JSON object on stdout:

    {"findings": [{"id": str, "severity": "info"|"warning"|"fail",
                   "table": str?, "message": str}]}

Rules (PLAN.md 4.9):
  * The script never receives passwords or connection strings.  The environment
    is stripped to a tiny allow-list, and the results payload is scrubbed.
  * Output is size-limited and parsed strictly; invalid output is a warning.
  * Each script has a 60 s time limit (configurable) and ships its SHA-256 in
    the run log.
  * A "fail" finding stops the run being signed off as PASS.  "info" and
    "warning" findings do not change the verdict.
  * Numbers here are illustrative only: pick a threshold that your own auditors
    agree on before a real migration.

This example reads per-table control totals that a caller placed in the results
JSON ahead of time and raises a failing finding when any total exceeds a fixed
threshold.  It is public sample code and holds no customer-specific logic.
"""

import json
import sys

# An agreed maximum for the control total of the accounts table.  Adjust for a
# real migration; Dmux never computes this number itself.
CONTROL_TOTAL_LIMIT = 1_000_000


def read_control_totals(payload):
    """Pull per-table control totals out of a results payload.

    In a real run the totals come from the mapping's control-total columns,
    keyed by table name.  The hook only sums or compares numbers; it never sees
    row values.
    """
    totals = payload.get("control_totals", {})
    return totals


def main():
    raw = sys.stdin.read()
    if not raw.strip():
        json.dump({"findings": []}, sys.stdout)
        return
    payload = json.loads(raw)

    findings = []
    totals = read_control_totals(payload)

    for table, total in totals.items():
        try:
            value = float(total)
        except (TypeError, ValueError):
            findings.append(
                {
                    "id": "custom.control-total-unreadable",
                    "severity": "warning",
                    "table": table,
                    "message": "control total is not a number and was not checked",
                }
            )
            continue

        if value > CONTROL_TOTAL_LIMIT:
            findings.append(
                {
                    "id": "custom.control-total-over",
                    "severity": "fail",
                    "table": table,
                    "message": (
                        f"control total {value:,.2f} exceeds the agreed "
                        f"limit of {CONTROL_TOTAL_LIMIT:,}"
                    ),
                }
            )
        else:
            findings.append(
                {
                    "id": "custom.control-total-ok",
                    "severity": "info",
                    "table": table,
                    "message": f"control total {value:,.2f} is within the agreed limit",
                }
            )

    json.dump({"findings": findings}, sys.stdout)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
