# Redis single-key GET reference

The Redis reference accepts an explicit version 2 configuration with
`"read_api": "get"`, `"batch_size": 1`, and `"read_percent": 100` to measure
actual RESP2 `GET key` calls. Each worker keeps one connection with one outstanding
command. This is a single-instance Redis reference; persistence, replication,
server identity, CPU placement, and measurement isolation still require the
enclosing fixture's evidence.

Version 1 configurations omit `read_api` and retain their existing MGET/MSET
configuration, wire-size, metric, and report schemas. Version 2 requires an
explicit `"get"` or `"mget"`; missing, null, unknown, and mismatched selectors are
rejected. Explicit version 2 MGET preserves the existing batch behavior. GET
rejects every batch size other than one and every read percentage other than 100.

GET warmup and measurement encode the `GET` command and decode a single bulk
value or nil. MGET arrays cannot satisfy this decoder, and GET bulk replies cannot
satisfy the MGET decoder. The distinction follows Redis's
[GET](https://redis.io/docs/latest/commands/get/) and
[MGET](https://redis.io/docs/latest/commands/mget/) response contracts. The benchmark
dataset requires populated, deterministic values, so a nil result during traffic
is a retained data-integrity failure. Initialization, the missing-key guard,
seeding, the untouched sentinel, and final full-dataset verification continue to
use MGET/MSET and their original accounting.

Reports use the configuration's version. GET reports identify
`bounded_redis_point_get_diagnostic`, label warmup/measurement as `get`/`mset`
(with zero MSET traffic), and retain `mget`/`mset` for setup/verification. They add
GET request/response byte bounds while retaining the setup MGET/MSET bounds.
Every failed GET is one read-failure call, one input item, and one whole-call
latency sample; failures are never retried. Only a later logical call may reconnect.
Malformed framing, wrong lengths, and unexpected values cannot become successful
samples. The 1 MiB wire cap and all existing pending-input, call, and deadline
bounds remain in force.

The deterministic key/value generator, scheduling, cutoff/drain rules, histogram
arithmetic, source/build validation, and fixed-rate inputs are unchanged. Existing
`*_batches_per_second` report/load field names continue to count whole calls. In
GET mode one whole call contains exactly one KV operation, so the call and input
item rates coincide; no rate is multiplied or rescaled. Pair validation explicitly
matches Redis GET with native `point_get`, and Redis MGET with native `batch_get`.
An MGET containing one key is not accepted as a GET pairing.

Local validation covers legacy serialization, concrete GET framing, binary/nil
values, wrong reply shapes and cardinalities, a dropped reply with no command
replay followed by a distinct logical call, whole-call failure metrics, schema
negative cases, and mismatched pairing rejection. These are client correctness
checks. Matched release measurements and throughput/latency conclusions require a
separate frozen build and controlled fixture run.

For actual SET and mixed GET/SET measurement, version 3 requires both API
selectors. See [the shared contract](../../../docs/POINT-WRITE-MEASUREMENT.md).
Version 2 GET remains a pure-read diagnostic with its original schema.
