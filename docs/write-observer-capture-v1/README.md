# Write observer capture evidence

See the [report and next experiment](../WRITE-OBSERVER-CAPTURE.md).
[summary.json](summary.json) contains both timing orders, pooled integer-histogram
results and all eight endpoint-leader diagnostic observations with explicit
whole-capture scope. The default-off observer does not promote a runtime change.

[evidence.tar.gz](evidence.tar.gz) contains 386 original small files, totaling
62,590,563 uncompressed bytes. Archive size is 1,924,954 bytes; SHA-256:
`3680268bd17199bc9a7c71bce0c6a40aaa3386c770da755badb5ea22f7c50c88`.
[archive.json](archive.json) maps every archive member to its original path,
size and SHA-256. Full member and gzip-stream readback passes `167e9f/0`, with
source inputs unchanged. Executables, source checkouts and retained database
payloads are excluded; their original identities and retention receipts remain.

The archive preserves:

- Both build attempts and the qualified default/diagnostic release manifests,
  feature artifacts, bounded supervision and independent build readback.
- Original failed capture and successful reused row 0, corrected continuation,
  unchanged inherited driver/isolator, source diffs, controls including their
  initial failures, commands, fixed original budget and both isolation records.
- All 261 frozen capture inputs, including 144 diagnostic checker inputs,
  native reports, complete result records, data readbacks and lifecycle evidence.
- The unchanged checker, its original 13 controls and corrected negative
  fixture, first actual successful analysis, all three-node distributions,
  input hashes, terminal and concise observations.
- The reviewed summary script, actual-report and unequal-population arithmetic
  controls, review, original summary, command terminal and packaging script.

Actual capture continuation: `7565/275fe1/0`. Frozen manifest SHA-256:
`4a30e107b35d7157321fe74f85a38e530b36704290a1c89d816d5da03765d4a7`.
Independent checker: `9465de/0`; result SHA-256:
`11311dee4d8a31ff20dbc18f5d52038296ae2a4677591f2a00d699af81e928de`.
Pooled summary: `08a7db/0`; SHA-256:
`fd849a2e8ecb7a94fda2623b72991d0b2b1f23f59479fa606501de599f44ebc6`.

The original absolute-path records are preserved for provenance. To replay
offline analysis elsewhere, explicitly remap extracted input paths and create
a new manifest with those unchanged file hashes. Do not overwrite or relabel
the original manifest. Local retained database files are needed for a fresh
payload audit; this metadata archive cannot substitute for them.
