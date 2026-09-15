# First write-stage capture and exporter correction

This packet preserves a failed live diagnostic and the subsequent tested source
correction. It does not establish accepted observer overhead, new write
performance, Redis parity or Chaos Mesh acceptance. CRC remains selected.
See the [report and structural safety argument](../WRITE-STAGE-CAPTURE.md).

## Actual results

| Check | Result |
| --- | --- |
| Matching releases at `8c00085` | Default and combined tracing/diagnostics builds and independent executable/source/Cargo readback pass. |
| Focused capture controls | Eight controls pass, including first-invalid preservation and the actual fixture method-resolution path. |
| Row 0, default | 33,709 measured calls; native validation, independent dataset scan, fresh drains and cleanup pass. |
| Row 1, instrumented | Native report completes 32,562 measured calls; the first fresh post-client snapshot has five lost leader recording calls and is refused. Subsequent drain and independent dataset validation do not run. |
| Rows 2 and 3 | Never launched. |
| Failure-state audit | Exact refusal, retained first-fresh snapshots, eight exited owned lifetimes and original container CPU restoration confirmed. |
| Correction | 276 Raft library tests pass, one existing fixture test ignored; combined-feature all-target workspace Clippy passes with warnings denied. |

The capture was four planned two-second BatchPut(64), c64 rows using three
tmpfs WAL voters. Instrumentation enables both `write-stage-tracing` and
`write-path-diagnostics`, so any future overhead comparison must identify both
features. A client-exit snapshot follows the client's own verification and is
not an exact measurement-stop boundary. The failed row is not rescued by
discarding its first invalid observation or relaxing loss checks.

The source correction makes status export try the existing pump gate before
copying the fixed trace arrays. Both guards are released before allocation or
serialization. It prevents this exporter/recording lock conflict; it does not
prove negligible scheduling overhead. Busy export remains unavailable and
must be refused by the capture reader. No release or live capture of the
corrected source has run in this packet.

## Original attempts and execution records

Matching release build: session `54360`, terminal `cd1a78/0`; independent
release readback: `8509d8/0`. Capture controls: `a8cc5d/0`.
Actual capture: session `90381`, terminal `db2939/1`, with complete cleanup and
exact restoration. The original loss refusal is the authoritative run result.

The independent audit first failed on permissions while inspecting retained
root-owned directory metadata (`b0cdfe/1`). The same audit code subsequently
passed with read access (`7ce083/0`); it did not rerun the workload or read the
WAL payload. Both attempts are preserved. Prospective full-success audit code
is retained for provenance but was not executed as successful-capture evidence.

Correction validation: session `7908`, terminal `1a4aac/0`. Independent source
review checks all production recording callers, pump-gate ownership and guard
release before vector allocation. The five existing proof inventories and
their source pins are unchanged; their theorem/model suites were not rerun.
The correction tests are separate from the original `8c00085` qualification.

## Portable contents

[runs.tar.gz](runs.tar.gz) contains 249 regular members / 14,950,829 logical
bytes: frozen preparation and controls, original helpers and diffs, actual
execution and runtime metadata, raw status snapshots, native reports,
retention receipts, independent audit/review, correction logs and source pins.
Its size is 1,981,891 bytes and SHA-256 is
`34da13d9a6568698aa24bc91b2d840876cdb2cd99a36361f1bc6b92a08cd4b48`.
Every member was decoded, hashed and compared with its original bytes:
[readback](readback.json), [inventory](inventory.json). Assembly/readback
terminal: `3e1e1f/0`.

[release-metadata.tar.gz](release-metadata.tar.gz) separately retains the
29-member original release qualification metadata. Its
[inventory](release-metadata-inventory.json) and
[independent readback](release-readback.json) bind the clean source,
compiler/codegen records, feature selections and retained executable identities.
Historical initial live-session records are not current process status; their
terminal records take precedence.

The 4,234,585,489 bytes of original database payload stay on the data volume and
are excluded from the portable archive. Original fixture receipts cover exact
copy and full readback before scratch removal; the later audit checks receipts
and current file metadata, not another payload hash. Executables, compiler
caches, proof tools and GitHub issue snapshots are also excluded. This packet
does not supply all inputs needed to execute the campaign on another machine.

New bulk outputs remain under `/mnt/data/kv9-work`. The existing NVMe compiler
cache is reused and active performance fixtures retain their declared tmpfs
placement. No old evidence was moved or deleted and no hosted CI was dispatched.

Next: qualify a matching pair from the corrected clean source and run a fresh
four-row capture with unchanged loss, coverage and correctness checks. Its
baseline must not reuse a row from this failed, different-source attempt.
