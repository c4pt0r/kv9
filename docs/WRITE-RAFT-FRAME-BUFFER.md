# Experimental: one allocation for a Raft WAL frame

This isolated, unmeasured experiment starts from selected `11113f6`. It changes
only record construction in `DiskRaftStorage::write_record_unsynced`: allocate
the complete record once, append the existing kind and payload, hash the final
body slice and fill the reserved checksum bytes. It removes the separate body
allocation and the copy of that body into the final record. It does not combine
this change with the independent CRC slicing experiment.

The intended frame remains `length_BE32 || FNV1a_BE32(kind || payload) || kind ||
payload`. The length refusal, FNV function, one `write_all`, write metric/error
mapping, caller sync, failed-writer fencing and publication order are unchanged.
The eight header bytes and body occupy disjoint ranges; writing checksum bytes
4 through 7 cannot change the hashed body starting at byte 8. The accepted
payload bound keeps the body and complete-frame lengths representable.

The historical post-CRC profile identifies allocation/copy/compare as a broad
CPU category, but does not attribute a fraction to this particular buffer.
Source inspection establishes the removed allocation/copy. It does not predict
throughput, latency, allocation behavior of the compiler or physical disk gains.
Existing group commit remains in place. No checksum format or Raft consistency
change is proposed.

Three source-bound SMT checks now pass: universal frame-byte equality, body
preservation after writing the checksum, and representable lengths/safe slice
bounds for every payload admitted by the existing refusal. Three deliberately
wrong layouts produce countermodels: a shifted hash input, a shifted checksum
range and an omitted kind byte. The checker requires every production byte of
`storage.rs` outside the exact buffer replacement to match selected `11113f6`.
It treats the unchanged FNV function as an arbitrary deterministic 32-bit
function: identical input establishes identical checksum without changing or
reproving FNV arithmetic. Sequence operations follow the
[Z3 sequence model](https://microsoft.github.io/z3guide/docs/theories/Sequences/).

This establishes layout/arithmetic equivalence under Rust vector, slice,
serialization and compiler premises. It is not a whole-Rust, durability or
Raft proof. The first proof run retained a syntax error: the bounds variable
declaration was accidentally placed on a comment line. The corrected file
passes all six queries with the same source and solver; both runs remain at
`/tmp/kv9-write-raft-frame-buffer-proof-{first,second}`. No runtime was rerun.

Local source qualification now passes 710 workspace tests and doctests,
including existing persistence failure tests; 23 existing tests remain ignored.
The new compatibility test exercises the actual writer against the historical
encoder for all 256 kind bytes and 15 payload lengths, then replays all 3,840
records. Workspace formatting and Clippy with warnings denied pass. All checks
used the shared Cargo cache lock with explicit first-party invalidation, four
build jobs, helper CPUs, offline dependencies and the original disk guards.

[Original proof and source-check records](raft-frame-buffer-source-v1/README.md)
include both proof attempts and the successful source run. The source snapshot
was unchanged throughout the checks; subsequent changes only document and
package their results. The prototype is based on the selected server, so its
test population is independent of the CRC candidate's coincidentally equal
710-test count.

Exact release, ordinary recovery, actual Chaos Mesh and matched throughput/
latency with full regression coverage remain pending. Do not promote this
experiment or use it in the current CRC A/B campaign. No performance result or
improvement is claimed here.
