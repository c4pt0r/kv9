#!/usr/bin/env python3
"""Fresh, retained Lean CRC proof and exact-source Rust const-table binding; no Cargo."""

if not __debug__:
    raise SystemExit("FAIL: Python assertions must remain enabled (PYTHONOPTIMIZE=0)")

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
PROOF = ROOT / "proofs/lean/crc32"
FOUNDATIONS = {"propext", "Classical.choice", "Quot.sound"}
NAMES = {
    "CRC32": ["masked_index", "bit_mask", "bit_step_xor", "bits_xor", "high_mask_bit", "high_step", "high_byte_shift", "byte_split",
              "byte_transition", "list_equivalence", "table_fold_append", "parts_flatten",
              "checksum_equivalence", "singleton_equivalence", "fragmentation_invariant"],
    "RustTable": ["compiled_table_length", "compiled_table", "compiled_byte_transition",
                  "compiled_list_equivalence", "compiled_parts_equivalence",
                  "compiled_checksum_equivalence", "compiled_fragmentation_invariant"],
}


class Rejected(Exception):
    pass


def require(condition, message):
    if not condition:
        raise Rejected(message)


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def save(path, data):
    Path(path).write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")


def declaration(text, needle):
    require(text.count(needle) == 1, f"source declaration missing/duplicated: {needle}")
    start = text.index(needle)
    cursor = text.index("{", start) + 1
    depth = 1
    while depth and cursor < len(text):
        depth += (text[cursor] == "{") - (text[cursor] == "}")
        cursor += 1
    require(depth == 0, f"unterminated source declaration: {needle}")
    if text[cursor:cursor + 1] == ";":
        cursor += 1
    return text[start:cursor]


def bind(text, expected):
    actual = {needle: declaration(text, needle) for needle in expected}
    for needle, reference in expected.items():
        # These declarations contain no strings/comments: only formatting may differ.
        require(re.sub(r"\s+", "", actual[needle]) == re.sub(r"\s+", "", reference),
                f"source/model contract mismatch: {needle}")
    return actual


def command(argv, work, label, timeout=180, env=None):
    record = {"argv": list(map(str, argv)), "cwd": str(work), "pid": None,
              "started_unix_ns": time.time_ns(), "cpu_affinity": sorted(os.sched_getaffinity(0))}
    with (work / f"{label}.stdout").open("xb") as stdout, (work / f"{label}.stderr").open("xb") as stderr:
        process = subprocess.Popen(argv, cwd=work, env=env, stdout=stdout, stderr=stderr)
        record["pid"] = process.pid
        save(work / f"{label}.command.json", record)
        try:
            code = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
            record.update(timeout=True, exit_code=process.returncode, completed_unix_ns=time.time_ns())
            save(work / f"{label}.command.json", record)
            raise Rejected(f"command timeout: {label}") from None
    record.update(exit_code=code, completed_unix_ns=time.time_ns())
    save(work / f"{label}.command.json", record)
    return code, (work / f"{label}.stdout").read_text() + (work / f"{label}.stderr").read_text()


def compile_table(rustc, source, work):
    source += "\nfn main() {\n"
    source += "    std::hint::black_box((crc32(&[]), crc32_parts(&[])));\n"
    source += '    for (i, value) in CRC32_TABLE.iter().enumerate() { println!("{i} {value:08x}"); }\n}\n'
    (work / "table.rs").write_text(source)
    code, log = command([rustc, "--edition=2021", "-Dwarnings", "-O", "table.rs", "-o", "table"],
                        work, "rustc")
    require(code == 0, f"extracted Rust compilation failed: {log}")
    code, log = command([str(work / "table")], work, "table", timeout=15)
    require(code == 0, "compiled table execution failed")
    lines = log.splitlines()
    require(len(lines) == 256, "compiled table output length mismatch")
    values = []
    for index, line in enumerate(lines):
        match = re.fullmatch(r"([0-9]+) ([0-9a-f]{8})", line)
        require(match is not None and int(match[1]) == index, "compiled table index/value mismatch")
        values.append(int(match[2], 16))
    save(work / "compiled-table.json", values)
    return values


