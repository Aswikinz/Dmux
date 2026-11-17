#!/usr/bin/env python3
"""Returns one failing finding: the overall verdict must not stay PASS."""
import json
import sys


def main():
    payload = sys.stdin.read()
    if payload.strip():
        json.loads(payload)
    sys.stdout.write(
        json.dumps(
            {
                "findings": [
                    {
                        "id": "custom.control-total-over",
                        "severity": "fail",
                        "table": "public.transactions",
                        "message": "control total exceeds the agreed threshold",
                    }
                ]
            }
        )
    )
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
