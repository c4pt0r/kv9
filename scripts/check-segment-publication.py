#!/usr/bin/env python3
"""Inject real Linux syscall errors at segmented WAL publication boundaries.

This checks production I/O/error paths and explicitly constructed allowed
namespace outcomes. It is not a physical power-cut simulator, a filesystem
proof, or Chaos Mesh. The stream probe's checkpoint restore is a stated fixture
premise; actual Raft/MinIO authority is tested by the separate process fixture.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

from wal_layout import read_layout, wal_snapshot

OPERATIONS = ("rotation", "checkpoint", "migration", "migration-empty")
PUBLICATION_CUTS = ("rename_before", "rename_after", "dirsync_before", "dirsync_after")
HOUSEKEEPING_CUTS = ("unlink_before", "unlink_after", "unlink_dirsync_before", "unlink_dirsync_after")
STAGING_CUTS = ("segment_sync_before", "segment_sync_after")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def run(argv, cwd, log, env=None):
    process = subprocess.run(list(map(str, argv)), cwd=cwd, env=env,
                             text=True, capture_output=True, timeout=90)
    log.with_suffix(".stdout").write_text(process.stdout)
    log.with_suffix(".stderr").write_text(process.stderr)
    save(log.with_suffix(".command.json"), dict(argv=list(map(str, argv)), exit=process.returncode))
    return process


def build(repo, output):
    result = run(["cargo", "build", "--locked", "-p", "kv9-engine", "--example",
                  "segment_publication_probe", "--message-format=json-render-diagnostics"],
                 repo, output / "probe-build")
    require(result.returncode == 0, "publication probe build failed; see retained compiler output")
    records = [json.loads(line) for line in result.stdout.splitlines()]
    paths = {row["executable"] for row in records if row.get("reason") == "compiler-artifact"
             and row.get("target", {}).get("name") == "segment_publication_probe" and row.get("executable")}
    require(len(paths) == 1, "Cargo did not identify exactly one publication probe")
    binary = output / "segment_publication_probe"
    shutil.copy2(paths.pop(), binary)
    return binary


def probe(binary, action, operation, directory, expected, output, env=None):
    result = run([binary, action, operation, directory, expected], output,
                 output / (action + "-" + operation), env)
    require(result.returncode == 0, f"{action} {operation} failed; see {output}")
    receipt = json.loads(result.stdout)
    require(receipt["action"] == action and receipt["operation"] == operation,
            "probe receipt differs from requested operation")
    return receipt


def durable_root(path, data):
    # Materialize one explicitly chosen namespace outcome in an isolated copy.
    # This is an allowed-outcome fixture, not a claim that host fsync was undone.
    with path.open("wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    descriptor = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def one_case(binary, library, seed, operation, cut, output):
    output.mkdir()
    directory = output / "data"
    shutil.copytree(seed, directory)
    before = wal_snapshot(directory)
    expected = "success" if cut is None else "housekeeping" if cut in HOUSEKEEPING_CUTS else "fenced"
    env = dict(os.environ)
    env.pop("LD_PRELOAD", None)
    env.update(LD_PRELOAD=str(library), KV9_PUBLICATION_TARGET=str(directory / "catalog.wal"),
               KV9_PUBLICATION_PARENT=str(directory), KV9_PUBLICATION_SEGMENTS=str(directory / "catalog.segments"),
               KV9_PUBLICATION_OLD_ROOT=str(output / "old-root.bin"),
               KV9_PUBLICATION_TRACE=str(output / "syscalls.jsonl"), KV9_PUBLICATION_CUT=cut or "none")
    receipt = probe(binary, "attempt", operation, directory, expected, output, env)
    events = [json.loads(line) for line in (output / "syscalls.jsonl").read_text().splitlines()]
    if cut:
        require(sum(row["event"] == cut for row in events) == 1, "requested syscall fault did not occur exactly once")
        require(receipt["operation_ok"] is False and "os error 5" in receipt["operation_error"],
                "publication failure was not the injected EIO")
    require(receipt["write_acknowledged"] == (expected != "fenced"), "subsequent write ignored publication certainty")
    after = wal_snapshot(directory)
    # Empty-source migration first frames the old source durably. Capture the
    # actual predecessor bytes immediately before rename, not the initial empty
    # file whose contents already changed through a successful file fsync.
    old_root = (output / "old-root.bin").read_bytes()
    new_root = (directory / "catalog.wal").read_bytes()
    if cut in (*PUBLICATION_CUTS, *STAGING_CUTS):
        for name, original in before.items():
            if name != "catalog.wal":
                require(after.get(name) == original, "publication failure changed or deleted a previously selected segment")
        if cut == "rename_before" or cut in STAGING_CUTS:
            require(new_root == old_root, "failed pre-rename publication changed the selected root")
        else:
            require(new_root != old_root, "post-rename failure did not publish a new visible root")
    if cut in HOUSEKEEPING_CUTS:
        require(any(row["event"] == "publication_dirsync_succeeded" for row in events),
                "housekeeping ran without a successful publication directory fsync")
        require(read_layout(directory).checkpoint["index"] == 7, "housekeeping lost its durable checkpoint")
    outcomes = {}
    # Before rename only the old root is legal. After a successful directory
    # fsync only the new root is legal. In the intervening ambiguous window this
    # fixture materializes both complete roots under the stated rename premise.
    choices = {"visible": new_root}
    if cut in ("rename_after", "dirsync_before"):
        choices["old_namespace"] = old_root
    for name, root in choices.items():
        recovery = output / name
        recovery.mkdir()
        clone = recovery / "data"
        shutil.copytree(directory, clone)
        durable_root(clone / "catalog.wal", root)
        outcomes[name] = probe(binary, "recover", operation, clone, expected, recovery)
    if cut in ("unlink_after", "unlink_dirsync_before"):
        missing = [name for name in before if name not in after]
        require(missing, "unlink uncertainty case removed no physical file")
        for damaged in (False, True):
            name = "resurrected_covered_corrupt" if damaged else "resurrected_covered"
            recovery = output / name
            recovery.mkdir()
            clone = recovery / "data"
            shutil.copytree(directory, clone)
            for relative in missing:
                data = bytearray((seed / relative).read_bytes())
                require(data.startswith(b"KV9SEG01") and len(data) > 60, "resurrection needs actual old record bytes")
                if damaged:
                    data[-1] ^= 1
                durable_root(clone / relative, data)
            require((clone / "catalog.wal").read_bytes() == new_root, "resurrection changed committed checkpoint authority")
            outcomes[name] = probe(binary, "recover", operation, clone, expected, recovery)
    save(output / "case.json", dict(operation=operation, cut=cut, expected=expected, before=before,
                                    after=after, events=events, receipt=receipt, outcomes=outcomes))
    return dict(operation=operation, cut=cut, namespace_outcomes=list(outcomes), expected=expected)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--probe", type=Path)
    parser.add_argument("--minio", action="store_true", help="also require real MinIO for empty-checkpoint-tail migration")
    args = parser.parse_args()
    require(sys.platform == "linux", "syscall injection requires Linux and a dynamically linked glibc probe")
    repo = Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    binary = args.probe.resolve() if args.probe else build(repo, output)
    library = output / "segment_publication.so"
    result = run(["cc", "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror", "-shared", "-fPIC",
                  repo / "scripts/faults/segment_publication.c", "-ldl", "-o", library], repo, output / "shim-build")
    require(result.returncode == 0, "syscall shim build failed; see retained compiler output")
    report = dict(version=1, complete=False, probe_sha256=sha(binary), shim_sha256=sha(library), cases=[],
                  premises=["One owned local writer", "Successful fsync preserves prior acknowledged bytes",
                            "Atomic same-directory rename selects a complete old or new root before directory fsync",
                            "Checkpoint authority and exact state are simulated in the stream-only probe"],
                  controls=[], limits=["Not a physical power-loss emulator or filesystem proof", "Not actual Chaos Mesh",
                                      "Does not certify real Raft/MinIO checkpoint authority"])
    save(output / "report.json", report)
    operations = (*OPERATIONS, *(("migration-checkpoint",) if args.minio else ()))
    report["real_minio_checkpoint_migration"] = args.minio
    for operation in operations:
        seed_output = output / (operation + "-seed")
        seed_output.mkdir()
        seed = seed_output / "data"
        probe(binary, "seed", operation, seed, "success", seed_output)
        for cut in (None, *PUBLICATION_CUTS, *(HOUSEKEEPING_CUTS if operation == "checkpoint" else ()),
                    *(STAGING_CUTS if operation.startswith("migration") else ())):
            case = one_case(binary, library, seed, operation, cut, output / (operation + "-" + (cut or "baseline")))
            report["cases"].append(case)
            save(output / "report.json", report)
            print(json.dumps(case), flush=True)
    # A no-injection run cannot satisfy a fault receipt. Requiring an EIO in
    # addition to a nonzero probe result rules out unrelated startup failures.
    for name in ("missing-shim", "wrong-target"):
        control = output / name
        control.mkdir()
        directory = control / "data"
        shutil.copytree(output / "checkpoint-seed/data", directory)
        env = dict(os.environ)
        env.pop("LD_PRELOAD", None)
        if name == "wrong-target":
            env.update(LD_PRELOAD=str(library), KV9_PUBLICATION_TARGET=str(directory / "unrelated.wal"),
                       KV9_PUBLICATION_CUT="dirsync_before", KV9_PUBLICATION_PARENT=str(directory),
                       KV9_PUBLICATION_TRACE=str(control / "syscalls.jsonl"))
        result = run([binary, "attempt", "checkpoint", directory, "fenced"], repo, control / "probe", env)
        require(result.returncode != 0 and "failed publication" in result.stderr,
                "fault harness control did not reject missing injection at the intended guard")
        report["controls"].append(dict(name=name, rejected=True))
    report["complete"] = True
    save(output / "report.json", report)
    print(f"PASS: {len(report['cases'])} publication cases; {len(report['controls'])} harness controls")


if __name__ == "__main__":
    main()
