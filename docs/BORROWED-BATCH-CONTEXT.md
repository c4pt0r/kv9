# Borrowed batch context authorization

This change removes the temporary `Vec<&[u8]>` from batch context checking and
removes one redundant routing lookup of the first key. It preserves the logical
authorization decision and the order of all remaining checks under the premises
below. It does **not** preserve every possible transient storage-error trace.
The baseline is `af4c4e31bdef2b1294931c27e9802bc04e6aeaf5`.

## Transformation and source correspondence

`crates/server/src/runtime.rs` represents a read batch as
`KeySpan::BatchKeys(&[UserKey])` and a write batch as
`KeySpan::BatchPairs(&[(UserKey, Value)])`. These borrow the original request
storage. They neither sort nor deduplicate keys and never modify values.
`anchor()` reads the first key, or `[]` for an internal empty batch.

The common `gate::check_context_in` body retains this order on **one `MetaTxn`**:

1. Resolve the keyspace; reject absence or a non-Raw API type.
2. Resolve the anchor using `Tables::region_for_key_in`.
3. Compare both epoch components against that resolved region.
4. Check each remaining batch position against the same region and transaction.
5. Mint the same region/epoch `ValidatedFence` only after every check passes.

The old step 4 checked positions `0..n`; the new step 4 checks `1..n` in the
original order. `try_for_each` preserves early return at the first failed
remaining lookup. The common `check_key` closure retains the original lookup,
`RegionNotFound`, and region-ID mismatch handling.

The five production constructors are the prepared batch read, resident batch
read, prepared batch write, synchronous batch read, and synchronous batch write.
They borrow exactly the input later passed to `RawExecutor`. Point and range
spans retain their existing branches. Read barriers, lifecycle checks, the
captured read view, the inline byte/key budgets, admission ownership, one-entry
batch writes and unknown-write retry classification are outside this change.

## Conditional refinement argument

Fix a request keyspace and a transaction `T`. The logical metadata contents and
transaction overlay do not change while the gate runs. `MetaTxn` reads one
immutable `ReadView`; the gate has only `&MetaTxn`, so it does not alter the
overlay. Successful reads return the rows/indices of that view. This is a
logical-value premise, not a promise that every read succeeds.

For a nonempty request let `k[0], ..., k[n-1]` be its positional keys; for a pair
batch, `k[i]` is the first component of the original pair. Borrowing the request
and materializing references have the same `k[i]` at every position. Shared
borrows remain live for the synchronous gate and do not permit request mutation.

Let `R_T(k)` denote routing on those fixed contents, retaining absence and
deterministic malformed-routing verdicts. When the shared header succeeds, its
anchor lookup has already returned a region `r = R_T(k[0])` and the supplied
epoch equals `r`'s epoch. Therefore the redundant logical check at position zero
must accept: re-resolving the same anchor on the same contents returns that same
owner. This conclusion requires actual anchor resolution, not merely checking
whether the key lies between previously supplied region bounds.

The old owner condition is

```text
for every i in [0,n): R_T(k[i]) resolves and has ID r.id.
```

The new condition is the identical predicate on `[1,n)`, together with the
already established anchor fact. Splitting `[0,n)` into position zero and its
ordered suffix proves equivalence. Only a position already known to accept is
removed, so the set of failing remaining positions and its least position are
unchanged. Both implementations mint the same fence from `r`, or reject before
minting. The data operation and write receipt are not changed by this argument.

If the shared header rejects, both executions reject at the same header step
under matching header read results. In particular a stale epoch is still
reported before a later cross-region key, gap, or malformed suffix route.
If the header succeeds, and the remaining reads have the same outcomes, the
suffix checks execute in the same order and return the same first error.

## Boundaries and counterexamples to stronger claims

