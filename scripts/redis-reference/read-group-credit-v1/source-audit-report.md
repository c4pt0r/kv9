# Read-credit source gate readback

Accepted bounded readback for clean commit `57ff6851e40ed63c837189d6eb0a11190704725a`, parent `5ee897a2f58c57bdf17ea1757c96224adf0f0dbb`. All 148 copied build inputs match the gate snapshot and current source; the separate read-credit test file, retained runner and all 42 logs match their recorded hashes.

The full local Raft gate passed **218 tests/doctests**: 187 unit, 1 lock-order integration, 13 malformed-frame integration, 5 membership integration and 12 doctests. Warnings-denied all-target Raft Clippy finished successfully. The older 185-unit-test run remains separately retained with its different test-file/control-runner hashes.

All **14 controls / 42 phases** have the exact baseline/mutant/restored sequence **0/101/0**. Each phase compiled successfully and ran exactly its named test; every mutant failed at the intended semantic assertion, including the three new controls compiled against `driver/read_credit_tests.rs`. All seven tests in that separate file passed in the full gate. Expected mutation bytes, unchanged inline tests, final restored source and final executable/dependency bindings were independently checked.

Historical mutant executables share an overwritten output path. Their retained Cargo/source/log records and execution-time hashes are available; only the final restored executable can now be independently rehashed. Full-gate terminal codes are read from the retained root result/logs, not from a fresh execution.

Two local audit attempts remain unchanged: the first declaration parser omitted six Tokio async tests; the second fixed-base-HEAD check detected the concurrent source commit. The third changes only that parser and explicitly binds the clean child commit, preserving every source-hash check. No Cargo, tests, runtime or formal checks were rerun. Formal/release/performance/Chaos and full candidate acceptance are outside this audit.

Accepted report: `audit-third.json`, SHA-256 `eee9b687d3d8808d2786a31ad6ce3b66f9b6ae9d1bb10b649b7ff993c69bc1dc`. All three readback commands are terminal (1, 1, 0); no owned process remains live.
