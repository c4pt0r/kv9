# Single-buffer Raft frames on the CRC baseline

The combined write candidate passes local source, proof, ordinary recovery and
actual Chaos Mesh qualification. Candidate
[`e9249f2`](https://github.com/c4pt0r/kv9/commit/e9249f2cbd069dcdc44312be826a68494cf694db)
is published on `experiment/write-frame-buffer-crc`. Its [completed write screen](WRITE-FRAME-BUFFER-CRC-PERFORMANCE.md)
does not establish a general gain: loaded point throughput falls 0.517%, loaded
batch throughput changes +0.379% with worse pooled p99, and both loaded workloads
reverse direction across orders. The candidate remains experimental.

## Change and proof scope

The Raft WAL writer constructs its final frame in one buffer, avoiding the
separate body allocation and copy. The byte layout remains
`BE32(length) || BE32(FNV1a(kind || payload)) || kind || payload`.
Length limits, one `write_all`, synchronization, error handling, commit and
response fences are unchanged. Integrated slicing-by-eight CRC and the current
lease-epoch storage additions are preserved. Default reads use Safe ReadIndex.

The source-bound SMT checker proves three universal properties: complete frame
equivalence, preserved body bytes, and safe representable lengths. Three negative
controls produce countermodels for shifted hash input, shifted checksum placement
and an omitted record kind. The compatibility test covers 3,840 frames. The
unchanged CRC checker passes all 47 distinct Lean statements, fresh restoration,
256 fallback and 2,048 slicing entries, and eight rejection controls.

These proofs establish the specified encoding/checksum transformations under
explicit Rust primitive, allocation/slice and compiler premises. They do not
constitute a proof of the entire Rust implementation or the Raft protocol.

## Local qualification

| Check | Result |
| --- | --- |
| Workspace tests and doctests | 790 passed; 23 existing ignored |
| Formatting, Clippy, experimental-lease compilation | Passed |
| Clean default release and independent source readback | 866 source files; all bound to e9249f2 |
| Ordinary streaming/unary recovery | 337 operations: 309 OK, 28 unknown; six fresh drains; seven exited lifetimes |
| Actual Chaos Mesh campaign | All 21 fault windows and complete-history checks passed |
| Chaos operation history | 9,805 operations: 9,170 OK, 613 unknown, 22 refused |
| Final recovery and cleanup | Four fresh replica drains; all 33 observed server lifetimes exited |
| Archive verification | All 4,258 files read back; 938,844,491 decoded bytes |

The fault campaign exercises voter failures, network partition and delay,
admission overload, EIO/ENOSPC, missing-log and replacement-PVC refusal/recovery,
and endpoint migration. Unknown writes remain unknown throughout history checking.
Cleanup affects only the new owned namespace; historical namespace identities
are preserved. The independent audit predates cleanup and retains its original
`cleanup_complete: false`; the separate post-cleanup records establish completion.

The server executable SHA256 is
`96bdb92ea90f131c6434033f912193c24531bc17b7bfaf8207e4c46902aa1714`.
The native recovery client SHA256 is
`5959b48e6b99777dc18628b63acc9f4c636c01822ae92dee90454c34d9d2a466`.
The source tree SHA256 is
`43fbcb002533602ca397ad877327543c9cd6d4e061a6af8f170efda7aa154cea`.
Both executables retain opt-level 3, ThinLTO, one codegen unit, unwind and empty
production features. Test-only pressure features remain separate.

## Measurement and next development steps

The original CRC main executable at `bd42e60` and this exact candidate have now
completed their paired write screen using the same native v3 client: point Put
and BatchPut(64), concurrency 1 and 64, eight two-second smokes and sixteen
ten-second timed cohorts in two opposite complete orders. All 7,247,954 measured
calls succeed once; full independent retention and lifecycle checks pass.
Twenty-eight runtime-preparation and five reporting controls pass. The
[performance report](WRITE-FRAME-BUFFER-CRC-PERFORMANCE.md) retains whole-call
latency, rates, all outcomes and the actual capacity observations.

This is a shared-host volatile-tmpfs diagnostic with unchanged Raft/WAL semantics.
Real-disk, power-loss and cross-host acceptance remain separate. No new Redis
comparison or general frame-buffer speedup is established. The latest exact-main
write numbers are in that report; the [original CRC full regression](WRITE-CRC-FULL-REGRESSION-PERFORMANCE.md)
remains a separate read/write/mixed campaign. Keep CRC main and next obtain
current-source CPU attribution before choosing another implementation. The
[write development order](WRITE-PERFORMANCE-NEXT.md) and subsequent dynamic
multi-Raft and range-splitting roadmap remain in force.

All checks ran locally. Hosted CI was not dispatched.
