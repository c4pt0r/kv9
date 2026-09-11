# Read-group-credit local Chaos acceptance

Accepted exact clean revision `57ff6851e40ed63c837189d6eb0a11190704725a`. Image preparation, the single fixture run (session 56168), and unchanged full audit, leaf reader and final source verifier (session 60804) all exited 0. No rerun or predicate change was needed.

Full atomic history: **2079 calls**, **1,780 OK / 224 refused / 75 unknown**,4,159 JSONL lines including the header. All outcomes and both existing checker search attempts are retained: the zero-unknown-effect search was invalid, and the unchanged guided whole-effect search returned a valid witness. This is normal checker search, not a discarded fixture failure.

| Window | Contained BatchGet OK/refused/unknown | Contained BatchPut OK/refused/unknown |
| --- | ---: | ---: |
| baseline | 47/0/0 | 46/0/0 |
| vip-delay | 47/0/0 | 46/0/0 |
| delay-healed | 45/0/0 | 47/0/0 |
| vip-partial-loss | 67/0/3 | 64/0/6 |
| loss-healed | 48/0/0 | 49/0/0 |
| vip-partition | 0/0/13 | 0/0/13 |
| partition-healed | 51/0/0 | 49/0/0 |
| exact-tcp-reset | 52/0/0 | 52/0/0 |
| reset-healed | 51/0/0 | 52/0/0 |
| quorum-loss | 0/47/0 | 0/48/0 |
| quorum-healed | 53/0/0 | 53/0/0 |

All eleven original windows passed. Both negative windows contained zero successful operations of any kind: client partition completed 37 unknown calls; quorum loss completed 136 refused calls. Coarse phase labels also include transition activity; the negative claim uses the checked contained wall windows.

Actual Chaos Mesh Service-VIP delay, 30% loss/correlation0, client partition and three directed voter partitions retained active fault identities, current Service/EndpointSlice bindings, packet/socket effects and simultaneous healthy control paths. The exact native netem leaf `5:`/parent `1:4` advanced **0→153 drops** in the same attested namespace/process lifetime. The legacy parent+child sum is not reported as packet count. Reset uses the explicitly non-Chaos exact-tuple SOCK_DESTROY helper, with old/new sockets and unchanged native process.

After native exit 0, each replica supplied two serial fresh empty Serving/nonfatal observations with running registries. Public 64 requests/64 MiB, async-read 128 and async-apply 128 bounds stayed unchanged. Full-lifetime batch counters rose 629 inline/0 blocking; these include setup/verification and are not attributed to individual calls or fault windows.

Five owned Pod/container lifetimes exited; the native workload process was absent. Namespace `kv9-native-link-acceptance-57ff685-20260911-1` (UID `a634958a-3913-47c8-af27-7a9c18edc41c`) and both inode-bound owned node directories were removed. All eight historical namespace UIDs remained unchanged. The image preparation probe was also removed. The fixture retained 50 files /12,437,174 bytes, archived and checked before cleanup.

Source binding: 594 tracked files and all 80 frozen preparation inputs unchanged; all 93 predecessor frozen inputs read back. Runner/audit/contract/observers/vendor/leaf bytes are unchanged. Only source/build/image/owned paths and preparation probe identifiers changed. Server SHA256 `6d76779a3e421388692b01ce08bd289477283fc05fd28b734eaed9138e0f7e30`; release manifest `7e5179f856b8828c26a6bf68ce0056755e16e12bbdc35c9a11845178712caabd`; native client `071c51ca191875b442089c851d6c5f44d4a546b4fcc4aebc8c1be77dca6bf311`. Default local package features are empty in both standalone release build graphs. Docker config image `071961e4da06252551859005e44e6ea7728bd93fc5a72e8377613f53f9380a2e`; actual CRI manifest `b92cab4b03a17834a64095c266e2141a89e77409c7782387dbd2e6e9b99a711b`.

Raw: `/tmp/kv9-native-link-acceptance-57ff685-attempt1`. Exact commands/PIDs/logs, outcomes and terminal records are in this report directory. Evidence hashes:

- `audit-first.json`: `6f1aae6e28c9fb9e3a0b119fa31c660a8dd499b1e9839dda39fb21b378a2552e`
- `leaf-first.json`: `26877d49785df3a9c43146b166099f6f858c2acde352a23bb9ac5c5043aa4081`
- `source-final.json`: `c5d9b481a2e1fbe00d3cc259fddc311faaaeef275d42c85b3dcb1d528cb15fa1`
- `result.json`: `a0681f5b92bdbb377c0105ea06f80ded9c0b660b7a872b8c9939ad71e81913b7`
- `inventory.json`: `364443ee681f156b8b0f4b472772ca68cf2899a142048ed23fb5be239907ce41`

Scope: one shared Kind host with volatile node tmpfs, correctness-only. This does not establish throughput, cross-host behavior, disk/power-loss recovery, inter-voter partial-loss/storage-stall coverage, the full 21-window matrix, or full grouped-read/Ready/Rust formal refinement. No repository or historical artifact was modified.
