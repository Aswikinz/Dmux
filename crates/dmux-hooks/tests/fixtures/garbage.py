#!/usr/bin/env python3
"""Prints garbage that is not the hook schema — the runner must reject it."""
import sys


def main():
    sys.stdout.write("this is not json\n")
    sys.stderr.write("some diagnostic\n")


if __name__ == "__main__":
    main()