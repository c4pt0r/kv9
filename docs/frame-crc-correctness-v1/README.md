# Frame buffer on CRC: local correctness evidence

Candidate [`e9249f2`](https://github.com/c4pt0r/kv9/commit/e9249f2cbd069dcdc44312be826a68494cf694db) constructs each Raft WAL frame in one buffer on the integrated CRC baseline. Frame bytes, synchronization, quorum and response fences remain unchanged. This evidence concerns correctness qualification; it establishes no performance gain or runtime promotion.

- 790 workspace tests/doctests pass; 23 existing tests remain ignored. Formatting, Clippy and explicit experimental-lease compilation pass.
- Frame encoding passes three universal SMT properties and three countermodel controls. CRC passes 47 distinct Lean statements, fresh restoration, all compiled tables and eight rejection controls. Rust primitive/compiler premises remain explicit; these are not whole-Rust/Raft proofs.
- The clean default release binds 866 source files. Ordinary streaming/unary recovery retains 337 operations: 309 OK and 28 unknown, with six fresh drains and seven exited lifetimes.
- Actual Chaos Mesh passes all 21 windows. Complete histories retain 9,805 operations: 9,170 OK, 613 unknown and 22 refused. Four final drains, all 33 observed server lifetime exits, full archive readback and owned-namespace cleanup pass. Eight historical namespace identities remain preserved. Unknown writes remain unknown.

Original runtime terminals are source `38793/131335/0`, release `81371/f79bb2/0`, ordinary recovery `71494/a233d6/0`, Chaos `34911/6b428a/0`, and post acceptance `73657/eab7c5/0`. The original independent audit predates cleanup and retains `cleanup_complete: false`; subsequent cleanup records establish that separate completed step. Dedicated link/quorum, cross-host and power-loss gates remain separate scopes.

## Published bytes

The source/proof/recovery/post supplement contains **458 files / 7,468,808 decoded bytes**, stored in one **1,066,208-byte** part. Its inventory SHA256 is `4fe0c807af1f70fcc8cc42549ee19105472c7e4ac5eb1dead280342031219cbe`. Production executables, compiled proof outputs and ordinary recovery WAL/data remain local and retain their original inventory references.

The original pre-cleanup Chaos archive is preserved without filtering or recompression in **45 parts**, totaling **92,343,933 bytes**, SHA256 `b3bc55144f53c83bbdcb5369c2388a4905e024e3a221e94a723477e46a88ecbc`. Its original archive inventory describes 4,553 members, including 4,258 files / 938,844,491 file bytes. All original members were read back before cleanup. The separate cleanup supplement reflects the actual chronology.

Packaging, metadata verification and split-byte verification all passed at actual direct tool terminal **593129/0**. [Publication receipts](publication/terminal.json) bind the original commands, results and generated inventories. Portable verification checks archived bytes; it does not rerun tests, proofs, histories or Chaos:

```sh
python3 -B verify-source-recovery.py --root . --inventory-sha256 4fe0c807af1f70fcc8cc42549ee19105472c7e4ac5eb1dead280342031219cbe
python3 -B chaos-parts.py --output chaos-original --verify-only
```

The [main qualification report](https://github.com/c4pt0r/kv9/blob/fe8a8617e269a5cf1a845a03e7879558f61303ea/docs/WRITE-FRAME-BUFFER-CRC-QUALIFICATION.md) records implementation and next-step context. All checks were local; hosted CI was not dispatched.
