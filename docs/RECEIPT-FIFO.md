# Constant-time command receipt FIFO maintenance

Tracking: #20 and #9. This isolated representation candidate starts from
asynchronous write waiting `a00e39f`. It changes the in-memory command receipt
container from Vec to VecDeque. Every receipt, chronological position, capacity
limit and exact-outcome rule is preserved. No new lookup algorithm is included;
exact matching still scans from the oldest retained entry.

## Implementation contract

The driver records at most 1,024 command receipts. Previously every append to a
full Vec inserted one item and drained its oldest item, shifting the retained
entries once per applied command. A Raw apply group calls this helper separately
for each command, so group commit does not eliminate those shifts.

The new helper pops the oldest entry when the FIFO is full, then pushes the new
entry at the back. A full FIFO reuses its allocation. VecDeque front/back replace
the old first/last observations in eviction detection and status. Iteration stays
in chronological order, including across physical wraparound. No sorting, index
arithmetic, timestamp inference or new receipt reconstruction is introduced.

The existing applied mutex covers the complete update, including its eviction
and insertion; observers cannot see the intermediate state. Group application,
engine/Ready persistence, publication, exact term/index/fence/manifest selection,
unknown outcomes and safe replacement retries retain their existing boundaries.
The asynchronous and synchronous waiters still consume the same discriminator.

## Sequence-equivalence proof

Let R = 1,024. Represent an externally observable container by its chronological
sequence S, with |S| <= R. Let suffix_R(X) be the final min(R, |X|) elements of X.
The old Vec operation is V(S, e) = suffix_R(S ++ [e]). The new deque operation is
D(S, e) = S ++ [e] when |S| < R, and tail(S) ++ [e] when |S| = R. The equality
D(S, e) = V(S, e) holds for every S and e in this domain:

- If |S| < R, appending produces at most R elements, so suffix_R removes none.
- If |S| = R, R > 0 ensures S is nonempty. Appending produces R+1 elements,
  and selecting its final R elements removes exactly the old head. This is
  tail(S) ++ [e], in the same order and with unchanged entry values.

Both cases preserve the length bound. The initial containers represent the
empty sequence. Induction over an arbitrary finite stream e1, ..., en therefore
establishes equality of their represented sequences after every completed
update, and identifies both with suffix_R([e1, ..., en]). No monotonicity,
uniqueness or contiguity assumption about receipt indices is needed for this
representation theorem.

The consumer observations are functions of that represented sequence: length;
its oldest/newest item; and the first element with the requested index, including
its recorded term and exclusive outcome. Equal sequences give equal observations.
Consequently the same full-ring oldest-index check returns Unconfirmed for the
same evicted positions, and the same matching entry or existing passed-watermark
logic chooses the same exact outcome or replacement. The mutex makes the
pop/push intermediate state unobservable to these consumers. Thus this change
preserves existing receipt decisions under the standard container and locking
contracts. It does not independently establish the correctness of those decisions
or the underlying consensus, persistence and publication premises.

The theorem covers successful allocations/operations. Initial growth while the
FIFO fills can allocate; no out-of-memory recovery claim is added. In the full
case, popping reduces the length below already allocated capacity, so the next
push cannot require growth. Both endpoint operations have constant-time steady
maintenance cost; the unchanged lookup still has a 1,024-entry scan bound.
Machine-checked protocol composition and exact-candidate fault/performance gates
retain their original scope.

## Validation

A deterministic wraparound test publishes more than three capacities of distinct
term/index/outcome records, with index gaps and plain, fence-rejected and manifest
verdicts. After every update it compares the actual iteration with the expected
chronological suffix of the original input, checks oldest/newest observations,
checks allocation capacity stays fixed once full, and requires actual physical
wraparound. Existing asynchronous exact-apply, eviction, failure, cancellation,
fence and replacement tests remain applicable.

`scripts/check-receipt-fifo-controls.py` compiles isolated production mutations
that evict the newest receipt, insert at the wrong end, or grow a full allocation
before eviction. Each must fail its named assertion, with exact baseline and
restored passes and unchanged tests. Existing asynchronous write controls are
also required because their driver storage has changed. These checks establish
local implementation evidence, not a new throughput number or acceptance for
untested failure domains.

### Local record, 2026-09-10

The workspace passed 644 tests/doctests, with zero failures and 23 ignored.
All-target Clippy with warnings denied and formatting passed. Three FIFO controls
and seven inherited async-write controls passed all 30 baseline/mutant/restored
phases, and final production inputs match both accepted copied source inventories.
The unchanged default three-process RawKV fixture passed leader failover, writes,
deletes and original-directory restart. All four observed process lifetimes
executed the default-feature binary with SHA-256 `e9a235f13741ed17b9945b3ee39c0e9ecf7a2ed86cc563d48e2451a97b30a084` and exited; source and
binary remained unchanged during execution.

Retained logs and raw inputs are `/tmp/kv9-receipt-fifo-workspace-first.log`,
`/tmp/kv9-receipt-fifo-clippy-first.log`, `/tmp/kv9-receipt-fifo-controls-first`,
`/tmp/kv9-receipt-fifo-async-controls-first`, and
`/tmp/kv9-receipt-fifo-process-first`. These are local implementation results.
There is no performance comparison or actual Chaos acceptance for this FIFO
revision yet. The earlier a00e throughput result is not attributed to this change.
No hosted CI was dispatched.
