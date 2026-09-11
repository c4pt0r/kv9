# RPC event interval: eleven-window Chaos acceptance

Exact `917243fd1b501843d75899cc167f7eff32b5bbb2` completed once: runtime session 26114 exit 0; unchanged independent audit session 50600 exit 0; mandatory leaf-only and retained-input checks exit 0.

This candidate is the jemalloc 629 base with production Tokio event interval eight. It excludes authorization94 metadata reuse, the fixed global-queue experiment and profiling instrumentation.

All eleven windows passed. The complete atomic history has 2,106 calls / 4,212 events: 1,803 OK, 228 refused and 75 unknown. Every original outcome is retained. BatchGet has 739 calls / 5,906 positional items; BatchPut has 736 calls / 5,885 positional items, including setup/verification and repeated keys.

| Window | Seconds | BatchGet OK / refused / unknown | BatchPut OK / refused / unknown |
|---|---:|---:|---:|
| baseline | 16.230 | 47 / 0 / 0 | 46 / 0 / 0 |
| vip-delay | 24.640 | 47 / 0 / 0 | 46 / 0 / 0 |
| delay-healed | 16.940 | 46 / 0 / 0 | 46 / 0 / 0 |
| vip-partial-loss | 35.158 | 71 / 0 / 5 | 73 / 0 / 4 |
| loss-healed | 17.534 | 49 / 0 / 0 | 49 / 0 / 0 |
| vip-partition | 17.829 | 0 / 0 / 12 | 0 / 0 / 13 |
| partition-healed | 18.080 | 50 / 0 / 0 | 50 / 0 / 0 |
| exact-tcp-reset | 18.490 | 52 / 0 / 0 | 52 / 0 / 0 |
| reset-healed | 18.761 | 54 / 0 / 0 | 53 / 0 / 0 |
| quorum-loss | 19.110 | 0 / 47 / 0 | 0 / 48 / 0 |
| quorum-healed | 19.381 | 54 / 0 / 0 | 55 / 0 / 0 |

Same native Pod/container/netns lifetime and exact netem leaf `5:` parent `1:4` show 178 - 1 = **177 new drops**. Parent queue counters are not added. Raw commands, identities and hashes remain in acceptance-leaf-readback.json.

Both negative windows had no newly successful fully contained operations and included completed batch refusals/unknowns. Phase-wide counts may include calls crossing fault boundaries and must not replace contained-window counts. The original checker retains an invalid restricted zero-unknown-effect hypothesis and then a valid full-semantics witness including uncertain effects. No fixture was rerun; the accepted final history is valid.

The original fresh two-export post-client empty drain passed from 4 serial sample sets. All five owned Pod/container lifetimes exited; host launcher 755246 and all owned helpers are absent. Namespace UID `837f7994-93da-4d1d-a8b8-419be23a1927` and both exact owned node directories are removed; all eight historical namespace UIDs are unchanged. 50 regular retained files / 12,619,025 bytes passed readback before removal.

Batch status counters across the full fixture envelope are 628 inline completions and 3 blocking submissions. These include setup/verification and cannot be attributed to individual calls or fault windows.

This is exact-candidate correctness on one shared Kind host with volatile tmpfs. It is not disk/power-loss, cross-host, performance, inter-voter partial-loss, storage-stall or 21-window acceptance. Network faults use actual Chaos Mesh; SOCK_DESTROY is explicitly non-Chaos. All 583 source files and 99 frozen preparation inputs remain unchanged.

Raw evidence: `/tmp/kv9-native-link-acceptance-917243f-attempt1`. Exact result digests and full counts: `/tmp/kv9-rpc-event-chaos-acceptance-first/RESULT.json`.

## Local checks, safety boundary and selection

Default/experimental-RPC workspace runs pass 707/717 tests, with 23 existing
ignored tests per configuration; the configurations overlap. Workspace
all-target experimental Clippy passes with warnings denied. Actual production
runtime stream/unary leader-kill and original-directory restart histories pass
357 calls (325 OK, 32 unknown). The unknown writes in this Chaos campaign are
26 BatchPut, eight PUT and seven DELETE; they are retained without blind replay.

The [conditional safety correspondence](RPC-EVENT-INTERVAL-SAFETY.md) covers
only the scheduling delta from the unchanged jemalloc base. It does not complete
machine-checked composition of the whole implementation or change historical
proof source boundaries. ReadIndex, quorum, successful-pump and applied-index
requirements remain unchanged.

The [matched five-second comparison](RPC-EVENT-INTERVAL-PERFORMANCE.md) improves
GET throughput by 8.649% / 8.585%, with mean latency around 195 us and
unchanged/lower p99 buckets. The shorter screen's second-repeat p99 regression
is retained. Select this as the next isolated read-performance increment;
master/default promotion, sustained/write/mixed capacity, broader memory
behavior and Redis parity remain open.

All work was local. No hosted CI was dispatched. The original #9 checklist
remains unchanged; these results do not finish a broader roadmap package.

- `summary.json` SHA-256: `b993fa303e0a0f0f7540734658deedb72f4b7f98f8951a08aafb31baca5d7229`.
- `acceptance-audit.json` SHA-256: `5fd7d83d39cef989c4d4d9baf01824ed837e285d1df9c03804fb57cc7c84ef92`.
- `acceptance-leaf-readback.json` SHA-256: `7a41bca333dad939d459b73c27131742e62bdcd3fdccabe9931e0cdb8bef7200`.
- `acceptance-retained-inputs.json` SHA-256: `25154fb2fd0cf7709b53a3439727f0d4ba43370ab12d448b20649321947d82c6`.
- Closeout `RESULT.json` SHA-256: `2342f372a802390580b9b774678e7bf5130f9799e414d7daabcb8931364a0cb4`.
