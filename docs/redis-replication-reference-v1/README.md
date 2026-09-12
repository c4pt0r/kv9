# Three-copy Redis reference: local correctness checkpoint

The [v4 client and write plan](../WRITE-PERFORMANCE-NEXT.md) now have local
source and real-process correctness evidence. This changes the benchmark
client, not KV9's write algorithm. No throughput/latency benchmark was accepted.

## Final source and client checks

The [validation summary](validation-summary.json) binds the final eight-file
client/shared-source inventory to debug executable SHA-256
`f26bb4b120ee3e005f604fa817d23756c9e452821a626a7fc61c3124e8357cfb`.
The manifest explicitly records dirty development source over parent
`9efe5594c5d386879380ae4d995508d8cdb4c69d`; it is not a clean release artifact.
Source hashes match before/after compilation and before/after all final runs.

All **26 tests pass**, with zero failed or ignored, followed by warnings-denied
Clippy, debug build and formatting. Seven tests cover strict confirmation
configuration, same-connection SET/MSET + WAIT framing, shortfall accounting,
lost/invalid replies, the original whole-call deadline, no WAIT for reads/async
writes and version 1–3 report compatibility. Existing point/batch and unknown
write tests remain. No failed or uncertain logical call is replayed.

Final source supervisor session `86803` exits zero (`a44872`). Compilation uses
the retained first-party invalidation lock, four Cargo jobs and offline cached
dependencies. The 96-GiB preflight, 80-GiB free floor, 16-GiB additional reserve,
five-second disk samples and 1,200-second command cap remain unchanged.

Four final real-client cases pass on a fresh owned Redis primary and two
replicas. The payload is 128 bytes, with four concurrent workers, 128 mutable
keys and one reserved sentinel. Setup and eight warmup calls precede each
64-call measured section. These finite debug runs stop at the operation limit;
all reports explicitly have timing_eligible=false.

| Case | Successful logical writes | Successful input items | WAIT replies [0, 1, 2 replicas] |
| --- | ---: | ---: | --- |
| SET + WAIT 1 | 64 | 64 | [0, 36, 28] |
| SET + WAIT 2 | 64 | 64 | [0, 0, 64] |
| MSET(64) + WAIT 1 | 64 | 4,096 | [0, 34, 30] |
| MSET(64) + WAIT 2 | 64 | 4,096 | [0, 0, 64] |

The total is **256 successful logical writes / 8,320 input items**, zero unknown
writes, errors or retries. Each case records 64 data-command attempts and 64
confirmation attempts. A fresh SET followed by WAIT 2 on one verification
connection establishes a later replicated offset; all 129 keys then compare
byte-for-byte across all three processes and pass the independent deterministic
value/sentinel checker. Client processes exit and are reaped. Final client
session `97557` exits zero (`4a1edc`).

The compatibility fixture uses Redis 7.0.15, loopback ports 17379–17381, save="",
appendonly=no, 64-MiB maxmemory per process and no eviction. Helpers and clients
use CPUs 6–15,22–31; this is a correctness allocation, not the timing allocation.
Fixture session `36197` exits zero (`cb8ba0`); all three children exit zero,
are reaped and absent. Owned ports are free. The pre-existing system Redis is
untouched.

## Real stopped-replica controls

The earlier owned fixture checks SET and MSET(64) with WAIT 1/2, then stops
exactly one recorded replica using SIGSTOP before fresh writes. WAIT 1 succeeds
for both APIs; WAIT 2 returns only one acknowledgment and is classified as
unknown confirmation. All eight logical controls retain their original replies:
**six successes, two expected shortfalls, 260 key writes and zero retries**.
The paused replica resumes with the same PID/start/run identity. A fresh WAIT 2
barrier and complete 4,096-key readback pass on all three nodes.

These controls exercise Redis semantics using the separate RESP fixture. The
Rust shortfall/deadline tests bind client failure handling using scripted peers;
the final real Rust-client campaign above exercises healthy confirmations.
Do not describe those as a Rust-client Chaos or failover campaign. Original
fixture session `53107` exits zero (`202369`), with all three children reaped.
Actual Chaos Mesh remains required for future KV9 runtime candidate promotion.

## Preserved failures and compatibility repair

The original compile attempt failed because a pre-existing test expected six
reason counters after the internal vector grew to seven. The next test run
exposed acceptance of an extra field on the unit-shaped async enum variant;
an empty struct variant fixes strict decoding. The resulting source passes
25 tests, Clippy and its original four actual-client cases.

The first process reader passed a list to an existing dictionary-based dataset
checker after the first client succeeded. The repaired reader revalidates that
original client result and runs the remaining three cases; it does not replace
the successful first cohort.

Review then found that new metric fields must be limited to v4 to preserve old
report readers. The compatibility test raises the final count to 26. Its first
Clippy run catches legacy report helpers made unused by the new dispatch; the
final dispatcher uses those helpers for versions 1–3. Final tests, Clippy and
four fresh real-client cases pass on this changed source. Both source families
and all failed invocations remain retained; their counts are not added together
as independent coverage or performance repetitions.

## Retained originals and remaining work

The [inventory](inventory.json) binds all **304 members / 13,287,685 bytes** in
`original-evidence.tar.gz`. The archive is **2,016,219 bytes**, SHA-256
`54155356d42d337127d656735cab296be463855a06e8cea4285f0f76f1915ba4`.
Every member was decoded and checked against its original bytes. Native
executables remain at their recorded local paths with hashes; the capsule is
not a complete environment image. Aggregate published original-evidence
archives occupy 63,628,489 bytes, within the existing 64-MiB allowance.

Next: independent v4 performance accounting/rejection checks, a clean release
client and the frozen 24-cohort KV9/Redis WAIT 1/WAIT 2 comparison. Then resume
the existing proven CRC slicing candidate on selected ThinLTO. WAIT confirms
replication, not fsync or Raft consistency. No equal-durability result, new KV9
proof, Chaos acceptance, hosted CI run or original checklist closure is claimed.
