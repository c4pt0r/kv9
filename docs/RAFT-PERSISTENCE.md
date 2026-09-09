# Raft vote durability and recovery

Date: 2026-09-08. C01/C04 increment under
[#11](https://github.com/c4pt0r/kv9/issues/11) and
[#14](https://github.com/c4pt0r/kv9/issues/14).

## Reproduced failures

Two tests failed against the original persistence ordering while executing the
real `DiskRaftStorage` codec and raft-rs Ready loop with a deterministic filesystem:

1. A voter sent a positive response for candidate 2 in term 7. File data was
   synchronized, but its directory entry and newly created ancestors were not.
   A modeled power loss removed those unsynced names. The restarted voter
   initialized a fresh log and granted candidate 3 in the same term. The test
   failed at `a durable voter must not grant two candidates in the same term
   after power loss`.
2. A partial write left four bytes of a new record and returned EIO. A later
   append returned success behind the damaged record; replay would stop before
   that later record. The test failed at `a failed append must refuse later
   acknowledgments until recovery`.

Initial red evidence: `/tmp/kv9-c01-known-defects-red.log`, two selected tests,
both failing at those assertions. Ordinary process reopen could not reproduce
the directory-entry failure because visible kernel state remained available.

## Persistence protocol

`DiskRaftStorage::open` now creates directories through the filesystem seam,
replays and repairs the log, and synchronizes the recovered prefix. It writes
the initial configuration when needed, then synchronizes the containing
directory and every canonical ancestor before returning a usable storage object.
Already-visible directories are also synchronized: a prior process may have
created them without completing publication. An error at any of these operations
refuses the open.

Each append, HardState update or indexed ConfState update holds one writer guard
through file persistence and in-memory publication. An error permanently removes
the writer handle. Further mutation attempts fail without performing filesystem
operations; reopening must replay and repair the log. A failed sync has an
unknown persistence outcome and never authorizes a response. Records exceeding
the replay format's maximum length are refused.

Raft integration retains the persist-before-send order in `RaftPeer::process_ready`:
append entries, persist the original HardState, call `advance_append`, persist
any later LightReady commit in a complete HardState, then publish outgoing messages
and committed work. The late commit repair and its proof are documented in
[READY-PUBLICATION.md](READY-PUBLICATION.md). Persistence errors return `Error::Raft` without unwinding through
mutex guards. The peer records its first failure, disables ticking and inbound
processing, clears its outgoing/apply/read queues, and refuses campaigns,
proposals, read barriers and configuration application until reopen. The driver
publishes the failure in `status().fatal`; the existing runtime exit path remains
usable and terminates nonzero. Successful
reopen also synchronizes any complete unsynced records exposed by the previous
process, so a recovered vote cannot disappear on the next power loss.

The distinction between file sync and directory sync follows the
[Linux fsync contract](https://man7.org/linux/man-pages/man2/fsync.2.html).
Durability assumes a local filesystem and device that honor successful sync,
exclusive ownership of the log, stable administrator-provided symlinks and mount
configuration, and no loss or corruption of already durable media. Filesystem
errors outside these assumptions require refusal or separate recovery evidence.

## Abstract proof

`proofs/lean/DurableVote.lean` projects one voter onto one fixed term:

- `memory`: the selected candidate, or no vote.
- `durable`: the vote covered by a successful data sync.
- `published`: durability of the complete directory ancestry.
- `replies`: ghost history observed by receivers, retained across local crashes.

`choose` permits an empty vote or the same candidate. `sync` copies the selected
vote to durable state. `publish` establishes the namespace. `reply` requires both
durable vote data and publication. `crash` may retain or lose an unsynced name,
but must retain a published name, then reconstructs memory from surviving durable
state. `stop` emits nothing. A sync that reports an error after its effect may
take a sync transition and stop without a reply.

The invariant has two clauses:

1. If the durable vote names candidate c, memory names c.
2. For every emitted reply naming c, the durable vote names c and the namespace
   is published.

`initial_invariant` proves the base case. `step_preserves_invariant` handles
every transition. In the choose case the old durable vote constrains the allowed
candidate; in the sync case clause 1 preserves all prior replies; in the reply
case the two explicit preconditions establish clause 2. In the crash case any
prior reply implies publication, forcing retention. `reachable_invariant`
inducts over arbitrary finite executions, and `replies_agree` follows because two
replies must equal the same durable vote. This is an unbounded safety proof.

Conditional progress requires successful storage operations, eventual delivery,
a communicating majority, continued Raft ticks and the upstream election
protocol. This proof does not establish election termination or fairness. A
permanent disk failure stops this voter; a surviving majority is responsible for
availability.

## Implementation correspondence and open refinement obligations

| Abstract operation | Implementation event | Evidence and boundary |
|---|---|---|
| choose | raft-rs 0.7.0 `can_vote` predicate and vote update for a real RequestVote | Inspected pinned source; term monotonicity, log freshness and all upstream Raft invariants remain assumptions of this projection |
| sync | successful `write_record` data sync, or synchronization of recovered log bytes | Actual codec runs with both OS and modeled files; no mechanical proof of the Rust codec or successful device behavior |
| publish | successful `sync_ancestors` after log synchronization | Every ancestor is visited; failed opens return no storage handle; model explicitly separates immediate directory children from file contents |
| reply | outgoing vote returned by `RaftPeer::pump` after Ready persistence | Regression drives real RequestVote messages and checks positive responses before and after modeled power loss |
| crash | replay into a new DiskRaftStorage and RaftPeer incarnation | Model invalidates old file handles and selects durable/unsynced persistence; actual restart tests cover the OS adapter |
| stop | failed storage mutation removes its writer; Ready returns an error and disables the peer | Error/seed and driver matrices check no subsequent I/O, outgoing messages or apply advancement; runtime status remains observable |

This establishes checked abstract lemmas and concrete regression evidence. It
does not mechanically verify Rust, prove the frame checksum collision-free, or
establish all-term Raft safety, log matching, leader completeness, membership
transitions, snapshots or metadata invariants. Those obligations remain tracked
by the [mandatory correctness contract](CORRECTNESS-GATES.md).

### Terminal failure argument

The failure transition sets `fatal`, disables `alive` and empties the three peer
publication queues under `peer.inner`. The only transitions that tick or accept
messages require `alive`; proposals, campaigns, read-index requests, Ready
processing and configuration application check `fatal` under the same mutex.
There is no production transition that clears it. By induction over subsequent
calls in that incarnation, no further Ready is processed, no response is emitted
and no new read confirmation or apply position is published by this peer. A
request concurrent with the failure can finish its earlier proposal assignment,
but an assignment is not an acknowledgment; the driver cannot apply that failed
Ready or create its success receipt. Previously durable, successfully applied
operations remain valid and need not be retracted.

In the fixed-term Lean projection this failure maps to `Step.stop`, preserving
the vote invariant. Recovery creates a fresh peer from synchronized replayed
state. This is a source-level refinement argument, not a mechanically checked
concurrency proof. The driver regression executes actual append, HardState and
ConfState paths across 24 EIO/ENOSPC write/sync cells, checks terminal closure,
unchanged apply/configuration watermarks, empty transport/read output and stable
observable fatal state. Baseline cases separately prove successful vote delivery
and committed learner admission.

## Deterministic failure model

`kv9_common::fs::FileSystem` is generic; the default adapter directly invokes
`std::fs`. `ModelFs` is compiled only for tests or an explicit nondefault testing
feature. There are no environment variables, filesystem trigger files or global
injection registry in the production path.

The current model covers directory creation, append/create, reads, seeks,
truncation, data sync and directory sync. It records numbered operations and
supports errors before or after effects, EIO/ENOSPC and real `write_all` short-write
continuation. A crash can discard all unsynced state, retain it, or select a
replayable mixture of unsynced bytes and directory entries using a fixed seed.
Successful data sync protects existing bytes until a later mutation; successful
directory sync protects only immediate child names. Losing an ancestor removes
access to its entire subtree.

Four model controls independently establish that file sync does not publish a
name, directory sync does not publish data or ancestor names, an error after sync
can preserve its effect, and seeded partial persistence is reproducible while
retaining the synchronized prefix.

The Raft matrix currently exercises 15 named initialization cuts with 60
before/after EIO/ENOSPC and retry cells, plus 320 append/sync/error/seed cells.
Targeted tests also cover double-vote prevention, poisoned-write refusal and
repeated crashes after recovering a complete unsynced frame. These counts are
observed workload sizes, not exhaustive filesystem coverage.

Rename/unlink, replacement inode identity, engine WAL/checkpoint/pending paths,
concurrent fault schedules, expanded nightly seeds and independent
history checking remain required C01/C02 work. This initial append-log model
does not claim their coverage. The real Chaos Mesh Pod/Network and subsequent
[Raft I/O matrix](CHAOS-IO-VALIDATION.md) remain separate E2E gates and do not
stand in for modeled power loss.

## Reproduction

```sh
cargo test -p kv9-common fs:: -- --nocapture
cargo test -p kv9-raft storage::persistence_model --lib -- --nocapture
python3 scripts/check-proofs.py --lean /path/to/lean --self-test
```

The pinned Lean inventory now contains nine theorems. Four controls must be
rejected, including removal of the namespace precondition from the reply
transition. CI verifies the exact selected theorem/control counts externally.

Local workspace verification passed 422 normal tests and 20 doctests, alongside
Clippy with warnings denied, rustdoc link checks, formatting, actionlint and the
default-build filesystem-injection boundary probe. Logs are retained under
`/tmp/kv9-c01-*`. The first full run exposed a test-harness bind/rebind race
(`Address already in use`); both affected runtime helpers now retain bound
listeners and transfer them through the existing startup override. No service
protocol changed for that harness repair.

Four single-defect mutations were run through `scripts/mutation-guard.sh` at
commit `4803872ff1a72182a41d5e0888eebaf57ac4b0bf`. Every cell selected exactly one
test, passed its baseline, failed at the expected invariant after the one edit,
and passed after restoration:

| Removed protection | Discriminating test |
|---|---|
| Directory/ancestor publication | `acknowledged_vote_survives_loss_of_unsynced_namespace` |
| Failed-writer poisoning | `partial_append_failure_prevents_later_acknowledgment` |
| Vote-record data sync | `acknowledged_vote_survives_loss_of_unsynced_namespace` |
| Recovered-prefix data sync | `recovered_unsynced_tail_is_durable_before_a_repeated_crash` |

Raw Cargo outputs, including assertion text, are retained in
`/tmp/kv9-c01-mutation-capture-*`; guard logs are in
`/tmp/kv9-c01-mutation-*.log`. The default-build boundary was separately checked
in an owned archive checkout: removing only the testing-module feature guard
made `fs-testing-boundary.sh` fail at its production-injection assertion; baseline
and restored source passed. Its logs are `/tmp/kv9-c01-fs-boundary-{baseline,mutant,restored}.log`.

The default production binary at the same implementation commit passed the real
Chaos Mesh Pod/Network matrix, including every-voter failure and typed isolated
read/write refusal. Local log: `/tmp/kv9-c01-chaos.log`, exit 0. Retained evidence:
`/tmp/kv9-chaos-e2e.gGkNrH`, including all seven fault resource records. This run
checks the OS-backed runtime under application/network faults. The subsequent
I/O increment passed all six real Chaos Mesh voter/EIO/ENOSPC cells, including
explicit fatal exit, majority service and recovered apply progress. Log:
`/tmp/kv9-c01-io-chaos-reopen.log`; retained scene: `/tmp/kv9-chaos-e2e.PhkuUk`.
See [the I/O validation record](CHAOS-IO-VALIDATION.md) for exact scope and
backend limits. Actual host power-loss experiments remain separate obligations.

The terminal-failure increment passed 423 normal tests and 20 doctests, Clippy,
rustdoc, formatting, actionlint, and the unchanged nine-theorem/four-control
proof inventory. Its three additional single-defect mutations ran in the isolated
worktree `/tmp/kv9-c01-terminal-controls` at `dca83b6`: permitting inbound work
after failure, hiding driver fatal status, and reintroducing a Ready-lock panic.
Each selected exactly one discriminating test with a green baseline, intended
failure and green restored run. Logs: `/tmp/kv9-c01-terminal-controls.log` and
`/tmp/kv9-c01-terminal-capture-*`.
