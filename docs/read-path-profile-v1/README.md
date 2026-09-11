# Read-path diagnostic evidence

This bundle retains 91 exact files (14,205,935 decoded bytes; 964,370 compressed
bytes). `inventory.json` binds each member to its original local path and hash.
It includes both accepted five-second diagnostics, the first capped recording's
failure metadata, original CPU samples/renders, conservative switch-interval
results, reader tests, and exact-binary receipt-loop attribution.

Raw perf files and full switch dumps remain at the paths and hashes under
`local_raw`. Binaries, WALs and unrelated host process listings are excluded.
This is a compact evidence selection, not a full runtime archive. No benchmark,
proof or runtime is executed by the integrity verifier:

```sh
python3 docs/read-path-profile-v1/verify.py
```

The original absolute commands and source revisions support reconstruction
with the corresponding toolchain and fixture. The initial 128-MiB cap failure
is not accepted by the fresh 512-MiB run. CPU and OS-thread interval observations
do not establish end-to-end latency components or performance improvement.

See the [report](../READ-PATH-CPU-PROFILE.md) for protocol and limitations.
