# Receipt upper-bound source checkpoint evidence

Candidate `e2e23cca5e70a9ea0cc241877b3b35b5b6433d27` is an independent experiment based on `7f458bfba79a1ab654127bf1f63e76d2750abe79`. This package records source-level refinement and local development qualification. Main's selected write baseline remains unchanged; ordinary recovery, actual Chaos Mesh and matched performance acceptance are pending for this candidate.

`evidence.tar.gz` preserves **129 original small files / 732,336 decoded bytes**, including all three proof drafts, both failed proof drafts, the first failed TCP fixture run, its exact isolated recheck, the final source-bound proof run, subsequent local source checks, and the separate schema-2 synthetic observer checks. Original logs and source copies are unedited. Shared build artifacts and toolchain executables are not in this package; their original local paths and available identities remain in the records.

Archive size: **190,128 bytes**. SHA-256: `d9d85b462bd30f469af4c789598e6e65380d549a9d38e56617e08eb4f3aa9d38`. `inventory.json` lists every original path, archive member, size and SHA-256. Packaging read the complete archive and independently compared every member with its original input again. `summary.json` identifies passed checks and explicitly pending gates.

Actual terminal observations: final formal acceptance `536a88/0`; second development run `c93db6/0`; third development run `e080ec/0`; isolated TCP recheck `9d6d20/0`; new synthetic observer controls `cf491d/0`. The first instrumented Raft supervisor remains failed (`bda851/1`, Cargo exit 101), with 258 passed and one bind failure. An isolated recheck passing does not repair or retroactively pass that failed run. The original TCP fixture releases ephemeral port reservations before the listeners bind; no historical competing port owner was captured.

The proof checks 14 theorem statements / 82 obligations, exact legal-input assumptions and pinned imports, and both finite positives and wrong-result counterexamples. Three proof-output controls and two semantic rejection controls pass. `git-blob-readback.json` binds all 13 proof-contract file hashes to the experiment commit. Local default workspace totals are 792 passed / 23 ignored; the one scoped server diagnostic assertion, default and diagnostic Clippy, formatting and explicit production experimental-lease compilation pass. Earlier successful diagnostic Raft tests are reused without replay after only the server's diagnostic schema assertions change.

The schema-2 checker has 11 new synthetic controls; its 13 predecessor controls and original correction are retained without rerunning them. No live schema-2 capture has been accepted. Empty-ring skip events remain counted, but aggregate histories cannot disambiguate every empty-miss reclassification; the checker documents that limit.

[Experiment and remaining gates](../WRITE-RECEIPT-UPPER-BOUND.md).
