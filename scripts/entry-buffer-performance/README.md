# Single-buffer staged engine screen

This executes stage one of the [published plan](../../docs/entry-buffer-qualification-v1/next-performance-plan.json).
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

Allocation builds emit no elapsed time. In this corpus, both key and value
are nonempty, so each Put removes one buffer request and 24 requested Entry
bytes, for original and long keys alike. For a group of `n` mutations, the number
of replaced entries freed inside the window is `n` for unpinned overwrite,
`n - distinct_keys` for pinned overwrite, and zero for unique insertion. The
validator checks all six request fields and live-byte differences against these
independent group counts, plus peak bounds and aggregates. Input destruction
remains inside both operation windows. The new whole-batch length preflight
also remains inside the candidate's `write_applied` timer.

A separate untimed observation constructs a fresh final index, records requested
heap bytes and checks full reclamation on drop. Its inter-arm difference must
equal `-24 * (default_keys + 2)`, including both other-CF sentinels. This does not measure physical
jemalloc size classes, fragmentation, RSS, or long-lived snapshot retention.
All counter bookkeeping and JSON output stay outside operation windows.

Keep output under `/mnt/data/kv9-work`, preserve completed results, and do not
replay a failed matrix or relax its gates. Passing this component screen alone
does not authorize production promotion; database correctness/recovery/actual
Chaos Mesh and matched three-copy Redis throughput/latency remain required.


After a completed failed screen, `inspect_codegen.py ROOT` reviews both recorded
measured ELFs; `codegen.py ROOT` then builds one explicitly
unqualified safe accessor control. It changes only key-prefix slicing to use
`min(key_len, bytes.len())`, retains compiler/disassembly identities and executes
no workload. Its source proof and runtime model qualification remain pending;
see the report's declared next plan. Do not substitute this build for either
measured arm or assign it the failed screen's results.
