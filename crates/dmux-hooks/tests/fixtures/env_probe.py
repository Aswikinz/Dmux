#!/usr/bin/env python3
"""Reports which sensitive env vars leaked in; the runner must strip them all."""
import json
import os
import sys


def main():
    leaked = [k for k in os.environ if _is_sensitive(k)]
    sys.stdout.write(json.dumps({"findings": []}))
    # Write the leak report to stderr so the test can assert on it.
    sys.stderr.write("LEAK:" + ",".join(sorted(leaked)) + "\n")


def _is_sensitive(key):
    u = key.upper()
    return (
        u.startswith("PG")
        or u.startswith("DMUX_")
        or "PASSWORD" in u
        or "TOKEN" in u
        or "SECRET" in u
    )


if __name__ == "__main__":
    main()
