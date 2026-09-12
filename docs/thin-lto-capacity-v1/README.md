# ThinLTO capacity bootstrap and large-file qualification

The new retention helper passes its actual **1,073,741,841-byte synthetic**
writer/compression/independent-decode/fresh-restore qualification: **70393/0**,
receipt `c603b7`. Complete bytes match and the writer/codecs exit. The deliberately
compressible file does not estimate a WAL compression ratio. The preceding 53
helper controls remain accepted; helper source hashes are unchanged. No
performance campaign ran. Capacity for 36 smoke / 72 timed cohorts remains open.

The CRC72-only cold-controller derivative passed 16 original and seven new
controls (`94964/0`, `957615`). It binds one exact accepted 72-cohort campaign
and applies the existing compressed-size cap after child completion. Other
transaction, reference, byte-check and restore logic is unchanged.

Root reclaimed 1,132 exact `.o`/`.a` cache files under four locks, first
invalidating four associated build-script success markers. Original build-script
records and five pinned binaries remain unchanged. Receipt `dd66fa/0`; actual
free-space gain **428,720,128 bytes**. Future Cargo use must observe regeneration;
this operation did not run Cargo or qualify regeneration.

All 38 CRC transactions complete: **561 files / 8,589,901,660 logical bytes**,
190 successful preflight/init/stage/inspect/evict commands. First stage
`21272/0` (`4d8072`), first eviction `34f558/0`, remaining transactions
`19506/0` (`1e2d1b`). Each eviction uses its terminal VERIFIED catalog and new
catalog-bound release; full decoded-byte and original identity/reference checks
precede removal. The **96 GiB floor is unchanged**.

The first seven real WAL files, **107,138,169 bytes**, were restored to original
paths and independently rehashed (`f1bf4e/0`). They remain resident, with matching
supported mode/owner/access/modification metadata. Original inode/ctime identity
is not claimed. **554 files / 8,482,763,491 logical bytes remain COLD**.

Current payload allocation is **6,041,636,864 bytes**: 551 unique shared objects
plus ten retained duplicate staging copies, including pilot objects. Net payload
allocation saving is **2,442,665,984 bytes**, excluding transaction/reporting
metadata. `capacity-summary-v2.json` corrects the initial summary's omission of
5,177,344 allocated bytes in duplicate staging copies; both are retained.
After large-file restoration, observed free space was 104,597,102,592 bytes,
only 1,517,887,488 above 96 GiB. This does not establish full campaign capacity.

`crc72-preparation/commands.json` supplies exact restore commands. Each catalog
binds original hashes/identities, compressed objects and controller
`acb9d43072428df74a000e68609ee9f9e8d5d34e45dd935ac987435abcebe140`.
For example, for cold transaction 001:

```sh
sudo -n env PYTHONDONTWRITEBYTECODE=1 PYTHONOPTIMIZE=0 taskset -c 6-15,22-31 \
  /usr/bin/python3 -B /tmp/kv9-thin-lto-historical-capacity-preparation-first/cold.py \
  restore --txn /tmp/kv9-benchmark-cold-crc72-transaction-first-001
```

Restore needs decoded-byte capacity while objects remain, unchanged parent
identities and unoccupied original paths. It refuses collisions and preserves
durable mixed-state intents. Transaction 000 is already restored; 001–037 are
cold. Old full-input audits require rehydration first. Original inventories were
not rewritten; this overlay supersedes their CRC72 residency description.
The previous 552 cold files in other campaigns, current ThinLTO/global-queue
evidence and runtime binaries were not selected. Local objects are not an
independent backup.

This bundle retains **3,482 original metadata/helper files / 16,242,245 decoded
bytes / 1,483,352 compressed bytes**, including catalogs, releases, journals,
commands, controls and large-qualification results. `verify.py` checks every
selected member's size/hash and the archive part; integrity only. Synthetic and
compressed payloads remain local. The metadata bundle alone cannot restore WALs.
The initial README write was refused because packaging created this new output
directory as root; only that new directory's ownership was corrected.

Further capacity selection remains separate. No new QPS, promotion or industrial
gate completion follows from this work. CI remains local.
