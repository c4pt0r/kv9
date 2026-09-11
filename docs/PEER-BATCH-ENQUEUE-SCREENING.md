# Peer batch enqueue: insufficient performance gain

Do not select `ea5f498efd9d6d9ce2f1e0ef88acb6012216f4b7` as the next performance
increment. Single GET gains only **0.399% / 0.303%** against the same-run event8
control `917243f`, with second-repeat p99 regression. BatchGet(1) gains
**0.196% / 1.298%**. These results do not establish a useful Redis-gap step.
The accepted control remains unchanged; no candidate Chaos campaign follows.

The [source contract](https://github.com/c4pt0r/kv9/blob/ea5f498efd9d6d9ce2f1e0ef88acb6012216f4b7/docs/PEER-BATCH-ENQUEUE.md)
covers immediate bounded peer-channel enqueueing without creating a progress
timer and polling a second select. A full channel returns the exact batch into
the original send/RPC/timeout fallback. Successful immediate sends explicitly
consume Tokio's cooperative budget. The first RPC select, route generations,
coalescing limits and Raft guards remain unchanged. Queue acceptance is not
network delivery, and the conditional argument does not prove all of Tokio.

## Five-second matched screen

| API / repetition | Control calls/s | Candidate calls/s | Gain | Control mean us | Candidate mean us |
| --- | ---: | ---: | ---: | ---: | ---: |
| GET / 1 | 326,718.6 | 328,023.5 | 0.399% | 195.755 | 194.974 |
| GET / 2 | 325,481.7 | 326,467.1 | 0.303% | 196.505 | 195.905 |
| BatchGet(1) / 1 | 320,903.3 | 321,531.2 | 0.196% | 199.271 | 198.882 |
| BatchGet(1) / 2 | 317,529.6 | 321,650.6 | 1.298% | 201.389 | 198.808 |

GET p99 buckets change from 368.640–372.735 to 360.448–364.543 us in repetition
one, but from 364.544–368.639 to 368.640–372.735 us in repetition two.
BatchGet(1) is unchanged at 372.736–376.831 us in repetition one and improves
from 376.832–380.927 to 372.736–376.831 us in repetition two. These are bucket
intervals, not exact percentiles or averaged percentiles. Same-run Redis GET
reaches 502,755–506,401 calls/s; Redis MGET(1) reaches 501,555–505,906 calls/s.

The inherited five-second protocol uses 64 closed-loop workers, 128-byte values,
4,096 keys plus sentinel, batch size one and 128 warmup calls. Two repetitions
reverse six-arm order. Clients use CPUs 0–1; all voters share 2–5. No builds,
tests, faults, profiles or audits overlap timing. Other host services remain
unconstrained. KV9 has three Raft voters with tmpfs WAL and normal sync calls;
standalone Redis has save/AOF disabled. Durability and fault tolerance differ.
This is not sustained, write/mixed, larger-batch or cross-host acceptance.

## Validation and retained evidence

Final source passes 25 peer, 15 async-read and 23 point-stream tests, formatting
and server all-target Clippy. Actual production-runtime stream/unary
leader-kill/original-directory restart histories independently pass **379 calls:
344 OK, 35 unknown**. All 20 unknown writes have no attempt after uncertainty;
earlier safe not-leader refusals remain visible. The separate six-arm correctness
smoke passes 9,501,375 calls before timing.

The sole matched runtime (session34498) and frozen independent audit exit zero:
**23,025,553 successful measured calls**, including 12,941,985 KV9 calls, with
all other outcomes zero and no extra SDK attempts. Acceptance checks 40 exited
lifetimes, 24 drains, 24 writer/listener bindings, 1,168 resource observations,
2,326 source checks and 264 retained files / 44,378,737 bytes. All three original
container masks and historical namespace maps are preserved.

The pre-cooperative-budget focused run and the process auditor's report-only
attempt-count assertion failure remain retained. The latter assumed one total
attempt per unknown write; safe refusals can precede terminal uncertainty.
The accepted histories/audit were not changed or rerun. No broad workspace or
candidate Chaos campaign, master promotion or roadmap completion is claimed.
All validation is local; no hosted CI was dispatched.

Raw matrix: `/tmp/kv9-peer-enqueue-matched-diagnostic-attempt1/cohorts`.
Preparation/audit: `/tmp/kv9-peer-enqueue-comparison-preparation`.
Process audit: `/tmp/kv9-peer-enqueue-process-independent-first`.

| Artifact | SHA-256 |
| --- | --- |
| Candidate server | `7ae38358dea6d86aab2e6c41fb72834ff138abb25f664b364cab47d066154c6b` |
| Build manifest | `f1fb0a40214b8cec7e237010b102dc5035b26699d9f9c28422d16cc5273b68c5` |
| Matched audit | `0f3a26cb971293d2fa71e06a7d5785eb09ec0fa296ba69f6f621ee5dc961f7a6` |
| Independent statistics | `b1c9825e8877a0e648589e40d5863dabe83bfb4efa5a72cfaf329936c90bec51` |
| Process audit | `816055ff68bc7f6708139a448d70d8eec00d404c6718918f2564e99fc799f12f` |

Next test async worker population against the accepted event8 control.
Independent per-replica runtime sizing may create avoidable contention when
voters share a CPU budget. Do not carry this unsuccessful enqueue delta forward.
