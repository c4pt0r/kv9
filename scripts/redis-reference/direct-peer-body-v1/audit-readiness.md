# Independent c1/c64 auditor readiness

Ready for root review; no matched auditor, fixture, smoke or timed workload was executed by this subtask. Only the auditor, its pure-function controls, this note and the retained control log were written. Root-owned driver, protocol, wrapper and arguments remain untouched.

The accepted auditor base is `/tmp/kv9-stream-worker-pair-comparison-preparation/audit.py`, SHA-256 `847832b40028b3c5359cc40ccc54e6e76a6394ca90aa2d048421168762fb73a0`. Its source/build, actual executable and lifetime, resource coverage/placement, complete native/Redis outcome, selected API, final dataset, serial fresh drain, retained tmpfs hash and outer restoration predicates remain in place.

The new independent protocol check requires version 2, `kv9-direct-peer-body-c1-c64-v1`, points [1,64], and exactly 24 descriptors. Repetition 0 is c1's six original arms followed by c64's six; repetition 1 reverses the entire 12-arm order. Run IDs are p{repeat}{concurrency:04d}. Requested configuration is checked independently, including native max_in_flight equal to concurrency, unchanged deadlines/retries/caps/payload/duration, and distinct point_get/batch_get versus GET/MGET APIs. The driver is not imported as truth. Smoke and obsolete 12-cohort timing inputs fail this auditor.

Config pairing includes (repeat, concurrency, arm); old/new latency/QPS and RSS comparisons remain within one concurrency and repetition. All-voter RSS sums supplement per-voter records without assuming leader-role correspondence. HWM remains lifetime scope and a sum of individual high waters need not be a simultaneous peak. Latency values stay in nanoseconds. Expected totals are 80 owned lifetimes, 48 fresh-drain documents and 48 voter/listener bindings. CLI retains `--successful-arms`, now restricted to 24; this denotes completed accepted fixture arms, not silently reclassifying individual failed calls as successes. Original complete outcome/attempt accounting remains unchanged.

Candidate revision `6707bcccf15ea788ff231f4263f43a9f73fa63dd` is pinned to server `1c4ceb3b95ecca2ad900f65641c73ad097a13afbeba5304b8b8ac63358ff3a63` and manifest `029b7653ff227d4f4e15ef1de1236da2b6a824ca7e3cf0ab27ee3ac54640ee9b`. The latter two files were independently read back after root reported release completion. Control5ee, native03 and Redisb8 pins remain unchanged. Exact role paths and both server manifest hashes are checked before cohort acceptance; no rejected base server or pending-pin fallback is used.

After root confirmed build/process termination, the first offline contract run passed 10 tests, exit 0, with assertions enabled and bytecode disabled on CPUs 6-15,22-31. Controls cover the full reversal, duplicate/missing concurrency, smoke/protocol/bool substitutions, API relabeling, c1 max_in_flight incorrectly remaining64, changed limits, exact native/Redis configs, and cross-concurrency/missing/duplicate pairing. Controls extract only named pure functions/constants by AST and never execute the auditor's top-level artifact inspection. The original unexecuted construction attempt stopped before writing audit.py: its replacement guard reported `AssertionError: ('records = {}', 2, 1)` because the substring also matched allocator_records. Anchoring the declaration to complete lines resolved construction; no failed workload/audit was relabeled.

Frozen files:

- audit.py: `2c182758cdcb0668d133e9efc1adc06d3b610048917d23832607294543739d62`
- test_audit_contract.py: `5090400ddb9759795d415ae4b6e7da31a5518d39f9d79b518038c8996445452a`
- audit-contract-first.log: `1a1460eca77c2d864963cdbdd5555325f8ecf2d97ae7e7aca629ea3a3c8288ef`

The complete base-to-new auditor diff was read after the test run. Root's driver ordering/protocol fields were read for compatibility, independently of the control expectations. Actual smoke, timing and eventual one-use terminal audit remain root-controlled gates.
