#!/usr/bin/env python3
"""Validate debugger-decoded trees against independent inputs before comparing layouts."""
import argparse
from collections import Counter
import hashlib
import importlib.util
import json
from pathlib import Path


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--experiment", type=Path, required=True)
    args = parser.parse_args()
    root = args.experiment.resolve()
    used = {}

    def read(path):
        used[str(path)] = sha(path)
        return json.loads(path.read_text())

    summary = read(root / "capture-summary-v3.json")
    assert summary["complete"] and len(summary["cases"]) == 4
    spec = importlib.util.spec_from_file_location("inputs", root / "validate-inputs.py")
    inputs = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(inputs)
    models, metadata = inputs.reconstruct()
    model = models["original24", "overwrite"]
    accepted = read(root / "runs/inputs-accepted.json")
    assert accepted["complete"] and accepted["models"] == metadata
    prepared_path = root / "runs/prepare-baseline.json"
    prepared = read(prepared_path)
    assert sha(prepared_path) == accepted["prepared_sha256"]["baseline"]
    probes = next(w for w in prepared["read_inputs"]
                  if w["dataset"] == "original24" and w["workload"] == "overwrite")
    first_key = next(q for q in probes["queries"] if q["operation"] == "get_hit")["queries_hex"][0]
    result = {"complete": False, "snapshots": {}, "comparisons": {},
              "performance_evidence": False, "decoded_nodes": 0}
    normalized = {}
    for run in summary["cases"]:
        assert run["complete"] and run["exit_code"] == 0
        directory = root / ("layout-v3-" + run["name"])
        path = directory / "capture.json"
        assert sha(path) == run["capture_sha256"]
        capture = read(path)
        assert capture["complete"] and capture["binary_sha256"] == run["binary_sha256"]
        assert capture["query_hex"] == first_key
        pin = read(root / ("codegen-" + run["name"]) / "result.json")
        assert sha(Path(pin["binary"])) == capture["binary_sha256"]
        done = read(directory / "debugger-result.json")
        assert done["complete"] and sha(directory / "debugger-result.json") == run["debugger_result_sha256"]
        nodes = capture["nodes"]
        by_address = {node["node"]: node for node in nodes}
        assert len(nodes) == len(by_address) == 4096
        assert len({n["path"] for n in nodes}) == len({n["key_hex"] for n in nodes}) == 4096
        assert all(n["node_count"] >= 1 and n["entry_count"] >= 1 for n in nodes)
        decoded = {bytes.fromhex(n["key_hex"]): bytes.fromhex(n["value_hex"]) for n in nodes}
        decoded_rows = sorted(decoded.items())
        assert decoded_rows == model
        assert inputs.digest(decoded_rows) == next(m["final_state_sha256"] for m in metadata
                                             if m["dataset"] == "original24" and m["workload"] == "overwrite")
        visited = set()

        def check(address, path, lower, upper, parent_red):
            if address is None:
                return 1
            assert address in by_address and address not in visited
            visited.add(address)
            node = by_address[address]
            key = bytes.fromhex(node["key_hex"])
            assert node["path"] == path
            assert lower is None or lower < key
            assert upper is None or key < upper
            assert node["color"] in (0, 1)
            assert not (parent_red and node["color"] == 0)
            left = check(node["left"], path + "L", lower, key, node["color"] == 0)
            right = check(node["right"], path + "R", key, upper, node["color"] == 0)
            assert left == right
            return left + node["color"]

        assert by_address[capture["root"]]["color"] == 1
        black_height = check(capture["root"], "", None, None, False)
        assert len(visited) == 4096
        ordered = sorted(nodes, key=lambda n: n["key_hex"])
        fields = ("node", "entry", "key", "value")
        origins = {field: min(n[field] for n in nodes) for field in fields}
        global_origin = min(origins.values())
        normalized[run["name"]] = {
            "shape": [(n["path"], n["color"], n["key_hex"], n["value_hex"], n["node_count"], n["entry_count"],
                       n["key_cap"], n["value_cap"]) for n in ordered],
            "separate_offsets": {field: [n[field] - origins[field] for n in ordered] for field in fields},
            "combined_offsets": {field: [n[field] - global_origin for n in ordered] for field in fields},
            "mod4096": {field: [n[field] % 4096 for n in ordered] for field in fields},
            "memcmp": capture["runtime_memcmp"],
        }
        result["snapshots"][run["name"]] = {
            "nodes": len(nodes), "black_height": black_height, "max_depth": max(len(n["path"]) for n in nodes),
            "digest": inputs.digest(decoded_rows), "runtime_memcmp": capture["runtime_memcmp"],
            "allocations": {field: {"origin": origins[field], "virtual_pages": len({n[field] // 4096 for n in nodes}),
                                    "mod64_counts": dict(sorted(Counter(n[field] % 64 for n in nodes).items()))}
                            for field in fields},
        }
        result["decoded_nodes"] += len(nodes)
    for left, right in [("ordinary-baseline", "ordinary-candidate"), ("aligned64-baseline", "aligned64-candidate"),
                        ("ordinary-baseline", "aligned64-baseline"), ("ordinary-candidate", "aligned64-candidate")]:
        a, b = normalized[left], normalized[right]
        result["comparisons"][left + " vs " + right] = {
            "tree_shape_counts_capacity_and_bytes_equal": a["shape"] == b["shape"],
            "runtime_memcmp_identity_equal": a["memcmp"] == b["memcmp"],
            "allocation_relative_offsets_equal": {field: a["separate_offsets"][field] == b["separate_offsets"][field] for field in fields},
            "combined_relative_offsets_equal": {field: a["combined_offsets"][field] == b["combined_offsets"][field] for field in fields},
            "page_offsets_equal": {field: a["mod4096"][field] == b["mod4096"][field] for field in fields},
        }
    assert all(row["tree_shape_counts_capacity_and_bytes_equal"] and row["runtime_memcmp_identity_equal"]
               for row in result["comparisons"].values())
    result.update(complete=True, scope="Four first-map snapshots only. Identical relative virtual layouts do not establish identical physical placement, cache state, branch state or later-epoch placement. Debugger-affected samples are not performance evidence.")
    (root / "layout-analysis.json").write_text(json.dumps(result, indent=2) + "\n")
    (root / "layout-analysis-inputs.json").write_text(json.dumps(used, indent=2) + "\n")
    print(json.dumps({"complete": True, "decoded_nodes": result["decoded_nodes"],
                      "comparisons": result["comparisons"]}, indent=2))


if __name__ == "__main__":
    main()
