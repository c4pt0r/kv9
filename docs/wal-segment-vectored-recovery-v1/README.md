# Vectored WAL release and recovery evidence

Read the [qualification report](../WRITE-SEGMENT-VECTORED-RECOVERY.md).

`original-evidence.tar.gz` contains **154 original files / 3,800,835 decoded
bytes** in **336,762 compressed bytes**, SHA-256:

```text
15df7233200833661558c676bbc1a293562f81f1e0a5719afbaf65946b789d74
```

`inventory.json` maps every archive member to its original path, byte count and
hash. All members were decoded and compared with their original bytes. Complete
small process/WAL/history inputs are included; the two retained release
executables remain local and are separately hash-bound. Extract into a fresh
review directory.

`validation-summary.json` records exact source, release and client identities,
both transport histories, 352 operations, 323 OK / 29 unknown, six fresh drains
and seven exited lifetimes. Original terminals are:

- Release: `78629/03508f/0`; independent readback: `e8086d/0`.
- Recovery: `26460/c218ac/0`; independent audit: `b6edca/0`.
- Archive publication and full byte readback: `43e4a3/0`.

`package.py` retains the bounded publication procedure. This is release and
ordinary recovery evidence, not a Chaos Mesh or performance result.
