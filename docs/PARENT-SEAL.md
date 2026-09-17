# Parent seal execution (D04, part 3)

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
D04 [#25](https://github.com/c4pt0r/kv9/issues/25). Builds on
[split intents](SPLIT-INTENT.md) and the
[partition directory](PARTITION-DIRECTORY.md), in the safe order the
partition rules force: fence first, populate next, republish last.

## What this increment adds

**Group-side seal execution** (`SealSplitParent`, CLI
`seal-split-parent`). The committed split intent is the one authority:

1. The parent group's leader, holding the committed intent from local
   applied metadata, proposes one `Command::DataRange` transition through
   the group's OWN log: a CAS from the current row's digest to its sealed,
   version+1 successor — exactly the `may_follow` one-way shape the state
   machine has enforced since terminal sealing.
2. The fence is immediate and typed: `authorize` reads the engine's range
   row per request, so from the applied seal onward every write AND read
   at the parent refuses with a stale epoch — a refusal, never an unknown.
3. The fence is durable: recovery replays the sealed row and the restarted
   group keeps refusing. Re-sealing confirms idempotently; sealing without
   the committed intent refuses.
4. **The catalog directory is untouched.** Its unsealed parent binding
   keeps routing (into the fence) until the atomic one-to-two publication;
   the partition read model's tolerance for an engine row one sealed step
   ahead of the catalog (`may_follow` in `reconcile_ranges`) is exactly
   this window.

## What it deliberately does not do

No child population, no directory publication, no rerouting, no unseal
path of any kind. During the fence window the keyspace is deliberately
unavailable with typed refusals — the bounded write pause of a manual
split. The [3/6/9-host benchmark contract](HORIZONTAL-SCALING-PLAN.md)
remains the only acceptance path for effective horizontal scaling.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting.
- Accepted five-process e2e (`scripts/parent-seal-e2e.py`, first
  attempt): the committed intent flow, a wrong-operation seal refusal,
  the seal committing at version 2 with an idempotent confirming retry,
  immediate typed write AND read refusals at the parent, and the durable
  fence surviving a leader restart with the group recovered active.
- Eleven-theorem Lean model
  ([proofs/lean/parent-seal](../proofs/lean/parent-seal/README.md)) with
  five semantic mutation controls and two proof-policy controls; eighteen
  sibling Lean models and the retention TLA/TLAPS model re-accepted.

Validation packet: [docs/parent-seal-v1](parent-seal-v1/README.md).
