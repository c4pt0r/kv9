# Raw apply experiment evidence

This packet supports [the report](../RAW-APPLY-ATTRIBUTION.md). It contains
standalone experiment source and metadata, not a production implementation.

- `attribution.rs` and `direct.rs`: exact measured Rust harnesses.
- `analysis.json`: independently recomputed sample statistics and counts.
- `analyze-original.py`: checker for retained local originals and build identities.
- `terminals.json`: completed extraction, build, test, run and analysis receipts.
- `evidence.tar.gz`: 80 original files plus their archive inventory.
- `members.json`, `package-result.json` and `verification.json`: complete member
  inventory, original/archive identities and successful bounded readback.
- `verify.py`: independent archive reader; no extraction or workload execution.

The archive includes plans, Cargo files, raw samples/counters, exact joins,
source/build identities and original logs from both experiments. Original
absolute paths identify the retained environment; they are not portable build
instructions. The larger engine/Raft inputs and executables are retained locally
and deliberately excluded. A new timing run requires the exact inputs and source
identified in the report, and is not needed to verify this metadata packet.

Verify the packet from this directory, choosing a writable output path:

```sh
python3 -B verify.py \
  --members-sha256 e86850c2c2db4eb1216cb394b15556679150272043d0f7af66081c59089d1254 \
  --archive-sha256 b636b7926a2f4e70c6c49d89c33d56853f8ee8a2faa927c9982c46ceb157dd37 \
  --output /path/to/readback.json
```

Actual publication receipts: package `91c86e/0`, readback `1d6d2d/0`.
The archive is 758,716 bytes with 81 members; its originals total 5,803,564
bytes. The reader verifies complete hashes, bounded member contents, gzip EOF
and tar termination. It does not independently rerun parsers, the benchmark,
source proofs or Chaos Mesh. Production and database performance are unchanged.
