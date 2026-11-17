#!/usr/bin/env python3
"""A hook that hangs forever; the runner must kill it via the time limit."""
import time


def main():
    time.sleep(999)


if __name__ == "__main__":
    main()
