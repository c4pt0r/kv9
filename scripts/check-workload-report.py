#!/usr/bin/env python3
"""Verify a whole workload run, retained build and (when present) full history."""
import argparse
import json
from pathlib import Path
import sys

from workload_report import validate


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path)
    parser.add_argument("--build", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--expected-revision")
    parser.add_argument("--seconds", type=float, default=30)
    args = parser.parse_args()
    if not 0 < args.seconds <= 300:
        parser.error("checker time must be within (0, 300] seconds")
    try:
        result = validate(args.run, args.build, args.expected_revision, args.seconds)
    except (ValueError, TypeError, KeyError, IndexError, OSError, RecursionError) as error:
        result = dict(version=1, accepted=False, reason=str(error))
    with args.output.open("x") as stream:
        json.dump(result, stream, indent=2)
        stream.write("\n")
    if not result["accepted"]:
        sys.exit("FAIL: workload artifact verification: " + result["reason"])
    print("PASS: workload artifacts verified; full history independently checked=" +
          str(result["full_history_independently_checked"]).lower())


if __name__ == "__main__":
    main()
