# WAL preallocation release and ordinary recovery

Updated 2026-09-15. Matching default and `wal-payload-preallocation` releases
now pass actual three-voter recovery over both native streaming and unary RPC.
All four complete histories pass independent linearizability checks, including
unknown outcomes, final reads and progress after leader loss and restart.
The feature remains **off by default**. Actual candidate Chaos Mesh and
end-to-end throughput/latency qualification are next; this stage adds no QPS
or Redis comparison.

## Exact releases

Both servers were built from clean `86aa6fc07d82f4d49b43fbfe309e0e5c20276da5`.
Its Rust, Cargo and proof inputs are identical to the published
[source/proof checkpoint](WAL-PAYLOAD-PREALLOCATION.md) at `fb9d390`.
The only changes are two source-inventory helpers: compressed `docs/*.bin.gz`
corpora now consume the existing bounded documentation allowance. The first
preflight correctly stopped before Cargo when a 12,132,221-byte corpus was
misclassified as ordinary source. That refusal is retained. Eleven inventory
regressions pass, including exact/overflow boundaries, aggregate accounting,
full hashing and exclusion of non-documentation/lookalike paths. No byte limit
was increased and no input was omitted.

The clean inventory contains 1,529 files, with tree digest
`8b7e112c7645f6a42fdedd4efe97a5368d6a1249340141b4d53752c40677ca90`.
Actual builds use offline/locked Rust/Cargo 1.94, four jobs, opt3, ThinLTO,
one codegen unit and the retained NVMe Cargo cache. The cache helper holds its
exclusive lock and invalidates first-party artifacts before building. Original
Cargo messages, compiler invocations, source-before/after inventories and
retained ELF hashes pass independent readback.

| Role | SHA-256 |
| --- | --- |
| Default server | `04749125c176ab25b1a5cb9537ea55b9df3573201d3dfb7006b066cc8531c144` |
| Preallocated server | `1a38b0a0223650f42aa306131787937f55d7cfc9db2963b2d4bdf8f3d2c8566e` |
| Same-source native correctness client | `49ebe15e52f8851899372cf350cc8b7d9de1cc30fb3abcb8a76670d7a4369730` |
| Previously qualified performance client, retained for later timing | `1b8060eb168610328c10a27480de16bd8b2d6805166d8169638f85ceef0992c4` |

Only the candidate's root and engine packages enable
`wal-payload-preallocation`; its Raft/server packages and all default-server
packages have empty feature lists. The correctness client was built separately
from the same clean source with default features. Recovery catalogs explicitly
compose those original build manifests; they do not claim a simultaneous build.
The performance client was read back unchanged, without rebuilding it or
substituting it into correctness histories.

The recovery binder retains the original source, cleanliness, command, ELF and
Cargo-artifact checks. Its narrow extension requires exactly the declared
preallocation feature on the two intended packages. Both valid catalogs pass;
five controls reject a missing feature, extra feature, missing engine feature,
client feature and dirty source. The candidate is never relabeled as a
default-feature build.

## Actual recovery result

The frozen native scenario runs three ordinary WAL voters with a shared
advertised client endpoint. It exercises concurrent Get, Put, Delete, BatchGet
and BatchPut with four keys and batch size eight. In each transport case it
kills the current owned leader, checks progress through the surviving quorum,
restarts that node from its original directory, checks further progress and
verifies final batch values. This is a local process-failure test, not a
cross-host failure domain or actual Chaos Mesh injection.

| Server | Transport | Complete operations | OK | Unknown |
| --- | --- | ---: | ---: | ---: |
| Default | Streaming | 178 | 163 | 15 |
| Default | Unary | 188 | 172 | 16 |
| Preallocated | Streaming | 182 | 165 | 17 |
| Preallocated | Unary | 180 | 166 | 14 |
| Total | | **728** | **666** | **62** |

The independent audit reparses each original history and verifies its complete
linearization witness, including ambiguous fault outcomes. It recomputes
operation coverage and successful post-failure/restart windows from raw
invocation/return timestamps. All four final batch reads pass. Twelve fresh
voter drains require two advancing metrics exports with stable process
identity and zero pending work. All 14 owned lifetimes exit: ten server
lifetimes, including restarts, and four clients. No failed cleanup remains.

The completed fixtures retain 66 files / 2,572,835 bytes. Every original WAL
file and its data-volume copy passes size/hash readback. These small fixtures
are included in the published archive alongside the complete histories and
original logs. There was no throughput experiment or unchanged scenario rerun.

## Evidence and next work

The [runtime packet](wal-preallocation-runtime-v1/README.md) contains 239 archived
members / 13,278,536 uncompressed bytes. Full archive member readback passes.
Original build and recovery terminal handles are included; the release build,
two recovery runs and independent audit completed successfully. Earlier
source/proof regressions and microbenchmarks remain in their original packet
and were not rerun or counted as new tests.

Bulk output, logs, retained releases and archives use
`/mnt/data/kv9-work/wal-preallocation-runtime-20260915-first`. The two small
active WAL fixtures used fresh, explicitly declared NVMe directories through
new symlinks; no historical directory or symlink was replaced. Originals remain
available for readback. Compiler caches retain their declared location. Capacity
checks here are launch/point-in-time observations, not continuous monitoring.

Next run the candidate's actual Chaos Mesh qualification with exact image and
feature bindings, then compare point Put and BatchPut throughput, mean and p99
in both orders with the same qualified performance client. Keep the Redis
reference's acknowledgment/durability settings explicit. CRC remains selected;
the industrial roadmap and separate C04 pre-upload acceptance remain open.
CI stays local.
