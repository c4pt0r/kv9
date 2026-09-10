#!/usr/bin/env python3
"""Require publication regressions to reject isolated production-source faults.

Each baseline/mutant/restored triple builds a real probe against a private source
copy. The working repository is never edited. Compiler errors, timeouts and
unrelated runtime failures cannot satisfy the named negative control.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("publication_gate", HERE / "check-segment-publication.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)

FRAMING = '''        if std::fs::metadata(legacy.path())
            .map_err(checkpoint_io)?
            .len()
            == 0
        {
            let empty = WriteBatch::new();
            match self.index.volatile_applied_position() {
                Some(at) => legacy.append_applied(&empty, at)?,
                None => legacy.append(&empty)?,
            }
        }
'''
ELISION = '''            if at.is_none() && batch.is_empty() {
                return Ok(());
            }
'''
CHECKPOINT_PUBLISH = '''        self.poisoned = true;
        self.publish(&next)?;
        self.topology = next;
        self.poisoned = false;
'''
PREMATURE_UNLINK = '''        let previous = self.topology.clone();
        self.topology = next.clone();
        self.poisoned = false;
        self.reclaim_obsolete()?;
        self.topology = previous;
''' + CHECKPOINT_PUBLISH
REUSE_LEGACY = '''        if let Err(error) = migrated.install_as(&path) {
            log.backing = Some(WalBacking::Legacy(legacy));
            return Err(error);
        }
'''
CONTROLS = (
    ("missing-empty-source-frame", "persist.rs", FRAMING, "", "migration-empty", "segment_sync_before",
     "segment directory exists with a truncated WAL topology"),
    ("empty-frame-pins-stream", "persist.rs", ELISION, "", "migration-empty", None,
     "empty-source migration permanently pinned the stream"),
    ("reusable-failed-checkpoint", "wal_stream.rs", CHECKPOINT_PUBLISH,
     CHECKPOINT_PUBLISH.replace("self.poisoned = true", "self.poisoned = false"), "checkpoint", "dirsync_before",
     "failed publication acknowledged a subsequent write"),
    ("premature-covered-unlink", "wal_stream.rs", CHECKPOINT_PUBLISH, PREMATURE_UNLINK, "checkpoint", "rename_before",
     "publication failure changed or deleted a previously selected segment"),
    ("reused-unlinked-legacy-writer", "persist.rs", "        migrated.install_as(&path)?;\n", REUSE_LEGACY,
     "migration", "dirsync_before", "failed publication acknowledged a subsequent write"),
    ("omitted-publication-directory-sync", "wal_stream.rs", ".measure(|| sync_parent(&self.path))?;",
     ".measure(|| Ok::<(), Error>(()))?;", "checkpoint", "dirsync_before",
     "failed publication acknowledged a subsequent write"),
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    repo = HERE.parent
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    copied = output / "source"
    copied.mkdir()
    files = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=repo).split(b"\0")
    for name in sorted(set(files)):
        if not name:
            continue
        relative = Path(name.decode())
        source = repo / relative
        if source.is_file():
            destination = copied / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
    library = output / "segment_publication.so"
    result = gate.run(["cc", "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror", "-shared", "-fPIC",
                       copied / "scripts/faults/segment_publication.c", "-ldl", "-o", library], copied, output / "shim-build")
    gate.require(result.returncode == 0, "syscall shim did not compile")
    baseline_output = output / "baseline-build"
    baseline_output.mkdir()
    baseline = gate.build(copied, baseline_output)
    report = dict(version=1, complete=False, controls=[], shim_sha256=gate.sha(library),
                  baseline_probe_sha256=gate.sha(baseline))
    gate.save(output / "report.json", report)
    for name, filename, old, new, operation, cut, diagnostic in CONTROLS:
        cell = output / name
        cell.mkdir()
        source = copied / "crates/engine/src" / filename
        original = source.read_text()
        gate.require(original.count(old) == 1, f"{name}: source boundary must match exactly once")
        seed_output = cell / "seed"
        seed_output.mkdir()
        seed = seed_output / "data"
        gate.probe(baseline, "seed", operation, seed, "success", seed_output)
        baseline_case = gate.one_case(baseline, library, seed, operation, cut, cell / "baseline-case")
        (cell / "original.rs").write_text(original)
        changed = original.replace(old, new)
        (cell / "mutant.rs").write_text(changed)
        source.write_text(changed)
        mutation_output = cell / "mutant-build"
        mutation_output.mkdir()
        try:
            mutant = gate.build(copied, mutation_output)
            failure = None
            try:
                gate.one_case(mutant, library, seed, operation, cut, cell / "mutant-case")
            except (ValueError, AssertionError) as error:
                failure = str(error)
            gate.require(failure is not None, f"{name}: runtime fault was not detected")
            diagnostics = failure + "\n" + "\n".join(path.read_text() for path in (cell / "mutant-case").rglob("*.stderr"))
            gate.require(diagnostic in diagnostics, f"{name}: failure missed intended diagnostic: {failure}")
            (cell / "mutant-diagnostic.txt").write_text(diagnostics)
        finally:
            source.write_text(original)
        restored_output = cell / "restored-build"
        restored_output.mkdir()
        restored = gate.build(copied, restored_output)
        restored_case = gate.one_case(restored, library, seed, operation, cut, cell / "restored-case")
        gate.require(source.read_text() == original and baseline_case == restored_case,
                     f"{name}: restoration changed source or acceptance")
        record = dict(name=name, source=f"crates/engine/src/{filename}", operation=operation, cut=cut,
                      diagnostic=diagnostic, original_sha256=gate.sha(cell / "original.rs"),
                      mutant_sha256=gate.sha(cell / "mutant.rs"), detected=True, restored=True)
        report["controls"].append(record)
        gate.save(output / "report.json", report)
        print(json.dumps(record), flush=True)
    report["complete"] = True
    gate.save(output / "report.json", report)
    print(f"PASS: {len(report['controls'])} production-source baseline/mutant/restored controls")


if __name__ == "__main__":
    main()
