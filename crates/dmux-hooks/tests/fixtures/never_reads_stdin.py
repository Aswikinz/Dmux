#!/usr/bin/env python3
"""Sleeps without ever reading stdin; the runner must not block writing input."""
import time


def main():
    time.sleep(999)


if __name__ == "__main__":
    main()
