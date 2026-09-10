# WAL topology publication and reclamation protocol

This proof targets `crates/engine/src/wal_stream.rs` at
`1698f6ccfc42deca618ed6f30c22155119f32863`. Its boundary is rotation, checkpoint
adoption, selected-tail recovery and reclamation within one existing segmented
stream. Offline migration, initial stream creation, Raft log truncation, backup
retention, throughput and all of C04 are outside this boundary.

## Composition and explicit premises

The [model](../proofs/tla/wal_topology/WalTopology.tla) composes with the single-file
segment protocol published in commit
`87801875308aebc0551c87169aab492145e49ac9`:

- Exclusive store ownership supplies one serialized stream owner. Selected file
  tokens denote the exact stream identity, sequence and predecessor header.
- The primitive supplies inseparable data/position frames, synchronized successful
  acknowledgements, immutable selected closed files, complete validated replay,
  permitted unacknowledged active tails and failure fencing. The topology proof
  tracks complete durable logical records; the primitive owns byte cuts and
  uncertain unacknowledged frame persistence.
- Header-chain and position checks supply ordered records within and between
  selected files, including gaps and empty/unpositioned segments. Record tokens
  identify logical effects, not keys or physically materialized checkpoint rows.
- A checkpoint certificate binds exact committed scope/position and durable full
  state. `WTCover` is the history that this state accounts for. The caller must
  certify that authority and complete restoration before publication of recovered
  state. A local position comparison or upload receipt supplies neither premise.
- Checked topology bytes bind the anchor, retained closed suffix and active file
  together. The model assumes detection of the corruption it abstracts; it does
  not prove SHA-256 collision freedom, authenticate storage or verify Rust codecs.
- Successful file and parent-directory synchronization have their documented
  persistence effects. Before rename durability is known, recovery may observe
  either the complete old or complete new topology. Missing or invalid roots and
  retained files refuse recovery; they do not select an orphan or grant a writer.

The later version-4 local-base migration is not covered. A base that captures
unpositioned legacy records requires a distinct frozen-source/image authority;
this proof's ordinary checkpoint coverage deliberately excludes those records.
Base publication and retirement need a separate extension.

No theorem infers database availability from one local WAL. Replication and
object-store checkpoint authority remain separate component obligations.

## State and source correspondence

Visible and durable topologies are distinct records containing the checkpoint
anchor, first retained segment and active segment. The selected files form the
contiguous suffix from first through active; all preceding selected descriptors
are closed. This compact representation follows `Topology::validate`'s exact
chain checks and retains the active file even when its contents are covered.

Visible and durable file-presence sets separately represent successful unlink
and directory synchronization. A crash can restore any subset of files whose
unlink was not synchronized. Reclamation leaves ghost record content for the
proof to audit; physical presence alone determines whether a file can be opened.
Future orphan files may remain present, but their names never grant selection.
An incomplete unpublished successor has no validated segment token in this
abstraction: recovery ignores its bytes, and rotation retires its exact path
before synchronized creation. The primitive and decoder premises own that byte
validation boundary.

Acknowledgement here means successful local append; client and consensus
acknowledgement conditions remain separate composition obligations. These records
must remain accounted for by both possible recovery roots:
the root's certified checkpoint state plus its selected durable file contents.
During publication, no acknowledged successor record may depend on a root whose
parent directory has not been synchronized. Uncertain append failure or
ambiguous topology publication fences the owner; recovery is the only path back to usable writer
authority. Cleanup failure can leave previously unlinked garbage unsynchronized
and retains the already safe durable topology; it does not require fencing.

