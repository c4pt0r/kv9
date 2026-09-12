# Fixed-configuration leader lease transition proof

Tracking: [#9](https://github.com/c4pt0r/kv9/issues/9), #20.
This extends the [clock and history proof](LEADER-LEASE-PROOF.md) with an
executable protocol and an inductive safety proof. Selected runtime `11113f6`
still uses Safe ReadIndex. This checkpoint changes no Rust runtime or benchmark.

## What the proof establishes

[LeaseAuthority.tla](../proofs/tla/leader_lease/LeaseAuthority.tla) describes one
established leader/term, arbitrary intersecting election/grant quorums, arbitrary
nonzero round and read identifiers, and a higher-term replacement. Its
[TLAPS proof](../proofs/tlaps/lease_authority/LeaseAuthorityProof.tla) establishes
`LASpec => []LAInvariant` by initialization and all 18 next-action cases.
Time, committed indices and execution length have no finite bound in that
deductive theorem. The configured revocation counter fences permanently on
exhaustion; it never wraps.

The core invariant connects the following facts:

1. A published certificate contains a quorum of acknowledgements for that
   exact round. Each acknowledgement follows its recorded grant. Its immutable
   deadline is anchored at the round's start, before any grant.
2. Every recorded grant covers that certificate's deadline. A live voter retains
   the obligation through its current hold or recovery quarantine, even after a
   crash erased its volatile hold.
3. Any voter that already cast a higher-term vote has no unexpired recorded
   grant. A later lower-term request cannot acquire a grant from that voter.
4. A replacement requires an election quorum. Intersecting that quorum with
   the certificate's quorum would require a voter whose grant both expires
   before the current time and extends past the current time: a contradiction.
5. A read captures a fresh local committed frontier, waits for its applied
   prefix, retains that view, and finishes only under its still-live generation
   and deadline. Its view covers the frontier at its invocation.

`LAReadStepsAreLocal` separately proves that begin/view/finish/fallback steps
do not change renewal, grant, acknowledgement, vote, application, or committed
frontier state. `LAReadFinishAuthorized` connects a successful finish to both
the view fence and the absence of a replacement. Once a certificate exists and
application has caught up, the read can execute these local steps without an
intervening network step. This is the modeled zero-per-read-quorum-RTT claim.
It is not a bound on scheduler delay, local apply lag, or client/server latency.

An expired certificate cannot authorize `LAReadFinish`: its proven strict
deadline condition contradicts expiration. `LAReadFallback` is terminal for
this fast path and has no transition to success. In the implementation, a new
lease or fresh Safe ReadIndex must establish new authority before a read can
succeed. Expiration plus inability to obtain a quorum means unavailability or
the original timeout, never a local stale-read fallback. Writes retain Raft's
quorum-commit requirement. This service rule still depends on the clock and
voting promises being valid before the apparent expiration.

## Transitions and implementation contract

| Transition | Required behavior |
| --- | --- |
| Start round | Fresh identifier, original send-time deadline and revocation generation; never rebase on ACK arrival. |
| Grant | Exact established term/configuration/incarnation; reject prior higher votes; extend the voting hold before replying. |
| Deliver ACK | Add only that grantor's ACK to that same round; delayed or duplicate replies cannot move its deadline. |
| Activate | Exact pending round, live generation, unexpired deadline and quorum; publish only after the owning pump succeeds. |
| Revoke/rearm | Advance generation and clear active/pending authority. Rearming a retained role creates no certificate. Voter promises remain. |
| Voter crash/restart | Erase volatile holds on crash; retain durable higher votes; establish a conservative quarantine before voting or granting. |
| Leader crash/higher vote | Permanently fence this incarnation's lease state; restart cannot resurrect its certificates. |
| Higher vote/election | Enforce both promise and quarantine on every actual vote, including self-votes; election requires a quorum. |
| Read begin/view/finish | Capture a fresh frontier; acquire its exact applied view; validate the original certificate generation and deadline before success. |
| Read fallback | Abandon the fast path; implementation must retain the invocation's original admission/deadline/cancellation contract. |

Multiple renewal rounds may be outstanding. Starting another round does not
erase an older usable certificate; a late ACK for an older round cannot activate
it as the new pending round. Reads retain their own certificate while renewal
proceeds. Revocation invalidates certificates from the old generation together.
The abstract grant is idempotent per voter/round: repeated wire delivery is a
stuttering step. An implementation must preserve that identity rule.

## Exact abstraction boundary

- **Clock embedding:** `laNow` is mathematical observation time, not a shared
  clock service. Any actor can remain unscheduled while it advances. Grant
  durations range between `LAGrantMin` and `LAGrantMax`; leader usability is at
  most the minimum, and restart quarantine covers the maximum. The separate
  real-arithmetic proof supplies conservative physical-clock inequalities.
  This integer transition proof does not itself refine arbitrary real clock
  executions or qualify a concrete OS/VM clock, suspension behavior or rounding.
- **Raft composition:** initialization assumes a committed/applied current-term
  entry. Old/new commit actions abstract publication of agreed committed
  prefixes. They do not model packet replication, pending write replies, log
  truncation or elections in all terms. In particular, `laCommit` is not an
  omniscient timestamp of when bytes first reached a majority. The complete
  read history argument remains in the companion document.
- **Configuration and recovery:** the proof covers fixed intersecting quorums
  and one leader incarnation. It does not authorize dynamic membership,
  leadership-transfer exceptions, restarted certificate reuse, or follower
  lease reads. A new incarnation must acquire new authority.
- **Publication and views:** activation and apply are successful abstract
  publications. Binding them to Rust `Ready`/whole-pump completion, fatal error
  fencing, durable ACKs, exact immutable view/metadata checks, deadline and
  cancellation handling remains source-refinement work.
- **Safety and availability:** the model permits message loss, delay, arbitrary
  actor pauses, crash/restart and revocation interleavings. It proves safety,
  not eventual renewal or election. Recovery quarantine and outstanding promises
  can delay failover; they add no new service dependency or singleton.

The proof also tightens a previous hand-written explanation: write linearization
points cannot all be fixed at the instant of physical majority replication.
A pending write whose ACKs have not reached the leader can overlap a read that
legally precedes it. Ordering writes by log position and reads by their retained
prefix, while checking all four kinds of real-time precedence, gives the correct
linearizability construction. No runtime behavior changed with this clarification.

## Local validation and retained failures

The source-bound local acceptance and reproduction command are recorded in
[lease-authority-v1](lease-authority-v1/README.md). The deductive proof contains
27 theorems and 348 obligations. Its semantic gate checks the complete import
and theorem inventory, the exact legal-input assumption, pinned standard
modules, and absence of omitted proofs or added axioms.

The finite TLC configuration has three voters, all three majority pairs, one
round, one read, time 0 through 3, committed indices 1 through 2, leader window
1, grant/recovery duration 2 and revocation counter bound 1. Only the two
indistinguishable followers are permuted by symmetry. This preserves the
distinguished leader, quorum family, actions and checked safety properties.
The full successful exploration visits 9,674,978 distinct states. Finite model
checking supplements the parameterized proof; it does not establish unbounded
correctness by enumeration.

The first full exploration retained an unnecessary ghost vote timestamp and
hit the fixed 180-second bound. Removing that timestamp preserved all action
guards/effects on the remaining variables but still timed out without symmetry.
The timestamp was never used by an action; projecting it away preserves protocol
behaviors. The sufficient invariant now says historical grants of a higher
voter expired by the current time. The complete symmetric run kept the same
time/commit/round/read bounds and the same 180-second and 512-MiB limits. An
earlier time-zero-only run is retained as restricted exploration, not substituted
for the full configuration.

Fault controls generate actual TLC counterexample traces for lost restart
quarantine, granting after a higher vote, voting during a promise, publication
without a quorum, capturing an applied/stale frontier, and reading before apply.
Named reachability witnesses cover successful reads, replacement writes,
expiration, restart with a historical promise, apply lag, old generations and
two published rounds. The unconstrained two-round reachability search timed out
at 180 seconds and remains incomplete. The accepted renewal witness instead
replays an explicit 16-action sequence: acquire, read using three local steps,
advance past expiration, then acquire another round. Each step must satisfy the
unchanged protocol's bounded next-action relation. This is reachability evidence,
not an exhaustive two-round model run or a replacement for the failed search.
These traces are not Chaos Mesh E2E histories.

Failed proof drafts and a semantic-audit failure are retained. TLAPM accepted
a bound-variable shadow that the pinned SANY gate rejected; the final source
uses a distinct bound name and must pass both tools. No failed or timed-out
run is counted as acceptance. Hosted CI and performance measurement did not run.

## Next gate

Implement an isolated lease controller and bind its events to the actual Raft
voting/recovery and read-view paths. Establish source refinement and local
correctness before enabling a candidate. Then run actual Chaos Mesh histories
with asymmetric partitions, delayed renewal messages, process pauses, voter
restarts and failover; qualify the clock/observer mapping separately. Existing
Safe ReadIndex partition/delay/restart fixtures can help build this environment,
but their old results do not qualify a lease-enabled binary. Only after those
gates should a matched GET/mixed/batch throughput and latency campaign compare
the lease candidate with the retained Safe ReadIndex and Redis baselines.
