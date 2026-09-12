# ThinLTO actual Chaos Mesh reporting evidence

Candidate `02d0c01024b65a84b220c6948ff2224bfa7900bc` passes the original
21-window voter/storage/endpoint matrix after a concrete delay-observer selector
repair. The failed first runtime remains failed. Both attempts use the same
server, clients and image. No timeout, consistency or history predicate changes.

| Complete accepted history | Operations | OK | Refused | Unknown |
| --- | ---: | ---: | ---: | ---: |
| CLI RawKV/catalog | 5,778 | 5,286 | 14 | 478 |
| Persistent point | 1,489 | 1,468 | 8 | 13 |
| Native point/batch | 2,656 | 2,617 | 13 | 26 |
| Total | 9,923 | 9,371 | 35 | 517 |

Corrected runtime 42651/0 (`b399b7`), independent audit 32590/0 (`bcb717`),
archive 71600/0 (`6a2f39`), cleanup 82779/0 (`e19242`), and all-lifetime
readback `55e9eb/0`. Four final replicas drain; 33 observed server lifetimes
and 25 containers exit or are removed. Original eight namespace identities are
preserved. Full histories include unknown/refused outcomes, positive fault
effects and native atomic operations inside all required windows.

The selected reporting bundle contains **6,489 exact original files,
819,666,702 decoded bytes, 34,436,076 compressed bytes in 17 parts**.
It includes the failed and accepted preparation/runtime/audit/cleanup reports,
the exact selector repair and affected controls, and the separate new-run
compressed-retention preparation. The latter passes 53 helper controls but has
not run its deferred large synthetic qualification or a performance campaign.
Neither this bundle nor its verifier supplies new QPS/latency measurements.

Run `python3 verify.py` to verify every part and selected member's size/hash.
This verifies integrity only; it neither reruns a database nor substitutes for
the independently executed history/fault checks. Archive paths are namespaced
reporting paths, while inventory entries retain each original source location.
Absolute helper paths describe the original local environment.

The explicit omitted-input list retains size/hash or link-target descriptions.
It excludes executable payloads, bulky observations, symlinks and the original
compressed archives. Those remain local under their original inventories.
Complete PVC/device images were not collected. This is a compact reporting
selection, not a complete independently replayable machine or backup image.

The accepted original archive has 4,467 members / 942,647,571 input bytes and
SHA-256 `916b3148468a31f84ce0dfda826256077f64c8be89d2f3605b62237078ae94a5`.
The failed original archive has 1,504 members / 209,044,901 input bytes and
SHA-256 `c63d0a46f3933908792dc452be6d2c0ee34ed1cfed23fb84d3011bb3f7e1d06e`.
Both received full member readback and input rehash before their owned cleanup.

Preserved failures include the original 20-second delay failure, early helper
setup/role/readback failures, and root's premature lifetime-readback invocation
`aac352/1`: cleanup was still running, so the helper refused at its first missing
summary precondition. The unchanged readback later ran after cleanup reached
terminal. Reporting preparation `f16c01/1` refused a root-owned 0600 metadata
file before writing an archive or member; the same selection completed with
privileged reads, requiring the destination to remain empty. No original file
permissions or input evidence were changed to bypass that refusal.

All checks run locally. Broader point/batch performance, dedicated link/FSYNC
fault acceptance, whole-implementation proofs, independent-host availability,
bounded storage and Redis read parity remain separate open gates. CRC remains
selected; no hosted workflow was dispatched.
