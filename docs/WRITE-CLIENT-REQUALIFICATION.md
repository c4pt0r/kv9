# Requalifying the missing native measurement client

The historical fixed v3 measurement executable disappeared during external
cleanup. Its replacement is explicitly a new qualified-input candidate, not a
byte-identical restoration. Both server arms must use this same fixed client;
old numbers measured with the missing executable are historical references,
not a matched comparison with the replacement.

## Immutable inputs

| Input | Identity |
| --- | --- |
| Client source | Clean `0be806d9671e2c50701a64aa7889c8859b7648ba`, all 581 original file hashes. |
| Toolchain | Retained Rust/Cargo 1.94.0, release opt3, default features; original builder and flags. |
| Missing historical client | 5,594,104 bytes; `8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957`. |
| Rebuilt candidate client | 5,592,624 bytes; `1b8060eb168610328c10a27480de16bd8b2d6805166d8169638f85ceef0992c4`. |
| Selected CRC server | Clean `bd42e60`; ELF `106f517c84fabe790ea139f3c18661e14447abd9e257fdd6ba82bfabe6796eb9`. |
| Upper-bound candidate server | Clean `e2e23cc`; ELF `6f074e867eae45274c17b54aa763888e92db2c45d5757974fa52ee0f15936d49`. |

The retained rebuild comparison matches all 257 compiler-artifact records after
normalizing only source/target directory prefixes and cache freshness. That
does not prove byte identity: the original executable and complete original
compiler/linker invocations are unavailable. The reason for the 1,480-byte
difference remains unproven. No additional rebuild is needed to treat this
specific new executable as a separately tested comparison input.

Original recovery and rebuild metadata are retained in the
[checkpoint-owner development packet](checkpoint-owners-v1/README.md).
Client source, executable and build records remain under
`/mnt/data/kv9-work/performance-input-recovery-20260915-first/rebuild-first`.
No original source or retained measurement executable is overwritten.

## Source-level qualification completed

The retained 1.94 toolchain passes 15 benchmark-binary tests, 26 client-library
tests and 19 Python report tests. One existing client test requiring independent
history-fixture output remains ignored. The exact 581 source hashes and copied
measurement ELF are unchanged after these runs. Tests use the existing data-
volume compiler target and NVMe temporary fixtures; no global toolchain changes.

The selected tests cover singular Put dispatch and positive applied receipts,
unknown writes without replay, bounded typed redirects and original deadlines,
whole-batch uncertainty, malformed replies, deterministic keys/value bytes,
fixed-rate dropped work, histogram bounds and accounting. They do not execute
the copied measurement ELF against both production servers.

Evidence is in
`/mnt/data/kv9-work/performance-client-qualification-20260915-second`:
session **95176 / 2d9a84 / 0**, with separate command/terminal records. The first
attempt's non-root Git ownership refusal occurred before any test/build and is
preserved in `performance-client-qualification-20260915-first`. The correction
uses sudo with a process-local safe.directory for the exact checkout; no global
Git configuration changes were made.

## Runtime qualification and full comparison gates

The original workload and independent gates are:

1. Bind both exact server source trees, server ELFs and default Cargo records;
   bind the new client ELF to its own clean source/build/Cargo records. Do not
   relabel it with the historical hash or fabricate a combined server build.
2. Run all eight original two-second smoke cohorts: point Put and BatchPut(64),
   concurrency 1 and 64, on both servers. Validate actual singular wire kinds,
   full deterministic datasets, positive receipts, report accounting, original
   deadlines, zero hidden retries/drops/unknowns, fresh drains and process exits.
   These smokes qualify the executable; they are not the performance result.
3. With fresh capacity and CPU isolation, run the original sixteen ten-second
   cohorts in both opposite orders. Keep the same fixed client in every arm,
   three tmpfs WAL voters, explicit sync/quorum semantics, workload seeds,
   payloads, admission limits, resources and histograms.
4. Independently decode retained data and verify complete reports, all outcomes,
   source/process identities, observed storage floors, cleanup and both orders.
   Assess throughput and latency together. Do not promote from a single fast
   cohort, source-level tests or the short smoke measurements.

New preparation and retained output go to `/mnt/data/kv9-work`. Active voter
storage keeps the original tmpfs mode, which does not establish physical
power-loss durability. Moving retained output to the data volume requires
separate observations of `/`, the output filesystem and `/dev/shm`; root floors
must not disappear when output moves. The original 79,455,850,496-byte full
campaign envelope and all output/restore/decoder bounds remain required.
Any additional root stat observation runs identically in both comparison arms.

The new derived helper closure and its explicit input/root-guard changes are
under `upper-bound-requalified-preparation-20260915-first`. Its fresh binding
checks all 859 CRC, 1,116 upper-bound and 581 client source files together with
their retained executable and Cargo identities.

All eight actual smoke cohorts now pass, followed by independent complete
dataset, report accounting and process-lifetime checks: **803,116 successful
calls, 8,192,386 successful input items and 32 exited owned processes**. The
actual smoke terminal is `45876 / 559f87 / 0`; the independent reader exits
`905b8a / 0`. The exact smoke matrix hash is
`d3f60eaf897f5b24cabd79e2d56188d45889ac708b5c6c61bd77481dbc3ae9c6`.
Execution records use `upper-bound-requalified-execution-20260915-first`; the
reader's result is under the preparation's `readiness/smoke-readback` directory.
These checks qualify the new executable for the original comparison. Smoke
rates are not reported as performance results.

The sixteen-cohort matched comparison and enclosing retention audit remain
pending. CRC remains selected; there is no new QPS, latency or promotion result
in this qualification report.
