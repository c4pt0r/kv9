# Isolated triomphe extraction qualification

See [the qualification report](../../docs/ISOLATED-OUTLINE-QUALIFICATION.md).
These helpers compile and check the original registry archery/rpds composition
with only the qualified triomphe shared-clone extraction. They do not measure
performance or change production dependencies.

The retained experiment root supplies checksum-verified registry source trees,
`candidate-triomphe`, `qualification-plan.json`, the original corpus and protocol,
plus the independent corpus decoder. The evidence packet contains these inputs
except the already published corpus. Absolute paths are retained deliberately;
restore those paths or create a fresh experiment with updated bindings.

`build.py ROOT` creates both isolated ordinary-release harness workspaces, checks
registry source correspondence and cache freshness, runs release Clippy, then
retains executables, symbols, disassembly and relocations. It does not execute
the binaries.

`prove.py ROOT LEAN` rechecks the qualified guard/extraction lemmas and proves
composition with the unchanged dynamic pointer guard. All source bindings and
rejected controls are retained; the proof's Rust/library/compiler premises are
explicit.

`test.py ROOT` runs the isolated archery, rpds and engine/common library fixtures.
The engine fixture must contain only the common/engine workspace members and
workspace settings, without the root binary package. In the retained first
attempt, that root package was accidentally copied; `test-engine.py ROOT` records
the corrected engine-only continuation without repeating passed dependencies.
`tests-accepted.json` binds the successful runs and preserves the failed metadata
record. Engine/common Rust sources must exactly match the repository.

`prepare.py ROOT` consumes the accepted build/proof/test records and runs both
release semantic preparations on CPU 4, checking every final key and query with
the separate Python decoder. Helper processes use `taskset -c 6-15,22-31`.
No timers are activated by preparation mode.

All output directories must be fresh. The shared Cargo target is reused under
its build lock. Bulk evidence belongs under `/mnt/data/kv9-work`; do not move
latency-sensitive fixtures or change global TMPDIR. The next performance and
allocation companion are prospective work, described in the published plan.
