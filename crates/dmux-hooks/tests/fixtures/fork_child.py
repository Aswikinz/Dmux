#!/usr/bin/env python3
"""Forks a long-lived child then exits; the child must also be killed."""
import os
import sys
import time


def main():
    pid = os.fork()
    if pid == 0:
        # Child sleeps well past any sane time limit.
        time.sleep(999)
        sys.exit(0)
    # Parent exits immediately, orphaning the child.
    sys.exit(0)


if __name__ == "__main__":
    main()