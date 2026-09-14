# Retained FNV tail analysis

The worst c64 BatchPut64 tail, first FNV run07, coincides with more global IO
pressure. It does not show a larger sampling gap, higher CPU2–5 busy fraction,
or uniquely large voter RSS/thread population. These observations cannot
identify the cause of individual slow calls.

For runs06/07/08/09 (old forward, FNV forward, FNV reverse, old reverse), the
original whole-call p99 buckets are respectively [7.864,7.930],
[12.059,12.190], [6.685,6.750], and [7.340,7.406] ms.

- CPU2–5 busy fractions are 83.91%,82.46%,84.18%,83.94%; their observed iowait
  counter deltas are all zero. These counters include all activity on those CPUs.
- Global CPU PSI `some` fractions are 42.204%,42.221%,45.204%,42.903% over the
  recorded in-window intervals. The lowest-tail FNV reverse run has the largest
  CPU-pressure fraction of these four.
- Global IO PSI `some` deltas are 13.351,49.061,9.591,11.292 ms over approximately
  9.90–9.96 seconds. IO `full` deltas are 12.757,48.251,9.393,11.085 ms. Run07's
  largest adjacent roughly 52 ms interval has 17.43% IO `some` pressure; maxima in
  the other three are 2.40–6.13%. This is global stalled wall time, not disk IO
  duration attributable to a voter or to `sync_namespace`.
- All four have 189–190 in-window resource/host samples. Maximum resource gaps
  are 53.49–54.55 ms; there are no gaps above 75 ms. Host interval edge gaps are
  explicitly retained rather than interpolated.
- Voter2 grows from 48–59 to 71–89 observed threads; voters1/3 remain at 4. Final
  voter2 RSS is 2,589.5/2,629.2/2,791.5/2,627.4 MiB respectively. The reverse FNV
  run has the most threads and largest final RSS, while its p99 bucket is lowest.
  RSS is resident memory, not a cumulative allocation counter.

All 16 cohorts remain in `REPORT.md` and `summary.json`; no unfavorable cohort
was filtered out. For context, old c1 batch run13 has still higher global IO
PSI `some` (0.7102% versus run07's 0.4933%) under a different workload, so the
pressure signal is not unique to FNV. Cross-workload percentages do not isolate
the effect of that pressure.

The source-bound snapshots are coarse and sequential. Host PSI is machine-wide;
the retained histogram has no individual call timestamps. A global IO-pressure
increase and a larger endpoint namespace-publish histogram are compatible with
the same broad interval, but they are not a causally matched observation. This
analysis neither confirms nor rules out the root's directory-sync hypothesis.

All 65 selected cohort/matrix inputs matched the accepted audit inventory.
The 73-file input record also includes eight authority, terminal, summary and
source files, with their observed and applicable predeclared hashes retained.
Analysis completed with direct tool receipt 83a084, exit 0. No workloads, builds,
codecs, cleanup, source changes, or original-payload hashing were performed.

The complete all-16 report, analyzer, input hashes and interval observations are retained in the `host-analysis/` prefix of the [published evidence archive](https://github.com/c4pt0r/kv9/blob/483b8c3629b033734f5d7a2b8653a1352304a4b5/docs/published-directory-source-v1/README.md). Original benchmark inputs remain in [the accepted FNV campaign](https://github.com/c4pt0r/kv9/blob/fd2e18bd9471ad4f4e8aab6e3d0cd6f8c145e0ce/docs/fnv-writer-performance-v2/README.md). The subsequent [directory-publication candidate](WRITE-PUBLISHED-DIRECTORY.md) is separately source-qualified; it has no measured database speedup yet.
