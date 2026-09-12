# Outbound peer executor validation evidence

This bundle supports [the validation report](../PEER-EXECUTOR-ISOLATION-VALIDATION.md)
for `36ae89a774131b368cf1ed28e95df02ba325c8d4`. It retains 136 original files /
2,916,894 decoded bytes in one 338,121-byte archive part: source/default-release
checks, safe-cache receipts, frozen process preparation/contracts, complete
stream/unary histories, independent audit and root execution records.

Local source gates pass 224 default / 234 experimental overlapping server tests
and doctests, formatting and Clippy, with one existing ignored test per suite.
Original release/source readback and ordinary recovery pass: 356 calls, 328 OK
and 28 unknown; five server/two client lifetimes and six fresh voter drains.
Unknown writes remain in complete histories without blind replay.

No performance comparison, actual Chaos, host failure or full implementation
proof is established. Adding a worker changes the resource budget; a shared
three-worker control is necessary before attributing later gains to placement.
Incoming Raft RPCs still use the public listener/runtime.

Run `python3 verify.py` here for exact archive/member identity and safe paths,
without extraction. This checks integrity only. Raw WAL and executables remain
local at original paths under their manifests. The selection is not a standalone
full runtime re-audit and does not rerun tests or workload. No hosted CI ran.
