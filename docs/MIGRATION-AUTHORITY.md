# Committed migration authority and image retention owners

Updated 2026-09-16. S05 [#19](https://github.com/c4pt0r/kv9/issues/19) and
D03 [#24](https://github.com/c4pt0r/kv9/issues/24), following `de3e6a2`.

This increment adds the **committed authority half** of replica migration: a
replicated migration intent in the metadata Raft and exact source/destination
retention owners bound to one described data-group image. It moves no data,
starts no peer, captures no source cut and releases no pin. The offline
[joint installer](JOINT-SNAPSHOT-INSTALL.md) still returns diagnostics only,
and its receipt gains no authority from this increment.

## Replicated migration intent

`MigrationIntent` is planned under the catalog transaction lock and committed
as one TASKS row (kind 103) beside the existing creation (100), activation
(101) and range-binding (102) rows, inside the shared 255-row budget. It binds:

- the immutable 16-byte `operation` retry identity, nonzero and unique per
  logical migration;
- the committed source creation row: `creation_task`, `region`, `root`; the
  planner refuses an uncommitted, mismatched or missing creation, and requires
  the committed activation row for the same creation;
- the destination: `node`, its **current exact store incarnation** read from
  the certified node row, and the requirement that the destination is *not*
  one of the creation's initial replicas;
- nothing else. No image, cut, configuration or capability is chosen at
  intent time.

Planning is idempotent: an existing row with the same operation must match
every field exactly and returns the prior receipt; a changed retry refuses.
Decode enforces canonical bounded encoding (`KV9MIG01`, checksummed row
payload, nonzero fields) and readback re-validates the referenced creation
row, mirroring `committed_activations`. An intent alone authorizes exactly
one thing: binding image owners for this operation.

## Image owners in the retention ledger

`MigrationOwners` generalizes the checkpoint-owner derivation beyond
`META_REGION_0`: given the committed intent and one canonical
`CheckpointManifest` whose scope matches the committed creation's region and
the group's published `DataRange` (cluster, region, conf_ver, version), it
derives two tracking-only owners in the replicated retention ledger:

- **source owner** — id `sha256("kv9-migration-source-v1" || root ||
  operation)[..16]`, subject = the manifest content digest, resources = the
  manifest's complete SST closure (instance = object-key digest prefix,
  content = exact SST sha256). Committed path: `Register` + `Acquire`, then
  `Publish`. Published means "this exact closure is pinned for transfer",
  nothing more.
- **destination owner** — id `sha256("kv9-migration-destination-v1" || root ||
  operation || destination incarnation)[..16]`, added only through
  `Share { from: source, to: destination }`, so the ledger itself enforces an
  identical subject and closure. Share requires the published source owner.

The planner accepts manifest bytes as *description, not authority*: it
re-encodes and compares canonically, checks the 48 MiB cap, scope, cut
ordering and object naming, and refuses any mismatch with the committed
intent. `QuiesceAfterTransfer` and `Release` are **refused for migration
owners in this increment**; releasing the source requires a later committed
decision fed by validated durable destination-install evidence, which does not
exist yet. Physical object deletion remains disabled ledger-wide, so these
owners change bookkeeping only — but the bookkeeping is exactly what later
GC/truncation must consult.

## Explicit non-authority

- A decoded manifest, a local `InstalledImage` receipt, or both together never
  authorize voting, serving, source pin release or owner mutation.
- The migration intent does not admit Raft messages, does not change
  RegionManager reconciliation and does not alter the fixed-creation
  validation for existing groups.
- No runtime component acts on the new rows yet; readback APIs expose them for
  the next increment (unified-cut capture and destination evidence).

## Checked model

[Authority.lean](../proofs/lean/migration-authority/Authority.lean) proves the
finite-trace projection in 14 theorems: owners exist only under a committed
intent, the destination owner requires the published source owner and binds its
exact subject, quiesce/release/voting/serving are unreachable, pins and the
intent are monotone, a committed description alone binds nothing, and
confirmation is a pure receipt. The runner checks ten semantic mutations and
two proof-policy controls. Replication atomicity and ledger recovery semantics
are explicit premises from the existing retention-ledger TLA/TLAPS model, which
is re-run against the fenced `plan_retention` source. The five sibling Lean
models are rechecked after reviewing the additive command/RPC changes.

## Validation

Local Rust tests cover planner idempotency and refusals (missing or
unactivated creation, unregistered/initial-replica destinations, changed
retries, cross-bound row defects, the shared 255-row budget), the ledger
quiesce/release fence for migration kinds, and one full-cluster path: a
genuinely admitted fourth store via the production join flow, committed intent
through the public RPC, image binding against the committed range, published
owner readback, idempotent rebinding, second-image refusal and refused
quiesce/release through the generic retention surface. The
[portable validation packet](migration-authority-v1/README.md) records the
accepted checks and retained failures. No scaling, QPS or transfer claim
follows.

## Next mainline work

1. Capture the source engine state, historical ConfState and a unified applied
   cut under a driver-held owner, producing the manifest this increment pins.
2. Commit destination-install evidence bound to the intent, then a committed
   quiesce/release decision for the source owner — never from a local receipt.
3. Runtime installation through RegionManager, learner catchup after
   truncation, promotion from durable evidence, removal, then split/placement.

The [3/6/9-host benchmark contract](HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling. This increment adds no
QPS or measured scale-out result.
