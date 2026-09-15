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


inventory = ROOT / "input-inventory.json"
archive = ROOT / "original-evidence.tar.gz"
require(sha(inventory) == "50ca7844480d49e1f70634c43c61f095c1a55712f22568c3e3df0e4b5eaf46a4", "inventory hash")
require(sha(archive) == "4da2528d42ea0c33214f2e5b523e04791f16abe38d3894c620ee7ee7199c8967", "archive hash")
entries = json.loads(inventory.read_text())
expected = {entry["name"]: entry for entry in entries}
require(len(entries) == len(expected) == 167, "member population")
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
            if member.name in {"corrected-pilot/verification-first/result.json", "corrected-root/accepted-summary.json", "three-screen-eligibility/summary.json"}:
                retained[member.name] = data
    while stream.read(1024**2):
        pass
require(seen == set(expected) and decoded == 11051321, "complete readback")
result = json.loads(retained["corrected-pilot/verification-first/result.json"])
summary = json.loads(retained["corrected-root/accepted-summary.json"])
require(result["complete"] and result["roundtrip_accepted"] and result["original_objects_unchanged"], "pilot acceptance")
require(len(result["verified"]) == 6 and len(result["controls"]) == 8 and all(c["refused"] for c in result["controls"]), "pilot population")
for group in result["comparisons"]:
    rows = [row for row in result["verified"] if row["id"].startswith(group["group"] + "-")]
    ordinary = sum(row["ordinary_bytes"] for row in rows)
    patched = next(row["ordinary_bytes"] for row in rows if row["patch_bytes"] is None) + sum(row["patch_bytes"] or 0 for row in rows)
    require(ordinary == group["ordinary_three_objects_bytes"] and patched == group["one_ordinary_base_plus_two_patches_bytes"], "base-inclusive accounting")
    require(ordinary - patched == group["byte_difference"], "comparison arithmetic")
require(summary["reclaimed_bytes"] == 0 and len(summary["codec_lifetimes"]) == 30 and all(not row["same_lifetime_present"] for row in summary["codec_lifetimes"]), "retirement/lifetime scope")
eligibility = json.loads(retained["three-screen-eligibility/summary.json"])
require(eligibility["complete"] and eligibility["metadata_only"] and eligibility["payloads_read"] == eligibility["codecs_executed"] == 0, "metadata-only scope")
require(sum(row["eligible_target_allocated_bytes"] for row in eligibility["screens"]) == eligibility["totals"]["eligible_target_allocated_bytes"] == 51873718272, "eligibility ceiling")
print(json.dumps(dict(complete=True, members=len(seen), decoded_bytes=decoded,
                      pilot_members=6, refusal_controls=8, reclaimed_bytes=0)))
