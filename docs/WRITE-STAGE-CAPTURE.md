# First write-stage capture and exporter-contention correction

The first actual loaded-batch trace attempt exposed a diagnostic defect: the
instrumented leader lost five recording calls while status export shared the
collector lock. The unchanged reader correctly refused that capture. The run
stopped after its second attempted row, with cleanup and exact CPU restoration
complete. It supplies **no accepted observer-overhead comparison**.

## Exact attempt and limits

Both releases use the same 1,385 clean source files at `8c00085`, Rust/Cargo
1.94, opt3, ThinLTO and one codegen unit. Default has no features; instrumented
enables `write-stage-tracing` and `write-path-diagnostics`. Independent source,
Cargo feature, compiler-command and copied executable readback passes.

| Role | Executable SHA-256 |
| --- | --- |
| Default server | `cb77ef363437aa9893d931aad059e91cf5a3124a7ff31adeb12d4db3a55d5ebf` |
| Instrumented server | `e78217c464640076dce085737ab4668b1f83a09bf3ead728bc8900f722d24b23` |
| Requalified fixed native client | `1b8060eb168610328c10a27480de16bd8b2d6805166d8169638f85ceef0992c4` |

The prospective matrix was four two-second BatchPut(64), c64 cohorts in opposite
orders: default, instrumented, instrumented, default. It retained 4,096 keys,
128-byte values, seed 71, 128 warmups, TonicStream, three tmpfs WAL voters and
the existing sync/quorum/apply/reply fences. Client CPUs were 0–1 and voter
CPUs 2–5. Current-source checkpoint ownership distinguishes these releases
from earlier bd42/e2e comparisons. Short observer rates cannot replace the
ten-second performance gate, and tmpfs does not establish power-loss durability.

| Original row | Observed outcome |
| --- | --- |
| 0, default | 33,709 successful measured calls; complete native validation, independent final dataset scan, fresh drains and cleanup. |
| 1, instrumented | Native report completes 32,562 measured calls, but the first fresh post-client capture refuses five lost leader recording calls. No post-client drain or independent final dataset scan runs. The whole cohort remains failed. |
| 2 and 3 | Never launched. |

All three first-fresh post-client raw files from row 1 remain retained. Both
followers report zero lost calls and no terminal inspection rows. Node 2 has
512 retained group rows, 512 inspection rows and five lost calls; their unknown
positions cannot be reconstructed from the aggregate loss count. These rows
are not accepted by relaxing the reader or selecting a later snapshot.

The client process exits after its own verification phase, so this boundary is
not the precise measurement-stop edge. Each raw capture waits for two exporter
counter advances without inspecting trace quality, then preserves those first
fresh bytes before validation. Eight focused controls cover freshness, the
actual fixture method-resolution path, preservation of invalid observations,
hook restoration, changed later tails and the narrowed four-row storage budget.

Actual capture terminal: `90381 / db2939 / 1`. The independent failure-state
audit passes `7ce083/0`: row boundaries and refusals match, all eight recorded
owned lifetimes are gone, and the three original container identities and CPU
settings are restored. Its first nonprivileged metadata read failed on retained
directory permissions (`b0cdfe/1`); that result is preserved. The privileged
reader used the same audit code and did not execute another workload.

The original fixture-copy receipts cover 4,234,585,489 retained payload bytes
across both attempts, with exact copy/readback before scratch removal. The
later independent audit checks those receipts and current file metadata; it
does not rehash the WAL payloads. See the [portable original evidence](write-stage-capture-v1/README.md).

## Correction and safety argument

Every production group/terminal-inspection recording call occurs inside
`step_observed` or its apply path. Its only three callers—`step`,
`tick_and_step`, and the owner loop—hold the existing `pump_gate` throughout.
Before this correction, status export could acquire the separate trace lock
while that owner ran. A recording call then deliberately dropped rather than
blocking; the observed five-call loss exposed that conflict.

The exporter now tries `pump_gate` before trying the trace lock. If either is
busy or poisoned, it emits an invalid, unavailable snapshot. If both are held,
it copies only the fixed arrays/counters, releases the trace guard and pump
guard, and then allocates the output vectors. Serialization also remains outside
both locks. The JSON schema and all loss/refusal checks are unchanged.

Because production recording owns the same pump gate, successful export and
recording are mutually exclusive. Export cannot hold the leaf trace lock while
a production recording call tries it. The lock order is pump then trace for
both paths, with no inverse edge; there is no blocking acquisition in the
exporter. Erasing these feature-gated observations preserves the original
database transitions and acknowledgment conditions.

This proves the specific exporter/recording exclusion, not identical scheduling
or negligible overhead. A successful fixed-array copy can delay the next pump,
and an exporting thread can be preempted while holding the gate. Work is bounded;
wall-clock delay is not. Exports favor moments between pumps, and the first
fresh post-client snapshot can still be unavailable. Such a result must remain
a failed capture rather than trigger quality-based polling.

The correction passes 276 Raft library tests (one existing fixture test ignored)
and warnings-denied all-target workspace Clippy with both diagnostic features.
Two new tests exercise concurrent snapshots during a recording owner and the
actual driver gate. Terminal: `7908 / 1a4aac / 0`. Default production code is
unchanged. The five existing checkpoint/retention proof inventories and every
source pin still match; their unchanged theorem/model suites were not rerun.
Independent source review confirms all recording callers and guard release
before vector allocation. This is a structural proof of instrumentation,
not a new mechanized consensus proof or new Chaos Mesh result.

## Next capture

The separate [corrected-source capture](WRITE-STAGE-CAPTURE-RESULTS.md) now
completes all four focused rows with zero recording loss, independent acceptance
and measured observer overhead. It uses a newly qualified matching release pair
and fresh default rows. This refused attempt remains unchanged and contributes
no baseline row to that comparison. The accepted tails still do not constitute
complete request histories or identify a particular native-client p99 call.

All new logs, proof/build records and retained evidence remain under
`/mnt/data/kv9-work`; reusable compiler targets and latency-sensitive fixtures
keep their explicit NVMe/tmpfs placement. CRC remains selected. No original
industrial roadmap checkbox closes, and no hosted CI was dispatched.
