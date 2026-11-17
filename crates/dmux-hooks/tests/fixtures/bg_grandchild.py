#!/usr/bin/env python3
"""Spawns a background grandchild that keeps stdin+stdout open, then exits 0.

The runner must return promptly: once the parent exits, it kills the process
group, so the grandchild cannot keep the run alive.
"""
import os
import sys
import time


def main():
    pid = os.fork()
    if pid == 0:
        # Grandchild: keep stdin/stdout inherited (open) and sleep a long time.
        # We exec a process that ignores everything and just sleeps.
        os.execlp("sleep", "sleep", "999")  # noqa: keeps the fds open
        sys.exit(0)
    # Parent exits 0 quickly; the grandchild stays behind.
    sys.exit(0)


if __name__ == "__main__":
    main()
