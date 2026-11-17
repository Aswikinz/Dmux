#!/usr/bin/env python3
"""A well-behaved hook: reads results JSON on stdin and returns two findings."""
import json
import sys


def main():
    payload = sys.stdin.read()
    # The payload must be valid JSON for a real run; tolerate empty in tests.
    if payload.strip():
        json.loads(payload)
    findings = [
        {
            "id": "custom.control-total",
            "severity": "info",
            "table": "public.accounts",
            "message": "control total within threshold",
        },
        {
            "id": "custom.signoff",
            "severity": "warning",
            "message": "remember to attach the DBA sign-off",
        },
    ]
    sys.stdout.write(json.dumps({"findings": findings}))
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
