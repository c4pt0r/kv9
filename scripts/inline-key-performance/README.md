# Inline-key staged engine screen

This executes stage one of the [published plan](../../docs/inline-key-qualification-v1/next-performance-plan.json).
It uses the already qualified original-dependency engine copies; it does not
rerun qualification or enable the candidate in production.

`prepare.py NEW_ROOT` preserves the declared plan and original corpus, then
generates timing/counting harnesses from the previous engine-interface sources.
One shared transformation adds original/zero-padded-long inputs, dataset identity
and preparation-only refused-application checks. The original `apply_pass`
operation bodies and timing boundaries remain byte-for-byte unchanged.
`check_harness.py ROOT` independently verifies complete source correspondence
after erasing only instrumentation and explicitly untimed observations.

`build.py ROOT` creates separate baseline/candidate timing and counting release
executables. First-party artifacts are explicitly invalidated under the retained
cache guard; actual engine inputs and registry dependency identities are checked.
Generated sources, compiler records, executable hashes and every process's
launch/terminal identity are retained outside the repository target.

Run `run.py ROOT prepare`, then `validate.py ROOT prepare`; run `counting`, then
validate `counting`; only after acceptance run `timing`, then validate `final`.
Run the supervisor on CPUs `6-15,22-31`; each measurement process uses CPU 4.
The fixed sequence is four preparations, 16 count processes and 32 ABBA timing
processes. Every measured arm has twelve 106-group passes after one excluded pass.
The host is shared. Timing values are engine nanoseconds per group, not client
request latency or database QPS.

The validator independently decodes original mutations, pads before adding
unique ordinals, reconstructs all final byte maps and read probes, checks every
raw statistic, source identity and completed child, and enforces the unchanged
declared gates. Short writes must improve mean by at least 10% in all four cases,
with both order directions improving and pooled p99 no more than 2% worse.
Long-key mean and p99 may increase no more than 2%. If a write gate fails, the
remaining read/snapshot/range stage is not run and cannot be reported as passed.

Allocation builds emit no elapsed time. Each nonempty short key removes one
buffer request. Relative requested entry cost is `24 - key_length` for an inline
key, or `24` for a long key. For a group of `n` mutations, the number of replaced
entries freed inside the window is `n` for unpinned overwrite, `n - distinct_keys`
for pinned overwrite, and zero for unique insertion. The validator checks all
six request fields and the live-byte difference against these independent
group counts, and checks peak bounds/aggregates. Consumed input buffers are
destroyed inside the window in both arms; negative net bytes do not describe
the complete index footprint.

A separate untimed observation constructs a fresh final index, records requested
heap bytes and checks full reclamation on drop. Its inter-arm difference must
match every live key plus both other-CF sentinels. This does not measure physical
jemalloc size classes, fragmentation, RSS, or long-lived snapshot retention.
All counter bookkeeping and JSON output stay outside operation windows.

Keep output under `/mnt/data/kv9-work`, preserve completed results, and do not
replay a failed matrix or relax its gates. Passing this component screen alone
does not authorize production promotion; database correctness/recovery/actual
Chaos Mesh and matched three-copy Redis throughput/latency remain required.
