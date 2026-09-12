# Explicit Linux lease clock: local validation

The [clock adapter and sampled-time proof](../LEASE-CLOCK.md) now pass their
first local source/proof run. This is a conditional implementation checkpoint;
physical rate/error qualification, host suspend and lease-enabled Chaos Mesh
remain open. Default startup stays Safe ReadIndex. No performance run occurred.

## Source and actual process pause

The [source summary](source-summary.json) records nine successful commands:
formatting, clean first-party Engine/Raft test compilation, library tests,
server tests, default-feature restart protection, real process pause,
default/experimental server checks and all-target warnings-denied Clippy.
The compilation and test commands are separately recorded; the two server
checks account for two commands.

- Engine: **136 passed**, zero failed or ignored.
- Raft: **250 passed**, zero failed or ignored. Seven unit tests are new.
- Server: **207 passed**, zero failed and one pre-existing workload test ignored.
- Default-feature restart integration: one passed.
- Process-pause integration: two passed, comprising the actual parent test and
  its separately invoked child entry point. These are not two independent fault
  runs. The parent observed child PID `4006082` in a stopped state, held it for
  300 ms and resumed it. The child observed **300,169,571 ns** of elapsed time;
  previously acquired and newly requested read authority both expired.

The library total is **593 passed**. Test populations overlap prior checkpoints.
The existing ignored workload requires isolated history output and independent
validation; it is not counted as passed. The process test uses a single-voter
controller promise and actual BOOTTIME samples, not three-node RPC or Chaos Mesh.

Supervisor session `5916` terminates with exit zero (receipt `543ad4`). The
original 96-GiB preflight, 80-GiB free floor, 16-GiB additional reservation,
five-second disk samples and 1,200-second command limits remain unchanged.
The shared retained-build lock invalidates first-party artifacts before source
compilation. All 161 retained source files still match the checkout byte for
byte; the whole source inventory is identical before and after the run.

## Sampled-clock proof

The [proof summary](proof-summary.json) binds Z3 `4.8.9`, its executable hash,
all six source queries and their original outputs. Containment, restart
quarantine and shared-margin sufficiency return `unsat`. Removing error
reserves from containment or recovery, and using the leader-only margin, each
return `sat` with an explicit model. Terminal receipt: `9da93d`, exit zero.
These are conditional arithmetic results, not additional TLAPS theorems or
machine-checked Rust/kernel correctness.

## Retention and capacity

The [inventory](inventory.json) describes every member of
`original-evidence.tar.gz`, with original path, byte count and SHA-256. The
archive is **964,795 bytes**, SHA-256
`8cb3dd9c535f84258cf57798eb402cf8c7c246f035be7c8ae7082f8a8895cab0`.
Every member was read back and compared with its original bytes. The large
compressed test executable remains at its original local path with its hash in
the source result; executable payloads are not included in this small archive.

Before source validation, the delegated environment helper cold-compressed
29 of 32 explicitly selected inactive historical executable copies, stopping
after observed net recovery of **3,311,271,936 bytes**. Session `40419` ended
with exit zero (receipt `f21b09`). All role-bound hashes, complete gzip decode
checks and 58 reference checks passed. Three remaining candidates stayed
resident; no shared target, selected executable or historical outcome changed.

The archive includes the frozen cold inventory and exclusive no-overwrite
restoration catalog. The 29 gzip payloads remain under
`/tmp/kv9-lease-clock-storage-cold-first`; original-path historical validators
require restoration first. They are not transparently replayable against absent
executable paths, and compression is not a new correctness acceptance.

All stages here completed on their first invocation; no test failure was
discarded or replaced. No hosted CI, physical-clock calibration, host suspend,
lease-enabled Chaos campaign, benchmark or original industrial checkbox closure
is claimed.
