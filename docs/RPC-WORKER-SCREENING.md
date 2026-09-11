# One RPC worker: throughput and latency regression

Reject `711631b1b39d73741324518931524d352b8c028c`. Against unchanged event8
control `917243f`, one async worker loses **35.145% / 34.727%** GET throughput
and increases mean latency **54.219% / 53.232%**. BatchGet(1) throughput falls
**34.491% / 34.305%**, with mean latency **52.689% / 52.256%** higher.
Every candidate p99 bucket regresses. The retained version remains event8.

| API / repetition | Control calls/s | Candidate calls/s | Control mean us | Candidate mean us | Candidate p99 bucket us |
| --- | ---: | ---: | ---: | ---: | --- |
| GET / 1 | 327,193.1 | 212,202.1 | 195.474 | 301.458 | 442.368–446.463 |
| GET / 2 | 326,708.4 | 213,251.1 | 195.767 | 299.977 | 442.368–446.463 |
| BatchGet(1) / 1 | 319,616.6 | 209,378.7 | 200.074 | 305.491 | 446.464–450.559 |
| BatchGet(1) / 2 | 319,640.6 | 209,986.4 | 200.060 | 304.602 | 446.464–450.559 |

Control GET p99 is 364.544–368.639 / 368.640–372.735 us; control batch p99 is
372.736–376.831 us in both repetitions. Same-run Redis GET reaches
502,067–505,933 calls/s. These are histogram bucket intervals, not exact
percentiles. Pooled GET falls 326,950.709 to 212,726.615 calls/s (-34.936%);
pooled batch falls 319,628.620 to 209,682.548 (-34.398%).

The [source contract](https://github.com/c4pt0r/kv9/blob/711631b1b39d73741324518931524d352b8c028c/docs/RPC-WORKER-BUDGET.md)
changes only the existing multi-thread Tokio runtime's async worker count.
Event interval eight, separate blocking pool and dedicated Raft owner remain.
Its conditional safety argument supplies no new quorum, apply or ownership
authority. Synchronous dynamic-member authentication, decoding and resident
copies can occupy that worker; the initial-voter read fixture does not cover
dynamic-member authentication. This result does not establish an optimal count
for other workloads or CPU budgets.

The unchanged five-second protocol has 64 closed-loop clients, 128-byte values,
batch size one, two reversed six-arm repetitions and fixed native03/Redis b8
clients. All voter processes share CPUs 2–5; clients use 0–1. No build, test,
fault, profiling or audit work overlaps timing. Other host services remain
unconstrained. KV9 uses three voters and tmpfs WAL with normal sync calls;
standalone Redis disables save/AOF. Durability/fault tolerance differ. These
short pure-read measurements do not establish sustained or write/mixed capacity.

Focused checks pass 68 peer/read/batch/stream tests, formatting and Clippy.
Production-runtime stream/unary leader-kill/original-directory restart histories
independently pass 371 calls: 336 OK, 35 unknown, with all seven processes exited.
Unknown writes have no replay after uncertainty. A six-arm correctness smoke
passes 7,655,240 calls before timing.

The sole timing runtime (session16826) and first frozen independent audit pass
**20,736,623 successful measured calls**, including 10,690,294 native calls.
All other outcomes and dropped slots are zero. Acceptance verifies 40 exited
lifetimes, 24 fresh drains, 24 writer/listener bindings, 1,173 resource samples,
2,326 source checks and 264 retained files / 44,378,302 bytes. All three original
container masks and historical namespace maps are restored/preserved.

No broad workspace or candidate Chaos campaign follows the rejected screen.
No master/default promotion or broader roadmap completion is claimed. All work
is local; no hosted CI was dispatched. Next test the intermediate two-worker
population from the unchanged event8 control, keeping CPU allocation fixed.

Raw matrix: `/tmp/kv9-rpc-worker-matched-diagnostic-attempt1/cohorts`.
Frozen preparation and audit: `/tmp/kv9-rpc-worker-comparison-preparation`.
Process audit: `/tmp/kv9-rpc-worker-process-independent-first`.

| Artifact | SHA-256 |
| --- | --- |
| Candidate server | `27aa8e55078e242a0502d2b80782c2e7c2d251f635596ba00da5f8faa195efbb` |
| Build manifest | `2d55707a8abcf11326720518c54889b82150b0269fd4db4813b3c19fc627f6b8` |
| Matched audit | `5f7c6753775a6c94f100dda1794df80550d0be095e624ed5209d2c784b606e0a` |
| Independent statistics | `86a718b903a319693e6064ccdbb409f7813a31dcc5d900ff22114b5d1ce22af4` |
| Process audit | `c003cbdfea5b15d2005066feb81f12ee7a994dfa8be8beea49ef3c8e7210b2be` |
