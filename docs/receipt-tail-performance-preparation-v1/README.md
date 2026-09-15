# Receipt tail hint: matched write screen preparation

The original CRC `bd42e60`, receipt candidate `a6ac335` and fixed native v3
client `0be806d` now pass the actual source, executable, compiler and default
feature readers. They bind 859, 869 and 581 source files respectively. Eight
source-binding controls and eight smoke-reader controls pass; actual terminal
`b4e952/0`. These are preparation checks, not a new database correctness or
performance campaign.

The complete comparison retains eight smoke and sixteen timed cohorts, point
Put and BatchPut(64), concurrency 1 and 64, two opposite orders, ten-second
timed windows, 128 warmups, 128-byte values, 4,096 keys plus sentinel and seed
71. Throughput, complete-call latency, final data readback, voter drains,
process lifetimes and complete retained evidence remain required. The previous
storage-v3 policy is unchanged. No builds or measured workloads ran here.

This archive contains the frozen driver, auditor, retention and isolation
helpers, explicit commands and source substitutions, preparation lineage,
original role-binding result, all focused control output and root execution
records. The inherited 71 controls remain available; they were not rerun as
part of these 16 focused checks. Original database executables and prior
campaign payloads remain locally retained.

Run `python3 -B docs/receipt-tail-performance-preparation-v1/verify.py`.
It checks all 98 members / 1,174,930 original bytes and gzip EOF without
extracting or executing archived helpers. It also reads the original focused
test counts and role populations. The archive is 344,800 bytes, SHA-256
`582af35a32429add728f835ce4bb02abcff697dc6be3da9a2f1461f6bf043d14`.

Fresh capacity and exclusive measurement conditions are still required before
launch. This package grants no runtime release and reports no new QPS or p99.
