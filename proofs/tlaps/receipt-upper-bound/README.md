# Conservative receipt upper-bound refinement

This experiment keeps the original bounded vector and full first-match receipt result, while rejecting an impossible future-index lookup before scanning. It is independent of the held receipt-tail hint candidate. It does not change proposal, Raft, persistence, apply, or response admission rules.

## Invariant and refinement

`AppliedReceipts.maximum_index` starts at zero. Every insertion assigns `max(previous_bound, receipt.index)` under the existing applied-ring mutex. Eviction never lowers the bound. Every retained receipt therefore has `index <= maximum_index`, for arbitrary insertion order, duplicates, gaps, eviction and extreme unsigned indexes. The operation uses comparisons rather than index arithmetic; `u64::MAX` is a valid conservative bound and never wraps.

If `query > maximum_index`, no retained receipt can match and the original linear search returns `None`. Otherwise the candidate executes the original first-match search and returns the entire stored `(index, term, outcome)`. A high evicted index can cause unnecessary scans but cannot hide a retained record. Original vector allocation/append/prefix-drain order and capacities are preserved; `Default` supplies the empty vector and zero bound.

The parameterized TLA+ model permits arbitrary natural-number indexes and queries, arbitrary nonempty term/outcome sets and any positive finite capacity. Its single module assumption only defines those legal inputs. No ordering, success, quorum, timing or lookup-safety axiom is assumed. The vector equality and bound are inductive across arbitrary pushes and stuttering. `UBLookupSame` proves full result equality, including absence and duplicate ordering. `UBContextRefinement` is ordinary equality congruence for any subsequent observer of that result; it does not assume that absence means failure or success.

## Code boundary

`source-contract.json` pins all changed Rust files, the exact model/proof/config/inventory and the checker/auditor sources. Unique inverse driver edits must reconstruct the parent driver byte for byte. The checker also refuses any additional changed runtime file in Cargo/crates/src/proto/config inputs. Thus the surrounding fatal check, separately sampled unified watermark, command watermark, eviction-unknown branch, term replacement and full outcome propagation remain the reviewed parent implementation. Synchronous and asynchronous deadline/cancellation/stop handling and the persistence/quorum/apply/reply paths have no edits.

The source mapping is reviewed and hash-bound; this is not a verified Rust compiler or a proof of the entire running Raft implementation. It establishes equality at each ring lookup under the existing mutex. It makes no claim about identical wall-clock schedules or global liveness. A faster lookup may change real-time scheduling; ordinary recovery and actual fault-injected end-to-end acceptance remain mandatory.

## Checks

Run from the repository root with the pinned local tools and a fresh output directory:

```sh
PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 python3 scripts/check-receipt-upper-bound-proof.py \
  --tlapm /tmp/kv9-p0-tools/tlapm-1.6.0-pre-20260731/bin/tlapm \
  --jar /tmp/kv9-p0-tools/tla2tools-v1.7.4.jar \
  --output /tmp/kv9-upper-bound-proof-acceptance-20260915-first
```

The final checker audits parsed module imports, assumptions, theorem names and missing proofs before a strict, fresh TLAPS run (14 statements and 82 obligations). This integrated check adds source identity and semantic inventory validation absent from the successful preliminary draft; draft outputs and both earlier proof failures remain preserved. TLC exhausts capacities 1 and 2 over indexes `{0,1,3}`, queries `{0,1,2,3,4}`, two terms and three distinct outcomes. Two independent deliberately broken models must produce an actual lookup-refinement violation: lowering the bound to the latest inserted index, and rejecting `query == bound`. The negative configurations omit the internal bound invariant so it cannot mask the intended wrong-result witness. Separate semantic controls must reject an omitted proof and an added false axiom. Output controls reject empty, zero-obligation and missing-summary proof output.

Rust differential traces cover empty/zero indexes, gaps, more than three ring rotations, duplicate/reordered complete receipts and evicting `u64::MAX`. The diagnostic control covers real skipped scans and fallback comparisons. Existing diagnostic Raft checks, the default workspace and server snapshot bound are separate evidence. Schema 2 adds one bounded skipped-ring-length distribution; schema-1 captured data and its checker retain their original meaning.

This proof package alone does not establish a performance win, ordinary recovery, actual Chaos Mesh acceptance or the complete 8-smoke/16-ten-second matched screen. Those gates remain required before promotion.
