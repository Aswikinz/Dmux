#!/usr/bin/env python3
"""Spawns a grandchild that escapes the process group via setsid, keeps stdout
open, and sleeps; the parent exits 0 first.  The runner must return promptly
(with a hook warning) rather than hang joining a reader stuck on the pipe.
"""
import os
import sys
import time


def main():
    pid = os.fork()
    if pid == 0:
        # Detach from the process group so the runner's group kill cannot reach
        # this process, then keep stdout open and sleep past any sane limit.
        os.setsid()
        # Keep stdout open by not closing it; sleep a long time.
        time.sleep(999)
        sys.exit(0)
    # Parent exits 0 immediately.
    sys.exit(0)


if __name__ == "__main__":
    main()
