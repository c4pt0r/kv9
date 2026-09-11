# ReadCredit formal gate: independent terminal readback

Accepted within the single-invocation abstraction and conditional-admission scope. Root session 85784 and its retained exit marker report exit 0. This readback started no proof tool, build, fixture or runtime process.

All **37 records** match their retained result files: 18 TLC cases and 19 proof/audit cases. All **10 baseline/mutant/restored triples** have the exact one-file mutation and restoration; 240 copied-source bindings match. SANY inventories resolve the four owned modules locally, recognize only the two pinned standard modules, retain exactly the approved legal-input assumption, and enumerate 28 owned theorems. The intended proof-hole and custom-axiom cases were rejected before proof execution.

The fresh baseline proves **196 ReadAdmission + 69 ReadCredit = 265 obligations**. Across repeated controls, 34 separately retained proof executions comprise 30 successful executions and four intended semantic failures; they are repetitions, not additional unique theorems. Strict/no-fingerprint commands use distinct per-case/module cache paths. Intended failed obligation locations/counts and restored successful counts match. Both finite fingerprints agree: Credit2 has 832 states, Credit3 has 2,472 states, and the conditional-progress case has six states.

The fairness counterexamples are concrete:

- Missing release fairness: a real RACommit changes commit-term 0 to 1; the ready uncanceled request then stutters with occupied credit and zero submissions.
- Competing refill: after RACommit, RCRelease makes credit free, then RCOtherAdmission loops back to occupied. Release fairness holds, but admission is not continuously enabled, so weak poll fairness does not prevent starvation.

The revised wrapper adds this relevant current-term-commit prefix and fair commit. ReadCredit, ReadCreditProof and both validator hashes are identical across the three full attempts. This strengthens the retained trace's readiness coverage without changing the proof predicates or lowering the two-state threshold. The first attempt's initialization-evaluation failure and the second attempt's legitimate one-state stutter rejected by that unchanged threshold remain explicit failures; neither was relabeled.

The gate ran against uncommitted candidate contents. All 13 bound formal files now hash-match clean commit **57ff6851e40ed63c837189d6eb0a11190704725a**, parent accepted **5ee897a2f58c57bdf17ea1757c96224adf0f0dbb**. The separate committed-binding record verifies Git object bytes, current clean status and retained tool hashes. It does not claim the original invocation occurred after commit.

No new source-mapping blocker was found. Occupancy is a Boolean overapproximation with abstract release and immediate-singleton behavior; this is not an upstream queue-count conservation proof. Conditional admission assumes a stable, ready, uncanceled, non-expiring window without competing refill. Arbitrary competing traffic, end-to-end fairness/latency, grouped member/context freshness, Raft quorum/Ready/application composition and Rust refinement remain outside this acceptance.

Artifacts:

- `/tmp/kv9-read-group-credit-formal-independent-first/audit-first.json`: `ccd4302cddf3e4c6ad9da0c8fd71512d3d24ebd0602d0c681a5149191813952a`
- `/tmp/kv9-read-group-credit-formal-independent-first/inventory-first.json`: `16e4114361adafd7460e580b1bfc9ad18f70ba12a95dc9cefc3e20f45f48ab40`
- `/tmp/kv9-read-group-credit-formal-independent-first/committed-binding.json`: `dc452dd8ce7dc33fc73b663ef7a022c1d75c832934539d5ed232441eab0e13e9`
- `/tmp/kv9-read-group-credit-formal-independent-first/readback.py`: `8e980a2657154b69fe9df4f73e11d86212125c4b6a20f063c06080178f50f69d`
- `/tmp/kv9-read-group-credit-formal-independent-first/readback-first.log`: `b9260fb096eeb5edc4def5e8c877b582abefe7d5c92bf4c67707c17c026020cc`
- `/tmp/kv9-read-group-credit-formal-third/summary.json`: `81345fcb3d7da8a4ea0694dda18b2f5149a3af3a5fb1a247af8f014d51e522c2`
