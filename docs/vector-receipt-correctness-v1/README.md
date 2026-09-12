# Vector receipt correctness evidence

This selection contains 344 exact original files: 7,007,731 decoded bytes
and 1,116,225 compressed bytes in 1 archive part(s), each at most 2 MiB.
`inventory.json` maps safe relative archive names to original absolute paths,
byte counts and SHA-256 hashes. Both complete operation histories are included.

The source is `7c8cd25e9531809c9d5bd40faee116a7d6d5a3ff`. The source gate records
213 passing Raft tests/doctests, format checks and all-target Raft Clippy. Two
compiled controls preserve baseline/mutant/restored exits 0/101/0, their actual
source mutations and assertion failures. These are not full-workspace tests;
there is no exact-source Chaos Mesh qualification in this bundle.

The unchanged local proof gate records 13 distinct theorems and 122 fresh obligations
per positive run. Four positive runs discharge 488 obligations; this is not 52
distinct theorems. All 14 intended cases pass, including attributable certificate
and eviction counterexamples, rejection of omitted proof/unapproved axiom inputs,
and proof-output controls. The original model, proof, slice-contract/source mapping,
complete per-case logs and first preparation-only reconstruction failure are retained.
The actual container is a Rust Vec. `ARPushSequenceEquality` explicitly bridges its
push-then-drain behavior to the historical logical auxiliary sequence; it does not
call the Rust vector a deque. This is conditional local sequence/lookup refinement,
not whole-Rust/Raft, liveness, allocator, crash-durability or performance acceptance.

The ordinary three-voter WAL process run and independent audit record 351 operations:
stream 167 (151 OK, 16 unknown), unary 184 (170 OK, 14 unknown), totaling 321 OK and 30 unknown.
Unknown writes remain uncertain; safe refusal attempts before an uncertain terminal
outcome do not authorize replay after uncertainty. Both complete atomic histories,
leader-loss/restart intervals, six fresh voter drains, two client and five server
lifetimes, exact executable/source bindings and successful owned cleanup are retained.
These are one-host recovery observations, not Chaos or sustained-capacity evidence.

The original default-feature release records 596 source files and 11 first-observed
compiled units. Server SHA-256: `b21ba99a4dd2a55be5bfd771a22839b5f9930f3612d2aa7ac505c1c4ddd32465`;
workload: `4fcb5748571d7d172a7a730246a0e6d30227dfd6c27fd6c809b7c4143f346311`;
build manifest: `d7fdaaa6cb77eaafba64b22680acf967a01a96a7e4729c0868a2c11246e8c831`.
Original copied binaries remain local and are represented by their original hashes.
No native binaries, WAL/database data, perf recordings, full host process listings,
credentials or unrelated environment files are packaged. Formal caches and compiled
Java classes are omitted; original proof sources, logs and semantic inventories remain.

The candidate is held after a separate performance screen; this correctness selection
does not promote it. Packaging re-executes no tests, proofs, Cargo or runtime fixtures.

```sh
python3 docs/vector-receipt-correctness-v1/verify.py
```

The verifier is copied byte-for-byte from `docs/indexed-read-mixed-v1/verify.py`.
It checks archive/member byte integrity without extraction or correctness execution.