| Model transition | Source correspondence |
| --- | --- |
| `WTInit` | Entry after `create_new` has durably established an empty existing stream; creation itself is an explicit premise. |
| `WTAck`, `WTUnknown` | `append` uses the selected active primitive, synchronizes before success and poisons the stream if that call fails. Pre-admission validation refusal stutters; uncertain append may leave an unacknowledged complete frame. |
| `WTRotate` | `rotate` checks capacity/sequence bounds, poisons the stream and consumes the old writer through `seal`. No later branch may return authority before publication succeeds. |
| `WTCreate` | The exact unpublished successor is retired and recreated through `WalSegment::create`, which synchronizes its header and namespace before topology selection. Orphan body bytes are never replayed. |
| `WTCheckpoint` | `checkpoint_applied` checks the certified locally covered anchor, monotonic authority and unpositioned pins, then constructs the new anchor and retained set together. |
| `WTTemporarySync`, `WTRename`, `WTParentSync` | `publish` writes and synchronizes the complete temporary topology, renames it over the root, then synchronizes the parent. Old/new root ambiguity remains explicit until the last step. |
| `WTInstall` | Only successful publication assigns the in-memory topology, installs the successor when rotating and clears the poison flag. |
| `WTFail`, `WTCrash` | Failed publication leaves no write authority. A later crash chooses a permitted complete root and a permitted persistence outcome for prior unlinks. |
| `WTRead`, `WTRestore`, `WTRecoverySync`, `WTReplay` | `RecoveryPlan::read/recover` decodes the selected root, certifies/restores its anchor, checks the exact root bytes again, stabilizes the root, then fully validates the selected files before returning the writer. |
| `WTRefuse` | Failed checkpoint restoration or later validation discards unpublished recovery effects and returns no writer. |
| `WTUnlink`, `WTCleanupSync` | `reclaim_obsolete` removes only whole files preceding the retained suffix under the durably adopted anchor, then synchronizes that stream directory. Foreign streams, future orphans, straddling closed files and the active file are not reclaimed by checkpoint cleanup. |

The model permits conservatively retaining additional covered closed files. The
implementation chooses the first uncovered summary and uses predecessor coverage
for empty files. That concrete choice satisfies the model's coverage guard;
retaining additional files grants no additional deletion authority. A straddling
file stays intact and is validated completely before covered records are filtered.
The exact retained-tail claim concerns the selected record set; ordered replay
and data/position binding compose through the segment and header-chain premises.

## Deductive safety and conditional progress

The [TLAPS proof](../proofs/tlaps/wal_topology/WalTopologyProof.tla) is parameterized
by arbitrary positive segment capacity, record identities, unpositioned records,
certified checkpoint coverage and an arbitrary reclamation target segment.
Finite TLC instances are counterexample searches; they do not discharge these parameterized obligations.

The inductive invariants establish selected-file availability, exact anchor/set
coverage, acknowledged-record recoverability under either namespace outcome,
exclusive usable-writer authority, checkpoint pin preservation and exclusion of
orphan election. Separate action theorems establish deletion authority, refusal
of acknowledgements before durable selection, failure fencing and exact replay
of the certified anchor plus retained uncovered records.

| Named proof boundary | Established claim |
| --- | --- |
| `WTInvariantInit`, per-action `*Step`, `WTInvariantAlways` | Initial safety and preservation by every protocol action and stuttering, for all legal parameters. |
| `WTAcknowledgedRecovery`, `WTWriterAuthority`, `WTOrphanExclusion` | Both crash outcomes account for every acknowledgement; usable authority follows the selected durable root; recovery plans cannot elect orphan names. |
| `WTCheckpointContent`, `WTReplayExact`, `WTReplayStep` | An admissible anchor/set transition preserves logical contents, and successful replay publishes exactly the certified anchor plus the retained uncovered tail. |
| `WTDeletionAlways`, `WTClosedContentsPreserved` | Unlinked files are outside the durable retained suffix and wholly checkpoint-covered; selected closed contents stay immutable. |
| `WTAcknowledgementAlways`, `WTAckMonotonicPreserved`, `WTFencingAlways` | New acknowledgements use durably selected authority, historical acknowledgements are monotonic, and a fenced owner cannot resume without recovery. |
| Per-stage `*Progress`, `WTProgressProof` | An admitted operation settles under the explicitly fair, successful local continuation. |
| `WTUnlinkProgress`, `WTDurableUnlinkProgress`, `WTReclaimProgress` | An arbitrary already-eligible target is eventually absent from both visible and durable file sets under its unlink and directory-sync fairness. |

Publication progress starts only after capacity and coverage checks admit an
operation. Fair local execution with successful I/O and no further crashes
settles it in a usable writer or explicit recovery refusal. Cleanup progress is
separate: an already eligible covered file is eventually unlinked and the unlink
becomes durable when its unlink and directory synchronization receive fair,
successful execution. No theorem promises free capacity without a sufficiently
covering committed checkpoint, through unpositioned pins, or during endless I/O
failure.

