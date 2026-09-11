# Read-credit-on-CRC performance reporting evidence

This portable bundle contains the 72 original client reports, including all
phase/operation/outcome populations and raw whole-call histogram buckets, and
all 72 resource-coverage files consumed by the accepted statistics reader. It
also preserves the corrected audit and original input inventory, statistics
outputs and pure arithmetic core, original reader, frozen protocol/roles/source
and build bindings, driver/wrapper, root invocation/terminal records, and the
first audit failure with its one-literal hash correction and contract evidence.

`index.json` maps every copied original absolute path to a relative stored path.
It records original decoded bytes/SHA-256 and stored bytes/SHA-256 separately.
Gzip files have zero timestamps; their decoded contents are verbatim originals.
The top-level `READOUT.md` is a verbatim, indexed alias of the original
quantitative tables; detailed per-case and per-operation data is in the bundled
`statistics/statistics-first.json.gz`.
The original absolute-path helper files are evidence copies, not portable launch
commands. Their original paths have not been rewritten or relabeled.

Run the portable reporting verifier from any directory:

```sh
PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 python3 /path/to/read-credit-crc-performance-v1/verify.py
```

Optionally supply `--expected-index-sha256 HASH` from the publication record.
`--check-originals` additionally checks all original local files and therefore
requires the original host paths. The default verifier only reads this bundle.
It checks every indexed stored/decoded hash and recomputes all case aggregates,
pooled histograms/rates/CPU summaries, paired comparisons and all-phase totals
using the included unchanged arithmetic core. It never launches a workload,
subprocess, original reader or acceptance audit and writes no result files.

Full runtime acceptance remains local: the accepted audit records 4,311 retained
data files totaling 94,130,468,368 bytes, 240 exited owned lifetimes, 144 fresh
drains and 144 voter/listener bindings. WAL/data files, executable copies, full
source trees and detailed resource/process/drain observations are deliberately
omitted. Their accepted original inventory and hashes remain included. This
partial bundle cannot independently repeat the full runtime acceptance audit;
verifying its statistics does not newly prove cleanup, durability or history
properties. Benchmark aggregate reports are not a full linearizability ledger.

The experiment used two forward/reverse 10-second repetitions, concurrency 1
and 64, and point/batch64 read/write/mixed workloads on a shared host. KV9 uses
three voters and tmpfs WALs; standalone Redis has persistence and pipelining
disabled. These are not equal durability configurations. Whole-call batch
latency is never divided by key count, and percentiles come from merged raw
buckets rather than percentile averaging. No promotion or sustained-capacity
claim is made by this reporting bundle.

The first auditor failed before case reads because it retained the preceding
owned-buffer driver-arguments hash. Actual argv, wrapper, driver and the new
argument bytes matched the pre-run frozen preparation. The corrected auditor
changes only that literal, retains every other predicate, and writes a fresh
output. Both attempts and the root terminal records are included. The earlier
preparation metadata read failure and rebuilt-source provenance remain retained.

`package.py` records the bounded original-file selection and refuses to replace
an existing index or evidence file. It is host-specific construction evidence,
not required for portable verification. Generated verifier/package/README bytes
are separately bound by the index. Whitespace in original copies is preserved.
