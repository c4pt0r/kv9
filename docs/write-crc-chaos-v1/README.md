# CRC slicing8: actual Chaos Mesh qualification

The isolated candidate `e748620a7b0ba326ac5f4fa8e3a6b1ff48553c6a` passes
the original 21-window Chaos Mesh campaign, its independent retained-history
audit and final cleanup. This evidence branch adds reporting files to that
frozen source; it does not redefine the tested source or select the candidate.
The run completed on 2026-09-12 in America/Los_Angeles.

| Complete history | Operations | OK | Unknown | Refused |
| --- | ---: | ---: | ---: | ---: |
| CLI/catalog | 5,629 | 5,063 | 552 | 14 |
| Persistent point stream | 1,512 | 1,491 | 15 | 6 |
| Native point/atomic batch | 2,692 | 2,651 | 33 | 8 |
| Total | 9,833 | 9,205 | 600 | 28 |

Unknown writes remain unknown. The accepted histories, source and executable
bindings, fault-effect observations and four fresh final voter drains are
retained. All 34 observed server lifetimes and 25 owned containers have exited;
the owned namespace was removed and all eight historical namespace UIDs were
preserved. The independent audit precedes cleanup and correctly retains its
original `cleanup_complete=false`; subsequent cleanup receipts supplement it.

The campaign covers seed blackholing, each voter pod failing, partition,
admission overload, delay, EIO and ENOSPC on each voter, each missing-log and
replacement-volume refusal, and pending/recovered retained-volume endpoint
migration. A normal native baseline precedes the 21 fault windows. This is
single-host Kind evidence with actual Chaos Mesh effects, not independent-host
or power-loss qualification. Dedicated client-link/quorum qualification and
whole-Rust proof remain separate. No throughput or latency was measured here.

## Exact build and receipts

The default ThinLTO server SHA-256 is
`616eed1b823dfaa192a22b2b03a3e7d778bf71efdad7d2b875a49c4bfc94e476`.
The four-binary image is `kv9-chaos:write-crc-slicing8-e748620-full21-first`,
image ID `sha256:edf2bb8fd1c3e1b46b59e41ab91ef6e12dccb28b5868a0c5c2326929448f67de`.
Original source-map, verbose build/codegen, linkage, image probe and Kind/CRI
identity records are included. The production server has default features.

Actual runtime: session 95644, terminal `17104e`, exit 0. Independent audit,
archive and cleanup: session 82646, terminal `50ccf2`, exit 0; all six post-run
phases pass. [summary.json](summary.json) records counts and limits.
The earlier 47 distinct Lean theorem statements, 710 workspace tests/doctests
(23 existing ignored) and ordinary process recovery are separate qualifications;
these populations must not be added to the Chaos history count.

The bundle separately preserves 28 passing write A/B environment contracts:
8 driver, 15 auditor and 5 native-smoke schema controls (`c84db0`, exit 0).
Their rejected first draft and corrected helper-loop binding are retained.
These are preparation checks; no A/B workload has run or candidate speedup
been established. Original metadata-selector and auxiliary readback failures
also remain available, with their corrections and no runtime rerun.

## Portable evidence and scope

The archive contains 2,282 selected original files plus 14 publication
preparation records: **2,296 regular members, 441,886,011 decoded bytes**.
Eleven parts, each at most 2 MiB, total **22,883,666 compressed bytes**.
Concatenated SHA-256:
`4f714cf217403769af7e4a73cb25f1778aa74438b1b6e5da561d1e4b567f7751`.
[inventory.json](inventory.json) SHA-256:
`4145b5c96a635bc68c3da127b2d70c313c46b958afd4db8766fa417b8de26bf0`.

Root streamed every selected source against its pre-existing full SHA/size,
checked stable file identities and scanned credential patterns. No selected
bytes were silently redacted. Every published member and compressed part was
then independently read back. Packaging completed in session 15075, terminal
`7230ba`, exit 0; byte verification `da9f6a`, exit 0; concatenated compressed
hash/length verification `6f5f7d`, exit 0. [package.py](package.py) records the
finite selection and separate publication bounds: 128-MiB member ceiling,
512-MiB decoded/compressed ceilings and a 1-GiB added-artifact reservation
above the unchanged 96-GiB preflight floor.

Run from a checkout containing all parts:

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONOPTIMIZE=0 python3 docs/write-crc-chaos-v1/verify.py
```

This verifies byte identity without extraction or executing runtime helpers.
The full original histories, observer and admission-pressure records are
included. Forty-nine links are represented as literal JSON metadata; no
escaping symlinks are recreated. Source helper targets are separately bound.
The excluded ELF/WAL payloads, duplicate independent-copy tree and unrelated
host capacity censuses are not part of this reporting subset. Consequently,
this portable subset alone cannot replay every original full-file audit.

The retained full local archive remains
`/tmp/kv9-write-crc-slicing8-chaos-preparation-first/archive-first/evidence.tar.gz`:
93,158,941 bytes, SHA-256
`55061d073c41128a16644dd1724629d7889110e99de42c72be545dffd75d877d`.
Its original complete readback covers 4,177 files, 943,464,447 decoded file
bytes and 4,470 members including directories, inventory and links. Its
inventory and readback are included in the published subset; the archive
itself remains local. See the archived `publication-preparation/` selection,
exclusion and literal-link maps for exact membership.

The selected runtime remains `11113f6`. Matched write A/B, full point/batch and
mixed-read regression coverage are required before any candidate promotion.
All Raft, synchronization, apply and response fences remain mandatory. No
GitHub CI was dispatched and no original industrial checklist item closes.
