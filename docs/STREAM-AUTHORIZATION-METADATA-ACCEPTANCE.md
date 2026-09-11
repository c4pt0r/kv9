# Stream authorization metadata: eleven-window Chaos acceptance

Exact `94d8b9fe1b59c6b441267de7dc4352207dd7079c` completed once: runtime session 59147 exit 0; unchanged independent audit session 90323 exit 0; mandatory leaf-only and retained-input checks exit 0.

All eleven windows passed. The complete atomic history has 2,071 calls / 4,142 events: 1,762 OK, 232 refused and 77 unknown. Every original outcome is retained. BatchGet has 727 calls / 5,810 positional items; BatchPut has 724 calls / 5,789 positional items, including setup/verification and repeated keys.

| Window | Seconds | BatchGet OK / refused / unknown | BatchPut OK / refused / unknown |
|---|---:|---:|---:|
| baseline | 16.438 | 47 / 0 / 0 | 46 / 0 / 0 |
| vip-delay | 24.570 | 47 / 0 / 0 | 45 / 0 / 0 |
| delay-healed | 16.750 | 46 / 0 / 0 | 46 / 0 / 0 |
| vip-partial-loss | 35.202 | 63 / 0 / 3 | 64 / 0 / 2 |
| loss-healed | 17.557 | 49 / 0 / 0 | 49 / 0 / 0 |
| vip-partition | 17.856 | 0 / 0 / 13 | 0 / 0 / 13 |
| partition-healed | 18.166 | 51 / 0 / 0 | 50 / 0 / 0 |
| exact-tcp-reset | 18.526 | 51 / 0 / 0 | 52 / 0 / 0 |
| reset-healed | 18.715 | 52 / 0 / 0 | 52 / 0 / 0 |
| quorum-loss | 19.073 | 0 / 48 / 0 | 0 / 47 / 0 |
| quorum-healed | 19.419 | 54 / 0 / 0 | 55 / 0 / 0 |

Same native Pod/container/netns lifetime and exact netem leaf `5:` parent `1:4` show 158 - 0 = **158 new drops**. Parent queue counters are not added. Original raw command paths, identities and hashes are retained in acceptance-leaf-readback.json.

Both negative windows had no newly successful fully contained operations and included completed batch refusals/unknowns. Phase-wide counts can include calls crossing fault boundaries; they must not replace contained-window counts. The original checker retained its first inconclusive search and subsequent valid full-history witness; no workload was rerun.

Fresh post-client empty drain passed from five serial exports. All five owned Pod/container lifetimes exited; host launcher 3586695 and all owned helpers are absent. Namespace UID `3e0a6c16-092d-471c-a05c-c9febe0d0c59` and both exact owned node directories are removed; all eight historical namespace UIDs are unchanged. 50 regular retained files / 12,396,634 bytes passed readback before removal.

Batch counters across the full fixture envelope are 618 inline completions and 4 blocking submissions. They include setup/verification and are not attribution to individual calls or fault windows.

This is exact-candidate correctness on one shared Kind host with volatile tmpfs. It is not disk/power-loss, cross-host, performance, inter-voter partial-loss, storage-stall or 21-window acceptance. Network faults use actual Chaos Mesh; SOCK_DESTROY is explicitly non-Chaos. Source and all 98 frozen preparation inputs remain unchanged.

Raw evidence: `/tmp/kv9-native-link-acceptance-94d8b9f-attempt1`. Exact result digests and full counts: `RESULT.json`.

## Implementation and local checks

The [source correspondence](https://github.com/c4pt0r/kv9/blob/94d8b9fe1b59c6b441267de7dc4352207dd7079c/docs/STREAM-AUTHORIZATION-METADATA.md) preserves metadata contents supplied to fresh per-frame authentication and the five directly dispatched Raw handlers. It explicitly excludes allocation/object-identity timing; this is not a new core algorithm or a whole-implementation machine proof.

Default/experimental workspace checks pass 711/721 tests, with 23 ignored each; configurations overlap. Workspace all-target experimental Clippy passes. Actual stream/unary leader-restart process histories pass 366 calls (335 OK, 31 unknown). The separately [screened performance](STREAM-AUTHORIZATION-METADATA-SCREENING.md) improves single GET by 4.425% / 3.641% against f2. No master/default promotion, combined jemalloc result, sustained/write/mixed capacity or whole-roadmap completion follows from this isolated acceptance.

Unknown writes in this fault run: 26 BatchPut, 8 PUT and 8 DELETE, all retained without blind replay.

- Runtime summary SHA-256: `fa5beb3192ada0da85ebb588aa2a8294c2e3e2da510c8ad5b5399821a39003ef`.
- Independent audit SHA-256: `76778086a3dcad2ac2e31358f33f061961f2ab850474c6886aac969e296cd42a`.
- Leaf-only audit SHA-256: `4266026cb3b4c05a56386690a64c42757e40a26d8cd60100fcd0955d658a74da`.
- Source/preparation readback SHA-256: `5d80bc10813e628942f233363dfc293b33429387bfd1a57bdab7db6a3cc7043a`.

All work was local. No hosted CI was dispatched.
