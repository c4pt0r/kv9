# Bounded owner polling: default build and ordinary recovery

Candidate `2ca5fccb157b26b6c3c79eb52f7c7838f10a5c8c` now passes a clean
default production build and ordinary recovery. Its fixed 32-us optional poll
preserves Safe ReadIndex, the original atomic park predicate, tick deadlines,
successful pump/apply/view fences and durable write acknowledgements. Main still
selects `11113f6`; no new throughput or latency result is claimed.

The [source/proof checkpoint](https://github.com/c4pt0r/kv9/blob/2ca5fccb157b26b6c3c79eb52f7c7838f10a5c8c/docs/BOUNDED-OWNER-POLL.md)
retains 14 new TLAPS theorems / 49 obligations, the scheduling dependency,
finite models, negative controls, 714 default and 443 diagnostic tests, with
overlapping populations and all original failed attempts.

## Exact production build

The retained default server SHA-256 is
`c2a31c8158ddeb811bffe8b1babbfd062e571e2345669f9b6663d8794cc0c3b2`.
The [original readback](owner-poll-recovery-v1/summary.json) binds all 716 clean
source files, server and same-source recovery-client bytes, manifests, cache
invalidation and verbose codegen observations. Eleven first-party units compile
fresh; all 20 artifact observations have default features. ThinLTO, one codegen
unit, opt-level 3, unwind panic and the original target defaults remain fixed.
The only source changes after the Rust gate are five documentation/evidence
files, explicitly listed in the readback.

## Ordinary recovery result

The unchanged independent auditor accepts both streaming and explicit unary
full atomic point/batch histories through leader loss and original-directory
restart. Five retained helper contracts also pass before execution.

| Transport | Complete operations | OK | Unknown | Fresh voter drains |
| --- | ---: | ---: | ---: | ---: |
| Streaming | 189 | 175 | 14 | 3 |
| Unary | 174 | 159 | 15 | 3 |
| Total | 363 | 334 | 29 | 6 |

Unknown operations remain in the full consistency search; they are neither
discarded nor reported as successful acknowledgements. Both failure/restart
progress windows contain each required operation class. Five server and two
client lifetimes are identity-bound and exited. Original logs, raw histories,
WAL directories, build binaries/manifests and audit outputs are retained in the
[exact archive inventory](owner-poll-recovery-v1/archive-inventory.json).
Runtime `33692` and its independent readback both terminate successfully on
their first invocation; no recovery scenario is rerun.

The next screen preserves 12 two-second smoke cohorts and 24 ten-second timed
cohorts, fixed clients, control/candidate/Redis, c1/c64 GET/mixed and both full
orders. CPU, mean/p99 and every outcome remain part of the decision. This local
recovery result supplies no performance, actual Chaos Mesh, independent-host
failure or power-loss acceptance. Broader API and actual Chaos gates remain
mandatory before any default promotion or original industrial checklist closure.
