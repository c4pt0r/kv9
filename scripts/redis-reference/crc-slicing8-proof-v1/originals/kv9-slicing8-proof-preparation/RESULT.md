# Slicing-by-eight qualification result

PASS for the exact frozen CRC implementation in `/tmp/kv9-wal-crc32-slicing8`, WAL SHA `14bc5d0a56a2513f5155302bb982bd8ef41670cff7742649780ee42a946b209a`. This identifies the reviewed uncommitted prototype's source content, not a fabricated commit or release.

The third controlled gate terminated with exit 0 in session **57110**, actual tool receipt **7165e0**; PID **1745948** was reaped and is absent. All 26 subprocess records are terminal. Output: `/tmp/kv9-slicing8-proof-controlled-third`.

- **47 distinct theorems**: 15 existing bitwise/byte algebra, 7 existing compiled-byte theorems, 16 new slicing algebra/list/parts theorems, and 9 compiled-slicing theorems.
- **94 fresh successful theorem checks**: all four modules compiled from source in baseline and restored runs. No reused positive `.olean` files.
- **256 + 2,048 actual Rust const entries** checked by Lean kernel reduction, then used in universal compiled-block and arbitrary finite list/tail/fragment proofs.
- **Eight controls rejected**: incorrect compiled slicing recurrence, alternative compiled polynomial, wrong block row, proof hole, unapproved axiom, and source-contract changes to endian order, tail shift, and final complement. The custom-axiom mutant compiles but is rejected by the unchanged semantic axiom audit; the three source mutants are rejected before compilation.
- Every theorem's dependencies are within `{propext, Classical.choice, Quot.sound}`. No native-evaluation or other axiom is admitted. Positive source hashes remain unchanged.

The unchanged historical byte checker rejects the new `crc32_parts` contract. The new gate separately binds committed ca's byte implementation and the earlier bitwise implementation. Equivalence to ca follows through their common bitwise specification; repository routing must explicitly select this slicing gate for the new function.

This proves CRC arithmetic under the documented Rust/standard-library mapping. It does not claim a machine-checked Rust iterator/compiler refinement, WAL recovery or durability, cryptographic protection, or measured performance. The tables, all input bytes and initial 32-bit state are unrestricted by the universal block/list theorem. Empty parts and every tail length are included.

The initial elaboration attempts, first compiled-block rewrite failure, and two failed controlled-gate preparations remain retained and documented in `README.md`. They are not relabeled as passing attempts. The final gate changes only inventory parsing and the scope of a source-mutation anchor; accepted proof and source-contract bytes were not changed to satisfy a failing semantic control.

| Artifact | SHA-256 |
|---|---|
| `check-crc32-slicing8-proof.py` | `a8f7eb291b1458f96eff1ffd380cd8dadf0b675c483bc83bc0c63a8def9e1a71` |
| `Slicing8.lean` | `20ae98432bba62c7dc626c7c53ba3c377cbf3880d9b1c8536e5c311bdcfa04f2` |
| `RustSlicing.lean.in` | `464f890c5cb6eae1170052b5fb1ce470ec0fd74d24540de6ae7819ef2c25d5c5` |
| `slicing-source-contract.json` | `d1c5b07d1e04b9a09f4a978d18bff5fc32e3e9ba1dbb6cb0f8833af979a23988` |
| Accepted gate `result.json` | `ed858b572052d4e7b6d84a5f402d74d2efb8e891328167c7ac907b905da3fea6` |
| Accepted gate `inventory.json` | `cec00037067c83045264ca0e60c71d44757ebc6ed0af6a81d49b78aba79e3b5d` |
| `gate-third-tool-terminal.json` | `6c2b29d62cec52efa4220cf715967d5c08039d6e562b2214010b39d60556bdba` |
| `integration-manifest.json` | `e05f32ee73b179234204667c3803edf17e3960572e86ca3b8515850137e637f3` |

`integration-manifest.json` gives exact source paths, destination paths, sizes and hashes for the four files root may copy. `README.md` contains the reproducible command, old-checker behavior, and explicit source boundary. No Cargo, benchmark, fault workload, candidate mutation or git action was performed by this task.