def lean_check(lean, module, text, work):
    queries = "\n".join(f"#print axioms Kv9.CRC32.{name}" for name in NAMES[module])
    (work / f"{module}.lean").write_text(text + "\n" + queries + "\n")
    env = dict(os.environ, LEAN_PATH=str(work))
    code, log = command([lean, "-DwarningAsError=true", "-o", f"{module}.olean", f"{module}.lean"],
                        work, module, timeout=180, env=env)
    require(code == 0, f"Lean rejected {module}: {log[:4000]}")
    axioms = {}
    for name in NAMES[module]:
        qualified = f"Kv9.CRC32.{name}"
        pattern = re.escape(f"'{qualified}'") + (
            r" (?:depends on axioms: \[([^]]*)\]|does not depend on any axioms)")
        matches = list(re.finditer(pattern, log))
        require(len(matches) == 1, f"missing/duplicate axiom report: {qualified}")
        dependencies = {x.strip() for x in (matches[0][1] or "").split(",") if x.strip()}
        require(dependencies <= FOUNDATIONS, f"untrusted axioms: {qualified}: {sorted(dependencies)}")
        axioms[qualified] = sorted(dependencies)
    save(work / f"{module}.axioms.json", axioms)
    return axioms


def table_proof(template, values):
    require(template.count("@TABLE@") == 1, "table template marker mismatch")
    return template.replace("@TABLE@", ",\n".join(f"  0x{x:08x}" for x in values))


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--lean", required=True)
    parser.add_argument("--rustc", default="rustc")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    require(not args.output.exists(), "output must be a fresh directory; earlier attempts remain retained")
    lean = shutil.which(args.lean)
    rustc = shutil.which(args.rustc)
    require(lean and rustc, "Lean and standalone rustc must be available")
    output = args.output.absolute()
    output.mkdir(parents=True)
    result = {"accepted": False, "complete": False, "started_unix_ns": time.time_ns(),
              "scope": "CRC computation equivalence; not whole Rust, WAL recovery, Raft or hardware refinement",
              "controls": []}
    try:
        inputs = [PROOF / "CRC32.lean", PROOF / "RustTable.lean.in", PROOF / "source-contract.json",
                  ROOT / "proofs/lean/lean-toolchain", ROOT / "crates/engine/src/wal.rs", Path(__file__).resolve()]
        hashes = {str(p): digest(p) for p in inputs}
        snapshots = output / "inputs"
        snapshots.mkdir()
        for path in inputs:
            shutil.copyfile(path, snapshots / path.name)
        save(output / "source-inputs.json", hashes)
        contract = json.loads((snapshots / "source-contract.json").read_text())
        core = (snapshots / "CRC32.lean").read_text()
        template = (snapshots / "RustTable.lean.in").read_text()
        expected_version = (snapshots / "lean-toolchain").read_text().strip().split(":v")[1]
        code, version = command([lean, "--version"], output, "lean-version", timeout=15)
        require(code == 0 and version.startswith(f"Lean (version {expected_version},"), "Lean version mismatch")
        code, rust_version = command([rustc, "--version", "--verbose"], output, "rustc-version", timeout=15)
        require(code == 0, "rustc version probe failed")
        code, sysroot = command([rustc, "--print", "sysroot"], output, "rustc-sysroot", timeout=15)
        require(code == 0, "rustc sysroot probe failed")
        compiler = Path(sysroot.strip()) / "bin/rustc"
        require(compiler.is_file(), "actual rustc executable unavailable")
        rustc = str(compiler.resolve())
        result["toolchain"] = {"lean": lean, "lean_sha256": digest(lean), "version": version.strip(),
                               "rustc": rustc, "rustc_sha256": digest(Path(rustc).resolve()),
                               "rustc_version": rust_version.strip()}
        code, baseline = command(["git", "-C", str(ROOT), "show",
                                  f"{contract['baseline_revision']}:{contract['source']}"], output, "baseline")
        require(code == 0, "pinned baseline source unavailable")
        bind(baseline, contract["baseline"])
        candidate = (snapshots / "wal.rs").read_text()
        declarations = bind(candidate, contract["candidate"])
        rust_source = "\n\n".join(declarations.values()) + "\n"
        positive = output / "positive"
        positive.mkdir()
        values = compile_table(rustc, rust_source, positive)
        axioms = lean_check(lean, "CRC32", core, positive)
        axioms.update(lean_check(lean, "RustTable", table_proof(template, values), positive))
        result["theorems"] = axioms
        result["compiled_table_entries"] = len(values)

        # A real alternative polynomial compiles; its table must not prove IEEE equivalence.
        negative = output / "control-wrong-polynomial"
        negative.mkdir()
        require(rust_source.count("0xEDB8_8320") == 1, "polynomial control anchor mismatch")
        bad_values = compile_table(rustc, rust_source.replace("0xEDB8_8320", "0x82F6_3B78"), negative)
        require(bad_values != values, "polynomial control failed to alter the compiled table")
        shutil.copyfile(positive / "CRC32.olean", negative / "CRC32.olean")
        try:
            lean_check(lean, "RustTable", table_proof(template, bad_values), negative)
        except Rejected as error:
            require("Lean rejected RustTable" in str(error) and "decide" in str(error),
                    f"polynomial control failed outside the finite table proof: {error}")
            result["controls"].append({"name": "compiled wrong polynomial", "rejected": True, "reason": str(error)})
        else:
            raise Rejected("wrong polynomial accepted")

        anchor = "  unfold oldByte\n  rw [byte_split, bits_xor, high_byte_shift]\n  rfl"
        require(core.count(anchor) == 1, "proof control anchor mismatch")
        for name, mutant, marker in [
            ("proof-hole", core.replace(anchor, "  sorry"), "declaration uses `sorry`"),
            ("custom-axiom", core.replace("namespace Kv9.CRC32", "namespace Kv9.CRC32\naxiom fake : False")
             .replace(anchor, "  exact False.elim fake"), "untrusted axioms"),
        ]:
            directory = output / f"control-{name}"
            directory.mkdir()
            try:
                lean_check(lean, "CRC32", mutant, directory)
            except Rejected as error:
                require(marker in str(error), f"{name} failed outside intended control: {error}")
                result["controls"].append({"name": name, "rejected": True, "reason": str(error)})
            else:
                raise Rejected(f"control accepted: {name}")

        for name, mutant in [
            ("unmapped shift", candidate.replace("crc >> 8", "crc >> 7")),
            ("missing complement", candidate.replace("    !crc\n}", "    crc\n}")),
        ]:
            require(mutant != candidate, f"source control anchor mismatch: {name}")
            directory = output / ("control-" + name.replace(" ", "-"))
            directory.mkdir()
            (directory / "wal-mutant.rs").write_text(mutant)
            try:
                bind(mutant, contract["candidate"])
            except Rejected as error:
                save(directory / "rejection.json", {"rejected": True, "reason": str(error)})
                result["controls"].append({"name": name, "rejected": True, "reason": str(error)})
            else:
                raise Rejected(f"source control accepted: {name}")
        require(all(digest(p) == hashes[str(p)] for p in inputs), "input changed during gate")
        result.update(accepted=True, complete=True, checked_theorems=len(axioms))
    except (Rejected, OSError, subprocess.SubprocessError) as error:
        result.update(complete=True, failure=str(error))
        raise
    finally:
        result["completed_unix_ns"] = time.time_ns()
        save(output / "result.json", result)
        inventory = {str(p.relative_to(output)): {"bytes": p.stat().st_size, "sha256": digest(p)}
                     for p in sorted(output.rglob("*")) if p.is_file() and p.name != "inventory.json"}
        save(output / "inventory.json", inventory)
    print(f"PASS: {len(axioms)} kernel-checked theorems, 256 compiled entries, {len(result['controls'])} rejected controls")


if __name__ == "__main__":
    try:
        main()
    except (Rejected, OSError, subprocess.SubprocessError) as error:
        raise SystemExit(f"FAIL: {error}") from error
