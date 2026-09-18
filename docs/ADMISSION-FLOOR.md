# The metadata admission floor (#20 item 4)

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
issue [#20](https://github.com/c4pt0r/kv9/issues/20). Control traffic
must never be indefinitely starved behind a hot data client.

## What this increment adds

Public admission was already bounded (`KV9_PUBLIC_MAX_REQUESTS`,
`KV9_PUBLIC_MAX_ENCODED_BYTES`, typed refusals, per-class counters) —
but all five work classes shared ONE pool, so a raw/transaction flood
could exhaust every slot and starve admissions, catalog verbs and
recovery decisions behind it.

`KV9_PUBLIC_METADATA_RESERVED` (default **8**, validated below the
request limit, 0 disables) now keeps the last slots admissible only by
the metadata classes:

- non-metadata classes stop at `max_requests - reserved` with the NEW
  typed pre-append refusal `metadata_floor` (nothing proposed, nothing
  queued; the caller backs off and retries);
- metadata classes admit to the full limit; their `refused_floor`
  counter is structurally zero;
- raft-internal traffic (heartbeats, replication, ReadIndex
  confirmation) never passes public admission at all — together with
  the floor this completes item 4's progress reservation;
- the floor is DEFAULT ON: the guarantee is the deliverable.

A client-side gap found by the e2e and fixed here: the RPC clients
validate refusal labels against a whitelist, so the new typed refusal
was invisible to them until `metadata_floor` joined the list — without
the fix the flood saw generic unconfirmed errors instead of the typed
backoff signal.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting; new
  admission unit tests: a flood refuses typed at the floor while
  metadata admits to the full limit, releases reopen the shared pool,
  a floor at/above the limit refuses configuration, and the env parse
  covers the new variable.
- Accepted five-process e2e (`scripts/admission-floor-e2e.py`): a
  40-writer raw flood against a deliberately small pool (12 total, 8
  reserved) for a 20s window — **20,680 typed `metadata_floor`
  refusals**, **17/17 idempotent metadata probes succeeded**
  throughout, metadata classes counted zero floor refusals, and 1,337
  acked flood writes read back. Three retained earlier runs document
  the road: a probe conflating admission liveness with disk latency
  (now retried idempotently, with admission refusals still required to
  be zero), and two windows where the flood never saw the typed
  refusal — the second exposing the client whitelist gap.
- Fourteen-theorem Lean model
  ([proofs/lean/admission-floor](../proofs/lean/admission-floor/README.md))
  with five semantic mutation controls and two proof-policy controls;
  twenty-nine sibling Lean models and the retention TLA/TLAPS model
  re-accepted.

## Known limits, deliberately out of scope

Byte-level floors (the byte budget stays shared), cross-layer
backpressure budgets over raft/apply/upload queues and coordinated
cancellation (the rest of items 3–4), the C04 dual-WAL decision,
per-tenant fairness (T03), chaos coverage, performance claims.

Validation packet: [docs/admission-floor-v1](admission-floor-v1/README.md).
