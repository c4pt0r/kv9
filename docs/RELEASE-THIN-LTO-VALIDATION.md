# ThinLTO source, build and ordinary recovery validation

Candidate `02d0c01024b65a84b220c6948ff2224bfa7900bc` passes full-workspace release
checks and ordinary streaming/unary process recovery. Its
[earlier screen](RELEASE-THIN-LTO-PERFORMANCE.md) and subsequent
[complete72 point/batch comparison](RELEASE-THIN-LTO-FULL72.md) are accepted.
[Fresh main integration](RELEASE-THIN-LTO-MAIN-INTEGRATION.md) now passes as
`11113f6`, reproducing the qualified executable bytes and preserving the full72
batch-write p99 tradeoff. The records below remain the original `02d0c01`
qualification. The exact
build now also passes the [21-window actual Chaos matrix](RELEASE-THIN-LTO-CHAOS.md),
with 9,923 complete history operations and independently verified cleanup.

## Source and compiler scope

The [source change](https://github.com/c4pt0r/kv9/commit/02d0c01024b65a84b220c6948ff2224bfa7900bc)
adds `[profile.release] lto = "thin"` and `codegen-units = 1` on `40f014f`.
No runtime algorithm, dependency, feature default, target ISA, panic behavior or
overflow policy changes. The default read-stage observer remains disabled.

The final release workspace gate passes **709 tests/doctests**, with **23
existing ignored**, formatting and all-workspace/all-target Clippy. Its session
is `15927/0`, receipt `fac0cc`. The initial narrower Raft/server gate passed
435 tests with one ignored before a documentation-only scope correction; these
overlapping counts are not added. All 627 final checked source files match the
committed retained release map.

The test dependency graph enables testing features through dev dependencies.
A separate production build establishes the default server/workload graph:
11 first-party units rebuilt, 20 artifact observations, all feature sets empty.
The repository build-cache lock and explicit first-party invalidation are used.
Actual retained regular compiler commands show ThinLTO, one codegen unit and
opt-level 3. The exact compiler cfg confirms panic unwinding and the portable
default target; no native-CPU or panic override is present. Rust is 1.94.0 with
LLVM 21.1.8. Compiler correctness is assumed, not proved by these tests.

| Retained object | SHA-256 |
| --- | --- |
| Source tree | `c61e700243b734fd6ee310af1f39004e1f5cbc295c0228a2a6485986fab8f598` |
| Server | `dd028cb2f61633dda133b05d814a6d173a79a83145d8ced2f81dc0cf8e0f33bc` |
| Build manifest | `f336886f07cd42dd1bd37388351e1c93c41ed828d6da31c23e4e87726a2075a0` |
| Cache receipt | `3dbe19cfeddad0433f14692bd956a8c79b0dc6969dba20d145247b1ea2fb73fb` |
| Recovery workload | `bd18336b5da137adbe008bc69db15dbf7769b73a74a876b9461c493665086d83` |
| Workload build | `4dad471130dbbfe97aeadc8f235cbd24fd10ade179fb3e66a98da19f41a909ca` |
| Production codegen readback | `75df57584a19826afb7f3484c3eddb2fe7732bcfcab2bfaaf14910604129e1e8` |

Original production release: `24115/0`, receipt `cf4eba`; independent root
readback `984f60/0`. Its 40.665-second build follows cached workspace compilation;
it is not a cold-build comparison. The compilation disk reservation remained
within its original bounds; no compilation memory bound is claimed.

## Ordinary recovery

Five recovery preparation contracts pass. The unchanged process protocol tests
leader kill and original-directory restart while retaining overlapping point
and atomic batch histories for streaming and unary RPC.

| Transport | Complete-history operations | OK | Unknown | Fresh drains |
| --- | ---: | ---: | ---: | ---: |
| Streaming | 189 | 172 | 17 | 3 |
| Unary | 170 | 154 | 16 | 3 |
| Total | **359** | **326** | **33** | **6** |

Both complete bounded histories pass their independent checker. Unknown outcomes
remain in those histories; this is not an all-success benchmark count.
Successful atomic operations occur inside voter-loss and restart windows.
All five server and two client lifetimes exit. Source/input identity, fresh
drains and owned cleanup pass. Original execution: `74156/0`, receipt `557e27`;
independent audit: `81d6f9/0`, audit SHA
`6b2de37ebde8033faeca3068fdc595a4318563c8852b2ccb0bfc0b16e8e1322d`.

This is ordinary single-host process recovery, not actual Chaos Mesh,
independent host loss, full implementation refinement or industrial readiness.
Existing core proof obligations and availability gates remain open. No hosted
CI ran. Current artifact locations and immutable publication are linked from
the performance report.
