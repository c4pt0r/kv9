# Slicing-by-eight CRC on selected ThinLTO

Updated: 2026-09-12. This is an isolated write-path candidate based on selected
`11113f68f6a5df77da1ffb4fcec850953716ffa3`. It reapplies the exact implementation
from `e5662bbce1443e6907e3d832a7b300881796340c`. The only runtime change is the
engine WAL checksum update; Raft logic, acknowledgment/sync order, data format,
transport, allocation and worker scheduling are unchanged.

The byte table processes one dependent byte transition at a time. Slicing by
eight uses eight independent table contributions for each complete block and
the original byte transition for its tail. Initial/final state, polynomial and
fragment behavior remain identical. The old profile's batch-write checksum cost
and standalone function timing motivated this candidate; neither establishes a
new database speedup on ThinLTO.

The entire candidate `crates/engine/src/wal.rs` is byte-identical to the earlier
proved source, SHA-256
`14bc5d0a56a2513f5155302bb982bd8ef41670cff7742649780ee42a946b209a`.
The original [source contract and proof](../proofs/lean/crc32-slicing8/README.md)
are reused unchanged. This is reuse of an existing experiment, not a new CRC
algorithm or a combination with an unqualified scheduler candidate.

## Fresh local qualification

The [source and proof summary](write-crc-slicing8-v1/validation-summary.json)
records 135 Engine library tests passing, zero failed/ignored, workspace
formatting and Engine all-target warnings-denied Clippy. The alignment/split
regression compares every alignment 0..7, length 0..128 and split point with
the independent bitwise oracle. This count is scoped to the selected ThinLTO
base, not the later lease-work branch's larger test population.

Fresh Lean 4.33.1 kernel checks accept 47 distinct theorem statements and the
same 47 after restoration. The proof covers arbitrary 32-bit states, bytes,
input lengths and fragment boundaries, using all 256 fallback and 2,048 slicing
entries emitted by the actual extracted Rust constants. Eight deliberate
controls fail for the intended arithmetic, proof-axiom or source-mapping reason.
No proof or compilation failure occurred in this reapplication's first gate.

The reviewed Rust primitive/chunk-iterator mapping and compiler semantics remain
explicit premises. This is CRC computation equivalence, not a whole-Rust,
compiler, persistence, WAL recovery or Raft implementation proof. Historical
failed proof drafts and historical ordinary recovery retain their original
scope in the [earlier qualification](https://github.com/c4pt0r/kv9/blob/65511010e2fda8adba04efd831a39bcdca1979a4/docs/CRC32-SLICING-QUALIFICATION.md).

Source supervisor session 23381 exits zero (53668d), with the retained build
lock, 96-GiB preflight, 80-GiB free floor, 16-GiB additional reservation and
1,200-second command limits. Proof session 10629 exits zero (33adce). Both use
helper CPUs 6–15,22–31. Source/proof inputs are unchanged through their checks;
only this report and retained evidence are added afterward.

The [inventory](write-crc-slicing8-v1/inventory.json) binds every published
original byte and records excluded local executable/compiled-proof payloads.
All archive members were decoded and compared with their original files.

## Next gates

Build this committed source as a clean default release, run ordinary recovery,
and compare it against selected ThinLTO after the three-copy Redis write baseline.
Performance selection still requires full point/batch regression qualification
and actual exact-source Chaos Mesh histories. Retain the known loaded-write p99
problem; a checksum function speedup alone is insufficient. No new database QPS,
latency, Redis parity, default promotion or original industrial checklist closure
is claimed. Checks are local; hosted CI remains manual.
