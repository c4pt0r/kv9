# Upper-bound receipt lookup: full21 Chaos correctness evidence

The default-feature candidate `e2e23cca5e70a9ea0cc241877b3b35b5b6433d27`
completed the original full21 Chaos campaign and all six post phases.
Runtime terminal: **94911 / dafc9c / exit 0**. Post terminal:
**90186 / 11ea7e / exit 0**. The retained server SHA256 is
`6f074e867eae45274c17b54aa763888e92db2c45d5757974fa52ee0f15936d49`.

The three complete original histories are included without filtering:

| History | Invocations = returns | OK | Unknown | Refused |
|---|---:|---:|---:|---:|
| CLI/catalog | 5,550 | 4,964 | 565 | 21 |
| Persistent point client | 1,475 | 1,448 | 18 | 9 |
| Native batch client | 2,614 | 2,570 | 31 | 13 |
| Total | **9,639** | **8,982** | **614** | **43** |

`prepare.py` independently recounts every invocation/return pair, outcome and
contiguous event sequence from these exact included histories. The original
accepted history/effect checks, window records and their command/identity
evidence remain selected. This recount does not rerun those semantic checks.
The original post acceptance also records four fresh final drained replicas,
36 exited server lifetimes, 25 exited container states, removal of the exact
owned namespace, and eight historical namespace UIDs unchanged.

The original audit's `scope` prose still says `a6ac335`; it is retained verbatim.
Its machine `revision` and `runtime_revision`, source tree, server/client hashes
and image identity bind **e2e23cca**. Some inherited descriptive process labels
also remain historical labels. They do not replace those machine identities.
The audit ran before cleanup and therefore reports its original pre-cleanup
scope; separate successful cleanup and lifetime records complete that chain.

## Included original authority

The packet includes the original runtime/post terminals, independent audit and
individual check outputs, complete histories, accepted window effects and
identity records, fresh drain observations, cleanup and lifetime results,
default/native release and ordinary recovery readbacks/terminals, auxiliary
builds, image probe/load, finalizer and prebuilt receipts. All original current
preparation metadata failures remain separate: the initial authority-shape and
missing-audit-receipt observations, and the finalizer's cold-status-schema and
overbroad background-process-scan failures. Their corrected receipts do not
erase or relabel those attempts. The diagnostic-only release is explicitly
separate; the Chaos server uses the default production feature graph.

`selection.json` maps exact original paths to portable archive names with full
SHA256 and byte length. `omissions.json` records every excluded original
archive file and literal symlink metadata. No symlink is followed or recreated.
The original accepted full archive remains local:

- Path: `/tmp/kv9-upper-bound-chaos-preparation-20260915-first/archive-first/evidence.tar.gz`
- Compressed bytes: **92,332,752**
- SHA256: `9c069050bbaf3e88f8859151dac1ba5f88fc618cdaea49aa1a73ac21dc9d5ab1`
- Members: **4,552**; regular-file bytes: **923,659,003**.

Its exact inventory and completed full-member/input readback receipt are
included. ELFs/WAL payloads, the large observer stream, intermediate native
history-prefix capture bodies and the independent duplicate tree remain bound
to that full original authority. Prefix captures are not claimed identical to
the final history. None of those originals was removed or rewritten.

## Portable byte verification and limits

The selected originals occupy about 44 MB before packet metadata. `package.py`
streams them through gzip level 6 into numbered 2 MiB parts, verifying complete
input hashes and stable identities. Limits are 16 MiB per member, 64 MiB decoded
selected files, 32 MiB compressed, 64 MiB total new preparation/package disk
allocation, an 8 GiB available-space floor and a 1200-second deadline. This
separate evidence-packaging budget changes no workload or benchmark guard.

From the published package directory, run its standalone verifier with a fresh
output path:

```sh
python3 -B verify.py --output /tmp/upper-bound-chaos-portable-readback.json
```

It independently checks all parts, the complete compressed SHA/length, exact
member membership and SHA/length, bounds and the three complete history counts.
It extracts nothing and launches no services, database tests or external codec.
`verify-result.json` and actual tool receipts are retained separately after the
readback finishes. Original absolute paths are provenance locations, not a
portable runnable Chaos installation.

This is correctness acceptance for the original 21-window local environment.
Dedicated client-link/quorum, matched performance, physical power-loss,
cross-host behavior and whole-program proof remain separate claims.
