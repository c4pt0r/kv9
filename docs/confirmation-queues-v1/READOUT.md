# Confirmation queue diagnostic readout

Instrumented local message/batch waits over the complete drained client envelope, including initialization, warmup, measurement, verification and external readback. The three stages and seven message kinds have different populations, not additive request phases. Sequential counters and histograms are not a completion ledger; carry-in completions are possible. Public drain does not stop peer heartbeats. No request correlation, pure-network or pure-scheduler attribution, CPU profile, new performance acceptance or continuous-leadership claim. Batch-channel admission does not measure residence inside the channel. Sampling is every 64 attempts per stage/kind; abandoned samples have unknown duration. Quantiles are intervals from delta buckets, not differences or averages of endpoint percentiles.

Measured successes: **974,650**, all single attempt. All client phases: **999,488 logical calls**, **999,490 attempts**. Setup attempts are retained separately.

Node 2 is leader at both endpoints in both cells; nodes 1 and 3 are followers. This does not establish uninterrupted leadership between endpoints.

| Cell | Phase | Calls | Successes | Attempts |
|---|---|---:|---:|---:|
| crc-c1 | initialization | 8194 | 8194 | 8195 |
| crc-c1 | warmup | 128 | 128 | 128 |
| crc-c1 | measurement | 131944 | 131944 | 131944 |
| crc-c1 | verification | 4097 | 4097 | 4097 |
| crc-c64 | initialization | 8194 | 8194 | 8195 |
| crc-c64 | warmup | 128 | 128 | 128 |
| crc-c64 | measurement | 842706 | 842706 | 842706 |
| crc-c64 | verification | 4097 | 4097 | 4097 |

Complete tables: [90 stage/kind records](STAGE-COUNTERS.md), [630 outcome rows](ALL-OUTCOMES.md). Exact delta buckets, integer sums/counts, per-operation client phase accounting and endpoint identities are in [readout.json](readout.json).

This derivation binds selected inputs to the already accepted independent inventory. It does not repeat the full runtime, retention, source-tree or cleanup audit; bulky original evidence remains local.
