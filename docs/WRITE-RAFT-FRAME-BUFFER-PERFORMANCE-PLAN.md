# Raft frame-buffer write comparison: preparation qualified

The comparison environment passes **28 local controls**: eight driver, fifteen
auditor and five smoke-schema checks. No frame-buffer smoke or timed workload
has run, and no performance improvement is established. The selected runtime
remains `11113f6`; the CRC and frame-buffer candidates remain separate.

The candidate already passes [source, release and ordinary recovery](WRITE-RAFT-FRAME-BUFFER-RECOVERY.md)
and [21 actual Chaos Mesh windows](WRITE-RAFT-FRAME-BUFFER-CHAOS.md). This stage
prepares its matched write comparison using the accepted
[CRC write protocol](WRITE-CRC-PERFORMANCE.md).

## Fixed comparison

| Input | Binding |
| --- | --- |
| Selected server | Source `11113f68f6a5df77da1ffb4fcec850953716ffa3`; binary `dd028cb2f61633dda133b05d814a6d173a79a83145d8ced2f81dc0cf8e0f33bc` |
| Frame-buffer server | Source `01d128fd771dfbf0e6826ee5b6821411afac1fec`; binary `53945784b39f7c90951b96d6f0700f432412369c24f26dcdd1d27e1988541c8c` |
| Native v3 client | Source `0be806d9671e2c50701a64aa7889c8859b7648ba`; binary `8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957` |
| Workloads | Put and BatchPut(64), concurrency 1 and 64 |
| Smoke phase | Eight two-second cohorts |
| Timed phase | Sixteen ten-second cohorts; second repetition reverses the complete first order |
| Dataset | 4,096 keys plus sentinel, 128-byte values, seed 71, warmup 128 |
| Timing placement | Client CPUs 0–1; all three voters share CPUs 2–5 |

The driver, auditor, retention helpers and controls derive from the accepted CRC
comparison through recorded path, identity and hash substitutions. Workload
descriptors, accounting predicates, ordering and resource limits are unchanged.
The accepted smoke reader's native v3 import correction and its original failed
attempt are retained. No uncertain-write replay is introduced: healthy timing
acceptance requires every measured call to succeed with exactly one attempt.

Both throughput and whole-call mean/p99 must be reported per order and pooled,
with calls/s distinguished from batch items/s. Complete histories, fresh replica
drains, final data, source/binary identities, process lifetimes, resource samples
and retained bytes remain acceptance requirements. KV9 keeps its existing Raft,
sync, durable-apply and response fences. This is a single-host tmpfs-WAL screen;
it does not qualify power-loss durability or update the Redis comparison.

## Capacity prevents execution

The accepted CRC comparison retained **48,819,441,664 allocated bytes**, including
compressed objects and other files/directories. Its compressed object file
lengths alone were 48,501,030,853 bytes; that smaller number is not the full
resident footprint. Replaying those accepted volumes gives this scenario:

| Quantity | Bytes |
| --- | ---: |
| Recorded available space | 120,132,231,168 |
| Unchanged retention floor | 103,079,215,104 |
| Reference campaign resident footprint | 48,819,441,664 |
| One-cohort restore reserve | 17,179,869,184 |
| Metadata/final margin | 1,073,741,824 |
| Empirical required available space | **170,152,267,776** |
| Additional space at the recorded observation | **50,020,036,608** |

The gap is **50.02 GB / 46.59 GiB**. Sequential compression already retains
previous cohorts' objects, so sequencing does not remove this requirement.
Past cleanup is already reflected in the observation. The scenario uses measured
CRC volumes; an unmeasured candidate may produce more data. A fresh observation
and capacity qualification are required before execution.

All original limits remain: 32/96 GiB tmpfs/disk preflight, 16/64 GiB runtime
floors, the stricter 96 GiB retention/restore floor, 8 GiB per member, 16 GiB per
cohort, 128 GiB combined logical/physical campaign caps, 600-second codec and
1,800-second cohort deadlines. This stage changes none of them.

## Execution sequence and evidence

1. Qualify additional capacity and refresh source/environment bindings.
2. Run the complete eight-cohort smoke phase and its retained reader.
3. Refresh capacity/isolation, then run all sixteen timed cohorts with no
   overlapping builds, proofs, profiles, codecs or fault workloads.
4. Run independent acceptance and report throughput, latency and resource cost.
5. Require full point/batch/read/mixed regressions before any promotion. The
   [CRC 24-smoke/48-timed regression plan](write-crc-full-regression-plan-v1/README.md)
   remains a separate pending gate.

[Portable evidence](raft-frame-buffer-ab-plan-v1/README.md) retains the frozen
preparation, all three successful command outputs, the capacity arithmetic and
its original input metadata. The preparation's earlier pending-control fields
are preserved; the later control terminal supersedes only that status. Capacity
and runtime readiness remain false. No original industrial checklist item closes,
and no GitHub CI workflow is dispatched.
