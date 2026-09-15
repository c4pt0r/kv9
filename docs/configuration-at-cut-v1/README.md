# Configuration-at-cut evidence v1

Actual local component validation on 2026-09-15, based on `a9652ca` plus the
source pinned in this packet. See [the implementation and scope](../CONFIGURATION-AT-CUT.md).

- [evidence.tar.gz](evidence.tar.gz): 483 regular members, 7,001,209 decoded file
  bytes, 1,489,801 gzip bytes.
- SHA-256: `9ee357bb002cf707cbc18c09b57ea620b385ab9e930ed13962122f526a3d86c4`.
- [inventory.json](inventory.json): every member's size, SHA-256 and original
  retained path. After closing the archive, every member was independently
  compared byte-for-byte with its original. Paths under `/tmp` identify the
  original runs; extraction does not require those paths.

The packet includes source, strict proof/model logs, semantic audits, actual
counterexamples, complete source-control copies and logs, Cargo results,
development failures, and the pre-update #9/#14 bodies. It excludes Cargo
targets, executables, libraries, proof caches and TLC state directories.

## Accepted final runs

| Evidence | Retained run directory suffix | Actual outcome |
| --- | --- | --- |
| Rust tests, lint, format, default library build | `development-20260915-fourth` | 257 Raft library tests, none ignored; all four commands exit 0 |
| Cargo dependency preparation | `control-build-20260915-first` | Testing-feature library build exits 0; Cargo JSON identifies actual dependency artifacts |
| Compiled source controls | `source-controls-20260915-third` | Unchanged source passes seven selected tests; five separately compiled faults each fail the exact selected test with exit 101 |
| Full formal runner | `protocol-20260915-third` | 11 theorems / 95 obligations; three completed positive models; eight actual counterexamples; three intended proof rejections |
| Trace-verdict controls | `publication/trace-controls.json` | Actual one-state temporal stutter accepted only with explicit opt-in; default threshold, missing cycle and missing state still reject |

Run directories in the archive start with `runs/kv9-c04-configuration-`.
The final formal run uses fresh strict/no-fingerprint TLAPS and pins the
toolchain, parsed assumptions and source inventory. Its positive TLC counts
are 5,277/772, 32,005/3,416 and 1,936/176 generated/distinct states, with no
remaining queued states. The temporal counterexample also completes with
1,936/176 states and shows an actual stuttering cycle. Safety counterexamples
stop at the intended invariant failure and may retain queued states.

The seven source-control baseline tests overlap the 257-test suite. The 20
write/sync/short-write EIO/ENOSPC crash cuts are cases within one test, not 20
extra tests. Storage fixtures cover full initial/joint/stable configuration
payloads; they are not independently executed distributed membership changes.

Original unsuccessful development attempts remain unsuccessful. Their logs and
copied inputs are retained where emitted; `publication/development-failures.json`
identifies failures, including proof elaboration/obligation failures, two
inapplicable source wrappers, the first runner's escaped mutation and the second
runner's trace-shape rejection. The final gate does not retroactively pass them.

No new benchmark, hosted CI or Chaos Mesh run is claimed. This component still
needs integration with complete anchors, durable owners and actual distributed
fault acceptance. It neither closes C04 nor promotes a write candidate.
