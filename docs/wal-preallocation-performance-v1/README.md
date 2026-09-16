# Same-source WAL preallocation write results

Completed locally on 2026-09-16 UTC. Keep the feature default-off: loaded Put
improves 0.545% and loaded BatchPut(64) improves 1.474% pooled, but batch gains
change direction between orders and pooled p99 stays in the same bucket.
See [the report](../WAL-PREALLOCATION-PERFORMANCE.md) for the decision and scope.

- `ANALYSIS.md`, `PER-REPEAT.md`, `COMPARISONS.md` and `summary.json` retain
  weighted rates, integer histogram bounds, complete outcome counts and CPU.
- `evidence.tar.gz` contains 155 original files plus its inventory: 91,923,519
  original bytes, all 78 reporter inputs, all sixteen timed reports and all
  eight smoke reports, source/build bindings and actual terminal records.
- `members.json` records every original path, size and SHA-256. Paths document
  provenance; the archive is readable without extracting to those paths.
- `verification.json` confirms full gzip EOF, tar termination and every member
  hash. Large WAL payloads and executables remain local; this packet does not
  provide standalone WAL replay or independently rerun consistency tests.
- `AUDIT-IO.md` diagnoses the post-timing verification delay. It does not modify
  measured work or this completed audit.

Archive: **3,764,999 bytes**, SHA-256
`786d61d4d0e5d1b16f01244de8c63316cff2fbd34cc4fe039aae25d8e79f9708`.
The first metadata selection exceeded its inherited 80-MiB aggregate cap.
`publication-cap-revision.json` records the bounded increase to 96 MiB to keep
all inputs. The original failed exporter and verifier are retained in the
archive. Runtime and audit predicates and limits did not change, and no
workload or audit was repeated.

Readback on a filesystem with at least 8 GiB available:

```sh
python3 -B docs/wal-preallocation-performance-v1/verify.py \
  --members-sha256 b17062169d7c7f289175c47953ce7b1c88a74b02d04dd51e4c975c6515806f5c \
  --archive-sha256 786d61d4d0e5d1b16f01244de8c63316cff2fbd34cc4fe039aae25d8e79f9708 \
  --output /absolute/fresh/verification.json
```
