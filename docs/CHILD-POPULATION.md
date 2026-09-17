# Child population under the fence (D04, part 4)

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
D04 [#25](https://github.com/c4pt0r/kv9/issues/25). Builds on the
[parent seal](PARENT-SEAL.md): the parent is immutable at an exact
applied position, so its halves can copy without a moving target.

## What this increment adds

**Committed, verified child population** (`PopulateSplitChild`, CLI
`populate-split-child --half low|high`):

1. **Only under the fence.** The RPC refuses unless the parent's own
   range row is sealed — before the seal, the half could still move.
2. **Through the child's own log.** The child leader scans the LOCAL
   sealed parent engine snapshot over its half (`[start, split_key)` or
   `[split_key, end)`, bounded to the keyspace's encoded prefix) and
   proposes bounded `Command::Write` batches — ordinary committed user
   writes into the empty child group; no installer, no new apply
   semantics, no network snapshot.
3. **Digest-verified, or refused.** The parent-half digest accumulates
   over the ordered (key, value) stream during the copy; the child digest
   is then recomputed independently from the child's durable state. A
   mismatch refuses the call — a divergent child never returns success.
4. **Idempotent and resumable by rerun.** Committed puts converge; a
   rerun (including after a child leader restart) reproduces the exact
   digest. A shared group-handles registry (engine + driver per started
   group, bound or not) gives the backend access to unbound children.

## What it deliberately does not do

No directory publication, no rerouting (the keyspace still routes into
the parent's typed fence refusals), no unseal path, no automatic
triggers. The [3/6/9-host benchmark contract](HORIZONTAL-SCALING-PLAN.md)
remains the only acceptance path for effective horizontal scaling.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting.
- Accepted five-process e2e (`scripts/child-population-e2e.py`;
  `e2e-second` covers the exact final source — `e2e-first` failed on a
  wrong fixture expectation, retained: the harness assumed a probe key
  that was never written). The accepted run: population refuses before
  the seal → both halves copy with independently recomputed equal
  digests, distinct across halves, with exact row counts → reruns
  converge, including after a child leader restart → the parent stays
  fenced and the catalog untouched throughout.
- Eleven-theorem Lean model
  ([proofs/lean/child-population](../proofs/lean/child-population/README.md))
  with six semantic mutation controls and two proof-policy controls;
  nineteen sibling Lean models and the retention TLA/TLAPS model
  re-accepted.

Validation packet: [docs/child-population-v1](child-population-v1/README.md).
