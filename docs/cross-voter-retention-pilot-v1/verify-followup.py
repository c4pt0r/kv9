"""Read every original preparation member without extraction or helper execution."""
import gzip
import hashlib
import json
from pathlib import Path
import tarfile


ROOT = Path(__file__).resolve().parent


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


inventory = ROOT / "followup-input-inventory.json"
archive = ROOT / "followup-evidence.tar.gz"
require(sha(inventory) == "9af8672a9db328288ef3e928ce170d34f920277dc3b40bfb850c951861a85164", "inventory hash")
require(sha(archive) == "0ad4c34b52c050d14b99cee4d71c5fe7755c35edacef3d2553e1daffe29cb62e", "archive hash")
entries = json.loads(inventory.read_text())
expected = {entry["name"]: entry for entry in entries}
require(len(entries) == len(expected) == 68, "member population")
seen = set()
decoded = 0
retained = {}
with gzip.open(archive, "rb") as stream:
    with tarfile.open(fileobj=stream, mode="r|") as tar:
        for member in tar:
            require(member.isfile() and member.name in expected and member.name not in seen, "unexpected member")
            entry = expected[member.name]
            require(member.size == entry["bytes"] and member.size <= 16 * 1024**2, "member bound")
            with tar.extractfile(member) as source:
                data = source.read(member.size + 1)
            require(len(data) == member.size and hashlib.sha256(data).hexdigest() == entry["sha256"], "member bytes")
            decoded += len(data)
            require(decoded <= 32 * 1024**2, "aggregate bound")
            seen.add(member.name)
            if member.name in {"exact-object-run/result.json", "exact-object-run/readback-first/result.json", "exact-object-root/accepted-summary.json", "crc-full-eligibility/summary.json"}:
                retained[member.name] = data
    while stream.read(1024**2):
        pass
require(seen == set(expected) and decoded == 6007742, "complete readback")
producer = json.loads(retained["exact-object-run/result.json"])
readback = json.loads(retained["exact-object-run/readback-first/result.json"])
summary = json.loads(retained["exact-object-root/accepted-summary.json"])
require(producer["complete"] and producer["exact_objects_reproduced"] and readback["complete"] and readback["exact_object_compatibility"], "exact object compatibility")
require(len(producer["rows"]) == len(readback["rows"]) == 6, "six-member population")
for row in producer["rows"]:
    require(row["exact_original_compressed_bytes_match"] and row["actual_compressed"] == row["expected_original_compressed"], "original compressed identity")
require(sum(row["actual_compressed"]["bytes"] for row in producer["rows"]) == summary["compressed_bytes"] == 67878587, "compressed byte accounting")
require(sum(row["logical"]["bytes"] for row in readback["rows"]) == summary["logical_bytes"] == 100639065, "logical byte accounting")
require(summary["retired_bytes"] == 0 and len(summary["lifetimes"]) == 12 and all(not row["same_lifetime_present"] for row in summary["lifetimes"]), "retirement/lifetime scope")
eligibility = json.loads(retained["crc-full-eligibility/summary.json"])
require(eligibility["complete"] and eligibility["metadata_only"] and eligibility["payloads_read"] == eligibility["codecs_executed"] == 0, "metadata-only scope")
require(sum(row["eligible_target_allocated_bytes"] for row in eligibility["screens"]) == eligibility["totals"]["eligible_target_allocated_bytes"] == 27797909504, "additional allocation ceiling")
require(eligibility["totals"]["catalogs"] == 72 and eligibility["totals"]["unpaired_targets"] == 0, "additional catalog population")
print(json.dumps(dict(complete=True, members=len(seen), decoded_bytes=decoded,
                      exact_objects=6, retired_bytes=0)))
