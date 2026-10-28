#!/usr/bin/env python3
"""Check fixed-snapshot pagination, with source hashes and rejecting controls.

This models traversal, not Rust extraction, catalog GC, or scheduling budgets.
--lean-only still parses TLA but cannot mark the combined gate accepted.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
JAR_SHA256 = "936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88"
THEOREMS = [
    "initial_prefix", "advance_preserves_prefix", "run_preserves_prefix",
    "page_bounded", "nonempty_advance_progress", "run_suffix",
    "finite_exhaustion", "complete_history", "lookup_preserved",
    "full_final_page_exhausts",
]
INPUTS = [
    "crates/meta/src/store.rs", "crates/meta/tests/catalog.rs",
    "crates/meta/src/data_groups.rs", "crates/server/src/region_manager/tests.rs",
    *[str(p.relative_to(ROOT)) for p in sorted((ROOT / "crates/meta/src/data_groups").glob("*.rs"))],
    "proofs/lean/task-history/Pagination.lean",
    "proofs/tla/task-history/Pagination.tla", "proofs/tla/task-history/Safety.cfg",
    "scripts/prove-task-history.py",
]


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def clean(source):
    require(not re.search(r"\b(sorry|admit|axiom|unsafe)\b", source), "proof policy violation")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lean", type=Path, required=True)
    parser.add_argument("--jar", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--lean-only", action="store_true")
    args = parser.parse_args()
    args.lean, args.jar, args.output = (p.resolve() for p in (args.lean, args.jar, args.output))
    require(digest(args.jar) == JAR_SHA256, "wrong TLC jar")
    args.output.mkdir(parents=True, exist_ok=False)
    hashes = {name: digest(ROOT / name) for name in INPUTS}
    report = dict(accepted=False, lean_accepted=False, tla_accepted=False,
                  source_sha256=hashes, commands=[], rust_extraction=False,
                  catalog_gc=False, chaos_mesh=False)
    report_path = args.output / "result.json"

    def save():
        report_path.write_text(json.dumps(report, indent=2) + "\n")

    def run(label, command, cwd, env=None):
        result = subprocess.run(command, cwd=cwd, env=env, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        (args.output / f"{label}.log").write_text(result.stdout)
        report["commands"].append(dict(label=label, command=list(map(str, command)),
                                       exit_code=result.returncode))
        save()
        return result

    version = run("lean-version", [str(args.lean), "--version"], ROOT)
    require(version.returncode == 0 and "version 4.33.1" in version.stdout, "wrong Lean version")
    lean_source = (ROOT / "proofs/lean/task-history/Pagination.lean").read_text()
    tla_source = (ROOT / "proofs/tla/task-history/Pagination.tla").read_text()
    config = (ROOT / "proofs/tla/task-history/Safety.cfg").read_text()
    clean(lean_source)
    require(re.findall(r"^theorem (\w+)", lean_source, re.M) == THEOREMS,
            "theorem inventory mismatch")
    (args.output / "Pagination.lean").write_text(lean_source)
    (args.output / "Pagination.tla").write_text(tla_source)

    with tempfile.TemporaryDirectory(prefix="kv9-task-history-proof-") as temporary:
        work = Path(temporary)

        def compile_lean(label, source, positive):
            path = work / "Pagination.lean"
            path.write_text(source)
            result = run(label, [str(args.lean), "-DwarningAsError=true", "-o",
                                str(work / "Pagination.olean"), str(path)], work)
            require((result.returncode == 0) == positive, f"{label}: unexpected proof result")
            if not positive:
                require("unsolved goals" in result.stdout or "Type mismatch" in result.stdout
                        or "omega could not prove" in result.stdout,
                        f"{label}: rejected outside the semantic proof")

        compile_lean("lean-positive", lean_source, True)
        audit = work / "Audit.lean"
        audit.write_text("import Pagination\n" + "\n".join(
            f"#print axioms Kv9.TaskHistory.{name}" for name in THEOREMS) + "\n")
        result = run("lean-axioms", [str(args.lean), str(audit)], work,
                     dict(os.environ, LEAN_PATH=str(work)))
        require(result.returncode == 0 and len(result.stdout.splitlines()) == len(THEOREMS),
                "incomplete axiom audit")
        for line in result.stdout.splitlines():
            if "does not depend on any axioms" not in line:
                names = line.split("[", 1)[1].split("]", 1)[0].split(", ")
                require(set(names) <= {"propext", "Classical.choice", "Quot.sound"}, "untrusted axiom")

        controls = {
            "overlap-page": ("s.emitted ++ s.remaining.take width", "s.emitted ++ s.remaining.take (width + 1)"),
            "skip-row": ("s.remaining.drop width", "s.remaining.drop (width + 1)"),
            "frozen-cursor": ("s.remaining.drop width", "s.remaining"),
            "zero-page-budget": ("(positive : 0 < width)", "(positive : 0 ≤ width)"),
        }
        for label, (before, after) in controls.items():
            require(before in lean_source, f"missing control {label}")
            compile_lean(f"lean-negative-{label}", lean_source.replace(before, after, 1), False)
        for bad in ("axiom bad : False", "theorem bad : False := by sorry"):
            try:
                clean(lean_source + "\n" + bad)
            except RuntimeError:
                pass
            else:
                raise RuntimeError("proof policy control accepted")
        report.update(lean_accepted=True, theorems=THEOREMS,
                      lean_semantic_controls=len(controls), policy_controls=2,
                      lean_sha256=digest(args.lean), jar_sha256=JAR_SHA256)
        (work / "Pagination.tla").write_text(tla_source)
        result = run("tla-parse", ["java", "-cp", str(args.jar), "tla2sany.SANY",
                                   "Pagination.tla"], work)
        require(result.returncode == 0 and "Semantic errors" not in result.stdout
                and "Parse Error" not in result.stdout, "TLA parse failed")
        report["tla_parsed"] = True
        save()
        if args.lean_only:
            require(hashes == {name: digest(ROOT / name) for name in INPUTS}, "sources changed")
            print("TASK_HISTORY_LEAN_ACCEPTED: 10 theorems, 4 semantic controls; TLC gate open")
            return

        def check_tla(label, source, cfg, positive):
            case = work / label
            case.mkdir()
            (case / "Pagination.tla").write_text(source)
            (case / "Run.cfg").write_text(cfg)
            (args.output / f"{label}.cfg").write_text(cfg)
            result = run(label, ["java", "-XX:+UseParallelGC", "-Xmx512m", "-cp", str(args.jar),
                                 "tlc2.TLC", "-tool", "-workers", "1", "-config", "Run.cfg",
                                 "-metadir", str(case / "states"), "Pagination.tla"], case)
            require("TLC2 Version 2.19 of 08 August 2024" in result.stdout, "wrong TLC version")
            if positive:
                require(result.returncode == 0 and "No error has been found" in result.stdout,
                        f"{label}: model not accepted")
            else:
                require(result.returncode != 0 and "Invariant Safe is violated" in result.stdout,
                        f"{label}: missing expected counterexample")

        for width in (1, 2, 3, 128):
            check_tla(f"tlc-page-{width}", tla_source,
                      config.replace("PageSize = 2", f"PageSize = {width}"), True)
        tla_controls = {
            "live-snapshot": ("SubSeq(snapshot, cursor + 1, Limit)", "SubSeq(live, cursor + 1, Limit)"),
            "skip-row": ("SubSeq(snapshot, cursor + 1, Limit)", "SubSeq(snapshot, cursor + 2, Limit)"),
            "frozen-cursor": ("cursor' = Limit", "cursor' = cursor"),
            "early-stop": ("done' = (Limit - cursor < PageSize)", "done' = TRUE"),
        }
        for label, (before, after) in tla_controls.items():
            require(before in tla_source, f"missing TLA control {label}")
            check_tla(f"tlc-negative-{label}", tla_source.replace(before, after, 1), config, False)
        require(hashes == {name: digest(ROOT / name) for name in INPUTS}, "sources changed")
        report.update(accepted=True, tla_accepted=True, tla_semantic_controls=len(tla_controls))
        save()
        print("TASK_HISTORY_MODELS_ACCEPTED")


if __name__ == "__main__":
    main()
