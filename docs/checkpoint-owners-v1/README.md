# Automatic checkpoint-owner evidence v1

Actual local validation on 2026-09-15, based on `b2885b2` plus the exact sources
in this packet. [Protocol, format, source mapping and remaining work](../CHECKPOINT-OWNERS.md).
This increment adds the current checkpoint worker's durable pre-upload plan and
automatic Pending/Version owners. It does not complete C04/#14 or S07/#21,
enable object deletion, or establish complete reference/backfill coverage.

## Development packet

| Packet | Members | Decoded bytes | Gzip bytes | SHA-256 |
| --- | ---: | ---: | ---: | --- |
| [Development](development.tar.gz) | 1,534 | 16,971,713 | 4,050,511 | `bd3464ce63ddca6c826c06a9f8a6b6aacd6dcd9cae58a3532268e5e8d302a123` |

[Inventory](development-inventory.json) and [readback](development-verification.json)
compare every member to its original bytes and check all 166 frozen source pins.
Creation/readback terminal: **605789 / 0**. Executables, duplicate build
checkouts, target trees and generated proof caches remain local; the packet's
selector states each omission. Original failed attempts are retained alongside
their corrections. Packaging does not replay tests or qualify a benchmark.

The separate [runtime packet](runtime/evidence.tar.gz) contains 2,543 regular
members / 123,515,974 decoded bytes / 1,711,354 gzip bytes, SHA-256
`3c3b0d5db2720f9a80aaf93b5ca13fe112c67512def2a0dc2b5a09bd682fb26b`.
Its full byte readback passes **55cf14 / 0**. Both packets pass a scan against
seven known actual fixture secrets, with zero matches (**e24b99 / 0**); values
and hashes of those secrets are not retained. This is not an arbitrary-secret
detection claim. The [root packet/source readback](source-and-packet-verification.json)
also checks all three complete nested protocol logs, runtime companion files,
the 166 build inputs and all five current proof source inventories.

## Local gates

| Gate | Actual outcome / terminal | Packet path |
| --- | --- | --- |
| Workspace/all-target check | Pass, **76195 / f5a44d / 0** | `runs/checkpoint-owner-development-20260915-fourth` |
| Complete library suite | **731 pass, four ignored**, **41682 / de6f31 / 0** | `runs/checkpoint-owner-library-acceptance-20260915-first` |
| Eight applied-but-unconfirmed handoff prefixes and all-store reopen | One three-voter test passes, **11452 / e265a3 / 0** | `runs/checkpoint-owner-tests-20260915-second` |
| Planned/Prepared capability privacy; all-target Clippy with warnings denied | Pass, same combined terminal above, distinct inner command records | Same directory |
| Checkpoint-owner protocol | Seven statements / 43 obligations, two models, six counterexamples, three proof rejections; **46244 / 4a2153 / 0** | `runs/checkpoint-owner-protocol-20260915-second` |
| Four affected proof compositions | All pass, **35970 / e214ce / 0** | `runs/checkpoint-owner-composition-20260915-first` |
| Four isolated compiled engine faults | Exact guarding assertions fail; same baseline filters pass, **39218 / 2abbd2 / 0** | `runs/checkpoint-owner-source-controls-20260915-second` |
| Default and separate testing build | Both pass, **77488 / 96521d / 0** | `runs/checkpoint-owner-{default,testing}-build-20260915-first` |

The complete library run uses a fresh recorded NVMe fixture directory and four
test threads. It is a single successful full-library execution. The three
baseline source-control tests overlap that suite and are not additional tests.
The private helper's eight uncertainty cuts validate applied ledger outcomes;
actual worker settlement and object I/O are covered separately below.

The source-pinned compositions retain their original protocol statements:
anchor binding 7 / 26 obligations, frozen base identity 7 / 27, actual publication
9 / 36, and retention ledger 7 / 55. The new source hashes have an explicit
composition review. No new proof of upstream Raft or Rust refinement is claimed.

Frozen executables, both with 166 identical source pins:

| Build | Bytes | SHA-256 |
| --- | ---: | --- |
| Default | 46,981,864 | `0e7c335497e8f6bbe27c44073f579366bdc01d0742934f1af3f73a47a8735702` |
| Only `checkpoint-testing` added | 46,985,360 | `16b4e0325e3bd581195255b9afa70942043ee499a8e7b9edc226e688c00fa014` |

The testing executable is frozen but its deterministic pre-upload crash gate
has not run. Neither executable is a newly qualified performance candidate.

## Actual default leader failure

[Runtime packet, independent audit and original preparation failures](runtime/README.md)
retain the exact helper, selected descriptor/SST, full stopped-store archives,
complete Raw/owner observations and scoped cleanup evidence.

The actual default cell **23271 / 32b9ce / 0** completes in 124.55 seconds.
Independent audit **94289 / 2c5d7a / 0** checks **286 successful Raw calls**:
275 puts, two deletes, eight reads and one keyspace creation. The original
physical 16 MiB rotation predicate takes 270 filler writes within the unchanged
300-write cap; historical 280-filler/296-call totals are not forced into this run.

The selected recovery cut is index 1487, its true predecessor is generation 71,
and the actual winning manifest command is index 1496, publishing generation 72.
Independent replay of all three complete stopped-store Raft logs checks Pending
Held / Published / Quiesced / Released at indices 1493 / 1495 / 1502 / 1504,
and Version Held / Published at 1498 / 1500. Four real CLI observations bind the
same exact terminal owners before and after leader death. Exact selected SST
bytes are captured; the registered closure and recovery payload are the same
object set in this cell.

All 477 command lifetimes exit successfully and are absent afterward. The 1,492
storage samples retain an aggregate conservative maximum decrease of 614,907,904
bytes across host/output filesystems, below the unchanged 1 GiB budget, with
both 8 GiB continuous floors preserved. Actual PodChaos effect, exit 137, new
container identity and recovery on the same Pod/PVC/store are separate evidence
from the final owner observations. This is one process-failure cell on local
Kind with a tmpfs object store, not physical power-loss, host-loss, full21,
concurrent-history or physical-GC acceptance.

## Preserved development failures

The first three compile attempts identify a missed tuple consumer, a moved
backend Arc, and an inaccessible sibling test helper. The first runtime test
completes all eight applied uncertainty cuts, then asks for the leader too early
after reopening; the correction explicitly waits for agreement. The initial
proof draft leaves three of 43 obligations unproved. Its corrected proof adds
the missing definition expansion without weakening the model or statements.

The first full protocol control expected a transfer failure, but the same
unsafe quiescence first violates coverage under the unchanged invariant order.
The first compiled-control runner reaches the intended malformed-body assertion
but expects generic panic wording; the correction requires its exact custom
message and source line. Both failed runner attempts remain in the archive.
Neither an infrastructure error nor a compile failure counts as an accepted
negative control.

## Performance inputs and next work

`runs/performance-input-recovery-20260915-first` retains the original 581 client
source files, source inventories, historical build records, toolchain checks,
one explicitly recorded Rust/Cargo 1.94 rebuild and its comparison. The new
5,592,624-byte ELF differs from the missing historical 5,594,104-byte ELF despite
matching source and normalized compiler-artifact records. Its hash is
`1b8060eb168610328c10a27480de16bd8b2d6805166d8169638f85ceef0992c4`.
The cause is unproven and the replacement remains unqualified. This is recovery
metadata, not a performance result; the original full matched screen stays open.

Next exercise the separately designed `owned` pre-upload crash cut, preserving
original v2 bytes, live acquired ownership and typed outcome after restart.
Then persist negative-attempt history/abort release and backfill/fence complete
references before reader drainage, history authority and destination install.
Followers can defer their old local attempt while a new leader publishes; an
actual negative settlement must keep its owner pinned, not be counted as a
successful handoff. No original checkbox is closed by this increment.

New bulk automation output uses `/mnt/data/kv9-work`. Active latency-sensitive
fixtures and the reusable Cargo target stay on explicitly selected NVMe paths.
See [output placement and current capacity](../LOCAL-ARTIFACTS.md). All checks
are local; no hosted CI or new QPS is claimed.
