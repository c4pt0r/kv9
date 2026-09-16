# Live ownership and iterative/interleaved reclamation extension

Eight new modules add 70 theorems (554 checked together), preserving all 47
previous modules and seven contracts. They prove exact reference changes, live
ownership through COW and release, root reachability, physical last-pop/child-append
teardown, and ownership/reclamation across finite interleaved worker histories.

Run the complete qualification locally with a fresh output directory:

```sh
python3 scripts/resident-radix/prove-live-ownership.py \
  /mnt/data/kv9-work/radix-live-ownership-proof-FRESH \
  /mnt/data/kv9-work/lean-toolchain-restoration-20260915-first/installation/lean-4.33.1-linux/bin/lean
```

The verifier binds source/compiler hashes, compiles fresh copies, inventories
every theorem's axioms, and rejects 23 changed models, one proof hole and one
custom axiom. Two changed Rust sources test hash binding only.

The [report](../../../docs/RADIX-LIVE-OWNERSHIP-PROOF.md) explains the model and
its limits. Native Arc linearization is a contract, and graph identities/payloads
do not prove physical allocator or buffer behavior. Complete mutations,
concurrent COW composition, storage representability and exact-source Rust/model
differential execution remain open. There is no timing or runtime promotion.
