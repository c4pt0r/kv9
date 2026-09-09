# Bounded registration routing and seed coverage

Tracking: [#37](https://github.com/c4pt0r/kv9/issues/37). This complements the
[server scheduling/capacity repair](REGISTRATION-SCHEDULING.md). It changes client
routing policy, while retaining admission, identity and exact catch-up checks.

## Failure and implementation

Previously every retry began at the first declared seed. Each dial received the
whole remaining pass deadline. A seed that blackholed traffic could therefore
consume every pass without any healthy seed being contacted. A novel leader hint
was appended behind untried seeds, exposing it to the same starvation. The total
registration window also inherited the 50 ms discovery-probe timeout multiplied
by seed count, despite registration performing several durable consensus steps.

`NodeRuntime::advance_registration` now obtains a rotated copy of the immutable
root seed list from `RegistrationSeedCursor::order`. The cursor advances once per
pass, including a pass whose first dial consumes the whole window. A novel,
canonical, deduplicated leader hint is inserted at the next queue position.
Already queued or visited hints retain the existing cycle handling; a declared
seed can still get its own first turn on a later pass. The finite hint-hop cap
remains four. Registration gets a distinct five-second absolute pass deadline.
No redirect or dial extends it; discovery retains its separate probe budget.

The deadline is an operation budget, not a claim that all healthy deployments
complete registration in five seconds. Waiting backend work can outlive the
client deadline; server admission remains bounded by the owned capacity permit.
Timeouts do not retract a committed mutation or prove a failed registration.

## State, invariants and proof

Fix a nonempty ordered seed list `S` of length `N`, and a cursor `c < N`.
A pass selects `rotate_left(S, c)`, then sets `c` to zero if `c + 1 = N`, or to
`c + 1` otherwise. For an empty list, selection is empty and the cursor is unchanged.
The runtime's root seed list is fixed; resizing it during these claims is excluded.

The invariant is `0 <= c < N` and each selected list is a permutation of `S`.
Initialization has `c = 0`. For `c + 1 = N`, the next cursor is zero and `N > 0`.
Otherwise, `c < N` implies `c + 1 < N`. Rotation changes only order. Thus induction
preserves the invariant for any number of passes. There is no `usize` overflow:
`c < N <= usize::MAX` implies `c + 1 <= usize::MAX`.

`proofs/lean/Registration.lean` machine-checks five declarations for arbitrary
natural-number sizes and pass counts:

1. `next_seed_is_modulo` equates the branch update with `(c + 1) % N`.
2. `next_seed_stays_in_range` preserves the valid cursor range.
3. `every_seed_gets_a_first_turn` constructs an offset below `N` reaching any
   target position in the closed form.
4. `cursor_after_is_modulo` inducts over actual repeated branch updates, proving
   that the cursor after `k` passes is `(c + k) % N`.
5. `repeated_steps_cover_every_seed` combines those results: each seed is first
   within one cycle of the actual step function.

The Lean inventory audits transitive axioms and rejects a frozen-cursor mutation.
Rust tests cover every start for lists of size one through five, a first-seed
blackhole consuming the whole pass, and a valid hint followed by a blackholed
pending seed. Isolated Rust source controls remove cursor advancement, demote
hint priority, or restore the discovery-sized budget. Each must pass before the
mutation, fail at its intended assertion, and pass after source restoration.
These controls check the source mapping; they do not mechanically verify Rust.

Within one pass, let `D` be its initial absolute deadline, `V` its endpoint set,
and `h <= 4` its accepted novel-hint count. Every dial checks `now < D` and receives
only `D - now`. `D` is never reassigned. Each candidate is dialed at most once,
new hints require a successful insertion in `V`, and at most four are added.
Thus at most `N + 4` dials start, and no dial receives newly granted time. Transport
completion and OS scheduling are still required to return control; this is not
a hard real-time bound on an arbitrarily stalled process.

## Conditional progress and protocol refinement

Assume the process continues taking retry steps, a fixed reachable leader remains
available with a functioning quorum/storage, its admission is valid, backend work
is eventually scheduled and admitted, and a selected healthy route completes
within the pass deadline. If that leader is a declared seed, coverage gives it a
first turn within `N` passes. If a healthy seed returns a novel canonical path to
an undeclared leader, assume that path fits the hop limit and remaining deadline;
immediate hint priority prevents unrelated pending seeds from preceding that path.
Catch-up must then eventually apply the exact receipt. Under those premises the
existing bootstrap transition can reach `Serving`.

This does not promise progress during unlimited crashes, permanent overload,
expired admission, unreachable leaders, endless leadership changes or insufficient
deadlines. Multiple clients competing for one server permit have no new fairness
guarantee. In particular, merely reaching follower seeds is insufficient if their
leader cannot be contacted. No seed is a required singleton under the stated
quorum and reachable-route assumptions.

Hints remain routing candidates, not membership authority. Only a typed
`Registered` result installs the production receipt/catch-up capability. Serving
still requires exact `(term, index)` application, matching cluster identity and
local membership. The status file now exposes the received term and index (or
`none` before receipt) so E2E artifacts can correlate that gate. The existing
freeze-apply runtime test checks the same production receipt on both sides of
catch-up and verifies its status rendering.

TLA+ remains the primary consensus model. Candidate reordering and refused dials
stutter with respect to accepted Raft/catalog transitions. Existing metadata and
Ready TLAPS results retain their stated scopes; these arithmetic lemmas do not
prove the complete membership protocol, joint consensus or the Rust binary.

## Actual fault acceptance

`scripts/chaos-mesh-registration.sh` adds an actual joining node to the isolated
Chaos Mesh run. Before starting it, a two-way `NetworkChaos` partition isolates
seed 1 from all other database Pods. The other two voters elect a reachable
leader. TCP probes verify joiner-to-seed-1 failure and reachability to seeds 2/3
before starting the node and again after successful registration/read.

The normal join/start path must reach `Serving` as learner 4 with voters 1/2/3,
at least one recorded registration error, at least two attempts, a positive exact
receipt, and an applied index covering that receipt. The learner must preserve the
leader-only Raw read contract by returning an exclusive typed `NotLeader` hint; a
public linearizable read through the surviving voters must return the baseline
value. The independent history client
must complete writes and reads during the live fault; the full history checker
requires this twelfth fault window. The fault remains installed until all those
observations are retained, then heals before the remaining voter-failure matrix.

This is three voters plus one learner on one Kind host, not cross-host failure
tolerance. The object store remains the sole permitted external dependency
exception; routing does not add a coordinator service.
