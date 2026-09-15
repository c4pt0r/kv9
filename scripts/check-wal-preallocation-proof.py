#!/usr/bin/env python3
"""Check capacity arithmetic, exact Rust source mapping, and rejection controls."""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import time

ROOT = Path(__file__).resolve().parents[1]
PROOF = ROOT / "proofs/lean/wal-preallocation"
NAMES = [
    "u32le_length", "mutation_size", "body_size", "payload_size",
    "emission_bytes", "capacity_independent_bytes", "sufficient_reservation",
    "no_capacity_growth", "batch_reservation", "checked_frame_subtraction",
    "validated_lengths",
]
FOUNDATIONS = {"propext", "Classical.choice", "Quot.sound"}

spec = importlib.util.spec_from_file_location("crc_helpers", ROOT / "scripts/check-crc32-proof.py")
helpers = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helpers)
require, save, command = helpers.require, helpers.save, helpers.command


def digest(data):
    return hashlib.sha256(data).hexdigest()


def bind_sources(sources, contract):
    for name, expected in contract["sources"].items():
        text = sources[name]
        for needle, declaration in expected.get("declarations", {}).items():
            require(helpers.declaration(text, needle) == declaration,
                    f"source declaration mismatch: {name}: {needle}")
        require(digest(text.encode()) == expected["sha256"], f"source hash mismatch: {name}")


def check_mapping(sources, baseline):
    wal = "crates/engine/src/wal.rs"
    segment = "crates/engine/src/wal_segment.rs"
    declaration = helpers.declaration
    old = declaration(baseline[wal], "pub(crate) fn encode_batch(")
    old = old.replace("fn encode_batch(batch: &WriteBatch)",
                      "fn encode_batch_with_capacity(batch: &WriteBatch, capacity: usize)")
    old = old.replace("Vec::new()", "Vec::with_capacity(capacity)")
    require(old == declaration(sources[wal], "pub(crate) fn encode_batch_with_capacity("),
            "encoder changed beyond initial capacity and signature")
    require(declaration(sources[wal], "pub(crate) fn encode_batch(") ==
            "pub(crate) fn encode_batch(batch: &WriteBatch) -> Vec<u8> {\n"
            "    encode_batch_with_capacity(batch, 0)\n}", "default capacity changed")
    for needle in ["fn put_u32(", "fn cf_code("]:
        require(declaration(sources[wal], needle) == declaration(baseline[wal], needle),
                "wire primitive changed: " + needle)
    require(declaration(sources[segment], "pub(crate) fn encoded_size(") ==
            declaration(baseline[segment], "pub(crate) fn encoded_size("), "size validation changed")
    old = declaration(baseline[segment], "pub fn append(")
    new = declaration(sources[segment], "pub fn append(")
    start, end = new.index("        let _frame_size"), new.index("        let length")
    replacement = "        encoded_size(batch)?;\n        let payload = encode_batch(batch);\n"
    require(new[:start] + replacement + new[end:] == old,
            "append guards, framing, writes, sync or publication changed")
    require("pub const FRAME_HEADER_BYTES: usize = 32;" in sources[segment], "frame size changed")
    require("pub const MAX_RECORD_LEN: u32 = 64 * 1024 * 1024;" in
            sources["crates/engine/src/wal_v2.rs"], "record limit changed")


