# Admission-floor model

A machine-checked model of the metadata admission floor
(`crates/server/src/admission.rs`, `KV9_PUBLIC_METADATA_RESERVED`,
default 8 of `KV9_PUBLIC_MAX_REQUESTS`): issue #20 item 4's progress
reservation for control traffic. The admission ledger's own accounting
(bounded counts and bytes, typed refusals, release on drop) is a
premise from its implementation and unit tests; this model constrains
the FLOOR:

- raw and transaction load fill only the SHARED capacity: no
  non-metadata class ever consumes a reserved slot;
- a saturated shared pool refuses non-metadata work with a TYPED
  pre-append refusal (`metadata_floor` — nothing proposed, nothing
  queued, the caller may back off and retry);
- metadata work stays admissible up to the FULL limit through the
  flood — control traffic is never starved (raft-internal traffic —
  heartbeats, replication, ReadIndex confirmation — never passes
  public admission at all, completing the reservation);
- only metadata consumes the last capacity, and releasing shared
  capacity reopens it.

Fourteen checked theorems: `initial_safe`, `step_safe`, `run_safe`,
`no_raw_work_ever_consumes_the_floor`, `metadata_is_never_starved`,
`every_refusal_is_typed_and_pre_append`,
`the_total_fills_only_beyond_the_shared_pool`,
`a_saturated_shared_pool_refuses_raw_work_typed`,
`metadata_admits_while_capacity_remains`,
`a_release_reopens_the_shared_pool`,
`the_flood_cannot_reach_the_total_limit_alone`,
`metadata_admits_through_a_full_shared_pool`,
`only_metadata_consumes_the_last_capacity`,
`the_released_pool_admits_raw_work_again`.

`scripts/prove-admission-floor.py` checks the positive build, audits
every theorem's axioms (only `propext`, `Classical.choice`,
`Quot.sound`), compiles five semantic mutation controls that must fail
exactly at the defect (raw work consuming the floor; the flood starving
metadata; an untyped refusal; raw work filling the total limit; a
release that keeps the pool full), and refuses `sorry`/`axiom`
injections (two policy controls).

The model does not claim byte-level floors, cross-layer backpressure
budgets (raft/apply/upload queues), per-tenant fairness (T03), chaos
acceptance, or any performance property.
