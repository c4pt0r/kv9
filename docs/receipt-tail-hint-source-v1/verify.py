"""Independently verify every original reporting member without executing it."""
from pathlib import Path
import hashlib
import json

root = Path(__file__).resolve().parent
inventory = json.loads((root / "inventory.json").read_text())
expected = {row["name"]: row for row in inventory["files"]}
assert len(expected) == len(inventory["files"])
actual = {p.relative_to(root / "data").as_posix(): p
          for p in (root / "data").rglob("*") if p.is_file()}
assert set(actual) == set(expected)
total = 0
for name, path in actual.items():
    assert not path.is_symlink()
    value = path.read_bytes()
    assert len(value) == expected[name]["bytes"]
    assert hashlib.sha256(value).hexdigest() == expected[name]["sha256"]
    total += len(value)
assert total == inventory["total_bytes"]
print(f"PASS: {len(actual)} reporting files, {total} bytes")
