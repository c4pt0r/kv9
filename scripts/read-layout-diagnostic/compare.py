#!/usr/bin/env python3
"""Check the four exact ELF inspections before the bounded timing experiment."""
import argparse
import hashlib
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--experiment", type=Path, required=True)
    args = parser.parse_args()
    root = args.experiment.resolve()
    rows = {}
    for config in ("ordinary", "aligned64"):
        for arm in ("baseline", "candidate"):
            data = json.loads((root / f"codegen-{config}-{arm}/result.json").read_text())
            assert data["complete"]
            assert hashlib.sha256(Path(data["binary"]).read_bytes()).hexdigest() == data["sha256"]
            rows[config + "-" + arm] = data
    baseline = rows["ordinary-baseline"]["functions"]
    result = {"complete": True, "timing_authorized": True, "source_patch": False,
              "alignment_intervention": "64-byte alignment for all compiled Rust functions, not one isolated read function. Assembly shape equality excludes address/RIP-displacement values. ELF locations and branch/call distances still change.",
              "function_shapes_equal": {}, "functions": {}, "dispatch_tables": {}, "memcmp_symbols": {}}
    for name in ("answer", "read_panels"):
        for data in rows.values():
            assert data["functions"][name]["operand_shapes"] == baseline[name]["operand_shapes"]
            assert data["functions"][name]["direct_calls"] == baseline[name]["direct_calls"]
        result["function_shapes_equal"][name] = True
        result["functions"][name] = {key: {field: data["functions"][name][field]
                                         for field in ("start", "bytes", "mod64", "mod4096")}
                                      for key, data in rows.items()}
    for key, data in rows.items():
        function = data["functions"]["answer"]
        assert function["dispatch_table"]["relative_targets"] == baseline["answer"]["dispatch_table"]["relative_targets"]
        assert function["get_memcmp"]["symbol"] == baseline["answer"]["get_memcmp"]["symbol"]
        result["dispatch_tables"][key] = function["dispatch_table"]
        result["memcmp_symbols"][key] = function["get_memcmp"]
    output = root / "codegen-comparison.json"
    if output.exists():
        assert json.loads(output.read_text()) == result
    else:
        output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"complete": True, "timing_authorized": True, "function_shapes_equal": result["function_shapes_equal"]}))


if __name__ == "__main__":
    main()
