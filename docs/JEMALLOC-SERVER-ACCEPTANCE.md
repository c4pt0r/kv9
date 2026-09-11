# Linux jemalloc server: local acceptance

Candidate `629bee4fd9dcca02529703a06eefe9250fd5d1ac` changes the Linux
root binary's Rust global allocator to exact `tikv-jemallocator` 0.6.1.
The lockfile adds only that package and its native sys dependency. Raft,
metadata, storage, request framing, authentication, bounded admission and
uncertain-write handling are byte-identical to the accepted parallel-stream
base `f2c4e85`. The [implementation contract](https://github.com/c4pt0r/kv9/blob/629bee4fd9dcca02529703a06eefe9250fd5d1ac/docs/JEMALLOC-SERVER.md)
states the isolation and memory tradeoffs.

Under the successful-allocation contract, the source-level state transitions
and ownership invariants are unchanged. This argument does not prove the
allocator, whole runtime, out-of-memory behavior or all implementation adapters.
No new core algorithm or stronger formal-proof claim is introduced. Existing
proof scopes and broader refinement obligations remain open.

## Local gates

| Gate | Result |
|---|---|
| Default workspace | 707 passed, 0 failed, 23 ignored |
| RPC experiment workspace | 717 passed, 0 failed, 23 ignored |
| Both all-target Clippy configurations | Passed with warnings denied |
| Root binary tests | 4 passed |
| Process kill/restart E2E | 373 calls: 344 OK, 29 unknown; both complete atomic histories valid |
| Six-arm correctness smoke | 2,856,265 successful calls; all six cohorts completed |
| Exact Chaos Mesh fixture | 11 windows, full audit and positive netem-leaf readback passed |
| Matched performance diagnostic | All 6,558,446 measured calls succeeded; independent audit passed |

Unit-test executables in separately linked crates retain their own allocators;
the root binary declaration does not change them. The release process E2E,
correctness smoke, timing and exact image execute the allocator-bearing server.
Ignored tests are not counted as passes.

The process E2E covers ordinary streaming and unary APIs, leader termination
and restart from original data directories. Stream history has 185 calls,
170 OK and 15 unknown; unary history has 188 calls, 174 OK and 14 unknown.
Both complete atomic histories admit valid witnesses, including the first
zero-unknown-effect search. Unknown outcomes remain unknown and are not replayed.
Source/release bindings, fresh drain, retained storage and owned-process cleanup
passed without cleanup errors. This process fixture explicitly uses a 16 MiB
public byte limit; source defaults remain 64 requests and 64 MiB.

The correctness smoke uses background CPUs and establishes completion only.
Its rates are not performance acceptance. The separate matched diagnostic
uses identical native and Redis clients, two opposite-order repeats and the
same timing/resource protocol as the previous comparison. It improves single
GET by 4.47% and 6.24%, at a 23–26% increase in aggregate mean voter RSS.
See [the complete throughput, latency and memory report](JEMALLOC-SERVER-PERFORMANCE.md).

## Native allocator binding

The clean default-feature release binds 582 source inputs. Actual Cargo
build-script output identifies the release OUT_DIR; the record does not select
an arbitrary shared-target directory. Eight native output/configuration/header/
archive files are copied and hash-bound independently of the shared target.
The allocator and sys feature arrays are empty; bundled jemalloc is version
`5.3.0-1-ge13ca993e8ccb9ba9847cc330696e02839f328f7`, compiled at O3 with
static linking, a prefixed API, statistics disabled and no profiling enabled.

Background-thread support remains compiled, with `background_thread:false`
as its default. Build/runtime override variables are absent in the retained
checks, and the prefixed `/etc/_rjem_malloc.conf` symlink is absent. The image
probe likewise has no allocator override, preload or audit configuration.
This records the observed configuration; disabling Cargo default features
alone would not establish that all background-thread support was removed.

ELF relocation readback connects `__rustc::__rust_alloc` to `_rjem_mallocx`
and `_rjem_malloc` through its actual GOT entries. Two preceding display-only
attempts are retained: the combined demangled/mangled selector emitted no
function, and an address-based dump showed indirect rather than named targets.
The final explicit relocation resolution establishes the linkage; those display
attempts were not allocator or workload failures.

## Actual Chaos Mesh acceptance

The first exact-image run passed all eleven windows, followed by the unchanged
full independent audit and mandatory same-netem-leaf readback. The complete
atomic history has **2,081 calls: 1,795 OK, 70 unknown and 216 refused**;
a whole-history witness is valid. Unknown writes remain uncertain and are not
replayed.

The windows cover baseline, Service-VIP delay/heal, genuine partial loss/heal,
complete client partition/heal, exact socket reset/heal, and quorum loss/heal.
Delay, loss and partitions use actual Chaos Mesh. The same-process exact-tuple
TCP reset is the separately identified non-Chaos `SOCK_DESTROY` operation.

- Actual configured 30% zero-correlation loss advances the bound netem leaf
  `5:` / parent `1:4` from 1 to 177: **176 drops**. The separate positive-leaf
  gate excludes the inherited parent-plus-child double count.
- The contained client-partition window completes no successful operation;
  BatchGet has 14 unknown outcomes and BatchPut has 13.
- The contained quorum-loss window likewise has no successful operation;
  BatchGet and BatchPut each have 48 refused outcomes.
- Every positive/healed window completes successful batch reads and writes.
  Whole-fixture counters record 632 inline batch reads and one blocking
  submission; these include setup/verification and are not per-window or
  per-response attribution.

Run session 18044 and full audit session 46378 exited 0; the required leaf
readback also exited 0. Scoped cleanup passed. Owned namespace UID
`e4baa459-9c5e-4b05-9bd6-6dc4f17155c4` is absent, all eight historical
namespace UIDs remain unchanged, and the existing retained-data checks pass
for 50 members. No runtime or audit rerun occurred. The image preparation's
initial metadata path typo was corrected before image execution and remains
retained separately; it was not a running fixture failure.

Pods use unchanged source defaults with no admission limit overrides. This is
one shared Kind host with volatile tmpfs. It does not establish disk/power-loss
durability, cross-host behavior, the full 21-window matrix, remaining actual
inter-voter partial-loss/storage-stall scope, or complete implementation proof.

## Retained evidence

| Artifact | SHA-256 |
|---|---|
| Server executable | `d17f0ec296218c39e743312900a471a7827089e607bf3badbcb96037ca984acf` |
| Server build manifest | `ea2a751c5c48ed05376884b7f8dd3221eb4efdb0ba1b7478577389126fdcd19b` |
| Correctness workload executable | `c5078581529620d46123d871d21bdbfdf3d7d5233f3e857458671ec443b0bb4e` |
| Local gate summary | `60a20539621f9c1198b281141ed5baa3cf2bb3015cb44760fe1b651f3af3e4f9` |
| Native build/linkage summary | `d2e2cb83a1513363ec6121076940740be13bdbf0bba2ce59a659a606b696f972` |
| Source review | `27104da2679f335175b0ef56d0c006564c84c5e3aaba8e5c1fdfb783b4dfc204` |
| Process E2E summary | `c9456e574124ba1d558ea8158eb25f92b16b2c21c82b63d3f1058b3610c7df91` |
| Correctness smoke matrix | `42f2be90d3f7e85fa614e14933d0e230940269ecfd20b059a5c8924257cf716f` |
| Chaos plan | `b5160253a9b3fc531bc5704a8758d4265ba94dc8f56a52ec08e6e2a8175e3365` |
| Chaos raw summary | `37d5717e93ac84cc1aaa0b28235c8b3f588f74d9cab92c1735f9ba59ef710624` |
| Chaos full independent audit | `79d2c45dbd76bc4e785ab26af1068dae32b602705d4c841756889351e0fce57a` |
| Required netem-leaf readback | `0a374655d4aa24ee3c7f977a84ee5fea011ccae729e222100940bed9d14e5ec8` |

Release: `/tmp/kv9-jemalloc-server-release-first`.
Local checks/native provenance: `/tmp/kv9-jemalloc-server-local-first` and
`/tmp/kv9-jemalloc-server-*-first.log`.
Process histories: `/tmp/kv9-jemalloc-server-process-e2e-first`.
Smoke: `/tmp/kv9-jemalloc-comparison-smoke-attempt1`.
Timing: `/tmp/kv9-jemalloc-matched-diagnostic-attempt1/cohorts`.
Independent timing audit: `/tmp/kv9-jemalloc-matched-independent-first/results-first`.

Chaos raw: `/tmp/kv9-native-link-acceptance-629bee4-attempt1`.
Chaos independent audit/leaf: `/tmp/kv9-native-link-acceptance-629bee4-independent`.

The candidate is committed and pushed on `codex/jemalloc-server`.
This is an isolated experiment, with no default/master promotion. Larger
working sets, memory pressure, sustained load and write/mixed performance
remain unmeasured for this candidate. No hosted CI was dispatched.