| Case | Required behavior |
| --- | --- |
| Duplicate keys, including a repeated anchor | Only position zero is omitted. Every later duplicate is still visited; there is no value-based deduplication. Input-order read results and ordered write mutations remain unchanged. |
| Gap, missing routing index, or missing referenced row at the anchor | The shared anchor resolution returns no region before the optimization is reached. |
| Corrupt anchor routing entry | The unchanged decoder and index/row consistency checks reject before epoch or suffix processing. |
| Gap or corrupt routing row at a later position | That position still executes `region_for_key_in`; absence or the same malformed-data error is not replaced with an interval-only acceptance. |
| Stale epoch and cross-region suffix | The stale epoch still wins, because the common epoch check precedes every suffix lookup. |
| Internal empty batch | Both versions resolve anchor `[]`, perform the same header checks, and have no owner-loop elements. Public empty-batch rejection is unchanged. No theorem asserts an empty batch is automatically authorized. |
| Singleton batch | The successful anchor supplies its only owner fact; the new suffix is empty. |

`region_for_key_in` in `crates/meta/src/tables.rs` performs the same bounded
reverse index lookup, row fetch/decoding, row/keyspace/start consistency checks,
and end-bound check. This note relies on those existing semantics. It does not
prove the catalog globally has non-overlapping regions or repair arbitrary
corrupt catalogs that the existing routing routine would accept. It proves
equivalence to that routine's logical authorization behavior.

An immutable view can still fail an I/O operation. For example, the first anchor
lookup can succeed and the old redundant lookup can fail with a transient read
error. The optimized gate may then succeed, or reach a later suffix error. That
is an intentional reduction in read/failure surface, not exact error-trace
equivalence. More generally, removing a read can alter an external fault
schedule or cache timing. No coupling of errors by physical read-call ordinal
is assumed. Equivalence of complete successful executions requires successful
reads; suffix error-order equivalence requires matching outcomes for those
remaining checks and a successful redundant old anchor lookup. Any optimized
success is still authorized by the resolved anchor and every checked suffix.

Skipping the anchor without the successful header would be unsafe: for a
singleton whose owner check is false, an unchecked empty suffix accepts while
the original full owner condition rejects. Skipping by key equality would also
violate the promised visitation/error-order contract for duplicate positions.

## Parameterized proof and validation boundary

`proofs/tlaps/batch_context/BorrowedBatchContextProof.tla` contains six theorems
for arbitrary natural batch lengths and arbitrary positional key/owner
functions. No distinct-key or positive-length premise is imposed:

- `BCPositionCoverage`: anchor plus suffix covers every original position.
- `BCBorrowedProjection`: materializing the positional key projection equals
  borrowing that same function.
- `BCAuthorizationRefinement` and `BCGatedAuthorizationRefinement`: the owner
  predicate and header-gated result are equivalent given resolved-anchor success.
- `BCFailurePositionsRefinement` and `BCFirstFailureRefinement`: the rejected
  positions and first rejected position are unchanged under that premise.

The proof's Boolean owner function abstracts logical routing verdicts; it does
not model storage I/O, exact error payloads, Rust lifetimes, RPC cancellation,
Raft, or returned data/receipts. The source correspondence above and runtime
regressions remain necessary. There are no project axioms, omitted proofs,
finite test bounds, or new protocol assumptions hidden in the module.

**Validation:** all six theorems passed on the first fresh TLAPS run, discharging
16 obligations with `--strict --nofp --threads 1` and an owned empty cache.
Independent SANY parsing/semantic checking also passed. The copied/current
module SHA-256 is
`8d2c22d4a2f7ccbdc6ced1ab0b4aada96b350d1f1eac56f671aa92095881f6df`.
Commands, tool/input hashes, complete logs and terminal records are retained at
`/tmp/kv9-borrowed-batch-context-proof-first`; `summary.json` SHA-256 is
`ca36c7af568f6b7fabf911986ae1b4462905b138e86665cea1b0aed66721f9bb`.
This was the standalone conditional lemma check, not a rerun of the broader
protocol/import-control gates. Runtime tests, compiled controls, and final
runtime source binding remain separately reported by the implementation task.
No core Raft safety premise, quorum rule, applied-read barrier, or write atomicity
rule changes.
