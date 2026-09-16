# Scoped routing and bounded retry composition

`Routing.lean` proves 15 statements, including inductive safety over arbitrary
finite traces. It extends the existing client-retry argument to a shared budget
for discovery and data RPCs. Run:

```sh
python3 scripts/prove-routed-client.py --lean /path/to/lean --output /fresh/output
```

The source contract pins reviewed Rust and model files. The runner compiles
with warnings as errors, audits theorem axioms, and requires semantic mutations
and proof-hole controls to fail. This is an abstract model with a reviewed
correspondence; it is not verified Rust extraction or a complete Raft proof.

## Correspondence and premises

| Model | Implementation and required premise |
| --- | --- |
| One `State` / `Run` | One `RoutedRawClient::call` of an immutable logical write. Separate calls are separate operations. The workload runner never resubmits an unknown operation. Read retries do not create effects and are outside this write trace. |
| `lookup` | A read-only `LookupRawRoute` RPC, including a redirect or failure. Discovery and data consume the same `for ordinal` budget. Positive lookup can lead to `ready`; stale scope invalidation returns to `lookup`. Cached scope may begin in `ready` with zero counters and satisfies the same invariant. |
| `dispatch` | One scoped data request, only within the original attempt budget and absolute monotonic deadline. Transport connection setup and waiting are contained in that attempt. No fallback to legacy unscoped methods or an older stream opcode exists. |
| `refuse` | Only an exclusive typed NotLeader or scoped no-effect refusal permits another write attempt. Admission and crossing-batch refusals terminate. The premise is stronger than absence of an observed effect: the refused attempt cannot subsequently execute. It relies on the existing proposal/completion contract and ordered range fence, not arbitrary error strings. |
| `closed`, `sent`, `effects` | Closed attempts have no present or future effects. At most one unclosed dispatch can exist. The model allows late effects even after Unknown or caller termination. Effect IDs identify dispatches; the theorem excludes client replay of an uncertain logical write, not repeated server application of one dispatch. Raft apply ordering and the existing apply-once contract remain premises. |
| `unknown` | Every uncertain data-write result is terminal. No later discovery or data RPC belongs to that call. Cancellation does not certify that a submitted operation failed. Concurrent calls, transport cancellation isolation and resource ownership are separately tested. |
| `budget`, `deadline`, `tick` | Budget and deadline are immutable parameters across all transitions, including refresh. Time is monotonic. A deadline prevents initiating another RPC; it is not a hard real-time scheduler guarantee or a promise to cancel an accepted server write. |
| `discovered` | Positive responses must match configured root, tenant and keyspace, contain the lookup key and be unsealed. The metadata leader reads binding and endpoint directory from one post-ReadIndex transaction. Endpoint and leader hints are bounded candidates, not ownership authority. Followers return hints only. |
| `authorized` / full `Scope` equality | `RawDirectory::scoped` compares the canonical descriptor's root, creation digest, region, namespace, epochs, bounds and seal before selecting a private group API. The owning group rechecks live scope on its read snapshot or at ordered write apply. Hash equality for echoed response scope assumes collision resistance; numeric order abstracts lexicographic byte keys. |
| Whole batch | Every key must belong to one descriptor. Cross-range atomic batches are refused; there is no client-side fan-out. The existing atomic batch apply is a premise. |

## Limits

Compliant crash/partition peers and authenticated channels are assumed; this
does not address Byzantine servers. Client cache updates may race or retain an
old endpoint hint. Such hints cannot bypass the server's exact scope and ordered
epoch checks; bounded retries can still fail without availability. Fair lookup
candidate traversal and election-length backoff are implementation liveness
checks, not a liveness theorem here.

Current metadata still maps each newly created Raw keyspace to one full-range
group. This proof does not establish split ownership publication, movement,
replica replacement, automatic placement, complete D02 acceptance, independent
host availability, or throughput scaling. Actual Chaos Mesh histories and
benchmark results are separate evidence and must not be inferred from this file.
