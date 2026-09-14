# Current CRC main CPU diagnostic evidence

This checkpoint publishes the accepted point Put and BatchPut(64) CPU profile
of selected runtime `bd42e60f84657e22e36e34924a5c80a08eac623a`, server SHA-256
`106f517c84fabe790ea139f3c18661e14447abd9e257fdd6ba82bfabe6796eb9`.
The fixed native client is `0be806d9671e2c50701a64aa7889c8859b7648ba`, binary
`8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957`.

Both instrumented five-second c64 workloads pass their original validator and
independent CPU decoder. Point has 3,259 selected samples; batch has 3,106.
Nominal interval containment, excluded edges, all 32 bins, clock alignment,
zero sample loss, source/raw identity, retained-data checks and cleanup pass.
These are shared-host, volatile-tmpfs on-CPU observations, not QPS acceptance
or a disk/power-loss durability result. Normal Raft quorum and synchronization
remain. The historical byte-table profile is separate.

The archive contains 803 exact files, 36,908,139 decoded bytes and 4,117,081
compressed bytes in two parts. The output inventory SHA-256 is
`fa32fdc7d5894129c396fee58a7b44bdcaf0af74417043347d100a0c970befe3`.
It includes diagnostic sources, original bindings and receipts, reports,
measured samples, decoded perf text, resource/cleanup metadata and fresh
exact-binary disassembly with an executable instruction-attribution reader.
The original preparation README remains explicitly unexecuted preparation;
the actual runtime and decode receipts establish subsequent completion.

Actual runtime: session 49065, terminal 16d5c2/0. Independent decoder:
session 13979, terminal a9d140/0. Package: direct terminal 3d7283/0.
Independent archive readback: direct terminal f6df3d/0. Root's initial
inventory-reader schema error is retained separately; the corrected reader
passes before the first package attempt. No workload, decoder or packaging
retry was used.

Raw perf (58,662,664 point bytes and 52,702,772 batch bytes), database payloads
and executable originals remain local. Their recorded identities/catalogs
are in [omitted-payload-references.json](omitted-payload-references.json).
Portable verification checks archived bytes; it does not reestablish local
payload residency or execute the original database/CPU diagnostic.

Readable summaries and measured samples are provided at the top level. Exact
original paths, sizes and hashes are in [inventory.json](inventory.json).
The archive member rooted at
`originals/tmp/kv9-crc-main-current-profile-root-20260914-first/`
contains `attribute.py`, `instruction-attribution-first.json`, the three
disassembly files and their command receipts. It attributes batch FNV to
365/3,106 samples (11.751%) and point receipt search to 166/3,259 (5.094%).
Named allocator views overlap the original categories; percentages are not
request-latency fractions or predicted speedups.

Verify the portable bytes without extracting or launching a database:

```sh
PYTHONOPTIMIZE=0 python3 -B verify.py --root . \
  --inventory-sha256 fa32fdc7d5894129c396fee58a7b44bdcaf0af74417043347d100a0c970befe3
```

The original profile/decoder acceptance and portable byte verification are
separate. GitHub CI was not dispatched. No industrial work-package checkbox
is closed by this diagnostic.
