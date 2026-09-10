# Raw engine group commit

Tracking: #20, #41 and #9. Candidate based on exact runtime b3cbc35.

## Contract and scope

The driver composes a consecutive, already-committed Raw command prefix into
one existing engine `WriteBatch`. It writes that batch at the final command's
exact applied `(term, index)` and publishes all individual receipts only after
the write succeeds. It adds no batching delay, pending queue, service or file
format. Raft persistence, quorum confirmation and read barriers are unchanged.

A group contains at most 128 commands and 1 MiB of encoded command input. A
larger legal command takes the existing singleton path. These bounds cover the
additional group input, not total process RSS or all preexisting queues. A
non-Raw command, no-op, configuration entry, undecodable entry or size limit
ends the prefix. A later invalid entry is processed after the preceding valid
group; failure stops the driver without publishing a driver watermark past it.

Grouping validates every mutation's Default CF, complete Raw key prefix and
non-system keyspace. A `Fenced` or `Write` tag alone is insufficient: internal
commands can carry arbitrary physical keys. Catalog transactions and manifest
changes always use the existing singleton path, including when their payload
contains a Raw key. Fenced groups also require the adjudicator's explicit
`independent_of_raw_writes` contract; its default is false. The production
catalog adjudicator opts in because its only authoritative read addresses a
region row under the disjoint System key prefix. Arbitrary adjudicators retain
single-entry application.

## Ordered-state refinement argument

Let `S` be the engine state before a committed group `c1 ... cn`. Let `Ri(S)`
be command i's fence verdict (or unconditional success), and let `Bi(S)` be its
ordered mutation batch, empty for a deterministic stale-fence rejection.

1. Every preceding group mutation changes only validated non-system Raw keys.
   Catalog reads use System keys. The adjudicator contract therefore gives
   `Ri(S) = Ri(apply(B1 ++ ... ++ B(i-1), S))`. No metadata/manifest command is
   hidden inside the group. A read failure is an apply error, never a verdict.
2. An engine batch applies mutations in their supplied order. Consequently
   `apply(B1 ++ ... ++ Bn, S)` equals sequential application of all n batches.
   This includes repeated overwrites, deletes and empty rejected batches.
3. Every position is checked before publication: indices strictly advance
   beyond the current applied index; terms are nonzero and nonregressing,
   including the previously durable engine position. The final position is
   copied from the last command, never synthesized from separate maxima.
4. The existing positioned-write contract durably couples all mutations and
   that final position. A successful write therefore establishes the complete
   committed prefix through the tail. WAL replay already permits position
   gaps; intermediate engine records are not required for reconstructing this
   state. Replaying those Raft commands after recovery skips indices covered by
   the recovered position.
5. Live receipts keep each command's exact term, index and original verdict.
   They become visible only after the common durable write returns success.
   Before-effect failure publishes neither data nor receipts; after-effect
   failure may recover the entire group, but still returns no success receipts
   and poisons the live driver. A failed write's outcome remains unknown.
   Existing frame recovery either reconstructs a complete valid record or
   discards/refuses an incomplete/corrupt record under its stated contract.

Atomic visibility of several consecutive commands admits their original log
order as the serialization order. The applied/SM lock order and driver-applied
publication remain unchanged. No public quorum read can receive a barrier
ahead of a failed group. A checkpoint freezes the composed state and its exact
tail atomically; it never observes an invented intermediate position. The
engine's local data revision still changes on each nonempty Raw application;
its consumer tests equality for change detection, not commands-per-revision.

This is a written source refinement using the existing atomic batch/WAL
contract, not a machine-checked proof of Rust. Parameterized composition proof,
source mutation controls and actual exact-candidate fault histories remain
acceptance requirements. The candidate does not close #20 or a roadmap item.

## Validation plan

Resident tests execute real state-machine and Raft driver paths for ordered
overwrite/delete composition, per-entry stale verdicts, later epoch-read
failure, before/after durable-effect failures, every intermediate position,
physical namespace restrictions, opt-in restrictions, catalog epoch barriers,
entry/byte bounds and a legal oversized singleton. Real legacy and segmented
WAL tests observe one sync for three commands, reopen one composed record with
the exact final state/position and verify replay does not apply it again.

Run the normal workspace tests, warning-denying all-target Clippy and the real
three-process Raw fixture locally before freezing a measured revision. Use the
unchanged paired Redis-reference protocol, keeping all outcomes and exact
build identities. Actual MinIO/Chaos Mesh and checked proof composition remain
separate gates. No throughput result is claimed until the candidate is measured.

The initial local workspace run passes 601 tests/doctests with 23 intentional
ignores; warning-denying workspace all-target Clippy and the default binary
build pass. All nine new Raw grouping tests pass, including both real WAL
layouts. The first selected-test compile attempt missed a trait import and the
driver helper's Arc return type; that failed attempt is retained and not counted
as a protocol failure. The corrected selected run and complete workspace run
pass. Exact-candidate process, benchmark, proof and Chaos acceptance are pending.
