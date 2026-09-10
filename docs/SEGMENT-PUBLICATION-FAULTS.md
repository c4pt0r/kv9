# Segment publication syscall faults

Tracking: S01/#15, C01/#11 and C04/#14. This local gate checks production
publication/error paths and explicitly permitted recovery outcomes. It does
not replace the topology proof, the complete C01 filesystem model or Chaos Mesh.

## Empty-source migration bug

An initially empty legacy WAL, or an empty tail after legacy checkpoint
compaction, is valid. If migration created its segment directory and failed
before final publication, reopening misclassified the old zero-length root as
a truncated segmented topology. The old recovery authority was still present,
but the engine could not recover automatically.

`enable_segmentation` now synchronizes an empty framing record in a zero-length
legacy source before creating migration artifacts. A checkpoint-backed source
retains its exact applied position; an empty unpositioned source conveys none.
Migration omits only empty unpositioned batches, which change neither data nor
progress. Positioned empty batches still convey progress, and nonempty
unpositioned histories retain their pin. Thus framing does not permanently
disable reclamation. Missing or truncated new roots still fail closed.

The original failure is retained at
`/tmp/kv9-segment-publication-empty-regression/migration-empty-rename_before/visible/`.
It reports `segment directory exists with a truncated WAL topology` from the
actual recovery path after injected EIO at rename. It is not counted as passing.

## Executed paths and outcomes

`crates/engine/examples/segment_publication_probe.rs` drives the production
`SegmentedWal` and `WalEngine` interfaces. The Linux/glibc shim in
`scripts/faults/segment_publication.c` loads only into this owned process,
selects one exact topology path and records the syscall boundary before
returning EIO. No engine/server fault hooks are added. The runner identifies
the executable through Cargo's structured output and retains binary/shim hashes.

Five operations run in the full gate: rotation; checkpoint publication with
covered closed files and a live tail; populated legacy migration; initially
empty migration; and empty-tail migration backed by an actual MinIO checkpoint.
Each has a success baseline and errors before/after rename and publication
directory fsync. Migration also fails before/after the new segment file fsync.
Checkpoint cases add errors before/after unlink and unlink-directory fsync.

Publication uncertainty must reject subsequent writes. Cleanup failure after
durable checkpoint publication must still allow them. Every recovery checks
exact cross-column-family values and progress, writes another record and
reopens again. Migration recovery retries migration. The initially empty case
also seals the recovered stream and asserts that it contains no unpositioned pin.

The shim captures the actual predecessor root immediately before rename,
including any synchronized legacy framing record. Isolated copies materialize:

- The old root before rename.
- Both complete old/new roots after rename but before confirmed directory fsync.
- Only the new root after successful directory fsync, even if the wrapper then
  reports an error.
- Resurrected covered files after unsynchronized unlink, preserving the new
  checkpoint and later acknowledged write. Corrupting those unselected covered
  records also cannot force their replay.

These are explicit crash-state constructions under successful-fsync and atomic
rename premises, not a physical power-loss emulator or proof of the filesystem.
They do not detect every omitted upstream sync. Each directory has one owned
writer; unrelated mutation and checksum collisions are outside the contract.
The stream-only checkpoint simulates committed authority and full restore;
`--minio` uses actual SST upload/restore for checkpoint-backed migration.
Neither alone grants Raft authority; separate runtime histories test that path.

## Commands and controls

```sh
python3 scripts/check-segment-publication.py --output /tmp/publication-new
# Requires KV9_OBJECT_STORE_ENDPOINT, BUCKET, ACCESS_KEY and SECRET_KEY.
python3 scripts/check-segment-publication.py --minio --output /tmp/publication-minio-new
python3 scripts/check-segment-publication-controls.py --output /tmp/publication-controls-new
```

Without MinIO, 28 cases run. With MinIO, 35 cases and 49 explicit recovery
outcomes pass, plus two controls rejecting a missing shim or wrong target.
The source gate builds six baseline/mutant/restored triples in a private source
copy. Named runtime failures are required; compiler failures and timeouts cannot
count as counterexamples.

| Removed protection | Required failure |
| --- | --- |
| Empty-source framing | Interrupted empty migration cannot recover |
| Empty unpositioned batch elision | Migrated stream stays pinned |
| Failed-checkpoint fencing | A subsequent write is acknowledged |
| Publication before unlink | A selected segment disappears |
| Retiring the old migration writer | The unlinked old writer accepts a write |
| Publication directory fsync | The required sync is bypassed and a write is acknowledged |

All six controls passed locally. Four real MinIO engine tests and 577 workspace
tests also passed (23 external-environment cases ignored in the ordinary run).
Workspace/all-target Clippy with warnings denied passed. The existing 21-window
Chaos matrix passed separately on `1698f6c`, before this empty-source fix; it is
not relabeled as acceptance of the changed runtime or these syscall-specific
cuts. S01/C01/C04 remain open. All checks run locally.
