# Composed lowering experiment evidence

See [the report](../RAW-LOWERING-COMPOSED.md) for results and scope. This is a
standalone experiment; production is unchanged.

- `composed.rs`: exact measured harness and two standalone tests.
- `analysis.json`, `analyze-original.py`: independent payload/state reconstruction
  and recomputation of all raw timing statistics.
- `terminals.json`: failed initial build, corrected successful build/tests,
  completed timing and independent analysis receipts.
- `evidence.tar.gz`: original plans, Cargo/source identities, both build logs,
  raw results, successful process terminal and checker inputs.
- `members.json`, `package-result.json`, `verification.json`, `verify.py`: complete
  inventory and successful bounded archive readback.

The packet contains 55 original metadata/source files plus the archive inventory:
56 members, 653,749 compressed bytes, 4,958,586 original bytes. Large group/Raft
inputs and the retained executable remain local. Source paths describe the
original environment; metadata verification does not rebuild or rerun it.

From this directory, choosing a fresh absolute output path:

```sh
python3 -B verify.py \
  --members-sha256 869c70837313b2adb70239ae463da0dc03e44fcd50df8fb4caa098eef2d2e8fa \
  --archive-sha256 a81324c6f7a2216489f86319c31b3c8ecdaed1132cc8c1d4cea683a393407310 \
  --output /path/to/readback.json
```

Actual publication receipts: package `3a4c5e/0`, readback `d9d168/0`.
Verification reads every member and checks hashes, gzip EOF and tar termination;
it does not supply a runtime proof, new Chaos acceptance or database QPS.
