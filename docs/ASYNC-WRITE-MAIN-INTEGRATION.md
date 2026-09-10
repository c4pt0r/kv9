# Selected asynchronous RawKV runtime integration

Tracking: #9, #13 and #20. This staged integration brings the selected performance
lineage into main after local correctness checks and exact-source Chaos Mesh
validation. It does not claim Redis-class throughput or complete industrial
database acceptance.

## Source selection

Merge `2627825cd2ba361fa8d7b83871ebea80686856a1` integrates exact
`23bc58b6d9e15bc46764a30ba4cd3390c36dd281` into main base
`7a0e62564a25a70713f5ccee3a6d4ffb94872b9b`. Main's subsequent documentation through
`325c0001cbdf57492d2357b3fa7df66a08a27a54` is preserved, including the accepted
fault report, CPU diagnostic and Redis-first product sequence.

All 131 files in `src/`, `crates/`, `proto/`, `.cargo/` and the root Cargo/toolchain
inputs match the selected source byte for byte, with no added or removed paths.
This includes tests and build inputs, beyond the narrower 124 Rust/proto/TOML/lock
comparison. Cargo manifests and the lockfile are unchanged from main. Main-only
proofs, documentation and tools remain present. The additional Redis reference
Rust tool has its own workspace and is not part of the production build graph.

The source contains append and Raw engine group sync, combined Ready persistence,
asynchronous quorum-read preparation, sealed read groups, resident GET execution,
asynchronous exact-apply waiting and diagnostic process-start/boot identity.
It excludes the unselected proposal queue `95fb5cd`, indexed receipt experiment
`73ddb0d`, FIFO `c7313ec`, serialization `fb25950`, CRC and RPC prototypes. Selection
is defined by exact source identity, including its existing receipt structures.

Only two Rust merge conflicts required resolution: `driver.rs` and `lib.rs` were
resolved to the exact selected files. Three documentation conflicts retained
main's fuller status/scheduling/socket acceptance records. No new request,
consensus, persistence or cancellation behavior was invented during integration.

## Fresh local validation

Validation ran in an owned worktree and private Cargo target on logical CPUs
6–31. The copied target had separate inodes; the root/shared target was untouched.
All commands completed on their first runtime attempt:

| Check | Result |
| --- | --- |
| `cargo test --workspace --locked` | 645 tests/doctests passed, zero failed, 23 ignored |
| Workspace/all-target Clippy with `-D warnings` | Passed |
| Standalone `cargo build --locked --bin kv9` | Passed |
| Unchanged three-process RawKV E2E | Failover, subsequent writes, delete/range delete and original-directory restart passed at term 2/index 14 |
| Executing-process identity | Four PID/start/boot lifetimes matched the recorded executable; zero unbound observations; all exited |
| Status replay controls | Stale same-PID, missing identity and unavailable identity records rejected |

The exact default executable has SHA-256
`679ca69832e5114ce53cf644eb02cf0d72997549fa7dd640f4aab83f97a425e1`.
Both the standalone build and the fixture's later rebuild retain Cargo JSON
showing empty feature lists for root, engine, Raft and server. The executable
did not change across the fixture. All source inputs were rechecked afterward.
The 23 ignored tests retain their original requirements; this table does not
claim to execute them.

The original execution manifests truthfully record pre-commit main HEAD and the
staged merge state. Subsequent source comparisons bind those bytes to the
committed integration; the original manifests are not rewritten as clean-commit
runs. A first overly broad source predicate also counted main's independent
Redis-reference tool and rejected the comparison. That observer result is
retained, along with the corrected build-boundary comparison; no runtime test
was rerun or weakened to address it.

## Fault evidence and formal boundary

The [accepted exact-source Chaos run](ASYNC-WRITE-CHAOS-ACCEPTANCE.md) executes
`23bc58b` with independently identified default binaries, all 21 original fault
windows, complete CLI and persistent histories, positive physical effects,
PID/start/boot-bound observations and drained public/read/apply ledgers. Its
bounded owned delay gate strengthens observation coverage while preserving the
original faults, workloads and checkers. Both earlier rejected attempts remain
rejected. The integration source equality and fresh process checks establish
the reuse boundary; the old Chaos executable is not relabeled with the new
build hash, and no extra full matrix was run for documentation-only differences.

The accepted [scheduling/completion proof](RAFT-SCHEDULING-ACCEPTANCE.md) and
[Raw group composition proof](RAW-GROUP-PROOF.md) retain their exact stated source
and premise boundaries. The latter is mapped to `fe650ed` and explicitly excludes
combined Ready persistence and later read authority. The selected Ready/read/
resident/write documents provide conditional arguments and compiled behavioral
controls. This integration does not turn those arguments into a newly
machine-checked whole-lineage refinement. That composition work remains open.

This is one-host correctness evidence. It does not establish physical power-loss
behavior, cross-host availability, sustained capacity, multi-Raft scaling or the
absence of every possible service-critical singleton. The
[performance report](../scripts/redis-reference/results/11cae97-a00e39f-c64-tmpfs-diagnostic.md)
retains the observed 18.4% PUT and 10.6% mixed gains, the -0.26% GET result and
the large Redis gap. Those measurements belong to `a00e39f` with explicitly
volatile tmpfs and are not a new durable-performance result for this merge.

## Retained integration evidence

- Validation, source comparison, exact executable and source copies:
  `/tmp/kv9-async-write-main-integration-evidence`.
- Complete process logs, stores, Cargo JSON and lifetime observations:
  `/tmp/kv9-async-write-main-integration-process-first`.
- Full runtime-tree comparison:
  `/tmp/kv9-async-write-main-integration-evidence/runtime-tree-comparison.json`.
- Local command records and complete logs:
  `/tmp/kv9-async-write-main-integration-evidence/validation.json`.
- Process execution/replay/cleanup record:
  `/tmp/kv9-async-write-main-integration-process-first/identity.json`.
- Independent source/build/outcome/lifetime audit:
  `/tmp/kv9-async-write-main-integration-evidence/independent-audit.json`.

The complete integration inventory is
`/tmp/kv9-async-write-main-integration-evidence-inventory.json`: 198 files,
183,115,531 bytes, SHA-256
`bfcde969e09c780d0147af20f24e5a57316a7eeaee12b452dc5088c8466b84a1`.
Every original file was independently reopened and hash-verified; the sibling
`-verification.json` records the readback. The earlier full Chaos evidence has
its own inventory in the linked fault report.

No hosted CI, GitHub mutation or cluster action was needed for this private
integration. Publication and original roadmap checklist updates remain separate.
