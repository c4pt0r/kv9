# ThinLTO main integration

Main selects the qualified ThinLTO runtime in commit
[`11113f68f6a5df77da1ffb4fcec850953716ffa3`](https://github.com/c4pt0r/kv9/commit/11113f68f6a5df77da1ffb4fcec850953716ffa3).
Fresh local source checks, a separately retained default production build and
ordinary streaming/unary leader-loss recovery pass. The new server and recovery
client reproduce the qualified candidate's executable bytes exactly.

The selection retains the [full72 comparison](RELEASE-THIN-LTO-FULL72.md):
all 12 workload cells improve throughput and mean, including c64 GET +8.465%,
but loaded BatchPut(64) pooled p99 increases from 8.258–8.323 ms to
8.389–8.520 ms. This is an explicit throughput/tail tradeoff. Redis read parity
remains open. These figures retain their original experiment identity;
integration does not create another QPS measurement.

## Source and correctness boundary

The integration starts from main `c6e486e` and copies exactly seven paths from
qualified `02d0c01024b65a84b220c6948ff2224bfa7900bc`: the release profile and
the six source/manifest paths for the default-off read-stage observer from
parent `40f014f`. All **148 runtime/build-boundary paths** match `02d0c01`,
including added/deleted paths. Cargo.lock, toolchain/compiler settings and the
three retained-build/cache helpers also match. Main's newer documents, tools
and proofs are preserved.

Release uses ThinLTO and one codegen unit. No dependency, default feature,
transport, owner scheduling, ReadIndex grouping, quorum, persistence or
acknowledgement policy changes. The optional observer is removed from the
default build; its fields and metrics do not participate in correctness.
Keeping its exact qualified source avoids creating an unmeasured profile-only
combination against the older CRC source.

Existing proofs and their implementation gaps retain their original scope.
Fresh Safe ReadIndex, sealed groups, successful whole-pump completion and exact
apply/view fences remain unchanged. Source correspondence and identical
executables support reuse of the original qualification under the same failure
model; compiler correctness remains an assumption. This is not a new protocol
proof or complete Rust refinement.

## Fresh local checks and build

| Check | Result |
| --- | --- |
| Full workspace release tests/doctests | 709 passed, 23 existing ignored |
| Explicit `kv9-raft/read-stage-timing` Raft/server release tests | 438 passed, 1 existing ignored |
| Formatting | Passed |
| Full workspace/all-target release Clippy, warnings denied | Passed |
| Explicit observer-feature/all-target release Clippy, warnings denied | Passed |

The overlapping test counts are not added. Source-check session `97355/0`
(`858cb9`) retains every command and before/after snapshot. The explicit
observer-feature Clippy closes the historical command-coverage gap; the older
stage named `diagnostic-clippy` actually selected `kv9-raft/testing`.

The subsequent production build uses a separate build-cache lock transaction,
first-party invalidation and the unchanged retained-build helper. All 11
first-observed first-party units rebuild; all 20 production artifact observations
have empty feature sets. All **650 checked files** match the clean committed
integration snapshot. Actual regular server/workload rustc commands show
ThinLTO, one codegen unit and opt-level 3. The compiler cfg confirms unwinding
and the portable default target, with no ISA/panic override. Rust is 1.94.0,
LLVM 21.1.8.

The first build completed (`47627/0`, `7e522c`), but its wrapper omitted verbose
Cargo logging. Readback `edc4a4/1` correctly rejected the missing actual rustc
command. A separate fresh build adds `CARGO_TERM_VERBOSE=true`, with unchanged
source and predicates. It completes as `15498/0` (`d570ea`); independent
readback `ce70ec/0` passes. The original build and failed readback remain retained.

| Integrated retained object | SHA-256 |
| --- | --- |
| Source tree | `c146f7f8dfa351f5dbeee784f2cba0dc071047e84b3c78021920d0a4c6e42a6d` |
| Server | `dd028cb2f61633dda133b05d814a6d173a79a83145d8ced2f81dc0cf8e0f33bc` |
| Build manifest | `582ee4f5840aa8cf04168ee8aeffe92250ed0807ee3f53aea93b25d3494b809d` |
| Cache receipt | `5751e4796ab0b430d92d66c12a8ca046be2c2b9fbc1cf616b6ec59c942517564` |
| Recovery client | `bd18336b5da137adbe008bc69db15dbf7769b73a74a876b9461c493665086d83` |
| Recovery client manifest | `631d75993bd8dc7e502117cc44161de1ceaf75e04e4ff4850e5e57a1433b7df7` |
| Production codegen readback | `df19a3ea58959430ffa7a9a9f9c9ce0f48689a13e18617cfa9603c0680fbdff4` |

Server and client hashes match the original `02d0c01` build; the new source
and build manifests preserve their own identity. Compilation remains inside
its recorded disk reservation. No cold-build speed or build-memory claim is made.

## Fresh ordinary recovery

The unchanged source-bound fixture kills the leader and restarts its original
directory while point operations overlap atomic batches on normal endpoints.

| Transport | Complete-history operations | OK | Unknown | Fresh drained voters |
| --- | ---: | ---: | ---: | ---: |
| Streaming | 140 | 127 | 13 | 3 |
| Unary | 151 | 135 | 16 | 3 |
| Total | **291** | **262** | **29** | **6** |

Both full histories pass independent atomic/native and Raw KV checking.
Unknown outcomes remain in the histories. Successful point and atomic batch
operations occur inside both voter-loss and restart windows. All five server
and two client lifetimes exit. Source/artifact identity, fresh drains and
cleanup pass. All three retained metric documents have schema v2 and the same
26 default metric names.

Execution: `56800/0` (`ea8c1a`). Independent audit: direct tool `373069/0`,
with no numeric root session. Audit SHA-256:
`c91a6cfab97d4e61d53bbc4022b45118c2f0be0a4e283c6b4da73dfeb8cd553b`.

The original [21-window actual Chaos Mesh run](RELEASE-THIN-LTO-CHAOS.md)
remains attributed to `02d0c01` and its original build manifest. Its executable
bytes match this build, but no new Chaos campaign ran during integration.
Ordinary one-host recovery cannot establish independent host loss, device
power loss or broader industrial acceptance. No hosted CI was dispatched.

## Next performance work

Investigate the batch-write tail at a [common offered load](BATCH-WRITE-TAIL-NEXT.md),
then follow the [quorum-path plan](QUORUM-LATENCY-NEXT.md) to isolate remaining
single-GET latency. Preserve the closed-loop result and keep notification
candidate `42e0117` separate. Dynamic multi-Raft and automatic splits follow
the read-performance milestone under the existing correctness/storage gates.

Local originals are under `/tmp/kv9-thin-lto-main-integration-root-first`,
`/tmp/kv9-thin-lto-main-integration-source-first`,
`/tmp/kv9-thin-lto-main-integration-release-verbose-first`,
`/tmp/kv9-thin-lto-main-integration-process-e2e-first` and
`/tmp/kv9-thin-lto-main-integration-recovery-preparation-verbose-first/results-first`.

[Immutable reporting evidence](https://github.com/c4pt0r/kv9/blob/ab7804a6fba1f3cc2cbd94bcfad8b5321d311f77/docs/release-thin-lto-main-integration-v1/README.md)
retains 201 exact files: 4,820,687 decoded bytes in one 565,744-byte archive part.
Packaging `8dc54a/0` and independent archive verification `c44a1f/0` complete.
Inventory SHA: `29ad60c3d4264e5248ef25d56d566098d05e1ddb41c118e5de058e7ac8364392`.
The root publication preflight's initial inventory-shape error is also retained;
it occurred before packaging and was corrected without changing frozen inputs.
The bundle includes complete histories, build/source logs and all three voters'
metrics/status metadata. It excludes WAL payloads and executable binaries;
archive integrity checking does not rerun the original recovery audit.
