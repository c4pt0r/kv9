# Independent v3 measurement-client smoke readback

Accepted the first runtime (session 19187, exit 0) and first independent audit (PID 1441303, exit 0). All 667,384 measured calls and 670,684 total calls across setup, warmup, measurement and verification succeeded with one attempt each. Other outcome/reason populations are zero.

| Case | Backend | Measured read calls | Measured write calls | Input items | Stop | Changed keys |
|---|---|---:|---:|---:|---|---:|
| point-r000 | native | 0 | 36 | 36 | duration | 49 |
| point-r000 | redis | 0 | 100,000 | 100,000 | operation_limit | 128 |
| point-r050 | native | 33 | 20 | 53 | duration | 29 |
| point-r050 | redis | 50,057 | 49,943 | 100,000 | operation_limit | 128 |
| point-r100 | native | 34,967 | 0 | 34,967 | duration | 0 |
| point-r100 | redis | 100,000 | 0 | 100,000 | operation_limit | 0 |
| batch-r000 | native | 0 | 32 | 128 | duration | 111 |
| batch-r000 | redis | 0 | 100,000 | 400,000 | operation_limit | 128 |
| batch-r050 | native | 33 | 23 | 224 | duration | 91 |
| batch-r050 | redis | 50,057 | 49,943 | 400,000 | operation_limit | 128 |
| batch-r100 | native | 32,240 | 0 | 128,960 | duration | 0 |
| batch-r100 | redis | 100,000 | 0 | 400,000 | operation_limit | 0 |

All six Redis cases honestly report `operation_limit` at 100,000 calls and are ineligible for timing. Native cases report duration completion. The enclosing fixture and this review make no performance claim.

Revalidated exact version-3 selectors and paired point GET/PUT ↔ GET/SET at batch size 1, and BatchGet/BatchPut ↔ MGET/MSET at batch size 4. All cases use four workers, 128 keys plus the sentinel, 128-byte values, seed 71, 32 warmup calls, 500 ms, a 100,000-call cap and 1,500 ms deadline. Warmup/measurement API labels and sizes match selected APIs; initialization/verification retain batch APIs. Wire-path identity is source/build-bound; this run does not retain a per-request packet trace.

Clients are clean revision `0be806d9671e2c50701a64aa7889c8859b7648ba`, both with 581 matched source files and empty Cargo feature arrays. Native binary is `8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957`; Redis client binary is `5a8ac274b8cc6548a072f5305b04936a08cc4d1ba84d50150f675bab563049af`. The separate native server remains accepted `5ee897a2f58c57bdf17ea1757c96224adf0f0dbb`, binary `0d5ffa081482945d213b88aef12222afab44a44ed03bf46cf27bd292db7b1711`.

Readback accepted 21 exited service/workload lifetimes, all source/executable/PID/start/boot/listener bindings and observed per-thread masks, 18 fresh two-export drain stages (54 voter bindings), and 313 retained files / 30,513,735 bytes. Native voters were killed only after final empty drains and data capture by the inherited cleanup helper; this does not demonstrate graceful server shutdown. No original runtime failure or cleanup error was retained.

Each final dataset contains 129 exact deterministic values with an unchanged sentinel. Write cases retain valid configured-budget nonces and deterministic write/key membership; read-only cases remain nonce zero. The original prelaunch draft assumed measurement nonces were contiguous. That claim was corrected before runtime and preserved separately: these aggregate reports do not establish the exact issued nonce set or concurrent operation order. No whole-history, linearizability, Chaos, crash durability or performance acceptance is claimed.

Audit SHA-256: `e718e4376fb8147f553a6928fd59aa2f9d6d406f2f5dc5724079a746b302b4a7`.
Readout SHA-256: `51e7f4c532cbaba186e20c9e18a8fbfe0cc909c01ff16b214515fce04e819d67`.
