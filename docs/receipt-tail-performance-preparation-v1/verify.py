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
require(sha(inventory) == "fd0210ebe0cac851961ece5112657759eeac34e40057784a8e8c5623f7acec82", "inventory hash")
require(sha(archive) == "582af35a32429add728f835ce4bb02abcff697dc6be3da9a2f1461f6bf043d14", "archive hash")
entries = json.loads(inventory.read_text())
expected = {entry["name"]: entry for entry in entries}
require(len(entries) == len(expected) == 98, "member population")
seen = set()
decoded = 0
retained = {}
with gzip.open(archive, "rb") as stream:
    with tarfile.open(fileobj=stream, mode="r|") as tar:
        for member in tar:
            require(member.isfile() and member.name in expected and member.name not in seen, "unexpected member")
            entry = expected[member.name]
            require(member.size == entry["bytes"] and member.size <= 8 * 1024**2, "member bound")
            with tar.extractfile(member) as source:
                data = source.read(member.size + 1)
            require(len(data) == member.size and hashlib.sha256(data).hexdigest() == entry["sha256"], "member bytes")
            decoded += len(data)
            require(decoded <= 16 * 1024**2, "aggregate bound")
            seen.add(member.name)
            if member.name.startswith("root/"):
                retained[member.name] = data
    while stream.read(1024**2):
        pass
require(seen == set(expected) and decoded == 1174930, "complete readback")
result = json.loads(retained["root/result.json"])
require(result["complete"] and not result["runtime_released"], "preparation scope")
require(result["source_controls"] == result["smoke_reader_controls"] == 8, "control counts")
require({key: value["source_files"] for key, value in result["roles"].items()} == {"old": 859, "new": 869, "client": 581}, "source populations")
for phase in ["targeted_changed_source_controls", "focused_smoke_reader_controls"]:
    log = retained["root/" + phase + ".stderr"].decode()
    require("Ran 8 tests" in log and log.rstrip().endswith("OK"), "original control result")
print(json.dumps(dict(complete=True, members=len(seen), decoded_bytes=decoded,
                      focused_controls=16, runtime_released=False)))
