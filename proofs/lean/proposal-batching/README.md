# Proposal-batching model

A machine-checked model of bounded proposal aggregation for public raw
writes (`crates/server/src/runtime/range_api.rs::WriteAggregator`, the
first slice of issue #20). In-flight SAME-EPOCH fenced writes of one
data group coalesce into ONE raft entry, proposed through the exact
unchanged submission/retry path and applied atomically at one position
that every participant receives as its receipt. The write path's own
safety (persist-before-send, exact retry identity, epoch fencing at
apply) is a premise from its design records; this model constrains the
AGGREGATION:

- writes merge only under one equal region fence — an epoch change
  (split seal, version bump) closes the open batch instead of merging;
- receipts fan out only AFTER the apply (a premature receipt is the
  named defect the controls target), and the applied position follows
  the one proposal of the one taken batch — the full chain is ordered;
- a taken batch never admits another writer, and the flush drops no
  write (every staged mutation rides the merged entry);
- arrival order inside the batch is never reordered;
- a retry keeps the exact command identity (the unchanged
  `finish_async_proposal` contract).

Disabled mode (`KV9_PROPOSAL_BATCH_OPS=0`, the default) is the exact
prior single-command path; the aggregator stages nothing.

Fifteen checked theorems: `initial_safe`, `step_safe`, `run_safe`,
`receipts_fan_out_only_after_the_apply`,
`the_batch_applies_only_after_its_one_proposal`,
`the_proposal_requires_the_taken_batch`,
`no_cross_epoch_write_ever_merges`, `no_writer_joins_a_taken_batch`,
`arrival_order_is_never_reordered`, `no_write_is_ever_lost`,
`a_join_requires_a_live_open_batch`,
`a_retry_keeps_the_exact_identity`, `the_take_is_permanent`,
`an_open_batch_alone_proposes_nothing`, `the_batched_chain_completes`.

`scripts/prove-proposal-batching.py` checks the positive build, audits
every theorem's axioms (only `propext`, `Classical.choice`,
`Quot.sound`), compiles five semantic mutation controls that must fail
exactly at the defect (a cross-epoch merge; a join into a taken batch;
receipts before the apply; an apply that skips the proposal; a flush
that drops a write), and refuses `sorry`/`axiom` injections (two policy
controls).

The model does not claim throughput or latency numbers (the benchmark
publishes measured before/after separately, under predeclared targets),
backpressure budgets, catalog-transaction batching, or chaos acceptance.
