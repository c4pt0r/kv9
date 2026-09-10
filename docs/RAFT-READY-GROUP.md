# Common synchronization for one original Raft Ready

Tracking: #20 and #9. This performance candidate extends append-slice batching
and Raw engine group commit. `DiskRaftStorage::persist_ready` synchronizes the
ordered entry frames and the original Ready HardState together. A later
LightReady commit retains its separate durability boundary. Candidate acceptance
still requires exact-source protocol composition and process/fault evidence.

## Contract and implementation

`PersistentRaftStorage::persist_ready(entries, hs)` has a default implementation
that calls the existing synchronous append and HardState operations. The disk
implementation holds its existing writer mutex across these steps:

1. Write each entry using the unchanged checksummed frame format, in order.
2. If supplied, write the complete term/vote/commit HardState frame after every
   entry frame. No memory state is published by these writes.
3. Synchronize once if there is any persistence work.
4. Under one memory write lock, append the entries and set the supplied
   HardState. Return success only after this publication succeeds.

Any error fences the writer. The Ready caller records a fatal peer error and
discards pending output. It neither advances this Ready nor exposes its messages,
read states or committed entries. Original Ready persistence still precedes
`advance_append`; a resulting LightReady commit is synchronously persisted before
the cycle becomes externally eligible. Configuration persistence is unchanged.

The caller skips storage entirely for a Ready with neither entries nor
HardState. An explicitly empty disk call does no I/O and still refuses a fenced
writer. There is no added timer, buffer holding a whole group, queue, or service.
Frame encoding retains its per-record allocation bound. The upstream caller owns
the entry-slice bound; this is not a whole-process memory-bound claim.

## Recovery argument

Assume valid upstream Raft input, a serialized writer, and the existing storage
contract: successful sync makes preceding bytes durable, and recovery replays
only the complete valid frame prefix and synchronizes it before reopening. A
checksum collision or arbitrary corruption is outside that contract. Raft must
preserve its committed prefix and supply a HardState commit covered by the log
after the supplied entries are applied. This change does not establish those
consensus premises.

Let `L0` and `H0` be the prior recoverable log and HardState, `E` the ordered
supplied entry sequence, and `H1` the supplied HardState when present. Let
`L1 = append(L0, E)`, where replaying a replacement entry removes the old suffix
starting at that entry's index. The following cases exhaust complete prefixes of
the bytes appended by this call:

- No new complete frame: recovery retains `L0, H0`.
- Some complete entry frames: recovery applies exactly that ordered prefix of
  `E`, retaining `H0`. The first replacement frame removes the old suffix, so
  replay cannot splice an old suffix behind a new one. The previous committed
  prefix remains covered by the upstream input premise.
- All entry frames, without a complete new HardState: recovery yields `L1, H0`.
- A complete new HardState: its position after all entry frames implies recovery
  has already reconstructed `L1`; recovery yields `L1, H1`. In particular, the
  new commit cannot become recoverable ahead of its supporting log.

These cases apply to any finite entry sequence, including an empty sequence.
With no supplied HardState the last case is absent. A reported failure may retain
any allowed complete prefix, including the whole group. An error after the sync
effect retains the complete group but does not authorize a successful reply.
Successful return implies the complete group was synchronized before any member
became visible in memory. An I/O failure implies no member was published in
memory and the writer cannot perform another operation until recovery. A memory
publication failure is also terminal; the no-memory-change assertion applies to
pre-publication encoding/I/O failures, not an arbitrary failure inside memory
publication itself.

Thus `commit <= recoverable log end` is preserved, and a failed cycle never gains
publication eligibility. The subsequent LightReady sync preserves the stronger
existing chain `acknowledged <= applied <= delivered <= durable commit <= log
end`. Physical responses for earlier successful cycles can still be in flight.

In the existing [Ready publication model](READY-PUBLICATION.md), success
corresponds to the composition of `RPPersistEntries` and
`RPPersistHardState`. A failed entry prefix corresponds to `RPFailBefore` or
`RPFailAfter` at the entry phase; complete entries without a new HardState
correspond to entry persistence followed by HardState failure; a complete new
HardState corresponds to entry persistence followed by `RPFailAfter` at the
HardState phase. These are multiple abstract steps for one concrete call. The
existing model has separate actions; its theorem must not be relabeled as a
mechanically checked refinement of this new combined operation. Parameterized
composition, term/vote/log identity and full implementation refinement remain
explicit acceptance work.

The combined call has finitely many writes and one sync for finite valid input.
Conditional on successful finite storage calls and fair driver execution, it
introduces no extra wait before Ready processing can continue. This gives no
latency bound, election guarantee, or liveness under repeated I/O failures.

## Deterministic implementation validation

The actual disk implementation passes 1,584 named write/sync/error/crash cells:
entry-only, HardState-only, extension with commit, and replacement with commit.
Every write and sync receives EIO and ENOSPC before and after its effect; every
write also receives a partial frame. Each failure is followed by loss, retention
and 16 seeded outcomes for unsynchronized bytes. Checks cover unchanged memory,
terminal fencing without further I/O, exact old/new suffix identity, commit
coverage, after-sync effects and a second immediate crash after reopening.

A real Raft follower supplies a joint Ready and emits a successful AppendResponse
in the positive case. All 22 named I/O failures suppress that response and fence
the peer. A real single-voter election observes one original-Ready sync and a
separate LightReady commit sync. Its subsequent ReadIndex performs no persistence
I/O. An independent acknowledged-Ready crash test checks recovered entries and
complete term/vote/commit without relying on sync-count assertions.

`scripts/check-raft-ready-controls.py` copies the actual source into an isolated
tree, records all copied file hashes, and runs four baseline/mutant/restored
triples. All baselines and restorations pass; compiling mutants fail the named
semantic assertions for omitted sync, memory publication before sync, ignored
sync errors, and HardState written before entries. Compilation failure, an empty
test selection, or an unrelated assertion cannot pass this gate. These are
`ModelFs` executions, not physical power loss or Chaos Mesh.

Local workspace validation passes 608 tests and doctests, with 23 ignored
external/optional tests. Warning-denying all-target workspace Clippy and the
default production build pass. The three existing append-batch source-control
triples also pass again after extraction of their shared unsynchronized frame
writer. The first real-follower test compile attempt used a nonexistent snapshot
field; its rejected transcript is retained. The corrected test checks repeated
peer refusal and absence of further I/O after failure. That compile attempt is
not counted as a semantic negative control.

## Performance and acceptance boundary

Only Ready calls containing both entries and HardState save a sync. Entry-only
and HardState-only calls retain their previous sync count, and the later
LightReady boundary cannot be removed for throughput. Single-outstanding-request
writes can therefore retain most of their latency. The unchanged paired Redis
reference protocol must measure the actual candidate before any benefit is
claimed. In-memory data residency does not remove kv9's durable quorum contract;
the standalone Redis reference has persistence disabled.

The preceding `fe650ed` candidate's actual MinIO/Chaos results and performance
numbers are not evidence for this source revision. Local workspace and process
checks, candidate-specific measurements, actual faults and checked protocol
composition are tracked separately. Main runtime and the original #9 checklist
remain unchanged; this increment does not complete #20. Daily verification runs
locally without dispatching GitHub Actions.
