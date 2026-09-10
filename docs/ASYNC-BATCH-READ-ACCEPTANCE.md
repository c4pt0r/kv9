# Async native batch-read checkpoint

This checkpoint validates the scheduling change at
`af4c4e31bdef2b1294931c27e9802bc04e6aeaf5` on
`codex/async-native-batch-read`. Native BatchGet now uses the existing async
quorum/apply barrier and bounded resident copying. Oversized results move the
same authorized snapshot to a blocking job. The implementation contract and
conditional refinement argument are in that revision's
[ASYNC-BATCH-READ.md](https://github.com/c4pt0r/kv9/blob/af4c4e31bdef2b1294931c27e9802bc04e6aeaf5/docs/ASYNC-BATCH-READ.md).

This is a correctness checkpoint, not a performance result or main-branch
promotion. The [previous batch baseline](NATIVE-BATCH-PERFORMANCE.md) remains the
latest accepted measurement. Directly matched point GET versus batch-size-one
controls and candidate-specific Chaos Mesh execution remain pending here.

## Local verification

| Gate | Result | Retained output |
|---|---|---|
| Default workspace | 704 passed, 0 failed, 23 ignored | `/tmp/kv9-async-native-batch-read-budget-workspace-first.log` |
| RPC-experiment workspace | 714 passed, 0 failed, 23 ignored | `/tmp/kv9-async-native-batch-read-budget-experimental-first.log` |
| Default all-target Clippy | Passed with warnings denied | `/tmp/kv9-async-native-batch-read-budget-clippy-first.log` |
| RPC-experiment all-target Clippy | Passed with warnings denied | `/tmp/kv9-async-native-batch-read-budget-experimental-clippy-first.log` |
| Compiled implementation controls | Seven controls, 21 accepted phases | `/tmp/kv9-async-native-batch-read-controls-first/manifest.json` |
| Default standalone release build | Clean source, server and native workload retained | `/tmp/kv9-async-native-batch-read-release-first/build.json` |
| Native process E2E | Streaming and explicit unary histories accepted | `/tmp/kv9-async-native-batch-read-process-e2e-first/summary.json` |

Ignored tests were not executed by the workspace commands. The earlier initial
workspace attempt failed a status-line count assertion when batch counters were
added; the assertion and saturating-counter coverage were updated before the
accepted runs. All checks ran locally; no hosted CI was dispatched.

The seven compiled controls independently replace one production behavior:
waiting for the index writer, dispatching a completed point read, discarding
point-read contention, skipping the point context gate, bypassing the batch
value-copy budget, replacing the captured batch snapshot, and skipping the batch
context gate. Every baseline and restored run passed its exact selected test;
every mutant compiled and exited 101 at the intended semantic assertion.
The runner retains all copied build inputs, executable hashes, logs and source
restoration checks. Its manifest SHA-256 is
`7e8185a408a323c0d9a8a5c2d54a67ef597b3de9d556c45c23537f21f8d1bf9b`.

## Release process histories

The native workload mixes atomic BatchGet/BatchPut with overlapping point
get/put/delete on three independent WAL voter processes. Each transport crosses
leader termination, majority re-election and restart using the original data
directory, then requires progress and a fresh drain.

| Transport | Checked operations | Successful | Unknown | History verdict |
|---|---:|---:|---:|---|
| Normal default streaming | 185 | 170 | 15 | Valid |
| Explicit unary | 184 | 167 | 17 | Valid |
| Total | 369 | 337 | 32 | Both valid |

Unknown operations remain in the complete histories; they are not successful
acknowledgements or automatically replayed writes. The history checker admits
their possible effects when finding the atomic history witness. All five voter
lifetimes and both owned workload processes exited. Cleanup reported no errors;
all bound artifacts and executed helpers retained their recorded hashes.

| Artifact | SHA-256 |
|---|---|
| Release build manifest | `704a12848b7459b8f22cf32d084e25adeea329199daf933cd24cb5dbe6cd495f` |
| Standalone server | `8217d3520ee719c548754296518fabd8e20d82aa5fb78a8ada2a1e526c98a758` |
| Native workload | `0b5116b72e6962a6d881269219d4451300aab7253f859de4ee11caddcd1a11f0` |
| Process E2E summary | `59ab74a627eff037dea88c590cb401e0030a171ab4918716189e9b5fa1ba2e9b` |

This run uses a shared local host and process termination. It is not actual
Chaos Mesh, cross-host, power-loss, sustained-load, or Redis-parity evidence.
The earlier native Chaos acceptance and TLA+/TLAPS results retain their original
source/model scopes. The roadmap's broader consistency, fault coverage and
performance items remain open.
