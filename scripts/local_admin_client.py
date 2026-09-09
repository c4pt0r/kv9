#!/usr/bin/env python3
"""Route local membership CLI calls only after an exclusive typed refusal."""
import argparse
from pathlib import Path
import json
import subprocess
import sys

from chaos_client import route


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin', required=True)
    parser.add_argument('--addresses', required=True)
    parser.add_argument('--timeout', type=float, default=25)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    if (not command or command[0] not in {'admit-node', 'promote-node'}
            or '--addr' in command):
        parser.error('only admit-node/promote-node without --addr are supported')
    candidates = [(int(node), address) for node, address in
                  (entry.split('=', 1) for entry in args.addresses.split(','))]
    if any(node <= 0 or not address for node, address in candidates):
        parser.error('positive node identities and nonempty addresses are required')
    with args.evidence.open('x') as evidence:
        def record(event):
            # Successful AdmitNode stdout contains a one-time ticket. Its
            # caller owns that output; routing diagnostics need only the cut.
            if event['outcome'] == 'success':
                event = dict(event, stdout='[successful response retained by caller]')
            evidence.write(json.dumps(event, sort_keys=True) + '\n')
            evidence.flush()

        def invoke(candidate, remaining):
            return subprocess.run(
                [args.bin, 'client', command[0], '--addr', candidate[1], *command[1:]],
                capture_output=True, text=True, timeout=remaining)

        sys.stdout.write(route(candidates, args.timeout, invoke, record))


if __name__ == '__main__':
    try:
        main()
    except (RuntimeError, ValueError) as error:
        raise SystemExit('FAIL: ' + str(error)) from error
