#!/usr/bin/env python3
"""Emits far more stdout than the runner's 1 MiB cap — must be rejected."""
import sys


def main():
    chunk = "x" * (1024 * 1024)  # 1 MiB in one write
    for _ in range(8):
        sys.stdout.write(chunk)


if __name__ == "__main__":
    main()