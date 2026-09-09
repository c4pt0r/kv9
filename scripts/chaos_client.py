#!/usr/bin/env python3
"""Route Chaos setup/probe RPCs; retry only explicit, side-effect-free refusals."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time


def refused(result):
    # kubectl adds exactly this line for a remote exit 1. Any additional error,
    # stdout, timeout, partial write or unknown outcome is terminal.
    return (result.returncode == 1 and not result.stdout and re.fullmatch(
        r"not_leader=true leader_node_id=(?:[1-9][0-9]*|unknown)\n"
        r"(?:command terminated with exit code 1\n)?", result.stderr) is not None)


def route(candidates, timeout, invoke, record, now=time.monotonic, pause=time.sleep):
    if not candidates or timeout <= 0:
        raise ValueError("nonempty candidates and a positive deadline are required")
    deadline = now() + timeout
    attempt = 0
    while (remaining := deadline - now()) > 0:
        candidate = candidates[attempt % len(candidates)]
        attempt += 1
        started = now()
        try:
            result = invoke(candidate, remaining)
        except subprocess.TimeoutExpired:
            record(dict(attempt=attempt, node=candidate[0], address=candidate[1],
                        started=started, ended=now(), remaining=remaining,
                        outcome="unknown_timeout"))
            raise RuntimeError("RPC deadline expired; outcome is unknown and was not retried")
        retry = refused(result)
        record(dict(attempt=attempt, node=candidate[0], address=candidate[1],
                    started=started, ended=now(), remaining=remaining,
                    outcome="refused" if retry else "success" if result.returncode == 0 else "terminal",
                    returncode=result.returncode, stdout=result.stdout, stderr=result.stderr))
        if result.returncode == 0:
            return result.stdout
        if not retry:
            raise RuntimeError("RPC failed without an exclusive NotLeader refusal; no retry: " + result.stderr)
        pause(min(0.1, max(0, deadline - now())))
    raise RuntimeError("routing deadline exhausted by explicit refusals")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--kubectl", default="kubectl")
    p.add_argument("--kubeconfig", required=True)
    p.add_argument("--namespace", required=True)
    p.add_argument("--addresses", required=True, help="comma-separated node=address candidates")
    p.add_argument("--exclude", type=int, default=0)
    p.add_argument("--timeout", type=float, default=25)
    p.add_argument("--evidence", type=Path, required=True)
    p.add_argument("command", nargs=argparse.REMAINDER)
    args = p.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    # Keep this helper's contract narrow: no multi-chunk writes or metadata RPCs.
    if not command or command[0] not in {"raw-put", "raw-get"} or "--addr" in command:
        p.error("only raw-put/raw-get without --addr are supported")
    candidates = [(int(node), addr) for node, addr in
                  (entry.split("=", 1) for entry in args.addresses.split(","))
                  if int(node) != args.exclude]
    token = os.environ["KV9_CLIENT_TOKEN"]
    with args.evidence.open("x") as evidence:
        def record(event):
            evidence.write(json.dumps(event, sort_keys=True) + "\n")
            evidence.flush()

        def invoke(candidate, remaining):
            return subprocess.run([
                args.kubectl, "--kubeconfig", args.kubeconfig, "exec", "-n", args.namespace,
                "kv9-history-client", "--", "env", "KV9_CLIENT_TOKEN=" + token,
                "/usr/local/bin/kv9", "client", command[0], "--addr", candidate[1],
                *command[1:],
            ], capture_output=True, text=True, timeout=remaining)

        sys.stdout.write(route(candidates, args.timeout, invoke, record))


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, ValueError) as error:
        raise SystemExit("FAIL: " + str(error)) from error