## Local verification gate

```sh
python3 scripts/check-wal-topology-protocol.py \
  --tlapm /path/to/pinned/tlapm \
  --jar /path/to/tla2tools-v1.7.4.jar \
  --output /tmp/wal-topology-protocol-new
```

The gate preserves the existing pinned-tool, semantic-inventory, exact-obligation
and strict fresh-cache conventions. Independent control groups keep separate
copied sources, caches and logs. Every protocol fault must produce the intended
TLC counterexample and named failed proof, followed by an unchanged successful
restoration. Syntax failures, timeouts and unrelated failed obligations do not
count as successful controls. Semantic and output controls also reject omitted
proofs, custom axioms and incomplete reported checks.

The protocol controls remove one mechanism at a time: acknowledgement before
parent synchronization, unlink before durable anchor publication, mismatched
anchor and retained set, dropping an uncovered closed file, namespace stabilization
before checkpoint restoration, loss of the unpositioned pin, loss of failure
fencing, orphan successor election, discarding the replay tail, missing parent-sync
fairness and missing unlink fairness. Five additional reachability witnesses
exercise acknowledged writes, ambiguous roots, non-durable unlink, future orphan
presence and pinned unpositioned history.

An optional `--resume-from /tmp/terminal-prior-attempt` revalidates retained
executions with identical copied inputs and expected results. It checks original
input bytes and commands, freshly repeats semantic inventory audits, and rechecks
every retained log through the same verdict functions. Reused records carry the
original result/log hashes and execution commands, including the original strict
fresh-cache TLAPS invocation. Missing, rejected or stale-input cases execute
fresh; modified retained bytes, commands or incomplete output are refused. The
prior attempt remains unchanged. Each JVM uses its own temporary directory so
concurrent SANY processes cannot delete another process's extracted standard
module before its hash is checked.

The acknowledgement and unpositioned reachability witnesses use the shared
validator's existing two-state negative-trace allowance. Positive exploration,
protocol-fault thresholds, complete temporal checks and named action coverage
retain their existing requirements.

Actual syscall-cut experiments are separate runtime evidence coordinated on the
mainline. This proof worktree runs no Cargo build, MinIO fixture, Chaos Mesh
experiment or hosted workflow.

## Accepted local evidence

The September 9, 2026 gate at `/tmp/kv9-wal-topology-protocol-fifth` completed
with exit 0 and an independent source, log, semantic-inventory and provenance
audit:

- 52 owned TLAPS declarations and exactly 647 obligations per positive proof.
- 44 TLC cases. The two complete safety models explored 3,587 and 28,260
  distinct states under both fingerprints; fair publication and cleanup
  continuations explored 12 and 4 states.
- 40 proof/audit cases: 27 distinct strict fresh-cache positive proof executions,
  11 named protocol rejections and two semantic rejections. This includes 23
  positive executions retained from the third run and four newly executed in
  the final run; reused logs are not counted as additional executions.
- Eleven baseline/fault/restored protocol groups, five reachability witnesses,
  two semantic control groups and eight output controls passed. Five additional
  retained-evidence control triples reject changed input bytes, missing completion
  and changed tool commands, and require fresh execution for stale or rejected
  prior cases.

The final gate revalidated 73 completed case records from the third attempt,
including fresh semantic audits, and executed its missing cases. The third run
remains a rejected whole-gate attempt, as do the earlier failed matcher/coverage
attempts and the fourth run's JVM temporary-file race. Their original records,
logs and source snapshots remain preserved. The final run and subsequent proof
work use CPUs 6–31; prior executions ran on the shared host without that affinity.
This document makes no performance or new runtime/Chaos Mesh acceptance claim.

Local evidence archive:
`/home/dongxu/kv9/target/correctness-evidence/2026-09-09-1698f6c-wal-topology-proof.tar.gz`
(3,790,820 bytes, 2,405 files), SHA-256
`8a12d3e07152b0a19e1ea939f0ac4f2c922f87956a97bf66ea9111243b844073`. Every archived file was read back and checked against its manifest.
