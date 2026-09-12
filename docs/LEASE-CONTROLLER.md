# Rust lease controller and integer timing validation

Tracking: [#9](https://github.com/c4pt0r/kv9/issues/9), #20.

The [proved transition protocol](LEASE-AUTHORITY-MODEL.md) now has a Rust
algorithm component in [`kv9_raft::lease`](../crates/raft/src/lease.rs). It is
compiled by unit tests or the explicit `experimental-leader-lease` feature.
There is no server configuration switch, message adapter, or lease read path
yet. Default runtime remains selected `11113f6` / Safe ReadIndex. No new
throughput, latency, clock-platform or actual lease Chaos result is claimed.

## Implemented behavior

`LeaderLease` stores one pending renewal, one active certificate, a monotonic
revocation generation, a non-reused round sequence and the latest observed
commit frontier. Exact ACKs are deduplicated in a set bounded by the immutable
voter configuration (at most 64 voters). Self counts only after its own voter
has made the promise. ACK collection does not publish read authority.

`VoterLease` retains its longest voting hold and a single latest grant for
idempotence/reordering checks. Every process start establishes a complete
conservative recovery quarantine, without trusting erased volatile state or
comparing clock origins across restarts. Higher-term observations do not erase
an old promise. The gate applies to actual votes and self-votes; it does not
replace Raft's ordinary election or durable-vote rules.

| Proved protocol event | Rust implementation |
| --- | --- |
| Start round | `start` records a send-time deadline and exact group/configuration/leader/term/incarnation/generation/sequence/promise identity. |
| Grant and deliver | `promise` establishes the hold; `acknowledge` checks the entire identity and counts each configured voter once. |
| Publish authority | `publish` independently rechecks current authority, current-term commitment, pending identity, expiry and quorum. |
| Revoke/rearm | `revoke` advances generation and discards pending/active authority; `rearm` creates no certificate. |
| Crash/recovery | `recover` starts voter quarantine; a new leader component has no active certificate. |
| Read begin/finish | `begin_read` captures a fresh committed frontier and original request deadline; `finish_read` consumes the ticket and validates its exact generation, deadline and retained view position. |
| Fatal/overflow | `fence` is terminal; generation/sequence/deadline overflow, observed clock-domain change or time regression cannot wrap or restore authority. |

A read ticket can retain an older published certificate while a subsequent
round is renewed. Its own deadline does not move. The ticket is neither Clone
nor Copy, enforced by a compile-time guard. A successful `ReadDecision` is an
algorithm result, **not** the driver's private `ReadBarrier`; it cannot establish
a production read view on its own.

The user's service rule is explicit: an expired certificate cannot authorize
read success. A fresh lease or Safe ReadIndex must establish new authority.
Failure to obtain quorum confirmation means typed unavailability or the original
timeout, never cached/empty success or a locally extended deadline. Writes retain
Raft majority-commit requirements.

## Integer timing proof

Let `S = 1e9`, `0 <= drift < S`, `a = S-drift`, `b = S+drift`, and `E` be
the stable voter-promise duration in nanoseconds. With qualified sampling margin
`m`, the constructor computes

```text
leader_window = floor(E*a/b) - m
recovery_quarantine = ceil(E*b/a) + m
```

It refuses zero/negative usable duration, invalid rates and unrepresentable
results. Products use `u128`; outputs and added local deadlines use checked
`u64` conversions/arithmetic. This keeps the [real-clock containment proof](LEADER-LEASE-PROOF.md)
conservative instead of rounding a lease outward.

The [integer proof inventory](../proofs/smt/lease_timing/inventory.json) first
checks the floor and ceiling division inequalities for arbitrary nonnegative
numerators and positive denominators. The composed constructor theorem then
instantiates these checked lemmas at `(E*a,b)` and `(E*b,a)`. It proves
`leader_window*b <= E*a`, `recovery_quarantine*a >= E*b`, and conservative
`u128` intermediate bounds for all successful constructor inputs. The stated
input ranges imply both instantiations' premises. Rust integer division and
`div_ceil` are trusted arithmetic primitives in this mapping; this is not a
formal proof of the Rust compiler or standard library.

The initial monolithic nonlinear/division solver query timed out after five
seconds. It remains inconclusive. Decomposing the division lemmas completed
the same safety claim within the unchanged five-second bound; the final
dependencies and original failed query are retained. Wrong floor/ceiling,
missing leader drift allowance, and downward recovery rounding each yield a
satisfying countermodel.

## Source validation

[Retained validation](lease-controller-v1/README.md) records:

- 205 Raft library tests pass, including 25 new lease tests; zero failed or ignored.
- Default test compilation, explicit experimental-feature compilation, formatting
  and experimental all-target Clippy with warnings denied pass locally.
- The same component also passes all 25 tests under standalone optimized `rustc`.
  Eight source mutations fail their intended tests. A Clone mutation fails the
  exact compile-time guard. Original/restored source hashes agree.
- Three integer arithmetic checks pass with four countermodels for incorrect
  variants. The exact pinned solver, proof dependencies and source inputs are
  recorded. No hosted CI or benchmark is dispatched.

The drift test advances the leader/self clock by nine units and the other
voters by eleven per ten real-time units. While a local read is authorized, an
election quorum cannot pass the voting gates; when enough voters can vote, the
leader's read is already expired. Other tests cover delayed/duplicate ACKs,
round mixing, revocation, completed versus overlapping writes, apply lag,
request deadlines, restarted clock origins, preserved terms and overflow.
Fault controls are source tests; they are not distributed linearizability or
Chaos Mesh histories. The 205-test and 25-test populations overlap.

## Remaining adapter obligations

The component deliberately accepts algorithm observations. It does not prove
that a caller supplies truthful Raft terms/frontiers, qualified clocks or actual
immutable-view positions. The production adapter must establish those facts:

1. Install exactly one controller for its owned peer/leader lifetime and mint
   a unique incarnation. Never reconstruct a controller with reused round identity
   or accept another controller's ticket. Bind immutable membership and maximum
   promise policy across restart/upgrade; a changed policy cannot shorten a
   surviving promise. The public experimental constructor is not this private
   installation capability.
2. Gate all actual votes, local campaigns, tick-driven self-votes and forced
   transfer paths before raft-rs can grant them. Capture the current local term,
   membership and leader from the same serialized peer state. A network field
   cannot substitute for those observations.
3. Add an exact, versioned renewal envelope and ACK path. Hold grants before
   replying; persist Raft state before outbound publication. Activate a leader
   certificate only after the whole owning pump succeeds. Fence fatal persistence
   or apply errors and authority/configuration transitions.
4. Sample the clock under the authority gate. Capture the current committed
   frontier, acquire its exact applied immutable view, validate metadata/range
   context on that same view and perform final lease validation. Keep admission,
   cancellation and original invocation budgets; never synthesize a Safe
   ReadIndex result from a lease decision.
5. Qualify the actual clock and run source/refinement checks, local fault tests,
   and actual Chaos Mesh partition/delay/pause/restart histories for the exact
   candidate binary. Existing Safe ReadIndex results cannot qualify it.

Only then compare matched GET/mixed/batch throughput and latency against the
retained Safe ReadIndex and Redis baselines. The controller is implementation
progress toward that gate, not a runtime promotion or full Raft refinement proof.
