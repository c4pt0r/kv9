# First follower WAL sync-stall qualification

This record preserves the original failed harness run and its successful
independent retained-evidence review. A subsequent archive-tool repair does
not change the original exit code. The qualification uses exact f2 source;
it does not accept the newer allocator or typed-dispatch candidate.


The original controller session **99004 exited 1**. All three runtime windows and post-client drain checks completed, but archive extraction rejected the supervisor FIFO; cleanup then failed because the regular-file inventory had not been written. Both errors, the original tar archive and partial extraction are unchanged. This report separates that failed harness result from subsequent verification of its retained evidence.

The separate closeout session **40817 exited 0**. It inventoried all 75 archive members, extracted and read back **58 regular files / 4,573,655 bytes** using the existing data filter, and retained the single FIFO as metadata without recreating a special file. All **2,707 original files** remain unchanged. Exact namespace UID `f12af42a-a32a-43b3-8963-630b38cfe219`, owned data/observer directory inodes and five Pod/CRI lifetimes were checked before cleanup. The namespace and directories are absent; all eight prior namespace UIDs are unchanged.

The first independent retained-evidence audit passed. It replayed 1,307 raw command records, six native client captures, sixteen server samples, eight I/O captures, two explicit writer transitions, the original complete atomic-history checker, three contained-call windows, and four fresh post-client drain samples. The source is exact default-feature f2 revision `f2c4e856d03f75ac6e4718f0aac7f4e7a1cf3d02`; this evidence does not cover subsequent typed or allocator candidates.

| Operation | Calls | Items | Successful | Unknown/refused |
| --- | ---: | ---: | ---: | ---: |
| GET | 66 | 66 | 66 | 0 |
| PUT | 65 | 65 | 65 | 0 |
| DELETE | 64 | 64 | 64 | 0 |
| BatchGet | 231 | 1,842 | 231 | 0 |
| BatchPut | 229 | 1,829 | 229 | 0 |

All **655 calls** have successful terminal outcomes. Counts include initialization and final verification; batch item counts preserve repeated keys. Native client retries remain governed by the unchanged source/report checks.

| Window | Duration | Contained successful BatchGet | Contained successful BatchPut |
| --- | ---: | ---: | ---: |
| Baseline | 12.178 s | 33 | 34 |
| Raft WAL sync stall | 20.191 s | 56 | 57 |
| Healed | 12.205 s | 33 | 34 |

The injected resource was actual `IOChaos latency`, 100% FSYNC, 500 ms, exact `/data/raft/raft.log`, selected voter-3 `fixture` container. Fault UID `f6bff7fd-541e-4f01-afda-f99deaf85f16` remained stable. The database was reopened after observing the FUSE mount: PID **408**, start tick **138334972**, FD **4**, mount **8007**, device **0:691**, inode **647388**. Raw command, Pod/CRI init, namespace, executable, descriptor and path records agree. The same descriptor appended new bytes during qualification.

The fresh metric interval bracketing the stall window contains **43 additional successful `raft_wal_record_sync` samples**, total **21,546,723,437 ns**, mean **501.087 ms**. No new error/cancellation or fast-bucket sample occurred. All 43 samples fall in the coarse **268.435456–536.870911 ms** bucket; the sum supports the mean, not an exact per-call 500-ms assertion. These are live record-sync samples from the reopened writer, not recovery-only samples or SIGSTOP timing.

The selected writer was confirmed stopped **34.343 s** after the conservative pre-create clock; the fault was absent by **34.541 s**, before both the 70-second watchdog cutoff and 90-second TTL. No watchdog or forced stop occurred. Independent majority PUT/GET service produced receipt index **141**, and all healed voters caught up through that index. The same native client remained alive across the fault; healthy-link probes, complete native histories and fresh empty drains passed.

This is a single-follower correctness qualification with WAL files backed by the node's volatile tmpfs. It establishes actual syscall-delay handling for this retained run. It does not establish persistent-media or power-loss durability, all-voter/leader stalls, throughput, or acceptance of another runtime source.

The retained `proposed-fifo-retention.patch` was **unapplied at this evidence checkpoint** and targets a new private copy of the inherited helper. It inventories and skips only the known owned supervisor FIFO during extraction, preserves that FIFO in the original archive, and retains the existing data filter for every other member. The frozen first runner/helper and Python's global extraction policy remain unchanged. A future controller must explicitly bind the new helper's source hash before another run.

Primary evidence:

- Original failed run: `/tmp/kv9-native-link-acceptance-f2-storage-stall-attempt1`.
- Separate completed retention/cleanup: `/tmp/kv9-storage-stall-f2-closeout-first`; summary SHA-256 `848bd09b388e8bf4f749b5213ff34ba06b1ea8a7a12487c37fe37051ed8981cc`.
- Original archive SHA-256: `f149a4a2c7021a2b40aa7999c7b7b029520a22dbead66776813d9076fca408f7`.
- Independent audit: `audit.json`, SHA-256 `f8c7ff450f89953c9543d8ecea23ae576b59ca6b6e57ef1c9c4800d584faff44`.

## Image preparation and scope

The fixture uses a launcher built within the pinned matching Ubuntu image to
avoid the host/image glibc mismatch. Its first image-build attempt failed when
the legacy builder rejected `COPY --chmod`. The reviewed second attempt uses
plain COPY plus explicit chmod with the same CPU restriction, base digests,
source and payloads. Build session 84260 and probe/import session 88005 exited 0;
all seven payloads, file modes, no-statx/filter compatibility, image identities
and owned probe cleanup passed. No database was started during image checking.

- Ready plan SHA: `0d6f7390a1298cb320abb7046d69d6e139dc0411ec4d08b664bac3f4d6db5530`.
- Image-binding SHA: `bd96925b5a512b7fd004907074b44ba837a1ed5b97ddda3253c6d6b81a66296f`.
- Image config: `sha256:522db3320c5576ac8b725e11e4c3a7d3856a8043d88d8c7f9905d0c017d98d9c`.
- Imported manifest: `sha256:601c47c948d1fa19026189b61f77b731f33e52c4f82e2e2dcd051a68ae8da677`.
- Closeout/audit evidence inventory: `81cd4ad33911c58356d920744a57aa78f6c5cb1fd6d43089e2ed26a8660992a9`.

The original run and closeout were terminal before the subsequent typed-dispatch
performance timing. All work was local. No hosted CI was dispatched, and the
broader storage/chaos roadmap package is not marked complete by this record.