def check_lean(lean, source, work):
    work.mkdir()
    require(re.findall(r"^theorem (\w+)", source, re.M) == NAMES, "theorem inventory changed")
    queries = "\n".join(f"#print axioms Kv9.WalCapacity.{name}" for name in NAMES)
    (work / "Capacity.lean").write_text(source + "\n" + queries + "\n")
    code, log = command([lean, "-DwarningAsError=true", "-o", "Capacity.olean", "Capacity.lean"],
                        work, "lean", env=dict(os.environ, LEAN_PATH=str(work)))
    require(code == 0, "Lean rejected capacity proof: " + log[:3000])
    axioms = {}
    for name in NAMES:
        qualified = "Kv9.WalCapacity." + name
        pattern = re.escape(f"'{qualified}'") + (
            r" (?:depends on axioms: \[([^]]*)\]|does not depend on any axioms)")
        matches = list(re.finditer(pattern, log))
        require(len(matches) == 1, "missing/duplicate axiom report: " + qualified)
        dependencies = {x.strip() for x in (matches[0][1] or "").split(",") if x.strip()}
        require(dependencies <= FOUNDATIONS, "untrusted axioms: " + qualified)
        axioms[qualified] = sorted(dependencies)
    save(work / "axioms.json", axioms)
    return axioms


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--lean", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    lean = shutil.which(args.lean)
    require(lean, "Lean required")
    output = args.output.resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    result = {"accepted": False, "complete": False, "started_ns": time.time_ns(),
              "scope": "Capacity and byte-sequence model with reviewed exact-source mapping; "
                       "not a mechanized Rust/compiler/allocator/Raft/I/O refinement.", "controls": []}
    inputs = [Path(__file__).resolve(), ROOT / "scripts/check-crc32-proof.py",
              PROOF / "source-contract.json", PROOF / "Capacity.lean", ROOT / "proofs/lean/lean-toolchain"]
    try:
        contract = json.loads((PROOF / "source-contract.json").read_text())
        inputs += [ROOT / name for name in contract["sources"]]
        before = {str(p.relative_to(ROOT)): digest(p.read_bytes()) for p in inputs}
        snapshots = output / "inputs"
        for path in inputs:
            destination = snapshots / path.relative_to(ROOT)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, destination)
        save(output / "source-inputs.json", before)
        code, version = command([lean, "--version"], output, "version", timeout=15)
        expected = (ROOT / "proofs/lean/lean-toolchain").read_text().strip().split(":v")[1]
        require(code == 0 and version.startswith(f"Lean (version {expected},"), "Lean version mismatch")
        result["toolchain"] = {"path": lean, "sha256": digest(Path(lean).read_bytes()),
                               "version": version.strip()}
        sources = {name: (snapshots / name).read_text() for name in contract["sources"]}
        bind_sources(sources, contract)
        baseline = {}
        for i, (name, entry) in enumerate(contract["sources"].items()):
            if "baseline_sha256" not in entry:
                continue
            code, text = command(["git", "-C", str(ROOT), "show",
                                  contract["baseline_revision"] + ":" + name], output, f"baseline-{i}")
            require(code == 0 and digest(text.encode()) == entry["baseline_sha256"],
                    "baseline source unavailable or changed: " + name)
            baseline[name] = text
        check_mapping(sources, baseline)
        proof = (snapshots / "proofs/lean/wal-preallocation/Capacity.lean").read_text()
        require(digest(proof.encode()) == contract["proof_sha256"], "reviewed proof changed")
        result["theorems"] = check_lean(lean, proof, output / "positive")

        def rejected(name, action, marker):
            try:
                action()
            except helpers.Rejected as error:
                require(marker in str(error), f"{name} rejected outside intended check: {error}")
                result["controls"].append({"name": name, "rejected": True, "reason": str(error)})
            else:
                raise helpers.Rejected("invalid control accepted: " + name)

        for name, old, new in [
            ("undercount-put", "=> 10 + key.length", "=> 9 + key.length"),
            ("wrong-frame-subtraction", "payload + 32 + 4 - 32 - 4", "payload + 32 + 4 - 31 - 4"),
            ("proof-hole", "payload + 32 + 4 - 32 - 4 = payload := by omega",
             "payload + 32 + 4 - 32 - 4 = payload := by sorry"),
        ]:
            require(proof.count(old) == 1, "control anchor mismatch: " + name)
            changed = proof.replace(old, new)
            rejected(name, lambda: check_lean(lean, changed, output / ("control-" + name)),
                     "Lean rejected capacity proof")
        changed = proof.replace("namespace Kv9.WalCapacity", "namespace Kv9.WalCapacity\naxiom fake : False")
        changed = changed.replace("payload + 32 + 4 - 32 - 4 = payload := by omega",
                                  "payload + 32 + 4 - 32 - 4 = payload := False.elim fake")
        rejected("custom-axiom", lambda: check_lean(lean, changed, output / "control-custom-axiom"),
                 "untrusted axioms")
        for name, path, old, new in [
            ("wrong-tag", "crates/engine/src/wal.rs", "out.push(0);", "out.push(1);"),
            ("wrong-reservation", "crates/engine/src/wal_segment.rs", "_frame_size - FRAME_HEADER_BYTES as u64 - 4",
             "_frame_size - FRAME_HEADER_BYTES as u64 - 3"),
            ("missing-sync", "crates/engine/src/wal_segment.rs", "self.file.sync_all()", "Ok(())"),
        ]:
            require(old in sources[path], "control anchor absent: " + name)
            changed_sources = dict(sources, **{path: sources[path].replace(old, new)})
            (output / ("control-" + name + ".rs")).write_text(changed_sources[path])
            rejected(name, lambda: bind_sources(changed_sources, contract), "source declaration mismatch")
        require(all(digest((ROOT / name).read_bytes()) == value for name, value in before.items()),
                "inputs changed during proof execution")
        result.update(accepted=True, complete=True)
    except Exception as error:
        result["failure"] = repr(error)
        raise
    finally:
        result["finished_ns"] = time.time_ns()
        save(output / "result.json", result)
    print(json.dumps({"accepted": True, "theorems": len(result["theorems"]),
                      "rejection_controls": len(result["controls"]), "output": str(output)}))


if __name__ == "__main__":
    main()
