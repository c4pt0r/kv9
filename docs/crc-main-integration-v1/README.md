# CRC main integration: source, proof and ordinary recovery

Exact integrated source: `bd42e60f84657e22e36e34924a5c80a08eac623a`.
The only production algorithm change from main `69101d7` is the exact proven
WAL slicing-by-eight CRC implementation previously measured at `e748620`.

- **789 workspace tests/doctests pass**,23 existing ignored; formatting,
  warnings-denied Clippy and explicit experimental-lease compilation pass.
- **47 distinct Lean theorem statements**, a fresh restoration of the same47,
  all256 fallback/2,048 slicing entries and8 intentional rejection controls pass.
- Clean default ThinLTO/codegen1/opt3 build provenance and all859 source files
  pass independent readback; production artifact feature sets are empty.
- Ordinary WAL recovery passes streaming and unary complete histories:
  **353 operations,325 OK and28 unknown**, six fresh drains and seven exited
  lifetimes. Unknown writes remain unknown and are not replayed for success.

Read [source-checks.json](source-checks.json), [proof-result.json](proof-result.json),
[release-readback.json](release-readback.json) and [recovery-audit.json](recovery-audit.json).
Original command logs, source snapshots, proof emitter binaries/generated modules,
complete process histories and recovery WAL bytes are in the archive. Production
server/native executables are referenced by their retained hashes and manifests.

The source/recovery archive has287 files,51,096,826 decoded bytes,13,473,200
compressed bytes in seven parts. Full original archive readback passed at
`455728/0`; independent published-part verification passed at `bc5fec/0`.

```sh
python3 -B verify-source-recovery.py --root . --inventory-sha256 0db4222cec1d5b0f991ac4f3c1b6d2310a477d226b789d7e580b8d498e005ad9
```

Actual root terminals: source23518/`55038d`/0; proof91323/`138e5c`/0;
release61302/`52ceb2`/0; recovery53764/`c563a8`/0; independent recovery audit
`526265`/0. The checker verifies portable bytes; it does not rerun proof or
runtime acceptance. Build compiler is Rust1.94.0; proof checker is Lean4.33.1.
Rust primitive/iterator/compiler semantics remain explicit proof premises.

Server SHA256: `106f517c84fabe790ea139f3c18661e14447abd9e257fdd6ba82bfabe6796eb9`.
Native client SHA256: `ec8002c00253fa6264a2f7bef3814e33dc871c7ef895e6163b7be38be2edffad`.

This checkpoint does not contain new Chaos Mesh or performance acceptance.
Exact-main Chaos is a subsequent gate; e748's measured QPS and Chaos outcomes
are not reattributed to this binary. Raft/sync/apply/response fences and normal
Safe ReadIndex remain. Local validation only; no hosted CI was dispatched.
