# Coalesced owner notifications

This candidate suppresses `Condvar::notify_one` when `WorkSignal.pending` is
already true. It starts from selected CRC runtime behavior on main `2516c4e`.
The previous peer-scheduling diagnostic recovered `WorkSignal::notify` in
124/2,967 CRC CPU samples, including kernel wake work. That identifies a target;
it does not establish how many calls are redundant or predict a speedup.

## Implementation boundary

`crates/raft/src/work.rs` changes the notify predicate from `!stopped` to
`!stopped && !pending`. The existing mutex still covers setting the hint,
consuming it before draining, and checking the predicate before parking.
A first notification wakes a parked owner. An already pending hint prevents
parking, and any notification after the owner consumes the hint rearms it.
Stop remains terminal and still broadcasts. Queue publication precedes the
hint; notification does not grant consensus, persistence or read authority.

This change does not alter the Raft quorum, fresh Safe ReadIndex, sealed read
group membership, successful pump/apply/view fences, durable write receipt,
deadline, cancellation, admission or reservation boundaries. It adds no worker,
lease, polling loop, generation counter or runtime instrumentation.

## Proof and concurrent tests

`CoalescedWake.tla` extends the unchanged `RaftSchedule` model. Its wake delivery
condition includes the false-to-true pending transition. `RSParkedSignal` implies
that a parked owner with a pending hint already has a delivered wake. The new
and old notify/hint actions are therefore equivalent on invariant states.
The proof lifts this equivalence to all steps, stuttering, terminal stop, and
conditional service/hint progress. The original fairness premise is inherited;
no repeated-notification or timeout fairness premise is introduced.

Run locally with the repository's pinned TLC 1.7.4 and TLAPS `4600b24`:

```sh
python3 scripts/check-coalesced-wake.py --jar /path/to/tla2tools.jar \
  --tlapm /path/to/tlapm --output /path/to/new-proof-output
```

The checker audits all imports, assumptions, theorem declarations and holes,
then freshly verifies the existing 33-theorem/294-obligation scheduling proof
and the new 14-theorem/64-obligation refinement. Finite two/three-capacity
models use two fingerprints; a fair continuation checks service progress.
Removing the first physical wake must fail both the finite invariant and the
deductive equivalence. Omitted proofs and injected assumptions must be refused.

Three channel-coordinated Rust tests cover a parked owner followed by multiple
producers behind one pending hint, publication during a bounded drain, and stop
waking the owner without allowing later publications to restart it. Timeouts
bound failures; they do not select the interleavings. Cleanup releases gates,
stops the signal and joins owned threads before assertions.

The abstract proof assumes the documented single-owner mutex/condition-variable
contract; it is not a machine-checked refinement of Rust or a proof of the whole
database. The tests check delivered work and terminal behavior, not the number
of physical wake system calls.

## Qualification and decision

The audited formal gate passes all 14 new theorems / 64 obligations and the
unchanged 33-theorem / 294-obligation dependency. Two earlier incomplete
stuttering derivations remain retained in the local development receipts.
Two-capacity and three-capacity finite checks explore 1,300 and 1,723 states
respectively with both fingerprints; the fair continuation explores 241 states.
The missing-first-wake model/proof controls, omitted-proof control, injected
assumption control and three proof-output controls all reject as intended.

Local release-profile source qualification passes 438 default Raft/Server
tests/doctests (one existing ignored test) and 214 Raft testing-feature
tests/doctests. These populations overlap. Formatting and warnings-denied
all-target Clippy pass for both configurations. The shared Cargo target is
locked and first-party units are invalidated before qualification; 30 artifact
observations and unchanged before/after source snapshots are retained.

Original local receipts are `/tmp/kv9-coalesced-owner-proof-first` and
`/tmp/kv9-coalesced-owner-source-first`. Process recovery, actual candidate
Chaos Mesh and performance are pending at this commit. No performance gain or
runtime promotion is claimed by this implementation.

Measure the clean retained candidate against the original CRC executable and
the fixed Redis client using c1 GET, c64 GET, and c64 mixed point operations.
Keep resource affinity, payloads, ordinary quorum/sync settings, run order and
latency accounting matched. Review mean and p99 as well as throughput; do not
expand a regressing candidate into a full matrix. Before promotion, complete
applicable point/batch correctness and actual Chaos Mesh acceptance. Tmpfs-WAL
loopback measurements do not establish durable-media or cross-host parity.
