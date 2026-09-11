# Explicit point and batch read/write measurement

Tracking: #9, #13 and #20. The current v2 point-read clients support pure GET
only, while native writes use BatchPut and Redis writes use MSET. Those clients
cannot establish actual point PUT/SET or point GET/PUT mixed performance. This
change adds an explicit version 3 measurement configuration for those APIs.
It does not change the database service, Raft or storage implementation.

## API selection and compatibility

Version 3 requires both selectors, with no implicit point/batch conversion:

| Native selector | Redis selector | Operation |
| --- | --- | --- |
| `read_api: point_get` | `read_api: get` | One GET |
| `read_api: batch_get` | `read_api: mget` | One batch read |
| `write_api: point_put` | `write_api: set` | One PUT / SET |
| `write_api: batch_put` | `write_api: mset` | One batch write |

Either point selector requires `batch_size: 1`. `read_percent: 0`, `50` and
`100` select write-only, deterministic mixed and read-only traffic. A mixed
percentage describes the deterministic selection rule, not a promise of an
exact finite 50/50 population. Report actual completed read and write counts.
Both selectors remain explicit even when one operation has zero measured calls.
Missing, null, unknown and wrong-version selectors are rejected.

Version 1 keeps its implicit batch APIs and original serialized schema. Version
2 keeps its explicit read selector, implicit batch writes and existing pure-read
restriction for point GET. New point-write/mixed semantics require version 3;
old measurements and retained artifacts are not reinterpreted.

Native point writes construct the SDK's actual Put operation. Redis point writes
emit RESP2 SET and require its proper response. MSET with one key remains a
separate API. Setup, missing-key checks, deterministic seeding, untouched sentinel
and final full-dataset verification keep their existing batch operations.
Only warmup and measurement use the configured selectors, and their operation
labels must match the actual APIs. Version 3 report models are
`bounded_native_api_performance` and `bounded_redis_api_performance`.

## Measurement contract

Keys, values, nonces, operation selection, cutoff/drain scheduling, deadlines,
retry/unknown accounting, histogram arithmetic and admission bounds stay the
same. Redis retains one outstanding command per connection, no pipeline and no
command replay after an ambiguous result. Native writes continue to require a
positive exact applied receipt; a read-shaped success cannot satisfy a write.
The independent validators recompute selected wire sizes and operation labels,
and native/Redis pairing checks both selectors as well as the shared workload.

Existing `*_batches_per_second` fields count whole API calls. Report calls/s and
input items/s separately: they coincide for point operations and batch size one.
For larger batches, a single call's latency is the whole batch latency, not an
individual key latency. Neither rate relabeling nor percentile averaging is
valid. Initialization/verification accounting retains its batch operation names.

This is measurement-client support. It establishes no throughput gain or new
production capability by itself. A fresh comparison must use the same new
client binaries for every server arm, bind source/build/runtime identities, run
separate correctness gates, and freeze its workload before timing. Old and new
client runs are not paired controls. Full correctness histories and Chaos Mesh
acceptance remain separate from aggregate performance reports. Standalone Redis
and three-voter Raft do not have equivalent replication or durability semantics.

## Local validation

Local focused validation passes 15 native Rust tests, 19 Redis Rust tests,
19 native Python tests and 17 Redis Python tests. Both Rust targets pass
formatting and warnings-denied Clippy. Source hashes are unchanged across each
gate; the Redis gate also rechecks every input of the earlier native gate.
The shared deterministic common source remains byte-identical to the parent.

Semantic controls cover actual Put construction and singular encoding, positive
applied receipts and wrong operation/reply rejection, concrete SET bytes and
malformed responses, real mixed GET/SET traffic on one connection, a consumed
SET with a lost reply followed by a distinct logical call, typed unknown
accounting, selected labels/sizes, paired APIs and legacy serialization. Python
checks aggregate reports and declared configuration; it cannot reconstruct raw
reply bodies that those reports do not contain. Source-bound Rust traffic and
its protocol tests check the actual operation and reply shape.

Retained local gates are `/tmp/kv9-point-write-v3-native-validation-first` and
`/tmp/kv9-point-write-v3-redis-validation-first`. Clean client releases and a
separate real-runtime smoke remain required. No version 3 performance result is
claimed. The work-signal screen used unchanged v2 native03c1 and Redis b8ec
binaries. No build, test or audit overlapped that timing; only light reads and
edits in this separate measurement worktree ran concurrently.
