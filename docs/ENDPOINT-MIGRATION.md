# Versioned endpoint authorization

Tracking: [#47](https://github.com/c4pt0r/kv9/issues/47), under the
[#42 store replacement boundary](https://github.com/c4pt0r/kv9/issues/42).
The [transport ownership foundation](RAFT-ROUTING.md) moves an accepted route
without leaking workers or mixing connection generations. This document covers
the catalog transition that authorizes that route. [Endpoint writer ordering](ENDPOINT-WRITERS.md) extends this foundation
with atomic admission revocation and serialized runtime route installation.
The public API, CLI, durable recovery gate and actual migration Chaos extension
are described in [Endpoint recovery](ENDPOINT-RECOVERY.md). Historical foundation
acceptance below does not by itself establish those later gates.

## Catalog transition

`kv9_meta::endpoint` reads the node's immutable store binding and Active state
from the transaction's one snapshot. An update carries the expected cluster,
node, store incarnation, old socket address and endpoint generation, plus the
desired socket address. Refusing a mismatched cluster/store, missing/inactive
node, stale precondition or exhausted generation stages no endpoint mutation.

The `nodes` row adds two tag-length fields:

| Tag | Meaning | Original-row interpretation |
| --- | --- | --- |
| 6 | Endpoint generation, an unsigned 64-bit integer | Absent means zero |
| 7 | Previous address of the most recent endpoint transition | Absent only at generation zero |

A changed result stages the new address, generation `g + 1` and previous address
together in one catalog row update. Any existing Pending/Consumed admission is
revoked in the same transaction. Confirmation and refusal leave a later admission
untouched. Other fields and unknown extension tags are
preserved. A same-address CAS also advances the catalog version; its duplicate
does not advance it again. The transport may still treat the unchanged socket
address as an idempotent connection update. Catalog versions and immutable
in-process `Arc` destinations serve different purposes.

A retry is `Confirmed` only if the current record has the exact requested next
generation, target address and previous address, and the same store binding.
It stages no writes. This supports a duplicate after an ambiguous response and
rejects an older request after A -> B -> A, even if its old or target address
appears again. Retaining the previous address prevents a request with an
incorrect original precondition from impersonating that duplicate. Only the most
recent transition is retained; a superseded retry gets a conflict. Identical
requests from different callers may confirm the same transition; no caller
identity or original proposal position is inferred from this record.

Generation arithmetic uses `checked_add`. Generation `u64::MAX` can be the final
valid result and can be confirmed by its duplicate, but cannot wrap to zero.
A malformed present version or inconsistent previous-address field is a catalog
error, not permission to reinterpret the row as its original generation.

These functions are planners. `Changed` describes staged state until the caller
commits the batch and observes its exact Raft position. `Confirmed` is neither a
recovered original receipt nor proof that the original caller won. Its runtime
response requires a fresh exact confirmation and must label that distinction.
The consensus caller must authenticate, hold the catalog planning mutex, drain
ambiguous prior proposals, plan from the resulting snapshot and propose under
the same term. The existing ordered barrier and term-fenced commit supply those
steps; a standalone local `MetaTxn::commit` is not a production consensus path.

## Proof and adversarial validation

`EndpointCAS.tla` models one existing Active node/store binding, a sampled
operator request, competing authorized updates, delayed evaluation and retries.
Addresses and store identifiers are arbitrary nonempty sets. The generation
limit is an arbitrary positive natural number; the deductive proof includes
`u64::MAX` without enumerating it. TLC separately instantiates limits and finite
endpoint/store sets.

The 11-declaration / 80-obligation `EndpointCASProof` establishes initialization
and inductive preservation, atomic address/version changes, exact change and
confirmation preconditions, and refusal without directory mutation. Results
capture the transition they describe even if another operator later changes the
directory. Under fair backend completion every pending request eventually has a
terminal result, which may be a conflict. An eligible request with no competing
updates eventually changes the endpoint. Neither theorem promises success under
indefinite superseding updates or supplies quorum/network availability itself.

The gate uses fresh caches, pinned TLAPS/SANY tools, exact theorem and obligation
inventories, a semantic import/assumption audit and strict terminal output checks.
Eight protocol mutations remove CAS generation, store binding, confirmation
generation, confirmation previous address, generation advancement, exhaustion
checking, refusal immutability or completion fairness. Each needs a passing
baseline, an intended TLC counterexample and failed deductive obligation, and a
passing restored source. Two witnesses establish reachable address reuse and
duplicate confirmation; proof holes, custom axioms and incomplete output fail.
The TLC terminal-success adapter spells out the stutter already allowed by
`[A]_v`; it does not disable deadlock checking or weaken a progress property.

Eight catalog tests exercise staged versus committed state, original rows,
conflicting operators, exact duplicates, ABA, wrong binding, inactive membership,
version exhaustion and corruption. Six compiled source mutations must fail their
selected assertion and pass again after restoration. This is a proof of the
catalog transition abstraction with implementation controls, not a proof of the
complete Rust binary or the as-yet-unimplemented migration workflow.

```sh
cargo test --locked -p kv9-meta
python3 scripts/check-endpoint-controls.py --output /tmp/kv9-endpoint-controls-new
python3 scripts/check-endpoint-protocol.py --jar /path/to/tla2tools.jar \
  --tlapm /path/to/tlapm --output /tmp/kv9-endpoint-protocol-new
```

Clean revision `57e3ae69fae663fdd9cc85458fc50468a99b5c9c` passed all 65 metadata
tests, including the eight endpoint tests, and all six source controls. The
complete protocol gate and independent source-bound audit accepted 32 TLC cases,
31 proof/audit cases and 21 positive fresh-cache proof runs with the exact
11-declaration / 80-obligation inventory. Clippy with warnings denied,
formatting, Python syntax and workflow validation passed. Archive:
`target/correctness-evidence/2026-09-09-57e3ae6-endpoint-cas.tar.gz`,
1,257,358 bytes, SHA-256
`8cc81134f6898be0a6b8fa0fe094f2bb2272446131421e7af8b86f6de1acc93f`.
It retains the exact source, all gate artifacts, controls and independent audit.
This evidence covers the catalog foundation; it is not migration E2E acceptance.

## Integration obligations carried into runtime acceptance

These obligations motivated the current [runtime implementation](ENDPOINT-RECOVERY.md).
All six are covered by its later accepted local increment. The list preserves
the foundation's original requirements and the distinction from its earlier tests.

1. Expose authenticated current-route read and conditional-update RPC/CLI calls
   with bounded admission, typed conflicts and unknown outcomes, and exact
   mutation/confirmation receipts. An update must not change the immutable root,
   store incarnation or Raft membership. The planner now cancels older pending
   or consumed admission authority atomically with an explicit route update;
   the public API must preserve that batch and observe its exact receipt.
2. Preserve the integrated writer ordering when adding that API. Registration
   now advances the endpoint version when it changes an existing address;
   consumed retries validate the current endpoint. Catalog capture/installation,
   registration and the successful-response fallback use the same local planner
   mutex. The new API must hold it through its own committed route installation.
3. Add canonical advertised-endpoint configuration separately from listener
   binding. Preserve Service/NAT use and coordinator-free recovery at a stable
   endpoint. A restarted Active store at a changed advertised endpoint must
   obtain committed authorization and apply its exact confirmation before public
   Serving. Keep durable receive/owner authority separate from that Serving gate.
4. Define initial-root fallback and current catalog routing during restart and
   leadership changes. The immutable root seed list must not become a required
   singleton discovery service. Add a parameterized proof of the complete
   confirmation/restart protocol and controlled implementation regressions,
   including the published `1d38bc4` changed-endpoint diagnostic and its passing
   unchanged-endpoint control.
5. Establish rollout and format rules before enabling migration. Unknown-column
   decoding alone does not make mixed writers safe: an older registration path
   could change an address without advancing its version. This foundation does
   not implement a mixed-version activation or downgrade protocol and does not
   expose an operator migration command yet.
6. Exercise the supported workflow with actual PodChaos: kill the original
   owner, retain its PVC and incarnation, keep the old endpoint unavailable,
   authorize a distinct new endpoint, and observe actual new-endpoint delivery
   and exact catch-up receipts while the surviving quorum serves. Both complete
   histories, source/build/fault provenance and independent corrupted-evidence
   controls are required. Independently prepared replacement PVCs stay refused.

The original 19-window Chaos matrix and local socket tests did not replace the
new migration cell. The later [21-window matrix](ENDPOINT-RECOVERY.md#accepted-local-increment)
supplies that evidence. [#42's acceptance record](REPLACEMENT-ACCEPTANCE.md)
reconciles the combined receive/replacement scope; P0 remains open.
