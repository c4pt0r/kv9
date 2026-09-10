#!/usr/bin/env python3
"""Reject transport/provenance corruptions of retained v2 and genuine legacy v1 histories."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

from workload_report import validate


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write(path, data):
    # Build controls initially link immutable originals. Never write through a link.
    if path.is_symlink():
        path.unlink()
    path.write_bytes(data)


def write_json(path, value):
    write(path, (json.dumps(value, sort_keys=True, indent=2) + "\n").encode())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", type=Path, required=True, help="genuine normal-build v2 full-history run")
    parser.add_argument("--build", type=Path, required=True, help="retained normal workload build directory")
    parser.add_argument("--legacy-run", type=Path, required=True, help="genuine experimental v1 full-history run")
    parser.add_argument("--legacy-build", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    inputs = {"v2": (args.run.resolve(), args.build.resolve()),
              "v1": (args.legacy_run.resolve(), args.legacy_build.resolve())}
    result = dict(complete=False, controls=[], baselines={}, sources={
        name: digest(Path(__file__).parent / name) for name in
        ("check-streaming-rpc-report-controls.py", "workload_report.py", "history/checker.py")})
    original_hashes = {}
    try:
        for label, (run, build) in inputs.items():
            checked = validate(run, build, seconds=60)
            report = json.loads((run / "report.json").read_text())
            if not checked["full_history_independently_checked"]:
                raise ValueError("control baseline must retain its complete checked history")
            if report["version"] != int(label[1:]) or "rpc_transport" not in report["configuration"]:
                raise ValueError("baseline is not the required genuine recorded transport/version")
            if label == "v2" and report["configuration"]["rpc_transport"] not in ("tonic_stream", "tonic_unary"):
                raise ValueError("v2 baseline is not the normal transport")
            if label == "v2":
                inventory = json.loads((build / "sources.json").read_text())
                artifacts = [row for row in map(json.loads, (build / "cargo.jsonl").read_text().splitlines())
                             if row.get("reason") == "compiler-artifact" and
                             row.get("target", {}).get("name") == "kv9-workload" and row.get("executable")]
                if "--features" in inventory["command"] or len(artifacts) != 1 or artifacts[0]["features"] != []:
                    raise ValueError("v2 control baseline requires the genuine default-feature workload")
            result["baselines"][label] = checked
            for directory in (run, build):
                for path in directory.iterdir():
                    if path.is_file():
                        original_hashes[str(path)] = digest(path)

        cases = [
            ("v2-missing-transport", "v2", "version 2 requires an explicit recorded RPC transport"),
            ("v2-unknown-transport", "v2", "unsupported experimental RPC transport"),
            ("v2-malformed-transport", "v2", "unsupported experimental RPC transport"),
            ("v2-noninteger-version", "v2", "invalid version 2 report version"),
            ("v2-missing-artifact", "v2", "version 2 requires exactly one retained workload Cargo artifact"),
            ("v2-duplicate-artifact", "v2", "version 2 requires exactly one retained workload Cargo artifact"),
            ("v2-nonexecutable-artifact", "v2", "version 2 requires a workload executable Cargo artifact"),
            ("v2-malformed-features", "v2", "invalid workload Cargo feature list"),
            ("v2-tarpc-missing-command", "v2", "experimental RPC configuration requires its explicit build feature"),
            ("v2-tarpc-missing-artifact-feature", "v2", "workload Cargo artifact does not attest the RPC experiment feature"),
            ("v2-tarpc-malformed-features", "v2", "invalid workload Cargo feature list"),
            ("v2-relabeled-v1", "v2", "experimental RPC configuration requires its explicit build feature"),
            ("v1-missing-command-feature", "v1", "experimental RPC configuration requires its explicit build feature"),
            ("v1-missing-artifact-feature", "v1", "workload Cargo artifact does not attest the RPC experiment feature"),
            ("v1-duplicate-artifact", "v1", "workload Cargo artifact does not attest the RPC experiment feature"),
        ]
        for name, label, expected in cases:
            source, original_build = inputs[label]
            folder = args.output / name
            folder.mkdir()
            run, build = folder / "run", folder / "build"
            shutil.copytree(source, run)
            build.mkdir()
            for path in original_build.iterdir():
                if path.is_file():
                    (build / path.name).symlink_to(path)
            report = json.loads((run / "report.json").read_text())
            config = json.loads((run / "config.json").read_text())
            inventory = json.loads((build / "sources.json").read_text())
            records = [json.loads(line) for line in (build / "cargo.jsonl").read_text().splitlines()]
            artifacts = [row for row in records if row.get("reason") == "compiler-artifact" and
                         row.get("target", {}).get("name") == "kv9-workload" and row.get("executable")]
            if len(artifacts) != 1:
                raise ValueError("baseline artifact uniqueness changed")
            if name == "v2-missing-transport":
                del config["rpc_transport"]
            elif name == "v2-unknown-transport":
                config["rpc_transport"] = "silent_fallback"
            elif name == "v2-malformed-transport":
                config["rpc_transport"] = {"transport": "tonic_stream"}
            elif name == "v2-noninteger-version":
                report["version"] = 2.0
            elif name == "v2-missing-artifact":
                records.remove(artifacts[0])
            elif name in ("v2-duplicate-artifact", "v1-duplicate-artifact"):
                records.append(artifacts[0])
            elif name == "v2-nonexecutable-artifact":
                artifacts[0]["target"]["kind"] = ["lib"]
            elif name == "v2-malformed-features":
                artifacts[0]["features"] = "rpc-experiment"
            elif name == "v2-tarpc-missing-command":
                config["rpc_transport"] = "tarpc_tcp"
            elif name in ("v2-tarpc-missing-artifact-feature", "v2-tarpc-malformed-features"):
                config["rpc_transport"] = "tarpc_tcp"
                inventory["command"].extend(["--features", "rpc-experiment"])
                if name == "v2-tarpc-malformed-features":
                    artifacts[0]["features"] = "rpc-experiment"
            elif name == "v2-relabeled-v1":
                report["version"] = 1
            elif name == "v1-missing-command-feature":
                index = inventory["command"].index("--features")
                del inventory["command"][index:index + 2]
            elif name == "v1-missing-artifact-feature":
                artifacts[0]["features"].remove("rpc-experiment")
            write_json(run / "config.json", config)
            report["configuration"] = config
            report["config_sha256"] = digest(run / "config.json")
            write_json(run / "report.json", report)
            write_json(build / "sources.json", inventory)
            write(build / "cargo.jsonl", ("".join(json.dumps(row) + "\n" for row in records)).encode())
            try:
                validate(run, build, seconds=60)
            except (ValueError, OSError) as error:
                if expected not in str(error):
                    raise RuntimeError(f"{name}: rejected for the wrong reason: {error}") from error
                result["controls"].append(dict(name=name, rejected=True, reason=str(error)))
            else:
                raise RuntimeError(name + ": corrupted evidence was accepted")
            write_json(args.output / "summary.json", result)
            print("PASS: rejected " + name, flush=True)
        result["restored"] = {}
        for label, (run, build) in inputs.items():
            checked = validate(run, build, seconds=60)
            if checked != result["baselines"][label]:
                raise ValueError("restored original verdict differs")
            result["restored"][label] = checked
        if any(digest(Path(path)) != expected for path, expected in original_hashes.items()):
            raise ValueError("control changed original retained evidence")
        result.update(complete=True, original_hashes=original_hashes)
    except Exception as error:
        result["failure"] = str(error)
        raise
    finally:
        write_json(args.output / "summary.json", result)
    print(f"PASS: {len(result['controls'])} transport/version controls rejected; genuine v1/v2 originals restored unchanged", flush=True)


if __name__ == "__main__":
    main()
