# Portable requalified performance metadata

This preparation exports the final report and acceptance metadata for the eight-smoke/sixteen-ten-second upper-bound-versus-CRC screen using the single requalified `1b8060eb` client. It does not run a benchmark, auditor, report generator, source test, WAL decoder, or cleanup. Packaging remains pending until all actual required terminals and final report outputs exist.

The selection includes every file bound by the frozen campaign and reporting-helper inventories, preserving original derivations, source/client qualification logs and failures. It adds the completed execution manifest's finite execution file list and named stage terminals; all five final audit JSONs; both complete matrices; timing isolation/restoration metadata; fresh role and smoke readback; retained role build manifests; every original input consumed by the final sixteen-cohort reporter; and all five report outputs. The preparation includes the old publisher/verifier source and exact deltas. No smoke rate is reinterpreted as performance.

Raw or compressed WAL objects, ELFs, source checkouts, credentials, issue9 snapshots and `continuation.json` are excluded. The original audit inventories retain the byte identities of excluded payloads, but this packet is **report/acceptance metadata, not a standalone complete WAL replay**. No original file is modified. Absolute source paths in `members.json` describe historical execution provenance, not a portable runnable installation.

## Exact retained bounds

- At most 8 MiB per original or manifest member.
- At most 80 MiB in selected original member bytes, and fewer than 512 original files (explicit reporting-only revision from 64 MiB).
- At most 32 MiB compressed and 80 MiB for the fully decoded tar stream, including headers/padding.
- Available space at least 8 GiB, helper CPUs 6–15 and 22–31, and a 1,200-second phase deadline.

The member/decoded/compressed limits are unchanged from `docs/write-receipt-tail-performance-v3`. Only the selected-original limit changed from 64 to 80 MiB under the root review recorded in `reporting-cap-revision.json`. Exceeding a bound preserves the failed result and reports the required increase; the exporter must not drop original report inputs or silently widen a limit. Gzip level 6 and USTAR tar are unchanged. The writer checks the compressed cap during encoding. The standalone verifier reads the complete gzip stream through EOF, checks its externally supplied SHA, checks every manifest/member SHA and length, and requires valid zero-block tar termination. It does not extract files or replay semantic acceptance.

## Root execution manifest

Supply one completed JSON manifest under `/mnt/data/kv9-work/upper-bound-requalified-execution-20260915-first`, plus its exact SHA-256 to `package.py`. `execution-manifest.template.json` is deliberately incomplete and cannot authorize packaging.

The required top-level fields are `schema: 1`, `complete: true`, exact `source_revision`, exact `client_binary_sha256`, `stage_terminals`, `files`, `audit_artifacts`, and `report_outputs`. Every pin is `{path: absolute_path, bytes: exact_integer, sha256: exact_digest}`. `files` is a unique finite list, at most 128 entries, of actual execution metadata in that execution root. Include original launch/invocation/child/tool terminal and acceptance records and any execution/preflight failures; omit issue9 files and `continuation.json`.

`stage_terminals` must contain exactly `smoke`, `smoke_readback`, `timing`, `audit`, and `report`, each pointing to an actual terminal in the execution root. General stage terminals may use either the original tool shape `{session_id: positive_integer, terminal: {chunk_id: actual_receipt, exit_code: 0}}`, or the normalized shape `{complete: true, session_id: positive_integer_or_null, terminal_receipt: actual_receipt, exit_code: 0}`. Pending or failed terminals are refused. The timing session must equal the final auditor's accepted timing session.

The audit terminal additionally must have the existing reporter-compatible `audit_path`, `audit_sha256`, `input_inventory_path`, and `input_inventory_sha256`, exactly binding the final audit files. The report terminal additionally must have `result_path` and `result_sha256`, exactly binding the final `results-first/summary.json`. A normalized direct audit terminal must also preserve the actual direct-tool fields required by the report helper. These are actual receipt bindings, not substituted successful counts.

`audit_artifacts` must contain exactly `audit.json`, `input-inventory.json`, `retention-decoder-receipts.json`, `logical-original-inventory.json`, and `combined-physical-retention.json`, at the frozen campaign's `results-first/` paths. `report_outputs` must contain exactly `summary.json`, `input-hashes.json`, `README.md`, `PER-REPEAT.md`, and `COMPARISONS.md`, at the frozen reporter's `results-first/` paths.

`ROOT-COMMANDS.json` gives the exact package and verifier argv. Root replaces only the explicit actual manifest/stream/member hashes. Both commands require fresh outputs and preserve any failed attempt. The packet's `package-result.json` returns the actual archive and manifest hashes for the independent verifier. `verification.json` is produced separately after that verifier actually exits; neither receipt is invented by preparation.

## Observed projected cap exceedance

Metadata inspection `19ee93/0` found 77 mandatory final reporter inputs totaling 70,480,177 bytes, already 3,371,313 bytes above the inherited 64 MiB original-member cap. The known external selection projection `632a7d/0` is 333 files / 82,218,113 bytes; its largest member is 4,761,180 bytes. Exporter preparation, final execution manifest/normalized receipts and tar inventory/header overhead remain additional. This is a projected exceedance, not an executed exporter refusal. No packaging or compression ran. Root will choose any separately recorded reporting-only cap revision after the complete finite selection and tar envelope are known; workload/storage/codec guards are unaffected.

Root explicitly approved a selected-original reporting-only cap of 80 MiB, contingent on the exact projection fitting the original 80 MiB decoded tar, 8 MiB member, 32 MiB compressed and fewer-than-512-original bounds. `project.py` reads existing metadata authorities and file lengths, without reading native reports/resources or using a codec, and computes the exact manifest digest and USTAR padding. Actual packaging requires that reviewed digest through `--expected-members-sha256`; changed selection or inputs fail before compression. Runtime campaign, storage and codec limits remain untouched. The original 64 MiB exporter/verifier draft is preserved in `origin/prepared-64MiB-*`.
