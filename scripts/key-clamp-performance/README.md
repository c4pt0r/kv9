# Three-arm safe accessor write screen

The [declared plan](../../docs/entry-buffer-performance-v1/next-key-clamp-plan.json)
compares the selected Vec/Vec baseline, failed single-buffer control and newly
qualified safe key-accessor candidate. The constructor, preflight, value access,
dependencies and harness are identical between the two single-buffer arms.

Run `prepare.py NEW_ROOT`, `check_harness.py ROOT`, then `build.py ROOT`. Six
ordinary ThinLTO releases identify fresh first-party artifacts, original registry
inputs and exact source/ELF hashes. The generated timing/counting operation
bodies match after erasing only instrumentation and untimed footprint metadata.

Run `run.py ROOT prepare` and `validate.py ROOT prepare`; then `counting` and its
validation; then `timing` and `validate.py ROOT final`. Supervisors use CPUs
`6-15,22-31`, measured processes CPU 4. There are six preparations, 24 allocation
processes and 48 timed processes. Eight cases cover original/long keys,
overwrite/unique insertion and with/without an old view. Fixed order is
baseline/control/new/new/control/baseline, with twelve passes after one excluded
pass. The entire new length preflight remains inside `write_applied` timing.

Input, position/refusal, old-view and CF-sentinel checks run on the actual release
paths. The validator independently reconstructs datasets, every raw statistic,
completed child identity and allocation record. All request counts, live/peak
bytes and final-index footprints must match exactly between control and new
accessor. Against baseline, both single-buffer arms have the separately checked
one-request/24-byte entry reduction for this nonempty-key/value corpus.

The report separates new-versus-control effects from new-versus-baseline
selection. The original gate is unchanged: all four original-key means improve
at least 10% pooled, both order directions improve, and pooled p99 increases at
most 2%; long-key pooled mean/p99 increase at most 2%. Individual order tails
remain visible. A failed gate stops the declared read/range stage. Group latency
is not client-request latency or database QPS; no time comes from counted builds.

Bulk output stays under `/mnt/data/kv9-work`, compiler target reuse is guarded,
and CI stays local. A passing component result alone does not promote production.
